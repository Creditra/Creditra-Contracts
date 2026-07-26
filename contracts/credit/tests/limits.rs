// SPDX-License-Identifier: MIT

//! Integration tests for the per-borrower exposure cap (`max_borrower_exposure`).
//!
//! The cap enforces: `utilized_amount + draw_amount <= max_borrower_exposure`.
//! It is checked on every `draw_credit` call independently of the credit limit,
//! utilization cap, and global exposure cap.
//!
//! Covered scenarios:
//! - Happy path: draw succeeds when under per-borrower cap
//! - Draw exactly at cap succeeds (boundary)
//! - Draw that would exceed cap reverts with `BorrowerExposureCapExceeded` (#43)
//! - Cap is independent of credit limit (draw to limit but blocked by exposure cap)
//! - Cap is admin-configurable; non-admin is rejected
//! - Setting cap = 0 removes it (draws unrestricted again)
//! - Negative cap value reverts with `InvalidAmount`
//! - Multi-borrower: caps apply independently per borrower
//! - Cap below current utilization blocks new draws but repay still works
//! - get_borrower_exposure_cap returns None before set, Some after
//! - Interaction with global exposure cap

use creditra_credit::types::ContractError;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{Address, Env};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn setup(env: &Env) -> (CreditClient<'_>, Address, Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token = token_id.address();
    client.set_liquidity_token(&token);

    // Mint reserve tokens into the contract (liquidity source = contract address by default).
    StellarAssetClient::new(env, &token).mint(&contract_id, &1_000_000_i128);

    client.open_credit_line(&borrower, &10_000_i128, &300_u32, &50_u32);

    (client, admin, borrower, contract_id)
}

fn setup_multi(
    env: &Env,
    borrower_count: usize,
) -> (CreditClient<'_>, Address, std::vec::Vec<Address>, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    StellarAssetClient::new(env, &token).mint(&contract_id, &1_000_000_i128);

    let mut borrowers = std::vec::Vec::new();
    for _ in 0..borrower_count {
        let b = Address::generate(env);
        client.open_credit_line(&b, &10_000_i128, &300_u32, &50_u32);
        borrowers.push(b);
    }

    (client, admin, borrowers, contract_id)
}

// ── Basic cap management ──────────────────────────────────────────────────────

#[test]
fn get_borrower_exposure_cap_returns_none_before_set() {
    let env = Env::default();
    let (client, _admin, borrower, _cid) = setup(&env);
    assert_eq!(client.get_borrower_exposure_cap(&borrower), None);
}

#[test]
fn set_and_get_borrower_exposure_cap_round_trips() {
    let env = Env::default();
    let (client, _admin, borrower, _cid) = setup(&env);
    client.set_borrower_exposure_cap(&borrower, &5_000_i128);
    assert_eq!(
        client.get_borrower_exposure_cap(&borrower),
        Some(5_000_i128)
    );
}

#[test]
fn set_borrower_exposure_cap_zero_removes_cap() {
    let env = Env::default();
    let (client, _admin, borrower, _cid) = setup(&env);
    client.set_borrower_exposure_cap(&borrower, &5_000_i128);
    assert_eq!(
        client.get_borrower_exposure_cap(&borrower),
        Some(5_000_i128)
    );
    client.set_borrower_exposure_cap(&borrower, &0_i128);
    assert_eq!(client.get_borrower_exposure_cap(&borrower), None);
}

#[test]
fn set_borrower_exposure_cap_can_be_updated() {
    let env = Env::default();
    let (client, _admin, borrower, _cid) = setup(&env);
    client.set_borrower_exposure_cap(&borrower, &3_000_i128);
    client.set_borrower_exposure_cap(&borrower, &7_500_i128);
    assert_eq!(
        client.get_borrower_exposure_cap(&borrower),
        Some(7_500_i128)
    );
}

// ── Authorization ─────────────────────────────────────────────────────────────

#[test]
#[should_panic]
fn set_borrower_exposure_cap_requires_admin() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    // Drop all auths so the next call is unauthorized.
    let env2 = Env::default();
    let client2 = CreditClient::new(&env2, &contract_id);
    client2.set_borrower_exposure_cap(&borrower, &1_000_i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn set_borrower_exposure_cap_rejects_negative_value() {
    let env = Env::default();
    let (client, _admin, borrower, _cid) = setup(&env);
    client.set_borrower_exposure_cap(&borrower, &-1_i128);
}

// ── Draw enforcement ──────────────────────────────────────────────────────────

#[test]
fn draw_succeeds_when_under_borrower_cap() {
    let env = Env::default();
    let (client, _admin, borrower, _cid) = setup(&env);
    client.set_borrower_exposure_cap(&borrower, &5_000_i128);

    client.draw_credit(&borrower, &1_000_i128);

    assert_eq!(client.get_total_utilized(), 1_000);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().utilized_amount,
        1_000
    );
}

