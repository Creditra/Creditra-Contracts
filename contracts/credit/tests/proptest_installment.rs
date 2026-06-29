// SPDX-License-Identifier: MIT
//! Property tests for installment-schedule advancement during repayments.
//!
//! The schedule is advanced from `repay_credit` through
//! `advance_repayment_schedule_after_repay`. These tests exercise random
//! repayment schedules and random repayment streams, asserting that
//! `next_due_ts` advances by exactly the whole number of principal installments
//! covered by the effective repayment amount:
//!
//! ```text
//! principal_repaid  = effective_repay - interest_repaid
//! installments_paid = floor(principal_repaid / amount_per_period)
//! next_due_ts       = previous_next_due_ts + installments_paid * period_seconds
//! ```
//!
//! Interest-only repayments and partial principal installments must not advance
//! the due date. Repayments above the remaining debt are capped by
//! `repay_credit`, so the expected model applies the same cap before computing
//! installment advancement.
use soroban_sdk::testutils::Ledger as _;

use proptest::prelude::*;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{token, Address, Env};

use creditra_credit::{Credit, CreditClient};

const INITIAL_TIMESTAMP: u64 = 1_000;
const INITIAL_NEXT_DUE: u64 = 2_000;
const CREDIT_LIMIT: i128 = 30_000;
const DRAW_AMOUNT: i128 = 10_000;
const COLLATERAL_AMOUNT: i128 = 15_000;
const TOKEN_BALANCE: i128 = 1_000_000;
const RATE_BPS: u32 = 2_500;
const SECONDS_PER_YEAR: u64 = 31_536_000;

/// Test harness for a funded borrower with an open, drawn credit line.
struct Ctx {
    env: Env,
    contract_id: Address,
    token_address: Address,
    borrower: Address,
}

impl Ctx {
    fn client(&self) -> CreditClient<'_> {
        CreditClient::new(&self.env, &self.contract_id)
    }
}

/// Build an initialized credit contract, configure the liquidity token, open a
/// line for one borrower, deposit the collateral required by the default 150%
/// collateral floor, and draw `DRAW_AMOUNT`.
fn setup_env() -> Ctx {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    env.ledger().set_timestamp(INITIAL_TIMESTAMP);

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);
    client.set_liquidity_token(&token_address);

    let token_admin = token::StellarAssetClient::new(&env, &token_address);
    token_admin.mint(&contract_id, &TOKEN_BALANCE);
    token_admin.mint(&borrower, &TOKEN_BALANCE);

    // The same token is used for collateral and repayments. Deposit enough
    // collateral before drawing so the default collateral-ratio guard is met.
    client.deposit_collateral(&borrower, &COLLATERAL_AMOUNT);
    client.open_credit_line(&borrower, &CREDIT_LIMIT, &RATE_BPS, &50_u32);
    client.draw_credit(&borrower, &DRAW_AMOUNT);

    Ctx {
        env,
        contract_id,
        token_address,
        borrower,
    }
}

/// Mint and approve the exact amount needed for a repayment attempt.
fn fund_repayment(ctx: &Ctx, amount: i128) {
    token::StellarAssetClient::new(&ctx.env, &ctx.token_address).mint(&ctx.borrower, &amount);
    token::Client::new(&ctx.env, &ctx.token_address).approve(
        &ctx.borrower,
        &ctx.contract_id,
        &amount,
        &u32::MAX,
    );
}

/// Model the installment advancement performed by the contract.
fn expected_next_due(
    current_next_due: u64,
    principal_repaid: i128,
    amount_per_period: i128,
    period_seconds: u64,
) -> u64 {
    let installments_paid = (principal_repaid / amount_per_period) as u64;
    current_next_due.saturating_add(installments_paid.saturating_mul(period_seconds))
}

/// Floor interest used by the contract's prorating helper.
fn accrued_interest(principal: i128, elapsed_seconds: u64) -> i128 {
    (principal as u128)
        .saturating_mul(RATE_BPS as u128)
        .saturating_mul(elapsed_seconds as u128)
        .checked_div(10_000_u128.saturating_mul(SECONDS_PER_YEAR as u128))
        .unwrap_or(0) as i128
}

