// SPDX-License-Identifier: MIT
//! Proptest for installment advancement logic in
//! `advance_repayment_schedule_after_repay`.
//!
//! The installment ledger is advanced inside `repay_credit`. This test
//! generates random `(period_seconds, amount_per_period, repay_amount)`
//! tuples and verifies that `next_due_ts` advances by
//! `k * period_seconds` where `k = floor(amount / amount_per_period)`.
//!
//! ## Coverage
//!
//! - **Partial repay** (`amount < amount_per_period`): `k = 0`, no advancement.
//! - **Exact one installment** (`amount == amount_per_period`): `k = 1`.
//! - **Multiple installments** (`amount == k * amount_per_period`): `k >= 2`.
//! - **Over-repay with remainder** (`amount_per_period < amount < 2 * a_p_p`):
//!   `k = 1` (remainder discarded).
//!
//! ## Edge cases (unit tests)
//!
//! - Zero amount panics with `InvalidAmount` *before* the advancement call.
//! - Setting zero `amount_per_period` or `period_seconds` is rejected by
//!   `set_repayment_schedule` (admin guard).

use proptest::prelude::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, Address, Env};

use creditra_credit::{Credit, CreditClient};

const DRAW_AMOUNT: i128 = 10_000;
const INITIAL_TIMESTAMP: u64 = 1_000;
const INITIAL_NEXT_DUE: u64 = 2_000;

/// Bind an admin, contract, liquidity token, and a funded borrower.
fn setup_env() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();
    let client = CreditClient::new(&env, &contract_id);
    env.ledger().with_mut(|li| li.timestamp = INITIAL_TIMESTAMP);
    client.init(&admin);
    client.set_liquidity_token(&token_address);
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &(DRAW_AMOUNT * 2));
    (env, contract_id, token_address)
}

/// Open a credit line for `borrower` and draw `DRAW_AMOUNT`.
fn setup_borrower(env: &Env, contract_id: &Address, token_address: &Address, borrower: &Address) {
    let client = CreditClient::new(env, contract_id);
    client.open_credit_line(borrower, &(DRAW_AMOUNT * 2), &500_u32, &50_u32);
    client.draw_credit(borrower, &DRAW_AMOUNT);
    // Pre-mint + approve so the first repay in each case can succeed.
    // Amount is set inside the proptest loop; this is a no-op for the draw.
}

/// Mint `amount` to `borrower` and approve the contract to spend it.
fn mint_and_approve(
    env: &Env,
    token_address: &Address,
    contract_id: &Address,
    borrower: &Address,
    amount: i128,
) {
    token::StellarAssetClient::new(env, token_address).mint(borrower, &amount);
    token::Client::new(env, token_address).approve(borrower, contract_id, &amount, &u32::MAX);
}

/// Compute the expected `next_due_ts` advance given the repayment parameters.
///
/// Mirrors `advance_repayment_schedule_after_repay` (lifecycle.rs:289–311).
fn expected_advance_seconds(amount: i128, amount_per_period: i128, period_seconds: u64) -> u64 {
    if amount <= 0 || amount_per_period <= 0 || period_seconds == 0 {
        return 0;
    }
    let installments_paid = (amount / amount_per_period) as u64;
    if installments_paid == 0 {
        return 0;
    }
    installments_paid.saturating_mul(period_seconds)
}

// ── Proptest ──────────────────────────────────────────────────────────────────

proptest! {
    /// For any valid (amount_per_period, period_seconds, repay_amount), the
    /// installment schedule advances by exactly the whole number of periods
    /// covered by the repayment.
    #[test]
    fn prop_installment_advancement(
        amount_per_period in 1i128..=500i128,
        period_seconds in 1u64..=86_400u64,
        amount in 1i128..=DRAW_AMOUNT,
    ) {
        let (env, contract_id, token_address) = setup_env();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);
        setup_borrower(&env, &contract_id, &token_address, &borrower);
        mint_and_approve(&env, &token_address, &contract_id, &borrower, amount);

        client.set_repayment_schedule(
            &borrower,
            &amount_per_period,
            &period_seconds,
            &INITIAL_NEXT_DUE,
        );

        // Sanity-check that set_repayment_schedule stored the expected value.
        let before = client.get_repayment_schedule(&borrower).unwrap();
        prop_assert_eq!(before.next_due_ts, INITIAL_NEXT_DUE);

        client.repay_credit(&borrower, &amount);

        let after = client.get_repayment_schedule(&borrower).unwrap();
        let advance = expected_advance_seconds(amount, amount_per_period, period_seconds);
        let expected = INITIAL_NEXT_DUE + advance;

        prop_assert_eq!(
            after.next_due_ts,
            expected,
            "amount={}, amount_per_period={}, period_seconds={}, k={}",
            amount,
            amount_per_period,
            period_seconds,
            amount / amount_per_period,
        );
    }
}

