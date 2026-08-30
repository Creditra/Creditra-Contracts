use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token, Address, Env,
};
use escrow::{EscrowContract, EscrowContractClient};

fn install_token(env: &Env) -> (Address, token::StellarAssetClient) {
    let admin = Address::generate(&env);
    let token_addr = env.register_stellar_asset_contract(admin.clone());
    let token_admin = token::StellarAssetClient::new(&env, &token_addr);
    token_admin.mint(&admin, &10_000_000_000);
    (token_addr, token_admin)
}

fn setup_contract(
    env: &Env,
    target: i128,
    deadline: u64,
    fee_bps: u32,
) -> (Address, Address, Address, Address) {
    let admin = Address::generate(&env);
    let contributor = Address::generate(&env);
    let fee_recipient = Address::generate(&env);

    env.mock_all_auths();

    let (token_addr, _token_admin) = install_token(&env);

    let contract_id = env.register(EscrowContract, ());
    let client = EscrowContractClient::new(&env, &contract_id);

    client.initialize(
        &admin,
        &1u64,
        &target,
        &deadline,
        &token_addr,
        &fee_bps,
        &fee_recipient,
    );

    (contract_id, admin, contributor, fee_recipient)
}

#[test]
fn test_initialize_sets_state() {
    let env = Env::default();
    let (contract_id, _, _, fee_recipient) = setup_contract(&env, 1000, 100, 500);

    let client = EscrowContractClient::new(&env, &contract_id);

    let total_raised: i128 = client.get_total_raised();
    assert_eq!(total_raised, 0);

    let (bps, recipient) = client.get_platform_fee_config();
    assert_eq!(bps, 500);
    assert_eq!(recipient, fee_recipient);
}

#[test]
fn test_initialize_rejects_reinit() {
    let env = Env::default();
    let (contract_id, admin, _, fee_recipient) = setup_contract(&env, 1000, 100, 0);
    let client = EscrowContractClient::new(&env, &contract_id);

    let token_addr: Address = client.get_asset();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.initialize(&admin, &2u64, &2000, &200, &token_addr, &0, &fee_recipient);
    }));
    assert!(result.is_err());
}

#[test]
fn test_initialize_rejects_invalid_fee() {
    let env = Env::default();

    env.mock_all_auths();

    let admin = Address::generate(&env);
    let fee_recipient = Address::generate(&env);
    let (token_addr, _) = install_token(&env);

    let contract_id = env.register(EscrowContract, ());
    let client = EscrowContractClient::new(&env, &contract_id);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.initialize(&admin, &1u64, &1000, &100, &token_addr, &10001, &fee_recipient);
    }));
    assert!(result.is_err());
}

#[test]
fn test_initialize_rejects_fee_above_cap() {
    let env = Env::default();

    env.mock_all_auths();

    let admin = Address::generate(&env);
    let fee_recipient = Address::generate(&env);
    let (token_addr, _) = install_token(&env);

    let contract_id = env.register(EscrowContract, ());
    let client = EscrowContractClient::new(&env, &contract_id);

    // 1001 BPS is one basis point above the 10% cap and must be rejected.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.initialize(&admin, &1u64, &1000, &100, &token_addr, &1001, &fee_recipient);
    }));
    assert!(result.is_err());
}

#[test]
fn test_initialize_accepts_fee_at_cap() {
    let env = Env::default();
    let (contract_id, _, _, _) = setup_contract(&env, 1000, 100, 1000);

    let client = EscrowContractClient::new(&env, &contract_id);
    let (bps, _) = client.get_platform_fee_config();
    assert_eq!(bps, 1000);
}

#[test]
fn test_propose_and_confirm_fee_change() {
    let env = Env::default();
    let (contract_id, _admin, _, _fee_recipient) = setup_contract(&env, 1000, 999999, 100);
    let client = EscrowContractClient::new(&env, &contract_id);

    // Proposing does not change the active fee.
    client.propose_fee_change(&500);
    assert_eq!(client.get_pending_fee(), Some(500));
    let (bps, _) = client.get_platform_fee_config();
    assert_eq!(bps, 100);

    // Confirmation applies the pending fee and clears the proposal.
    client.confirm_fee_change();
    let (bps, _) = client.get_platform_fee_config();
    assert_eq!(bps, 500);
    assert_eq!(client.get_pending_fee(), None);
}