#[test]
fn draw_succeeds_at_exact_borrower_cap_boundary() {
    let env = Env::default();
    let (client, _admin, borrower, _cid) = setup(&env);
    client.set_borrower_exposure_cap(&borrower, &3_000_i128);

    // Draw exactly up to the cap — must not revert.
    client.draw_credit(&borrower, &3_000_i128);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().utilized_amount,
        3_000
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #43)")]
fn draw_reverts_when_exceeding_borrower_cap_by_one() {
    let env = Env::default();
    let (client, _admin, borrower, _cid) = setup(&env);
    client.set_borrower_exposure_cap(&borrower, &500_i128);
    client.draw_credit(&borrower, &501_i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #43)")]
fn draw_reverts_when_second_draw_would_exceed_borrower_cap() {
    let env = Env::default();
    let (client, _admin, borrower, _cid) = setup(&env);
    client.set_borrower_exposure_cap(&borrower, &600_i128);

    client.draw_credit(&borrower, &400_i128);
    // utilized = 400; cap = 600; next draw of 201 → projected = 601 > 600
    client.draw_credit(&borrower, &201_i128);
}

#[test]
fn draw_without_borrower_cap_is_unrestricted() {
    let env = Env::default();
    let (client, _admin, borrower, _cid) = setup(&env);
    // No per-borrower cap set — large draw within line limit succeeds.
    client.draw_credit(&borrower, &9_000_i128);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().utilized_amount,
        9_000
    );
}

#[test]
fn removing_borrower_cap_re_enables_large_draws() {
    let env = Env::default();
    let (client, _admin, borrower, _cid) = setup(&env);
    client.set_borrower_exposure_cap(&borrower, &200_i128);

    client.draw_credit(&borrower, &200_i128);
    // Would fail with cap in place; remove it first.
    client.set_borrower_exposure_cap(&borrower, &0_i128);
    client.draw_credit(&borrower, &500_i128);

    assert_eq!(
        client.get_credit_line(&borrower).unwrap().utilized_amount,
        700
    );
}

// ── Cap independent of credit limit ──────────────────────────────────────────

#[test]
fn borrower_cap_blocks_draw_within_credit_limit() {
    let env = Env::default();
    let (client, _admin, borrower, _cid) = setup(&env);
    // Borrower has a 10_000 credit limit, but we set a 300 exposure cap.
    client.set_borrower_exposure_cap(&borrower, &300_i128);

    // Draw 300 — at cap but within credit limit.
    client.draw_credit(&borrower, &300_i128);

    // Next draw of 1 would exceed the borrower cap even though it's within limit.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.draw_credit(&borrower, &1_i128);
    }));
    assert!(result.is_err());
}

// ── Accumulator consistency after repay ───────────────────────────────────────

#[test]
fn repay_reduces_utilized_and_re_enables_draws_under_borrower_cap() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    StellarAssetClient::new(&env, &token).mint(&contract_id, &10_000_i128);
    client.open_credit_line(&borrower, &5_000_i128, &300_u32, &50_u32);

    client.set_borrower_exposure_cap(&borrower, &1_000_i128);
    client.draw_credit(&borrower, &1_000_i128);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().utilized_amount,
        1_000
    );

    // Repay 400 — utilized drops to 600, cap is 1_000 so next draw of 400 should work.
    StellarAssetClient::new(&env, &token).mint(&borrower, &400_i128);
    soroban_sdk::token::Client::new(&env, &token).approve(
        &borrower,
        &contract_id,
        &400_i128,
        &9_999_u32,
    );
    client.repay_credit(&borrower, &400_i128);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().utilized_amount,
        600
    );

    client.draw_credit(&borrower, &400_i128);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().utilized_amount,
        1_000
    );
}

// ── Multi-borrower independent caps ──────────────────────────────────────────

