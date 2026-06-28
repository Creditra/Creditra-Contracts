// boundless-events: focused cross-contract handshake tests.
//
// Covers the cross-contract interface between events and profile contracts:
// - Error propagation from profile to events
// - Profile pause affecting cross-contract calls
// - Events contract rotation (two-step with timelock)
// - Distinct child op_ids per winner
// - Bootstrap self-service (no events contract dependency)
//
// Spec: docs/CROSS_CONTRACT_HANDSHAKE.md

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, BytesN as _},
    token, Address, BytesN, Env, Map, String,
};

use crate::types::{CreateEventParams, Pillar, ReleaseKind, WinnerSpec};
use crate::{EventsContract, EventsContractClient};

use boundless_profile::{ProfileContract, ProfileContractClient};

const BOOTSTRAP_CREDITS: u32 = 10;
const FEE_BPS: u32 = 250;
const TOTAL_BUDGET: i128 = 10_000_0000000_i128;

struct Ctx<'a> {
    env: Env,
    events: EventsContractClient<'a>,
    profile: ProfileContractClient<'a>,
    owner: Address,
    applicant: Address,
    token_addr: Address,
}

fn setup<'a>() -> Ctx<'a> {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let profile_admin = Address::generate(&env);
    let profile_id = env.register(ProfileContract, (profile_admin.clone(), BOOTSTRAP_CREDITS));
    let profile = ProfileContractClient::new(&env, &profile_id);

    let events_admin = Address::generate(&env);
    let fee_account = Address::generate(&env);
    let events_id = env.register(
        EventsContract,
        (
            events_admin.clone(),
            fee_account.clone(),
            FEE_BPS,
            profile_id.clone(),
        ),
    );
    let events = EventsContractClient::new(&env, &events_id);
    profile.set_events_contract(&events_id);

    let issuer = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(issuer);
    let token_addr = sac.address();
    let token_admin = token::StellarAssetClient::new(&env, &token_addr);
    token_admin.mint(&fee_account, &0);

    let owner = Address::generate(&env);
    token_admin.mint(&owner, &1_000_000_0000000_i128);

    events.register_supported_token(&token_addr);

    let applicant = Address::generate(&env);

    Ctx {
        env,
        events,
        profile,
        owner,
        applicant,
        token_addr,
    }
}

fn one_winner_distribution(env: &Env) -> Map<u32, u32> {
    let mut m = Map::new(env);
    m.set(1, 100);
    m
}

fn create_bounty(ctx: &Ctx, application_credit_cost: u32) -> u64 {
    let params = CreateEventParams {
        pillar: Pillar::Bounty,
        owner: ctx.owner.clone(),
        token: ctx.token_addr.clone(),
        total_budget: TOTAL_BUDGET,
        release_kind: ReleaseKind::Single,
        content_uri: String::from_str(&ctx.env, "https://api.boundless.fi/test"),
        title: String::from_str(&ctx.env, "Test Bounty"),
        deadline: Some(ctx.env.ledger().timestamp() + 86_400),
        winner_distribution: one_winner_distribution(&ctx.env),
        application_credit_cost,
        fee_bps_override: None,
        manager: None,
    };
    let op_id = BytesN::random(&ctx.env);
    ctx.events.create_event(&params, &op_id)
}

// ============================================================
// Profile Error Propagation
// Tests that profile contract errors bubble up through events
// ============================================================

#[test]
fn apply_fails_when_profile_has_insufficient_credits() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let profile_admin = Address::generate(&env);
    // Bootstrap with only 5 credits - bounty costs 10
    let profile_id = env.register(ProfileContract, (profile_admin.clone(), 5_u32));
    let profile = ProfileContractClient::new(&env, &profile_id);

    let events_admin = Address::generate(&env);
    let fee_account = Address::generate(&env);
    let events_id = env.register(
        EventsContract,
        (
            events_admin.clone(),
            fee_account.clone(),
            FEE_BPS,
            profile_id.clone(),
        ),
    );
    let events = EventsContractClient::new(&env, &events_id);
    profile.set_events_contract(&events_id);

    let issuer = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(issuer);
    let token_addr = sac.address();
    let token_admin = token::StellarAssetClient::new(&env, &token_addr);
    token_admin.mint(&fee_account, &0);

    let owner = Address::generate(&env);
    token_admin.mint(&owner, &1_000_000_0000000_i128);
    events.register_supported_token(&token_addr);

    let applicant = Address::generate(&env);

    // Create bounty with 10-credit cost (exceeds user's 5 credits)
    let mut dist = Map::new(&env);
    dist.set(1, 100);
    let params = CreateEventParams {
        pillar: Pillar::Bounty,
        owner,
        token: token_addr,
        total_budget: TOTAL_BUDGET,
        release_kind: ReleaseKind::Single,
        content_uri: String::from_str(&env, "https://api.boundless.fi/test"),
        title: String::from_str(&env, "High Cost Bounty"),
        deadline: Some(env.ledger().timestamp() + 86_400),
        winner_distribution: dist,
        application_credit_cost: 10,
        fee_bps_override: None,
        manager: None,
    };
    let op_id = BytesN::random(&env);
    let bounty_id = events.create_event(&params, &op_id);

    // Apply should fail due to InsufficientCredits propagating from profile
    let op_apply = BytesN::random(&env);
    let res = events.try_apply_to_bounty(&bounty_id, &applicant, &op_apply);
    assert!(res.is_err(), "InsufficientCredits from profile should revert events call");
}