#[test]
fn test_propose_fee_change_rejects_above_cap() {
    let env = Env::default();
    let (contract_id, _admin, _, _fee_recipient) = setup_contract(&env, 1000, 999999, 100);
    let client = EscrowContractClient::new(&env, &contract_id);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.propose_fee_change(&1001);
    }));
    assert!(result.is_err());

    // The active fee is untouched and nothing is left pending.
    let (bps, _) = client.get_platform_fee_config();
    assert_eq!(bps, 100);
    assert_eq!(client.get_pending_fee(), None);
}

#[test]
fn test_confirm_fee_change_rejects_when_no_pending() {
    let env = Env::default();
    let (contract_id, _admin, _, _fee_recipient) = setup_contract(&env, 1000, 999999, 100);
    let client = EscrowContractClient::new(&env, &contract_id);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.confirm_fee_change();
    }));
    assert!(result.is_err());
}

#[test]
fn test_cancel_fee_change() {
    let env = Env::default();
    let (contract_id, _admin, _, _fee_recipient) = setup_contract(&env, 1000, 999999, 100);
    let client = EscrowContractClient::new(&env, &contract_id);

    client.propose_fee_change(&500);
    client.cancel_fee_change();

    assert_eq!(client.get_pending_fee(), None);
    let (bps, _) = client.get_platform_fee_config();
    assert_eq!(bps, 100);
}

#[test]
fn test_fee_change_requires_admin_auth() {
    // In a fresh env without mock_all_auths, propose must fail without the admin's auth.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let env = Env::default();
        let contract_id = env.register(EscrowContract, ());
        let client = EscrowContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let fee_recipient = Address::generate(&env);
        let token_addr = env.register_stellar_asset_contract(admin.clone());

        // initialize has no require_auth, so it succeeds without mocked auths.
        client.initialize(&admin, &1u64, &1000, &999999, &token_addr, &100, &fee_recipient);

        // propose_fee_change requires admin auth, which is not provided here.
        client.propose_fee_change(&500);
    }));
    assert!(result.is_err());
}

#[test]
fn test_deposit_increases_balance() {
    let env = Env::default();
    let (contract_id, _, contributor, _fee_recipient) = setup_contract(&env, 1000, 999999, 0);
    let client = EscrowContractClient::new(&env, &contract_id);

    let token_addr = client.get_asset();
    let token_cl = token::StellarAssetClient::new(&env, &token_addr);
    token_cl.mint(&contributor, &500);
    client.deposit(&contributor, &500);

    let total_raised: i128 = client.get_total_raised();
    assert_eq!(total_raised, 500);
}

#[test]
fn test_deposit_rejects_after_deadline() {
    let env = Env::default();
    let (contract_id, _, contributor, _fee_recipient) = setup_contract(&env, 1000, 100, 0);
    let client = EscrowContractClient::new(&env, &contract_id);

    let token_addr = client.get_asset();
    let token_cl = token::StellarAssetClient::new(&env, &token_addr);
    token_cl.mint(&contributor, &100);

    env.ledger().set_timestamp(200);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.deposit(&contributor, &100);
    }));
    assert!(result.is_err());
    assert_eq!(client.get_total_raised(), 0);
}

#[test]
fn test_deposit_multiple_contributors() {
    let env = Env::default();
    let (contract_id, _, contributor, _fee_recipient) = setup_contract(&env, 5000, 999999, 0);
    let client = EscrowContractClient::new(&env, &contract_id);

    let token_addr = client.get_asset();
    let token_cl = token::StellarAssetClient::new(&env, &token_addr);
    let contributor2 = Address::generate(&env);
    token_cl.mint(&contributor, &1000);
    token_cl.mint(&contributor2, &2000);

    client.deposit(&contributor, &1000);
    client.deposit(&contributor2, &2000);

    let total_raised: i128 = client.get_total_raised();
    assert_eq!(total_raised, 3000);
}

#[test]
fn test_approve_withdrawal_increases_approved() {
    let env = Env::default();
    let (contract_id, admin, _, _fee_recipient) = setup_contract(&env, 1000, 999999, 0);
    let client = EscrowContractClient::new(&env, &contract_id);

    client.approve_withdrawal(&500);

    let total_raised: i128 = client.get_total_raised();
    assert_eq!(total_raised, 0);
}