proptest! {
    /// A single random repayment advances the schedule by
    /// `floor(principal_repaid / installment)` periods.
    #[test]
    fn installment_advance_single_random_repayment(
        amount_per_period in 1_i128..=2_000_i128,
        period_seconds in 1_u64..=86_400_u64,
        repay_amount in 1_i128..=DRAW_AMOUNT,
    ) {
        let ctx = setup_env();

        ctx.client().set_repayment_schedule(
            &ctx.borrower,
            &amount_per_period,
            &period_seconds,
            &INITIAL_NEXT_DUE,
        );

        fund_repayment(&ctx, repay_amount);
        ctx.client().repay_credit(&ctx.borrower, &repay_amount);

        let schedule = ctx.client().get_repayment_schedule(&ctx.borrower).unwrap();
        let expected = expected_next_due(
            INITIAL_NEXT_DUE,
            repay_amount,
            amount_per_period,
            period_seconds,
        );

        prop_assert_eq!(
            schedule.next_due_ts,
            expected,
            "amount_per_period={amount_per_period}, period_seconds={period_seconds}, repay_amount={repay_amount}",
        );
    }

    /// A random sequence of repayments compounds schedule advancement correctly.
    ///
    /// The model caps each repayment to the outstanding debt, matching
    /// `repay_credit`'s `effective_repay = min(amount, utilized_amount)` rule.
    #[test]
    fn installment_advance_random_repayment_schedule(
        amount_per_period in 1_i128..=2_000_i128,
        period_seconds in 1_u64..=86_400_u64,
        repayments in proptest::collection::vec(1_i128..=4_000_i128, 1..8),
    ) {
        let ctx = setup_env();

        ctx.client().set_repayment_schedule(
            &ctx.borrower,
            &amount_per_period,
            &period_seconds,
            &INITIAL_NEXT_DUE,
        );

        let mut expected_due = INITIAL_NEXT_DUE;
        let mut outstanding = DRAW_AMOUNT;

        for requested_repay in repayments {
            if outstanding == 0 {
                break;
            }

            let effective_repay = requested_repay.min(outstanding);
            fund_repayment(&ctx, requested_repay);
            ctx.client().repay_credit(&ctx.borrower, &requested_repay);

            expected_due = expected_next_due(
                expected_due,
                effective_repay,
                amount_per_period,
                period_seconds,
            );
            outstanding -= effective_repay;

            let schedule = ctx.client().get_repayment_schedule(&ctx.borrower).unwrap();
            prop_assert_eq!(
                schedule.next_due_ts,
                expected_due,
                "amount_per_period={amount_per_period}, period_seconds={period_seconds}, requested_repay={requested_repay}, effective_repay={effective_repay}, outstanding={outstanding}",
            );
        }
    }

    /// Repayment streams with accrued interest only advance by the principal
    /// component after the interest-first allocation is applied.
    #[test]
    fn installment_advance_random_repayment_schedule_with_interest(
        amount_per_period in 1_i128..=2_000_i128,
        period_seconds in 1_u64..=86_400_u64,
        elapsed_seconds in 1_000_u64..=2_000_000_u64,
        repayments in proptest::collection::vec(1_i128..=5_000_i128, 1..8),
    ) {
        let ctx = setup_env();
        ctx.env.ledger().set_timestamp(INITIAL_TIMESTAMP + elapsed_seconds);

        ctx.client().set_repayment_schedule(
            &ctx.borrower,
            &amount_per_period,
            &period_seconds,
            &INITIAL_NEXT_DUE,
        );

        let mut expected_due = INITIAL_NEXT_DUE;
        let mut accrued = accrued_interest(DRAW_AMOUNT, elapsed_seconds);
        let mut outstanding = DRAW_AMOUNT + accrued;

        for requested_repay in repayments {
            if outstanding == 0 {
                break;
            }

            let effective_repay = requested_repay.min(outstanding);
            let interest_repaid = effective_repay.min(accrued);
            let principal_repaid = effective_repay - interest_repaid;

            fund_repayment(&ctx, requested_repay);
            ctx.client().repay_credit(&ctx.borrower, &requested_repay);

            expected_due = expected_next_due(
                expected_due,
                principal_repaid,
                amount_per_period,
                period_seconds,
            );
            accrued -= interest_repaid;
            outstanding -= effective_repay;

            let schedule = ctx.client().get_repayment_schedule(&ctx.borrower).unwrap();
            prop_assert_eq!(
                schedule.next_due_ts,
                expected_due,
                "amount_per_period={amount_per_period}, period_seconds={period_seconds}, elapsed_seconds={elapsed_seconds}, requested_repay={requested_repay}, effective_repay={effective_repay}, interest_repaid={interest_repaid}, principal_repaid={principal_repaid}, outstanding={outstanding}",
            );
        }
    }
}

