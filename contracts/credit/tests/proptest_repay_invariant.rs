// SPDX-License-Identifier: MIT

//! Property test: repay_credit never increases utilization.
//!
//! # What
//!
//! Verifies the fundamental accounting invariant that a repayment must never
//! increase the borrower's utilized amount. This is the simplest and most
//! critical invariant of the credit protocol — a violation would allow a
//! borrower to "repay" a negative amount and inflate their debt.
//!
//! # Property
//!
//! For any valid setup (open line, draw some amount), and any positive
//! repayment amount:
//!
//! ```text
//! utilization_after_repay <= utilization_before
//! ```
//!
//! # Why
//!
//! This is the core safety property of the credit protocol. If a repayment
//! could increase utilization, the contract would be deflationary for the
//! borrower and the global TotalUtilized accumulator could become inconsistent.
//!
//! # References
//!
//! - [`crate::lib::repay_credit`]
//! - Issue #646

use creditra_credit::types::CreditLineData;
use creditra_credit::{Credit, CreditClient};
use proptest::prelude::*;
use proptest::test_runner::Config as ProptestConfig;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::token::Client as TokenClient;
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{Address, Env};

// ── Strategies ────────────────────────────────────────────────────────────

/// Strategy for draw amount: small but non-zero draws to avoid overflow.
fn draw_amount() -> impl Strategy<Value = i128> {
    1_i128..=50_000_i128
}

/// Strategy for repay amount: small but non-zero repayments.
fn repay_amount() -> impl Strategy<Value = i128> {
    1_i128..=50_000_i128
}

/// Strategy for credit limit: reasonable range.
fn credit_limit() -> impl Strategy<Value = i128> {
    100_i128..=100_000_i128
}

/// Strategy for ledger timestamp advancement (seconds after draw).
fn time_delta() -> impl Strategy<Value = u64> {
    1_u64..=31_536_000_u64 // up to 1 year of interest accrual
}

// ── Helpers ───────────────────────────────────────────────────────────────

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

    // Mint enough to cover draws
    StellarAssetClient::new(env, &token).mint(&contract_id, &1_000_000_i128);
    StellarAssetClient::new(env, &token).mint(&borrower, &1_000_000_i128);

    // Approve contract to pull repayments
    TokenClient::new(env, &token).approve(&borrower, &contract_id, &1_000_000_i128, &u32::MAX);

    (client, token, contract_id, borrower)
}

// ── Property test ─────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, .. ProptestConfig::default() })]

    /// Tests that utilization never increases after a repayment.
    ///
    /// Covers:
    /// - Repay amount less than utilized (partial repay)
    /// - Repay amount exactly equal to utilized (full repay)
    /// - Repay amount greater than utilized (overpayment, capped)
    /// - With and without interest accrual (time advancement)
    ///
    /// # Shrinking
    /// On failure, shrinks to the minimal setup and amounts that
    /// trigger the invariant violation.
    #[test]
    fn utilization_never_increases_on_repay(
        (cl, draw, repay, delta) in (
            credit_limit(),
            draw_amount(),
            repay_amount(),
            time_delta(),
        ).prop_filter("draw must be <= credit limit", |(cl, draw, _, _)| draw <= cl)
    ) {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);

        let (client, _token, _contract_id, borrower) = setup(&env);

        // Open credit line
        client.open_credit_line(&borrower, &cl, &500_u32, &50_u32);

        // Draw credit
        client.draw_credit(&borrower, &draw);

        let line_before: CreditLineData = client
            .get_credit_line(&borrower)
            .expect("credit line must exist after draw");

        let utilized_before = line_before.utilized_amount;

        // Advance time to accrue interest
        env.ledger().set_timestamp(1_000 + delta);

        // Repay
        client.repay_credit(&borrower, &repay);

        let line_after: CreditLineData = client
            .get_credit_line(&borrower)
            .expect("credit line must exist after repay");

        let utilized_after = line_after.utilized_amount;

        // The invariant: utilization must never increase after repayment
        prop_assert!(
            utilized_after <= utilized_before,
            "utilization increased after repay!\n\
             utilized_before={}, utilized_after={}, delta={}\n\
             setup: credit_limit={}, draw={}, repay={}",
            utilized_before, utilized_after,
            utilized_after - utilized_before,
            cl, draw, repay
        );
    }
}