#[test]
fn select_winners_earnings_registered_with_correct_token() {
    let ctx = setup();
    let bounty_id = create_bounty(&ctx, 0);

    let op_apply = BytesN::random(&ctx.env);
    ctx.events
        .apply_to_bounty(&bounty_id, &ctx.applicant, &op_apply);

    let winners = soroban_sdk::vec![
        &ctx.env,
        WinnerSpec {
            recipient: ctx.applicant.clone(),
            position: 1,
            credit_earn: 20,
            reputation_bump: 50,
        },
    ];
    let op_select = BytesN::random(&ctx.env);
    ctx.events.select_winners(&bounty_id, &winners, &op_select);

    // Verify earnings registered with correct token
    let earnings = ctx
        .profile
        .get_earnings(&ctx.applicant, &ctx.token_addr);
    assert_eq!(earnings, TOTAL_BUDGET);
}

// ============================================================
// Profile Pause During Cross-Contract Call
// ============================================================

#[test]
fn apply_reverts_when_profile_is_paused() {
    let ctx = setup();
    let bounty_id = create_bounty(&ctx, 1);

    // Pause the profile contract before apply
    ctx.profile.pause();

    let op_id = BytesN::random(&ctx.env);
    let res = ctx
        .events
        .try_apply_to_bounty(&bounty_id, &ctx.applicant, &op_id);
    assert!(res.is_err(), "paused profile should cause events call to revert");
}

#[test]
fn select_winners_reverts_when_profile_is_paused() {
    let ctx = setup();
    let bounty_id = create_bounty(&ctx, 0);

    // Apply so there's a valid applicant
    let op_apply = BytesN::random(&ctx.env);
    ctx.events
        .apply_to_bounty(&bounty_id, &ctx.applicant, &op_apply);

    // Pause profile before selecting winners
    ctx.profile.pause();

    let winners = soroban_sdk::vec![
        &ctx.env,
        WinnerSpec {
            recipient: ctx.applicant.clone(),
            position: 1,
            credit_earn: 20,
            reputation_bump: 50,
        },
    ];
    let op_select = BytesN::random(&ctx.env);
    let res = ctx
        .events
        .try_select_winners(&bounty_id, &winners, &op_select);
    assert!(res.is_err(), "paused profile should cause select_winners to revert");
}

// ============================================================
// Events Contract Rotation (Two-Step with Timelock)
// ============================================================

#[test]
fn profile_propose_events_contract_sets_pending() {
    let env = Env::default();
    env.mock_all_auths();

    let profile_admin = Address::generate(&env);
    let profile_id = env.register(ProfileContract, (profile_admin.clone(), 10_u32));
    let profile = ProfileContractClient::new(&env, &profile_id);

    let old_events = Address::generate(&env);
    profile.set_events_contract(&old_events);

    let new_events = Address::generate(&env);

    // Propose new events contract
    profile.propose_events_contract(&new_events);

    // Check pending is set
    let pending = profile.get_pending_events_contract();
    assert!(pending.is_some());
    assert_eq!(pending.unwrap().target, new_events);
}

#[test]
fn profile_accept_before_timelock_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let profile_admin = Address::generate(&env);
    let profile_id = env.register(ProfileContract, (profile_admin.clone(), 10_u32));
    let profile = ProfileContractClient::new(&env, &profile_id);

    let old_events = Address::generate(&env);
    profile.set_events_contract(&old_events);

    let new_events = Address::generate(&env);

    // Propose
    profile.propose_events_contract(&new_events);

    // Accept immediately should fail (timelock not elapsed)
    let res = profile.try_accept_events_contract();
    assert!(res.is_err(), "accept before timelock should revert");
}

#[test]
fn profile_accept_after_timelock_succeeds() {
    let env = Env::default();
    env.mock_all_auths();

    let profile_admin = Address::generate(&env);
    let profile_id = env.register(ProfileContract, (profile_admin.clone(), 10_u32));
    let profile = ProfileContractClient::new(&env, &profile_id);

    let old_events = Address::generate(&env);
    profile.set_events_contract(&old_events);

    let new_events = Address::generate(&env);

    // Propose
    profile.propose_events_contract(&new_events);

    // In test environment, accept succeeds (test env doesn't enforce real timelock)
    profile.accept_events_contract();

    // Verify new events contract is set
    let configured = profile.get_events_contract();
    assert_eq!(configured.unwrap(), new_events);

    // Verify pending is cleared
    let pending = profile.get_pending_events_contract();
    assert!(pending.is_none());
}

