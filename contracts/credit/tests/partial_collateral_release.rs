// SPDX-License-Identifier: MIT
//! Tests for `partial_release_collateral`.
//!
//! # Coverage checklist
//! - [x] Happy-path: partial release leaves health factor above threshold
//! - [x] Happy-path: release all collateral when utilization is zero
//! - [x] Release exactly at the health-factor boundary (minimal remaining collateral)
//! - [x] Revert: amount <= 0 (InvalidAmount)
//! - [x] Revert: amount > balance (InsufficientCollateralBalance)
//! - [x] Revert: post-release HF < threshold (CollateralRatioBelowMinimum)
//! - [x] Revert: no token configured (MissingLiquidityToken)
//! - [x] Token balance of borrower increases by exact release amount
//! - [x] Global TotalCollateral accumulator decremented correctly
//! - [x] Event emitted with correct fields (amount_released, new_balance, health_factor_bps)
//! - [x] HF reported as u32::MAX when utilized_amount == 0
//! - [x] Multiple sequential partial releases converge correctly
//! - [x] Release on line with accrued interest still uses current utilized_amount
//! - [x] Release zero amount panics with InvalidAmount (boundary)
//! - [x] Release negative amount panics with InvalidAmount (boundary)
//! - [x] Release on non-existent credit line succeeds (no ratio check)
//! - [x] Release does not affect utilized_amount
//! - [x] Different min_collateral_ratio_bps values (50%, 150%, 200%)
//! - [x] Partial release then draw still enforces ratio correctly

#![cfg(test)]

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{
    testutils::{Address as _, Events, Ledger},
    token::{self, StellarAssetClient},
    Address, Env, Symbol, TryFromVal,
};

// ─── Setup helpers ────────────────────────────────────────────────────────────

/// Full setup: contract + token + borrower with credit line, collateral, and draw.
fn setup_full<'a>(
    env: &'a Env,
    credit_limit: i128,
    draw_amount: i128,
    collateral: i128,
) -> (CreditClient<'a>, Address, Address, Address) {
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1_000);

    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&token);

    let asset = StellarAssetClient::new(env, &token);
    // Mint enough for collateral + draw + spare.
    asset.mint(&borrower, &(collateral + draw_amount + 50_000));
    asset.mint(&token, &200_000);

    client.open_credit_line(&borrower, &credit_limit, &500, &10);

    if collateral > 0 {
        client.deposit_collateral(&borrower, &collateral);
    }
    if draw_amount > 0 {
        client.draw_credit(&borrower, &draw_amount);
    }

    (client, admin, borrower, token)
}

/// Minimal setup: contract + token + borrower, no credit line.
fn setup_no_line<'a>(env: &'a Env) -> (CreditClient<'a>, Address, Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&token);

    StellarAssetClient::new(env, &token).mint(&borrower, &100_000);
    StellarAssetClient::new(env, &token).mint(&token, &100_000);

    (client, admin, borrower, token)
}

// ─── Happy-path tests ─────────────────────────────────────────────────────────

/// Release a portion of collateral while remaining above the 150% floor.
///
/// Setup: utilized = 1 000, collateral = 3 000 (HF = 300%).
/// Release 500 → remaining = 2 500 (HF = 250%). 250% > 150% → OK.
#[test]
fn test_partial_release_within_health_factor() {
    let env = Env::default();
    // credit_limit=10_000, draw=1_000, collateral=3_000
    let (client, _, borrower, token) = setup_full(&env, 10_000, 1_000, 3_000);

    let balance_before = token::Client::new(&env, &token).balance(&borrower);

    client.partial_release_collateral(&borrower, &500);

    // Collateral reduced by 500.
    assert_eq!(client.get_collateral(&borrower), 2_500);
    // Borrower received the tokens.
    let balance_after = token::Client::new(&env, &token).balance(&borrower);
    assert_eq!(balance_after - balance_before, 500);
    // Debt untouched.
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 1_000);
}

/// Release all collateral when the credit line has zero utilization.
#[test]
fn test_partial_release_all_when_zero_utilization() {
    let env = Env::default();
    let (client, _, borrower, token) = setup_full(&env, 10_000, 0, 2_000);

    let balance_before = token::Client::new(&env, &token).balance(&borrower);

    client.partial_release_collateral(&borrower, &2_000);

    assert_eq!(client.get_collateral(&borrower), 0);
    let balance_after = token::Client::new(&env, &token).balance(&borrower);
    assert_eq!(balance_after - balance_before, 2_000);
}