// ── Edge case: no time advancement (no interest) ─────────────────────────

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, .. ProptestConfig::default() })]

    /// Tests that utilization decreases or stays same on repay without
    /// any time advancement (no interest accrual).
    #[test]
    fn utilization_never_increases_on_repay_no_interest(
        (cl, draw, repay) in (
            credit_limit(),
            draw_amount(),
            repay_amount(),
        ).prop_filter("draw must be <= credit limit", |(cl, draw, _)| draw <= cl)
    ) {
        let env = Env::default();

        let (client, _token, _contract_id, borrower) = setup(&env);

        client.open_credit_line(&borrower, &cl, &500_u32, &50_u32);
        client.draw_credit(&borrower, &draw);

        let line_before = client.get_credit_line(&borrower).unwrap();
        let utilized_before = line_before.utilized_amount;

        client.repay_credit(&borrower, &repay);

        let line_after = client.get_credit_line(&borrower).unwrap();
        let utilized_after = line_after.utilized_amount;

        prop_assert!(
            utilized_after <= utilized_before,
            "utilization increased without any interest!\n\
             utilized_before={}, utilized_after={}",
            utilized_before, utilized_after
        );
    }
}

// ── Deterministic edge case: overpayment ─────────────────────────────────

/// Verifies that overpayment (repay > utilized) is capped and does not
/// cause utilization to become negative or increase.
#[test]
fn overpayment_is_capped_and_never_increases_utilization() {
    let env = Env::default();

    let (client, _token, _contract_id, borrower) = setup(&env);

    client.open_credit_line(&borrower, &10_000, &500_u32, &50_u32);
    client.draw_credit(&borrower, &1_000);

    let line_before = client.get_credit_line(&borrower).unwrap();
    let utilized_before = line_before.utilized_amount;
    assert_eq!(utilized_before, 1_000);

    // Repay more than outstanding — should cap and not go negative
    client.repay_credit(&borrower, &10_000);

    let line_after = client.get_credit_line(&borrower).unwrap();
    let utilized_after = line_after.utilized_amount;

    assert_eq!(utilized_after, 0, "overpayment should zero out utilization");
    assert!(
        utilized_after <= utilized_before,
        "overpayment should never increase utilization"
    );
}

// ── Deterministic edge case: full repay with interest ────────────────────

/// Verifies that repaying after interest accrual still decreases utilization.
#[test]
fn full_repay_after_interest_decreases_utilization() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000);

    let (client, _token, _contract_id, borrower) = setup(&env);

    client.open_credit_line(&borrower, &100_000, &5_000_u32, &50_u32); // 50% APR
    client.draw_credit(&borrower, &10_000);

    let line_before = client.get_credit_line(&borrower).unwrap();
    let utilized_before = line_before.utilized_amount;
    assert_eq!(utilized_before, 10_000);

    // Advance by 1 year — interest will accrue
    env.ledger().set_timestamp(1_000 + 31_536_000);

    // Repay more than original principal to cover interest
    client.repay_credit(&borrower, &20_000);

    let line_after = client.get_credit_line(&borrower).unwrap();
    let utilized_after = line_after.utilized_amount;

    assert!(
        utilized_after <= utilized_before,
        "full repay after interest must decrease utilization"
    );
    // After full repay with sufficient amount, should be 0
    assert_eq!(utilized_after, 0);
}

// ── Deterministic edge case: repay to different borrower ─────────────────

/// Verifies that repaying borrower A's debt does not affect borrower B's utilization.
#[test]
fn repay_one_borrower_does_not_affect_another() {
    let env = Env::default();

    env.mock_all_auths();
    let admin = Address::generate(&env);
    let borrower_a = Address::generate(&env);
    let borrower_b = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    StellarAssetClient::new(&env, &token).mint(&contract_id, &1_000_000_i128);

    client.open_credit_line(&borrower_a, &10_000, &500_u32, &50_u32);
    client.open_credit_line(&borrower_b, &10_000, &500_u32, &50_u32);

    client.draw_credit(&borrower_a, &5_000);
    client.draw_credit(&borrower_b, &3_000);

    let b_before = client.get_credit_line(&borrower_b).unwrap().utilized_amount;

    // Repay borrower A
    StellarAssetClient::new(&env, &token).mint(&borrower_a, &5_000);
    TokenClient::new(&env, &token).approve(&borrower_a, &contract_id, &5_000, &u32::MAX);
    client.repay_credit(&borrower_a, &5_000);

    // Borrower B's utilization must be unchanged
    let b_after = client.get_credit_line(&borrower_b).unwrap().utilized_amount;
    assert_eq!(
        b_before, b_after,
        "repaying borrower A must not change borrower B's utilization"
    );
}