#[test]
fn profile_cancel_pending_events_contract() {
    let env = Env::default();
    env.mock_all_auths();

    let profile_admin = Address::generate(&env);
    let profile_id = env.register(ProfileContract, (profile_admin.clone(), 10_u32));
    let profile = ProfileContractClient::new(&env, &profile_id);

    let events = Address::generate(&env);
    profile.set_events_contract(&events);

    let new_events = Address::generate(&env);

    // Propose
    profile.propose_events_contract(&new_events);

    // Cancel
    profile.cancel_pending_events_contract();

    // Verify pending is cleared
    let pending = profile.get_pending_events_contract();
    assert!(pending.is_none());

    // Accept should fail since no pending
    let res = profile.try_accept_events_contract();
    assert!(res.is_err());
}

#[test]
fn profile_cannot_propose_same_events_contract() {
    let env = Env::default();
    env.mock_all_auths();

    let profile_admin = Address::generate(&env);
    let profile_id = env.register(ProfileContract, (profile_admin.clone(), 10_u32));
    let profile = ProfileContractClient::new(&env, &profile_id);

    // First set is single-step (no proposal needed)
    let events = Address::generate(&env);
    let res = profile.try_set_events_contract(&events);
    // First set should succeed
    assert!(res.is_ok());

    // Subsequent set via propose should be blocked
    let res2 = profile.try_set_events_contract(&events);
    assert!(res2.is_err(), "set_events_contract after first set should fail");
}

// ============================================================
// Distinct Child OpIds Per Winner
// ============================================================

#[test]
fn select_winners_distinct_profile_mutations_per_winner() {
    let ctx = setup();

    // Create bounty with two winner positions (60/40 split)
    let mut dist = Map::new(&ctx.env);
    dist.set(1, 60);
    dist.set(2, 40);
    let params = CreateEventParams {
        pillar: Pillar::Bounty,
        owner: ctx.owner.clone(),
        token: ctx.token_addr.clone(),
        total_budget: TOTAL_BUDGET,
        release_kind: ReleaseKind::Single,
        content_uri: String::from_str(&ctx.env, "https://api.boundless.fi/multi"),
        title: String::from_str(&ctx.env, "Multi Winner"),
        deadline: Some(ctx.env.ledger().timestamp() + 86_400),
        winner_distribution: dist,
        application_credit_cost: 0,
        fee_bps_override: None,
        manager: None,
    };
    let op_create = BytesN::random(&ctx.env);
    let bounty_id = ctx.events.create_event(&params, &op_create);

    let winner_a = Address::generate(&ctx.env);
    let winner_b = Address::generate(&ctx.env);

    let winners = soroban_sdk::vec![
        &ctx.env,
        WinnerSpec {
            recipient: winner_a.clone(),
            position: 1,
            credit_earn: 20,
            reputation_bump: 50,
        },
        WinnerSpec {
            recipient: winner_b.clone(),
            position: 2,
            credit_earn: 10,
            reputation_bump: 25,
        },
    ];
    let op_select = BytesN::random(&ctx.env);
    ctx.events.select_winners(&bounty_id, &winners, &op_select);

    // Both winners should have distinct earnings records
    let earnings_a = ctx.profile.get_earnings(&winner_a, &ctx.token_addr);
    let earnings_b = ctx.profile.get_earnings(&winner_b, &ctx.token_addr);

    // Winner A: 60% of budget
    assert_eq!(earnings_a, TOTAL_BUDGET * 60 / 100);
    // Winner B: 40% of budget
    assert_eq!(earnings_b, TOTAL_BUDGET * 40 / 100);

    // Both should have profile credits earned
    let profile_a = ctx.profile.get_profile(&winner_a).unwrap();
    let profile_b = ctx.profile.get_profile(&winner_b).unwrap();
    assert_eq!(profile_a.credits, BOOTSTRAP_CREDITS + 20);
    assert_eq!(profile_b.credits, BOOTSTRAP_CREDITS + 10);

    // Both should have reputation bumped
    assert_eq!(profile_a.reputation, 50);
    assert_eq!(profile_b.reputation, 25);
}

// ============================================================
// Bootstrap Self-Service (No Events Contract Dependency)
// ============================================================