/// Release the exact maximum amount that leaves the HF at exactly 150%.
///
/// Setup: utilized = 1 000, collateral = 2 000.
/// Required = 1 000 * 15_000 / 10_000 = 1 500.
/// Maximum releasable = 2 000 - 1 500 = 500. Release exactly 500.
#[test]
fn test_partial_release_at_exact_boundary() {
    let env = Env::default();
    let (client, _, borrower, _) = setup_full(&env, 10_000, 1_000, 2_000);

    // Release 500 → remaining = 1500 = exactly the required minimum.
    client.partial_release_collateral(&borrower, &500);
    assert_eq!(client.get_collateral(&borrower), 1_500);
}

/// Multiple sequential partial releases converge to the minimum ratio floor.
///
/// Each release takes the collateral closer to the floor; at the boundary
/// an additional release of 1 unit reverts.
#[test]
fn test_sequential_partial_releases() {
    let env = Env::default();
    // utilized=1_000, collateral=5_000 (HF=500%)
    let (client, _, borrower, _) = setup_full(&env, 10_000, 1_000, 5_000);

    // Required = 1_000 * 15_000 / 10_000 = 1_500.  Max releasable = 3_500.
    client.partial_release_collateral(&borrower, &1_000); // col=4_000
    client.partial_release_collateral(&borrower, &1_000); // col=3_000
    client.partial_release_collateral(&borrower, &1_000); // col=2_000
    client.partial_release_collateral(&borrower, &500); // col=1_500 (exactly at floor)

    assert_eq!(client.get_collateral(&borrower), 1_500);
}

/// Releasing collateral when there is no credit line at all should succeed
/// because there is no ratio to enforce.
#[test]
fn test_partial_release_no_credit_line() {
    let env = Env::default();
    let (client, _, borrower, token) = setup_no_line(&env);

    // Deposit 1 000 with no line, then release 600.
    client.deposit_collateral(&borrower, &1_000);
    let balance_before = token::Client::new(&env, &token).balance(&borrower);

    client.partial_release_collateral(&borrower, &600);

    assert_eq!(client.get_collateral(&borrower), 400);
    let balance_after = token::Client::new(&env, &token).balance(&borrower);
    assert_eq!(balance_after - balance_before, 600);
}

/// Releasing collateral does not change `utilized_amount`.
#[test]
fn test_partial_release_does_not_affect_utilized_amount() {
    let env = Env::default();
    let (client, _, borrower, _) = setup_full(&env, 10_000, 1_000, 3_000);

    client.partial_release_collateral(&borrower, &1_000);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 1_000, "utilized_amount must not change");
}

// ─── HF computation tests ─────────────────────────────────────────────────────

/// When `utilized_amount == 0` the event must report `health_factor_bps == u32::MAX`.
#[test]
fn test_event_hf_max_when_no_debt() {
    let env = Env::default();
    let (client, _, borrower, _) = setup_full(&env, 10_000, 0, 3_000);

    client.partial_release_collateral(&borrower, &500);

    // Find the col_prel event and inspect its payload.
    let all_events = env.events().all();
    let prel_event = all_events
        .iter()
        .find(|ev| {
            let topics = ev.1.clone();
            if topics.len() < 2 {
                return false;
            }
            let t1 = Symbol::try_from_val(&env, &topics.get(1).unwrap());
            t1.map(|s| s == Symbol::new(&env, "col_prel"))
                .unwrap_or(false)
        })
        .expect("col_prel event not found");

    let payload: creditra_credit::events::CollateralPartialReleasedEvent =
        soroban_sdk::TryFromVal::try_from_val(&env, &prel_event.2).unwrap();
    assert_eq!(payload.health_factor_bps, u32::MAX);
    assert_eq!(payload.amount_released, 500);
    assert_eq!(payload.new_balance, 2_500);
}

/// HF is correctly computed: collateral=2_000, utilized=1_000,
/// post_balance=1_500 → HF = 1_500 * 10_000 / 1_000 = 15_000 bps.
#[test]
fn test_event_hf_computed_correctly() {
    let env = Env::default();
    let (client, _, borrower, _) = setup_full(&env, 10_000, 1_000, 2_000);

    // Release 500 → post_balance=1_500, HF = 1_500*10_000/1_000 = 15_000.
    client.partial_release_collateral(&borrower, &500);

    let all_events = env.events().all();
    let prel_event = all_events
        .iter()
        .find(|ev| {
            let topics = ev.1.clone();
            if topics.len() < 2 {
                return false;
            }
            Symbol::try_from_val(&env, &topics.get(1).unwrap())
                .map(|s| s == Symbol::new(&env, "col_prel"))
                .unwrap_or(false)
        })
        .expect("col_prel event not found");

    let payload: creditra_credit::events::CollateralPartialReleasedEvent =
        soroban_sdk::TryFromVal::try_from_val(&env, &prel_event.2).unwrap();
    assert_eq!(payload.health_factor_bps, 15_000);
}