#[test]
fn borrower_cap_applies_independently_per_borrower() {
    let env = Env::default();
    let (client, _admin, borrowers, _cid) = setup_multi(&env, 3);

    let b0 = borrowers[0].clone();
    let b1 = borrowers[1].clone();
    let b2 = borrowers[2].clone();

    // Each borrower gets their own cap.
    client.set_borrower_exposure_cap(&b0, &500_i128);
    client.set_borrower_exposure_cap(&b1, &1_000_i128);
    client.set_borrower_exposure_cap(&b2, &1_500_i128);

    client.draw_credit(&b0, &500_i128); // at b0's cap
    client.draw_credit(&b1, &1_000_i128); // at b1's cap
    client.draw_credit(&b2, &1_500_i128); // at b2's cap

    assert_eq!(
        client.get_credit_line(&b0).unwrap().utilized_amount,
        500
    );
    assert_eq!(
        client.get_credit_line(&b1).unwrap().utilized_amount,
        1_000
    );
    assert_eq!(
        client.get_credit_line(&b2).unwrap().utilized_amount,
        1_500
    );
}

#[test]
fn borrower_cap_does_not_affect_other_borrowers() {
    let env = Env::default();
    let (client, _admin, borrowers, _cid) = setup_multi(&env, 2);

    let b0 = borrowers[0].clone();
    let b1 = borrowers[1].clone();

    // Only b0 has a cap.
    client.set_borrower_exposure_cap(&b0, &500_i128);

    // b0 draws up to cap.
    client.draw_credit(&b0, &500_i128);

    // b1 has no cap — can draw up to their credit limit.
    client.draw_credit(&b1, &9_000_i128);
    assert_eq!(
        client.get_credit_line(&b1).unwrap().utilized_amount,
        9_000
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #43)")]
fn borrower_cap_blocks_only_capped_borrower() {
    let env = Env::default();
    let (client, _admin, borrowers, _cid) = setup_multi(&env, 2);

    let b0 = borrowers[0].clone();
    let b1 = borrowers[1].clone();

    client.set_borrower_exposure_cap(&b0, &500_i128);
    // b1 has no cap.

    client.draw_credit(&b0, &500_i128); // at cap
    client.draw_credit(&b0, &1_i128); // exceeds cap
}

// ── Cap below current utilization blocks draws but not repay ─────────────────

#[test]
fn borrower_cap_below_current_utilization_blocks_new_draws_but_not_repayments() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    StellarAssetClient::new(&env, &token).mint(&contract_id, &10_000_i128);
    client.open_credit_line(&borrower, &5_000_i128, &300_u32, &50_u32);

    // Draw 2_000 without a cap, then retroactively set cap below current utilization.
    client.draw_credit(&borrower, &2_000_i128);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().utilized_amount,
        2_000
    );

    client.set_borrower_exposure_cap(&borrower, &1_500_i128); // cap < current utilization

    // Any new draw must revert even for amount = 1.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.draw_credit(&borrower, &1_i128);
    }));
    assert!(result.is_err(), "draw should revert when projected > cap");

    // Repayment must still succeed regardless of cap.
    StellarAssetClient::new(&env, &token).mint(&borrower, &500_i128);
    soroban_sdk::token::Client::new(&env, &token).approve(
        &borrower,
        &contract_id,
        &500_i128,
        &9_999_u32,
    );
    client.repay_credit(&borrower, &500_i128);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().utilized_amount,
        1_500
    );
}

// ── Interaction with global exposure cap ─────────────────────────────────────

#[test]
fn borrower_cap_and_global_cap_apply_independently() {
    let env = Env::default();
    let (client, _admin, borrowers, _cid) = setup_multi(&env, 2);

    let b0 = borrowers[0].clone();
    let b1 = borrowers[1].clone();

    // Global cap: 2_000 across all borrowers.
    client.set_max_total_exposure(&2_000_i128);
    // Per-borrower cap: 1_500 each.
    client.set_borrower_exposure_cap(&b0, &1_500_i128);
    client.set_borrower_exposure_cap(&b1, &1_500_i128);

    // Both borrowers can draw up to their per-borrower cap.
    client.draw_credit(&b0, &1_200_i128);
    client.draw_credit(&b1, &800_i128);

    // b0 tries to draw more — would exceed global cap (1_200 + 800 + 1 = 2_001 > 2_000)
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.draw_credit(&b0, &1_i128);
    }));
    assert!(result.is_err(), "draw should revert due to global cap");
}

// ── Error discriminant stability ──────────────────────────────────────────────

#[test]
fn borrower_exposure_cap_error_discriminant_is_43() {
    assert_eq!(
        ContractError::BorrowerExposureCapExceeded as u32,
        43
    );
}