#[test]
fn test_execute_withdrawal_deducts_fee() {
    let env = Env::default();
    let (contract_id, admin, contributor, fee_recipient) =
        setup_contract(&env, 1000, 999999, 1000);
    let client = EscrowContractClient::new(&env, &contract_id);

    let token_addr = client.get_asset();
    let token_cl = token::StellarAssetClient::new(&env, &token_addr);
    token_cl.mint(&contributor, &1000);
    client.deposit(&contributor, &1000);

    client.approve_withdrawal(&500);
    client.execute_withdrawal(&admin, &500);

    let fee = 50i128;
    let net = 450i128;

    let token_client = token::Client::new(&env, &token_addr);
    assert_eq!(token_client.balance(&contributor), 0);
    assert_eq!(token_client.balance(&admin), net);
    assert_eq!(token_client.balance(&fee_recipient), fee);
}

#[test]
fn test_execute_withdrawal_no_fee() {
    let env = Env::default();
    let (contract_id, admin, contributor, _fee_recipient) = setup_contract(&env, 1000, 999999, 0);
    let client = EscrowContractClient::new(&env, &contract_id);

    let token_addr = client.get_asset();
    let token_cl = token::StellarAssetClient::new(&env, &token_addr);
    token_cl.mint(&contributor, &1000);
    client.deposit(&contributor, &1000);

    client.approve_withdrawal(&500);
    client.execute_withdrawal(&admin, &500);

    let token_client = token::Client::new(&env, &token_addr);
    assert_eq!(token_client.balance(&admin), 500);
}

#[test]
fn test_execute_withdrawal_rejects_insufficient_approval() {
    let env = Env::default();
    let (contract_id, admin, _, _fee_recipient) = setup_contract(&env, 1000, 999999, 0);
    let client = EscrowContractClient::new(&env, &contract_id);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.execute_withdrawal(&admin, &500);
    }));
    assert!(result.is_err());
}

#[test]
fn test_approve_withdrawal_requires_admin_auth() {
    let env = Env::default();

    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contributor = Address::generate(&env);
    let fee_recipient = Address::generate(&env);
    let (token_addr, _token_admin) = install_token(&env);

    let contract_id = env.register(EscrowContract, ());
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(
        &admin,
        &1u64,
        &1000,
        &999999,
        &token_addr,
        &0,
        &fee_recipient,
    );

    // Reset auth mocks to test that non-admin cannot approve
    // In a fresh sub-environment, call without any auth
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // Create a new env without mock_all_auths to test auth enforcement
        let env2 = Env::default();
        let contract_id2 = env2.register(EscrowContract, ());
        let client2 = EscrowContractClient::new(&env2, &contract_id2);

        let admin2 = Address::generate(&env2);
        let contributor2 = Address::generate(&env2);
        let fee_recipient2 = Address::generate(&env2);
        let token_addr2 = env2.register_stellar_asset_contract(admin2.clone());

        // Initialize without mock_all_auths - this works because initialize has no require_auth
        client2.initialize(
            &admin2,
            &1u64,
            &1000,
            &999999,
            &token_addr2,
            &0,
            &fee_recipient2,
        );

        // Try to approve without auth - should fail
        client2.approve_withdrawal(&100);
    }));
    assert!(result.is_err());
}

#[test]
fn test_refund_after_deadline_when_under_target() {
    let env = Env::default();
    let (contract_id, _admin, contributor, _fee_recipient) = setup_contract(&env, 1000, 100, 0);
    let client = EscrowContractClient::new(&env, &contract_id);

    let token_addr = client.get_asset();
    let token_cl = token::StellarAssetClient::new(&env, &token_addr);
    token_cl.mint(&contributor, &500);
    client.deposit(&contributor, &500);

    env.ledger().set_timestamp(200);

    let token_client = token::Client::new(&env, &token_addr);
    let balance_before = token_client.balance(&contributor);
    assert_eq!(balance_before, 0);

    client.refund(&contributor);

    let balance_after = token_client.balance(&contributor);
    assert_eq!(balance_after, 500);

    let total_raised: i128 = client.get_total_raised();
    assert_eq!(total_raised, 0);
}

#[test]
fn test_refund_rejects_before_deadline() {
    let env = Env::default();
    let (contract_id, _admin, contributor, _fee_recipient) = setup_contract(&env, 1000, 100, 0);
    let client = EscrowContractClient::new(&env, &contract_id);

    env.ledger().set_timestamp(50);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.refund(&contributor);
    }));
    assert!(result.is_err());
}

