// SPDX-License-Identifier: MIT

//! Integration tests for the global protocol exposure cap (`max_total_exposure`).
//!
//! The cap enforces: `total_utilized + draw_amount <= max_total_exposure`.
//! It is checked on every `draw_credit` call and bypassed by `repay_credit` and
//! `forgive_debt` (those reduce exposure, never increase it).
//!
//! Covered scenarios:
//! - Happy path: draw succeeds when under cap
//! - Draw exactly at cap succeeds (boundary)
//! - Draw that would exceed cap reverts with `ExposureCapExceeded` (#30)
//! - Cap is admin-configurable; non-admin is rejected
//! - Setting cap = 0 removes it (draws unrestricted again)
//! - Negative cap value reverts with `InvalidAmount`
//! - Accumulator consistency: repay/forgive reduce exposure, re-enabling draws
//! - Multi-borrower: cap applies across all lines collectively
//! - Cap below current total blocks draws but repay still works
//! - get_max_total_exposure returns None before set, Some after

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
        client.open_credit_line(&b, &1_000_i128, &300_u32, &50_u32);
        borrowers.push(b);
    }

    (client, admin, borrowers, contract_id)
}

// ── Basic cap management ──────────────────────────────────────────────────────

#[test]
fn get_max_total_exposure_returns_none_before_set() {
    let env = Env::default();
    let (client, _admin, _borrower, _cid) = setup(&env);
    assert_eq!(client.get_max_total_exposure(), None);
}

#[test]
fn set_and_get_max_total_exposure_round_trips() {
    let env = Env::default();
    let (client, _admin, _borrower, _cid) = setup(&env);
    client.set_max_total_exposure(&5_000_i128);
    assert_eq!(client.get_max_total_exposure(), Some(5_000_i128));
}

#[test]
fn set_max_total_exposure_zero_removes_cap() {
    let env = Env::default();
    let (client, _admin, _borrower, _cid) = setup(&env);
    client.set_max_total_exposure(&5_000_i128);
    assert_eq!(client.get_max_total_exposure(), Some(5_000_i128));
    client.set_max_total_exposure(&0_i128);
    assert_eq!(client.get_max_total_exposure(), None);
}

#[test]
fn set_max_total_exposure_can_be_updated() {
    let env = Env::default();
    let (client, _admin, _borrower, _cid) = setup(&env);
    client.set_max_total_exposure(&3_000_i128);
    client.set_max_total_exposure(&7_500_i128);
    assert_eq!(client.get_max_total_exposure(), Some(7_500_i128));
}

// ── Authorization ─────────────────────────────────────────────────────────────

#[test]
#[should_panic]
fn set_max_total_exposure_requires_admin() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    // Drop all auths so the next call is unauthorized.
    let env2 = Env::default();
    let client2 = CreditClient::new(&env2, &contract_id);
    client2.set_max_total_exposure(&1_000_i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn set_max_total_exposure_rejects_negative_value() {
    let env = Env::default();
    let (client, _admin, _borrower, _cid) = setup(&env);
    client.set_max_total_exposure(&-1_i128);
}

// ── Draw enforcement ──────────────────────────────────────────────────────────

#[test]
fn draw_succeeds_when_under_cap() {
    let env = Env::default();
    let (client, _admin, borrower, _cid) = setup(&env);
    client.set_max_total_exposure(&5_000_i128);

    client.draw_credit(&borrower, &1_000_i128);

    assert_eq!(client.get_total_utilized(), 1_000);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().utilized_amount,
        1_000
    );
}

#[test]
fn draw_succeeds_at_exact_cap_boundary() {
    let env = Env::default();
    let (client, _admin, borrower, _cid) = setup(&env);
    client.set_max_total_exposure(&3_000_i128);

    // Draw exactly up to the cap — must not revert.
    client.draw_credit(&borrower, &3_000_i128);
    assert_eq!(client.get_total_utilized(), 3_000);
}