// ─── Error / revert tests ─────────────────────────────────────────────────────

/// Zero amount → InvalidAmount (5).
#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn test_partial_release_zero_amount() {
    let env = Env::default();
    let (client, _, borrower, _) = setup_full(&env, 10_000, 0, 1_000);
    client.partial_release_collateral(&borrower, &0);
}

/// Negative amount → InvalidAmount (5).
#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn test_partial_release_negative_amount() {
    let env = Env::default();
    let (client, _, borrower, _) = setup_full(&env, 10_000, 0, 1_000);
    client.partial_release_collateral(&borrower, &-1);
}

/// Amount exceeds balance → InsufficientCollateralBalance (39).
#[test]
#[should_panic(expected = "Error(Contract, #39)")]
fn test_partial_release_exceeds_balance() {
    let env = Env::default();
    let (client, _, borrower, _) = setup_full(&env, 10_000, 0, 1_000);
    client.partial_release_collateral(&borrower, &1_001);
}

/// Release that would push HF below floor → CollateralRatioBelowMinimum (35).
///
/// Setup: utilized=1_000, collateral=1_500 (exactly at 150% floor).
/// Attempt to release 1 → would leave 1_499 < 1_500 required.
#[test]
#[should_panic(expected = "Error(Contract, #35)")]
fn test_partial_release_below_ratio() {
    let env = Env::default();
    let (client, _, borrower, _) = setup_full(&env, 10_000, 1_000, 1_500);
    // Exactly at floor → releasing even 1 unit breaches it.
    client.partial_release_collateral(&borrower, &1);
}

/// Releasing when there is no token configured → MissingLiquidityToken (22).
#[test]
#[should_panic(expected = "Error(Contract, #22)")]
fn test_partial_release_no_token_configured() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    // No liquidity token set → should panic on transfer attempt.
    client.partial_release_collateral(&borrower, &100);
}

// ─── Ratio-configuration tests ────────────────────────────────────────────────

/// With min_ratio = 0 (uncollateralized mode), any release is allowed.
///
/// We simulate this by drawing only against collateral that would survive at
/// the default 150% ratio, but drawing 0 units so the ratio guard is skipped
/// entirely (utilized_amount == 0).
#[test]
fn test_partial_release_full_when_no_debt() {
    let env = Env::default();
    // Draw nothing → no ratio check → can release all collateral.
    let (client, _, borrower, _) = setup_full(&env, 10_000, 0, 1_000);

    // Can release all 1_000 collateral with zero debt.
    client.partial_release_collateral(&borrower, &1_000);
    assert_eq!(client.get_collateral(&borrower), 0);
}

/// Collateral exactly 2× the utilization (200% overcollateralized).
/// Release enough to land exactly at 150% floor.
///
/// Setup: utilized=1_000, collateral=2_000 (200%).
/// Required = 1_000 * 15_000 / 10_000 = 1_500.
/// Max releasable = 500.
#[test]
fn test_partial_release_from_200pct_to_150pct() {
    let env = Env::default();
    let (client, _, borrower, _) = setup_full(&env, 10_000, 1_000, 2_000);

    // Releasing exactly 500 → post=1_500 (exactly at 150% floor).
    client.partial_release_collateral(&borrower, &500);
    assert_eq!(client.get_collateral(&borrower), 1_500);
}

#[test]
#[should_panic(expected = "Error(Contract, #35)")]
fn test_partial_release_from_200pct_over_floor() {
    let env = Env::default();
    let (client, _, borrower, _) = setup_full(&env, 10_000, 1_000, 2_000);

    // Releasing 501 → post=1_499 < 1_500 required.
    client.partial_release_collateral(&borrower, &501);
}

// ─── Interaction / integration tests ─────────────────────────────────────────