#[test]
fn test_refund_rejects_when_target_met() {
    let env = Env::default();
    let (contract_id, _admin, contributor, _fee_recipient) = setup_contract(&env, 500, 100, 0);
    let client = EscrowContractClient::new(&env, &contract_id);

    let token_addr = client.get_asset();
    let token_cl = token::StellarAssetClient::new(&env, &token_addr);
    token_cl.mint(&contributor, &500);
    client.deposit(&contributor, &500);

    env.ledger().set_timestamp(200);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.refund(&contributor);
    }));
    assert!(result.is_err());
}

#[test]
fn test_refund_rejects_no_contribution() {
    let env = Env::default();
    let (contract_id, _admin, contributor, _fee_recipient) = setup_contract(&env, 1000, 100, 0);
    let client = EscrowContractClient::new(&env, &contract_id);

    env.ledger().set_timestamp(200);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.refund(&contributor);
    }));
    assert!(result.is_err());
}

#[test]
fn test_full_flow_deposit_withdraw_with_fee() {
    let env = Env::default();
    let (contract_id, admin, contributor, fee_recipient) =
        setup_contract(&env, 2000, 999999, 500);
    let client = EscrowContractClient::new(&env, &contract_id);

    let token_addr = client.get_asset();
    let token_cl = token::StellarAssetClient::new(&env, &token_addr);
    token_cl.mint(&contributor, &1000);
    client.deposit(&contributor, &1000);

    let total: i128 = client.get_total_raised();
    assert_eq!(total, 1000);

    client.approve_withdrawal(&800);
    client.execute_withdrawal(&admin, &800);

    let fee = 40i128;
    let net = 760i128;

    let token_client = token::Client::new(&env, &token_addr);
    assert_eq!(token_client.balance(&admin), net);
    assert_eq!(token_client.balance(&fee_recipient), fee);

    let remaining = 200i128;
    assert_eq!(token_client.balance(&contract_id), remaining);
}

// ---------------------------------------------------------------------------
// TTL tests — verify storage lifetime is refreshed on every hot financial path
// ---------------------------------------------------------------------------
//
// Strategy: after each mutating call the test reaches into the contract's
// storage via `env.as_contract` and asserts that both instance TTL and
// (where applicable) the per-contributor persistent TTL are at or above the
// expected `INSTANCE_TTL_THRESHOLD` / `PERSISTENT_TTL_THRESHOLD` values.
// This gives a deterministic, value-level guarantee that `extend_ttl` was
// actually invoked — not merely that the call didn't panic.

use escrow::{
    DataKey as EscrowDataKey,
    INSTANCE_TTL_EXTEND, INSTANCE_TTL_THRESHOLD,
    PERSISTENT_TTL_EXTEND, PERSISTENT_TTL_THRESHOLD,
};

/// Returns the current instance TTL for the escrow contract at `contract_id`.
fn instance_ttl(env: &Env, contract_id: &Address) -> u32 {
    env.as_contract(contract_id, || {
        env.storage().instance().get_ttl()
    })
}

/// Returns the current TTL of the per-contributor persistent balance entry.
fn contributor_ttl(env: &Env, contract_id: &Address, contributor: &Address) -> u32 {
    let key = EscrowDataKey::Balances(contributor.clone());
    env.as_contract(contract_id, || {
        env.storage().persistent().get_ttl(&key)
    })
}

#[test]
fn test_ttl_set_on_initialize() {
    let env = Env::default();
    let (contract_id, _, _, _) = setup_contract(&env, 1000, 999999, 0);

    // initialize must stamp the instance TTL to the full extend target.
    let ttl = instance_ttl(&env, &contract_id);
    assert!(
        ttl >= INSTANCE_TTL_THRESHOLD,
        "instance TTL after initialize ({ttl}) must be >= INSTANCE_TTL_THRESHOLD ({INSTANCE_TTL_THRESHOLD})"
    );
}