#[cfg(test)]
mod edge_cases {
    use super::*;

    #[test]
    fn partial_repay_does_not_advance() {
        let ctx = setup_env();
        ctx.client()
            .set_repayment_schedule(&ctx.borrower, &100_i128, &86_400_u64, &INITIAL_NEXT_DUE);

        fund_repayment(&ctx, 99);
        ctx.client().repay_credit(&ctx.borrower, &99);

        let schedule = ctx.client().get_repayment_schedule(&ctx.borrower).unwrap();
        assert_eq!(schedule.next_due_ts, INITIAL_NEXT_DUE);
    }

    #[test]
    fn exact_installment_advances_one_period() {
        let ctx = setup_env();
        ctx.client()
            .set_repayment_schedule(&ctx.borrower, &100_i128, &86_400_u64, &INITIAL_NEXT_DUE);

        fund_repayment(&ctx, 100);
        ctx.client().repay_credit(&ctx.borrower, &100);

        let schedule = ctx.client().get_repayment_schedule(&ctx.borrower).unwrap();
        assert_eq!(schedule.next_due_ts, INITIAL_NEXT_DUE + 86_400);
    }

    #[test]
    fn multiple_installments_advance_multiple_periods() {
        let ctx = setup_env();
        ctx.client()
            .set_repayment_schedule(&ctx.borrower, &200_i128, &3_600_u64, &INITIAL_NEXT_DUE);

        fund_repayment(&ctx, 600);
        ctx.client().repay_credit(&ctx.borrower, &600);

        let schedule = ctx.client().get_repayment_schedule(&ctx.borrower).unwrap();
        assert_eq!(schedule.next_due_ts, INITIAL_NEXT_DUE + 3 * 3_600);
    }

    #[test]
    fn over_repay_is_capped_to_outstanding_before_advance() {
        let ctx = setup_env();
        ctx.client()
            .set_repayment_schedule(&ctx.borrower, &3_000_i128, &60_u64, &INITIAL_NEXT_DUE);

        // Requested amount is greater than outstanding debt, but effective
        // repayment is capped to DRAW_AMOUNT by repay_credit.
        let requested = DRAW_AMOUNT + 5_000;
        fund_repayment(&ctx, requested);
        ctx.client().repay_credit(&ctx.borrower, &requested);

        let schedule = ctx.client().get_repayment_schedule(&ctx.borrower).unwrap();
        let expected = expected_next_due(INITIAL_NEXT_DUE, DRAW_AMOUNT, 3_000, 60);
        assert_eq!(schedule.next_due_ts, expected);
    }

    #[test]
    fn interest_only_repay_does_not_advance() {
        let ctx = setup_env();
        let elapsed_seconds = 1_000_000;
        ctx.env
            .ledger()
            .set_timestamp(INITIAL_TIMESTAMP + elapsed_seconds);
        ctx.client()
            .set_repayment_schedule(&ctx.borrower, &100_i128, &86_400_u64, &INITIAL_NEXT_DUE);

        let interest = accrued_interest(DRAW_AMOUNT, elapsed_seconds);
        assert!(interest > 0);

        fund_repayment(&ctx, interest);
        ctx.client().repay_credit(&ctx.borrower, &interest);

        let schedule = ctx.client().get_repayment_schedule(&ctx.borrower).unwrap();
        assert_eq!(schedule.next_due_ts, INITIAL_NEXT_DUE);
    }

    #[test]
    fn interest_plus_installment_advances_one_period() {
        let ctx = setup_env();
        let elapsed_seconds = 1_000_000;
        ctx.env
            .ledger()
            .set_timestamp(INITIAL_TIMESTAMP + elapsed_seconds);
        ctx.client()
            .set_repayment_schedule(&ctx.borrower, &100_i128, &86_400_u64, &INITIAL_NEXT_DUE);

        let repay = accrued_interest(DRAW_AMOUNT, elapsed_seconds) + 100;
        fund_repayment(&ctx, repay);
        ctx.client().repay_credit(&ctx.borrower, &repay);

        let schedule = ctx.client().get_repayment_schedule(&ctx.borrower).unwrap();
        assert_eq!(schedule.next_due_ts, INITIAL_NEXT_DUE + 86_400);
    }
}