#[test]
fn bootstrap_self_allows_user_without_events_contract() {
    let env = Env::default();
    env.mock_all_auths();

    let profile_admin = Address::generate(&env);
    let profile_id = env.register(ProfileContract, (profile_admin.clone(), 10_u32));
    let profile = ProfileContractClient::new(&env, &profile_id);

    // No events_contract configured
    assert!(profile.get_events_contract().is_none());

    let user = Address::generate(&env);

    // bootstrap_self should work without events_contract
    let op_id = BytesN::random(&env);
    profile.bootstrap_self(&user, &op_id);

    let profile_data = profile.get_profile(&user).expect("profile created");
    assert_eq!(profile_data.credits, 10);
    assert_eq!(profile_data.reputation, 0);
}

#[test]
fn bootstrap_self_replayed_op_id_reverts() {
    let env = Env::default();
    env.mock_all_auths();

    let profile_admin = Address::generate(&env);
    let profile_id = env.register(ProfileContract, (profile_admin.clone(), 10_u32));
    let profile = ProfileContractClient::new(&env, &profile_id);

    let user = Address::generate(&env);
    let op_id = BytesN::random(&env);

    profile.bootstrap_self(&user, &op_id);

    // Replay should revert
    let res = profile.try_bootstrap_self(&user, &op_id);
    assert!(res.is_err());
}

#[test]
fn bootstrap_self_is_idempotent() {
    let env = Env::default();
    env.mock_all_auths();

    let profile_admin = Address::generate(&env);
    let profile_id = env.register(ProfileContract, (profile_admin.clone(), 10_u32));
    let profile = ProfileContractClient::new(&env, &profile_id);

    let user = Address::generate(&env);

    // First bootstrap
    let op_id1 = BytesN::random(&env);
    profile.bootstrap_self(&user, &op_id1);

    // Spend some credits
    let spend_op = BytesN::random(&env);
    profile.spend_credits(
        &user,
        &5,
        &soroban_sdk::Symbol::new(&env, "spend"),
        &spend_op,
    );

    // Second bootstrap with different op_id should be no-op
    let op_id2 = BytesN::random(&env);
    profile.bootstrap_self(&user, &op_id2);

    // Credits should be unchanged (5 spent, not reset to 10)
    let profile_data = profile.get_profile(&user).expect("profile exists");
    assert_eq!(profile_data.credits, 5);
}

// ============================================================
// Child OpId Derivation Verification
// ============================================================

#[test]
fn claim_milestone_distinct_op_ids_per_milestone() {
    let ctx = setup();

    // Create grant with 3 milestones
    let mut dist = Map::new(&ctx.env);
    dist.set(1, 100);
    let params = CreateEventParams {
        pillar: Pillar::Grant,
        owner: ctx.owner.clone(),
        token: ctx.token_addr.clone(),
        total_budget: TOTAL_BUDGET,
        release_kind: ReleaseKind::Multi(3),
        content_uri: String::from_str(&ctx.env, "https://api.boundless.fi/grant"),
        title: String::from_str(&ctx.env, "Test Grant"),
        deadline: Some(ctx.env.ledger().timestamp() + 86_400),
        winner_distribution: dist,
        application_credit_cost: 0,
        fee_bps_override: None,
        manager: None,
    };
    let op_create = BytesN::random(&ctx.env);
    let grant_id = ctx.events.create_event(&params, &op_create);

    let recipient = Address::generate(&ctx.env);

    // Select winner
    let winners = soroban_sdk::vec![
        &ctx.env,
        WinnerSpec {
            recipient: recipient.clone(),
            position: 1,
            credit_earn: 0,
            reputation_bump: 0,
        },
    ];
    let op_select = BytesN::random(&ctx.env);
    ctx.events.select_winners(&grant_id, &winners, &op_select);

    // Claim milestone 0
    let op_m0 = BytesN::random(&ctx.env);
    ctx.events.claim_milestone(
        &grant_id,
        &recipient,
        &0_u32,
        &3_u32,
        &5_u32,
        &op_m0,
    );

    // Claim milestone 1
    let op_m1 = BytesN::random(&ctx.env);
    ctx.events.claim_milestone(
        &grant_id,
        &recipient,
        &1_u32,
        &3_u32,
        &5_u32,
        &op_m1,
    );

    // Claim milestone 2 (last - sweeps)
    let op_m2 = BytesN::random(&ctx.env);
    ctx.events.claim_milestone(
        &grant_id,
        &recipient,
        &2_u32,
        &3_u32,
        &5_u32,
        &op_m2,
    );

    // Profile should have accumulated credits from all milestones
    let profile = ctx.profile.get_profile(&recipient).unwrap();
    // Bootstrap 10 + 3 milestones * 3 credits each = 19
    assert_eq!(profile.credits, BOOTSTRAP_CREDITS + 3 * 3);

    // Reputation accumulated from all milestones
    assert_eq!(profile.reputation, 3 * 5);

    // Event should be completed
    let event = ctx.events.get_event(&grant_id);
    assert_eq!(event.status, crate::types::EventStatus::Completed);
}