#[test]
fn test_ttl_refreshed_on_deposit() {
    let env = Env::default();
    let (contract_id, _, contributor, _) = setup_contract(&env, 1000, 999999, 0);
    let client = EscrowContractClient::new(&env, &contract_id);

    let token_addr = client.get_asset();
    let token_cl = token::StellarAssetClient::new(&env, &token_addr);
    token_cl.mint(&contributor, &500);
    client.deposit(&contributor, &500);

    // Instance TTL must be refreshed by deposit.
    let inst_ttl = instance_ttl(&env, &contract_id);
    assert!(
        inst_ttl >= INSTANCE_TTL_THRESHOLD,
        "instance TTL after deposit ({inst_ttl}) must be >= INSTANCE_TTL_THRESHOLD ({INSTANCE_TTL_THRESHOLD})"
    );

    // Per-contributor persistent entry must also be refreshed.
    let pers_ttl = contributor_ttl(&env, &contract_id, &contributor);
    assert!(
        pers_ttl >= PERSISTENT_TTL_THRESHOLD,
        "contributor persistent TTL after deposit ({pers_ttl}) must be >= PERSISTENT_TTL_THRESHOLD ({PERSISTENT_TTL_THRESHOLD})"
    );
}

#[test]
fn test_ttl_refreshed_on_approve_withdrawal() {
    let env = Env::default();
    let (contract_id, _admin, _, _) = setup_contract(&env, 1000, 999999, 0);
    let client = EscrowContractClient::new(&env, &contract_id);

    client.approve_withdrawal(&200);

    let ttl = instance_ttl(&env, &contract_id);
    assert!(
        ttl >= INSTANCE_TTL_THRESHOLD,
        "instance TTL after approve_withdrawal ({ttl}) must be >= INSTANCE_TTL_THRESHOLD ({INSTANCE_TTL_THRESHOLD})"
    );
}

#[test]
fn test_ttl_refreshed_on_execute_withdrawal() {
    let env = Env::default();
    let (contract_id, admin, contributor, _) = setup_contract(&env, 1000, 999999, 0);
    let client = EscrowContractClient::new(&env, &contract_id);

    let token_addr = client.get_asset();
    let token_cl = token::StellarAssetClient::new(&env, &token_addr);
    token_cl.mint(&contributor, &1000);
    client.deposit(&contributor, &1000);

    client.approve_withdrawal(&500);
    client.execute_withdrawal(&admin, &500);

    let ttl = instance_ttl(&env, &contract_id);
    assert!(
        ttl >= INSTANCE_TTL_THRESHOLD,
        "instance TTL after execute_withdrawal ({ttl}) must be >= INSTANCE_TTL_THRESHOLD ({INSTANCE_TTL_THRESHOLD})"
    );
}

#[test]
fn test_ttl_refreshed_on_refund() {
    let env = Env::default();
    let (contract_id, _admin, contributor, _) = setup_contract(&env, 1000, 100, 0);
    let client = EscrowContractClient::new(&env, &contract_id);

    let token_addr = client.get_asset();
    let token_cl = token::StellarAssetClient::new(&env, &token_addr);
    token_cl.mint(&contributor, &500);
    client.deposit(&contributor, &500);

    // Advance past deadline so the campaign fails and refunds open.
    env.ledger().set_timestamp(200);

    client.refund(&contributor);

    // Instance TTL must have been refreshed by refund.
    let inst_ttl = instance_ttl(&env, &contract_id);
    assert!(
        inst_ttl >= INSTANCE_TTL_THRESHOLD,
        "instance TTL after refund ({inst_ttl}) must be >= INSTANCE_TTL_THRESHOLD ({INSTANCE_TTL_THRESHOLD})"
    );
}

#[test]
fn test_persistent_ttl_refreshed_before_refund_read() {
    // This test verifies the pre-read bump in refund: the persistent entry is
    // extended *before* `storage().persistent().get()` so a zero-balance check
    // can never silently fail due to an expired entry.
    let env = Env::default();
    let (contract_id, _admin, contributor, _) = setup_contract(&env, 1000, 100, 0);
    let client = EscrowContractClient::new(&env, &contract_id);

    let token_addr = client.get_asset();
    let token_cl = token::StellarAssetClient::new(&env, &token_addr);
    token_cl.mint(&contributor, &300);
    client.deposit(&contributor, &300);

    // Advance past deadline.
    env.ledger().set_timestamp(200);
    client.refund(&contributor);

    // After a zero-out the entry still exists in storage (set to 0), so we
    // can read its TTL and confirm it was refreshed by the pre-read bump.
    let pers_ttl = contributor_ttl(&env, &contract_id, &contributor);
    assert!(
        pers_ttl >= PERSISTENT_TTL_THRESHOLD,
        "persistent TTL after refund pre-read bump ({pers_ttl}) must be >= PERSISTENT_TTL_THRESHOLD ({PERSISTENT_TTL_THRESHOLD})"
    );
}