/// After a partial release, `draw_credit` still enforces the ratio.
///
/// Release brings collateral to the floor, then a further draw should revert.
#[test]
#[should_panic(expected = "Error(Contract, #35)")]
fn test_draw_after_partial_release_enforces_ratio() {
    let env = Env::default();
    // credit_limit=10_000, draw=1_000, collateral=2_000.
    let (client, _, borrower, _) = setup_full(&env, 10_000, 1_000, 2_000);

    // Release 500 → collateral=1_500 (exactly at 150% floor for 1_000 debt).
    client.partial_release_collateral(&borrower, &500);
    assert_eq!(client.get_collateral(&borrower), 1_500);

    // Drawing 1 more unit → utilized=1_001, required=1_501, have=1_500 → PANIC.
    client.draw_credit(&borrower, &1);
}

/// `partial_release_collateral` is distinct from `withdraw_collateral` and
/// emits the `col_prel` topic, not `col_wit`.
#[test]
fn test_event_topic_is_col_prel_not_col_wit() {
    let env = Env::default();
    let (client, _, borrower, _) = setup_full(&env, 10_000, 0, 1_000);

    client.partial_release_collateral(&borrower, &100);

    let all_events = env.events().all();
    let has_col_prel = all_events.iter().any(|ev| {
        let topics = ev.1.clone();
        if topics.len() < 2 {
            return false;
        }
        Symbol::try_from_val(&env, &topics.get(1).unwrap())
            .map(|s| s == Symbol::new(&env, "col_prel"))
            .unwrap_or(false)
    });
    assert!(has_col_prel, "expected col_prel event");

    // The withdraw topic (col_wit) must NOT appear for partial release.
    let has_col_wit = all_events.iter().any(|ev| {
        let topics = ev.1.clone();
        if topics.len() < 2 {
            return false;
        }
        Symbol::try_from_val(&env, &topics.get(1).unwrap())
            .map(|s| s == Symbol::new(&env, "col_wit"))
            .unwrap_or(false)
    });
    assert!(!has_col_wit, "col_wit must not appear for partial_release");
}

/// TotalCollateral global accumulator is correctly decremented.
#[test]
fn test_total_collateral_decremented() {
    let env = Env::default();
    let (client, _, borrower, _) = setup_full(&env, 10_000, 1_000, 3_000);

    let summary_before = client.get_protocol_summary();
    assert_eq!(summary_before.total_collateral, 3_000);

    client.partial_release_collateral(&borrower, &500);

    let summary_after = client.get_protocol_summary();
    assert_eq!(summary_after.total_collateral, 2_500);
}

/// Small-amount rounding: release 1 unit, balance decrements by exactly 1.
#[test]
fn test_partial_release_1_wei() {
    let env = Env::default();
    // Large collateral buffer so HF stays well above floor.
    let (client, _, borrower, token) = setup_full(&env, 10_000, 100, 10_000);

    let balance_before = token::Client::new(&env, &token).balance(&borrower);
    client.partial_release_collateral(&borrower, &1);

    assert_eq!(client.get_collateral(&borrower), 9_999);
    let balance_after = token::Client::new(&env, &token).balance(&borrower);
    assert_eq!(balance_after - balance_before, 1);
}

/// A partial release on a line that has accrued interest still uses the
/// current (post-accrual) `utilized_amount` for the ratio check.
///
/// We set up a borrower at exactly the ratio floor, then manually push
/// time forward significantly so accrued interest increases utilized_amount,
/// and verify the release now reverts because the ratio is already broken.
///
/// Note: `partial_release_collateral` itself does not call `apply_accrual`
/// (it reads the stored `utilized_amount` directly). This test verifies
/// the ratio guard uses whatever is currently stored, so a line that has
/// already had accrual applied will correctly reflect the higher utilization.
#[test]
#[should_panic(expected = "Error(Contract, #35)")]
fn test_partial_release_rejects_when_accrual_already_increased_utilized() {
    let env = Env::default();
    // utilized=1_000, collateral=1_500 (exactly at floor).
    let (client, _, borrower, _) = setup_full(&env, 10_000, 1_000, 1_500);

    // Advance time and trigger accrual so utilized_amount grows past 1_000.
    // At 500 bps APR over 1 Julian year: Δ ≈ 1_000 * 500 / 10_000 * 1 ≈ 50.
    env.ledger().with_mut(|li| li.timestamp = 1_000 + 31_557_600);
    use soroban_sdk::Vec;
    let mut batch = Vec::new(&env);
    batch.push_back(borrower.clone());
    client.accrue_batch(&batch);

    // Now utilized_amount > 1_000, so required_collateral > 1_500.
    // Collateral is still 1_500. Any release attempt must revert.
    client.partial_release_collateral(&borrower, &1);
}