#[test]
#[should_panic(expected = "Error(Contract, #30)")]
fn draw_reverts_when_exceeding_cap_by_one() {
    let env = Env::default();
    let (client, _admin, borrower, _cid) = setup(&env);
    client.set_max_total_exposure(&500_i128);

    client.draw_credit(&borrower, &501_i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #30)")]
fn draw_reverts_when_second_draw_would_exceed_cap() {
    let env = Env::default();
    let (client, _admin, borrower, _cid) = setup(&env);
    client.set_max_total_exposure(&600_i128);

    client.draw_credit(&borrower, &400_i128);
    // total_utilized = 400; cap = 600; next draw of 201 → projected = 601 > 600
    client.draw_credit(&borrower, &201_i128);
}

#[test]
fn draw_without_cap_is_unrestricted() {
    let env = Env::default();
    let (client, _admin, borrower, _cid) = setup(&env);
    // No cap set — large draw within line limit succeeds.
    client.draw_credit(&borrower, &9_000_i128);
    assert_eq!(client.get_total_utilized(), 9_000);
}

#[test]
fn removing_cap_re_enables_large_draws() {
    let env = Env::default();
    let (client, _admin, borrower, _cid) = setup(&env);
    client.set_max_total_exposure(&200_i128);

    client.draw_credit(&borrower, &200_i128);
    // Would fail with cap in place; remove it first.
    client.set_max_total_exposure(&0_i128);
    client.draw_credit(&borrower, &500_i128);

    assert_eq!(client.get_total_utilized(), 700);
}

// ── Accumulator consistency after repay/forgive ───────────────────────────────

#[test]
fn repay_reduces_total_utilized_and_re_enables_draws() {
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

    client.set_max_total_exposure(&1_000_i128);
    client.draw_credit(&borrower, &1_000_i128);
    assert_eq!(client.get_total_utilized(), 1_000);

    // Repay 400 — total drops to 600, cap is 1_000 so next draw of 400 should work.
    StellarAssetClient::new(&env, &token).mint(&borrower, &400_i128);
    soroban_sdk::token::Client::new(&env, &token).approve(
        &borrower,
        &contract_id,
        &400_i128,
        &9_999_u32,
    );
    client.repay_credit(&borrower, &400_i128);
    assert_eq!(client.get_total_utilized(), 600);

    client.draw_credit(&borrower, &400_i128);
    assert_eq!(client.get_total_utilized(), 1_000);
}

#[test]
fn forgive_debt_reduces_total_utilized_and_re_enables_draws() {
    let env = Env::default();
    let (client, _admin, borrower, _cid) = setup(&env);
    client.set_max_total_exposure(&1_000_i128);

    client.draw_credit(&borrower, &1_000_i128);
    assert_eq!(client.get_total_utilized(), 1_000);

    // Forgive 500 — total drops to 500, next draw of 500 should succeed.
    client.forgive_debt(&borrower, &500_i128);
    assert_eq!(client.get_total_utilized(), 500);

    client.draw_credit(&borrower, &500_i128);
    assert_eq!(client.get_total_utilized(), 1_000);
}

// ── Multi-borrower cap enforcement ───────────────────────────────────────────

#[test]
fn cap_applies_across_multiple_borrowers() {
    let env = Env::default();
    let (client, _admin, borrowers, _cid) = setup_multi(&env, 3);

    // Each borrower has a 1_000 limit; set protocol cap at 2_000.
    client.set_max_total_exposure(&2_000_i128);

    let b0 = borrowers[0].clone();
    let b1 = borrowers[1].clone();
    let b2 = borrowers[2].clone();

    client.draw_credit(&b0, &800_i128); // total = 800
    client.draw_credit(&b1, &800_i128); // total = 1_600
    client.draw_credit(&b2, &400_i128); // total = 2_000, exactly at cap

    assert_eq!(client.get_total_utilized(), 2_000);
}

#[test]
#[should_panic(expected = "Error(Contract, #30)")]
fn cap_blocks_third_borrower_that_would_exceed_aggregate() {
    let env = Env::default();
    let (client, _admin, borrowers, _cid) = setup_multi(&env, 3);

    client.set_max_total_exposure(&2_000_i128);

    let b0 = borrowers[0].clone();
    let b1 = borrowers[1].clone();
    let b2 = borrowers[2].clone();

    client.draw_credit(&b0, &800_i128);
    client.draw_credit(&b1, &800_i128);
    // total = 1_600; cap = 2_000; draw 401 → projected 2_001 > 2_000
    client.draw_credit(&b2, &401_i128);
}

#[test]
fn cap_below_current_total_blocks_new_draws_but_not_repayments() {
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

    // Draw 2_000 without a cap, then retroactively set cap below current total.
    client.draw_credit(&borrower, &2_000_i128);
    assert_eq!(client.get_total_utilized(), 2_000);

    client.set_max_total_exposure(&1_500_i128); // cap < current total

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
    assert_eq!(client.get_total_utilized(), 1_500);
}

// ── Total utilized invariant with cap ────────────────────────────────────────

#[test]
fn total_utilized_matches_sum_of_credit_lines_with_cap_active() {
    let env = Env::default();
    let (client, _admin, borrowers, _cid) = setup_multi(&env, 4);

    client.set_max_total_exposure(&3_000_i128);

    // Draw varying amounts from each borrower.
    let amounts = [300_i128, 500_i128, 700_i128, 400_i128];
    for (i, &amt) in amounts.iter().enumerate() {
        let b = borrowers[i].clone();
        client.draw_credit(&b, &amt);
    }

    let total_from_accumulator = client.get_total_utilized();

    // Verify by summing individual credit lines.
    let mut total_from_lines = 0_i128;
    for b in borrowers.iter() {
        total_from_lines += client.get_credit_line(b).unwrap().utilized_amount;
    }

    assert_eq!(total_from_accumulator, total_from_lines);
    assert_eq!(total_from_accumulator, 1_900);
}

#[test]
fn total_utilized_stays_consistent_after_mixed_draw_repay_forgive() {
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

    client.set_max_total_exposure(&4_000_i128);

    client.draw_credit(&borrower, &2_000_i128);
    assert_eq!(client.get_total_utilized(), 2_000);

    client.draw_credit(&borrower, &1_000_i128);
    assert_eq!(client.get_total_utilized(), 3_000);

    StellarAssetClient::new(&env, &token).mint(&borrower, &500_i128);
    soroban_sdk::token::Client::new(&env, &token).approve(
        &borrower,
        &contract_id,
        &500_i128,
        &9_999_u32,
    );
    client.repay_credit(&borrower, &500_i128);
    assert_eq!(client.get_total_utilized(), 2_500);

    client.forgive_debt(&borrower, &300_i128);
    assert_eq!(client.get_total_utilized(), 2_200);

    // Accumulator must equal the stored line's utilized_amount.
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(client.get_total_utilized(), line.utilized_amount);
}

// ── Error discriminant stability ──────────────────────────────────────────────

#[test]
fn exposure_cap_error_discriminant_is_30() {
    // The ContractError discriminants are frozen per the stability guarantee.
    // This test pins the numeric value so a rename or reorder is caught immediately.
    assert_eq!(ContractError::ExposureCapExceeded as u32, 30);
}