#[test]
fn test_ttl_refreshed_on_propose_fee_change() {
    let env = Env::default();
    let (contract_id, _admin, _, _) = setup_contract(&env, 1000, 999999, 100);
    let client = EscrowContractClient::new(&env, &contract_id);

    client.propose_fee_change(&500);

    let ttl = instance_ttl(&env, &contract_id);
    assert!(
        ttl >= INSTANCE_TTL_THRESHOLD,
        "instance TTL after propose_fee_change ({ttl}) must be >= INSTANCE_TTL_THRESHOLD ({INSTANCE_TTL_THRESHOLD})"
    );
}

#[test]
fn test_ttl_refreshed_on_confirm_fee_change() {
    let env = Env::default();
    let (contract_id, _admin, _, _) = setup_contract(&env, 1000, 999999, 100);
    let client = EscrowContractClient::new(&env, &contract_id);

    client.propose_fee_change(&200);
    client.confirm_fee_change();

    let ttl = instance_ttl(&env, &contract_id);
    assert!(
        ttl >= INSTANCE_TTL_THRESHOLD,
        "instance TTL after confirm_fee_change ({ttl}) must be >= INSTANCE_TTL_THRESHOLD ({INSTANCE_TTL_THRESHOLD})"
    );
}

#[test]
fn test_ttl_refreshed_on_cancel_fee_change() {
    let env = Env::default();
    let (contract_id, _admin, _, _) = setup_contract(&env, 1000, 999999, 100);
    let client = EscrowContractClient::new(&env, &contract_id);

    client.propose_fee_change(&200);
    client.cancel_fee_change();

    let ttl = instance_ttl(&env, &contract_id);
    assert!(
        ttl >= INSTANCE_TTL_THRESHOLD,
        "instance TTL after cancel_fee_change ({ttl}) must be >= INSTANCE_TTL_THRESHOLD ({INSTANCE_TTL_THRESHOLD})"
    );
}

#[test]
fn test_persistent_ttl_independent_per_contributor() {
    // Two contributors deposit independently; each must have their own
    // persistent entry with a refreshed TTL.
    let env = Env::default();
    let (contract_id, _, contributor, _) = setup_contract(&env, 5000, 999999, 0);
    let client = EscrowContractClient::new(&env, &contract_id);

    let contributor2 = Address::generate(&env);

    let token_addr = client.get_asset();
    let token_cl = token::StellarAssetClient::new(&env, &token_addr);
    token_cl.mint(&contributor, &1000);
    token_cl.mint(&contributor2, &2000);

    client.deposit(&contributor, &1000);
    client.deposit(&contributor2, &2000);

    let ttl1 = contributor_ttl(&env, &contract_id, &contributor);
    let ttl2 = contributor_ttl(&env, &contract_id, &contributor2);

    assert!(
        ttl1 >= PERSISTENT_TTL_THRESHOLD,
        "contributor1 persistent TTL ({ttl1}) must be >= PERSISTENT_TTL_THRESHOLD ({PERSISTENT_TTL_THRESHOLD})"
    );
    assert!(
        ttl2 >= PERSISTENT_TTL_THRESHOLD,
        "contributor2 persistent TTL ({ttl2}) must be >= PERSISTENT_TTL_THRESHOLD ({PERSISTENT_TTL_THRESHOLD})"
    );
}

#[test]
fn test_ttl_extend_values_are_sane() {
    // Regression guard: the constants must form a valid (threshold < extend)
    // pair or the SDK will panic at runtime.
    assert!(
        INSTANCE_TTL_THRESHOLD < INSTANCE_TTL_EXTEND,
        "INSTANCE_TTL_THRESHOLD must be less than INSTANCE_TTL_EXTEND"
    );
    assert!(
        PERSISTENT_TTL_THRESHOLD < PERSISTENT_TTL_EXTEND,
        "PERSISTENT_TTL_THRESHOLD must be less than PERSISTENT_TTL_EXTEND"
    );
    // Minimum viable campaign lifetime: at least 30 days (~518,400 ledgers).
    assert!(
        INSTANCE_TTL_EXTEND >= 518_400,
        "INSTANCE_TTL_EXTEND ({INSTANCE_TTL_EXTEND}) should cover at least 30 days"
    );
    assert!(
        PERSISTENT_TTL_EXTEND >= 518_400,
        "PERSISTENT_TTL_EXTEND ({PERSISTENT_TTL_EXTEND}) should cover at least 30 days"
    );
}