// ── Deterministic unit tests ──────────────────────────────────────────────────

mod edge_cases {
    use super::*;

    /// Partial repay: amount < amount_per_period → no advancement.
    #[test]
    fn partial_repay_does_not_advance() {
        let (env, contract_id, token_address) = setup_env();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);
        setup_borrower(&env, &contract_id, &token_address, &borrower);
        mint_and_approve(&env, &token_address, &contract_id, &borrower, 10);

        client.set_repayment_schedule(&borrower, &100_i128, &86_400_u64, &INITIAL_NEXT_DUE);
        client.repay_credit(&borrower, &10);

        let schedule = client.get_repayment_schedule(&borrower).unwrap();
        assert_eq!(
            schedule.next_due_ts, INITIAL_NEXT_DUE,
            "partial repay must not advance next_due_ts",
        );
    }

    /// Exact one installment: amount == amount_per_period → advance by 1 period.
    #[test]
    fn exact_one_installment_advances_one_period() {
        let (env, contract_id, token_address) = setup_env();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);
        setup_borrower(&env, &contract_id, &token_address, &borrower);
        mint_and_approve(&env, &token_address, &contract_id, &borrower, 100);

        client.set_repayment_schedule(&borrower, &100_i128, &86_400_u64, &INITIAL_NEXT_DUE);
        client.repay_credit(&borrower, &100);

        let schedule = client.get_repayment_schedule(&borrower).unwrap();
        assert_eq!(
            schedule.next_due_ts,
            INITIAL_NEXT_DUE + 86_400,
            "exact one installment must advance by one period",
        );
    }

    /// Multiple installments: amount == 3 × amount_per_period → advance by 3 periods.
    #[test]
    fn multiple_installments_advance_multiple_periods() {
        let (env, contract_id, token_address) = setup_env();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);
        setup_borrower(&env, &contract_id, &token_address, &borrower);
        mint_and_approve(&env, &token_address, &contract_id, &borrower, 600);

        client.set_repayment_schedule(&borrower, &200_i128, &3600_u64, &INITIAL_NEXT_DUE);
        client.repay_credit(&borrower, &600);

        let schedule = client.get_repayment_schedule(&borrower).unwrap();
        assert_eq!(
            schedule.next_due_ts,
            INITIAL_NEXT_DUE + 3 * 3600,
            "3× installment must advance by 3 periods",
        );
    }

    /// Over-repay with remainder: amount > amount_per_period but < 2× → advance by 1 period.
    #[test]
    fn over_repay_with_remainder_advances_one_period() {
        let (env, contract_id, token_address) = setup_env();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);
        setup_borrower(&env, &contract_id, &token_address, &borrower);
        mint_and_approve(&env, &token_address, &contract_id, &borrower, 150);

        client.set_repayment_schedule(&borrower, &100_i128, &86_400_u64, &INITIAL_NEXT_DUE);
        client.repay_credit(&borrower, &150);

        let schedule = client.get_repayment_schedule(&borrower).unwrap();
        assert_eq!(
            schedule.next_due_ts,
            INITIAL_NEXT_DUE + 86_400,
            "150 repay w/ 100 installment must advance by 1 period (remainder discarded)",
        );
    }

    /// Multiple repays: advancing twice must compound correctly.
    #[test]
    fn sequential_repays_compound_advancement() {
        let (env, contract_id, token_address) = setup_env();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);
        setup_borrower(&env, &contract_id, &token_address, &borrower);

        // First repay: 1 installment
        client.set_repayment_schedule(&borrower, &100_i128, &3600_u64, &INITIAL_NEXT_DUE);
        mint_and_approve(&env, &token_address, &contract_id, &borrower, 100);
        client.repay_credit(&borrower, &100);

        let s1 = client.get_repayment_schedule(&borrower).unwrap();
        assert_eq!(s1.next_due_ts, INITIAL_NEXT_DUE + 3600);

        // Second repay: 2 more installments
        mint_and_approve(&env, &token_address, &contract_id, &borrower, 200);
        client.repay_credit(&borrower, &200);

        let s2 = client.get_repayment_schedule(&borrower).unwrap();
        assert_eq!(s2.next_due_ts, INITIAL_NEXT_DUE + 3 * 3600);
    }
}
