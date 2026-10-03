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
//!
//! The late-fee surcharge applied during advancement is covered by the
//! extreme-ratio tests further down: overdue installments are counted
//! arithmetically (not by iterating), so a repayment that settles `10^12`
//! installments still charges the same aggregate fee, emits a single event, and
//! stays within the CPU budget.
use soroban_sdk::testutils::Ledger as _;

use proptest::prelude::*;
use soroban_sdk::testutils::{Address as _, Events, Ledger};
use soroban_sdk::{symbol_short, token, Address, Env, Symbol, TryFromVal};

use creditra_credit::{Credit, CreditClient};

const INITIAL_TIMESTAMP: u64 = 1_000;
const INITIAL_NEXT_DUE: u64 = 2_000;
const CREDIT_LIMIT: i128 = 30_000;
const DRAW_AMOUNT: i128 = 10_000;
const COLLATERAL_AMOUNT: i128 = 15_000;
const TOKEN_BALANCE: i128 = 1_000_000;
const RATE_BPS: u32 = 2_500;
const SECONDS_PER_YEAR: u64 = 31_536_000;

/// Debt used by the extreme-ratio tests.
///
/// With `amount_per_period = 1` a single repayment of this principal covers
/// `10^12` installments — the ratio the removed per-installment late-fee loop
/// could not survive within the CPU budget.
const EXTREME_DEBT: i128 = 1_000_000_000_000;

/// Ledger timestamp used by the extreme-ratio harness.
///
/// The draw and the repayment both happen at this timestamp, so no interest
/// accrues and the repayment is pure principal. Callers place `first_due_ts`
/// below it (`EXTREME_NOW - overdue_periods * period_seconds`, at most
/// `50 * 86_400` seconds earlier) so the subtraction cannot underflow.
const EXTREME_NOW: u64 = 10_000_000;

/// Generous CPU ceiling, in Soroban instructions, for one extreme repayment.
///
/// Soroban's per-transaction CPU budget is on the order of 100M instructions.
/// A per-installment loop over `10^12` installments needs many orders of
/// magnitude more, while the arithmetic implementation stays O(1); asserting a
/// value below the per-transaction budget therefore proves the loop is gone
/// without pinning an exact instruction count.
const EXTREME_REPAY_CPU_CEILING: u64 = 80_000_000;

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

/// Build a borrower whose outstanding debt is exactly `debt`.
///
/// Mirrors [`setup_env`] but with amounts large enough for the extreme-ratio
/// tests: collateral is `2 * debt` (above the 150 % floor) and the token head
/// room is `4 * debt` for both the contract and the borrower. The ledger is
/// pinned to [`EXTREME_NOW`] at draw time so a repayment made at the same
/// timestamp carries no accrued interest.
fn setup_extreme_env(debt: i128) -> Ctx {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    env.ledger().set_timestamp(EXTREME_NOW);

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);
    client.set_liquidity_token(&token_address);

    let headroom = debt.saturating_mul(4);
    let sac = token::StellarAssetClient::new(&env, &token_address);
    sac.mint(&contract_id, &headroom);
    sac.mint(&borrower, &headroom);

    client.deposit_collateral(&borrower, &debt.saturating_mul(2));
    client.open_credit_line(&borrower, &debt, &RATE_BPS, &50_u32);
    client.draw_credit(&borrower, &debt);

    // `repay_credit` pulls funds with `transfer_from`, so the borrower must
    // allow the contract to move the full debt before the repayment, plus any
    // late fee that is now pulled from the borrower alongside it.
    token::Client::new(&env, &token_address).approve(&borrower, &contract_id, &headroom, &u32::MAX);

    Ctx {
        env,
        contract_id,
        token_address,
        borrower,
    }
}

/// Count the `("credit", "late_fee")` events currently recorded in `env`.
///
/// The credit contract's events use `(symbol_short!("credit"), <suffix>)` as
/// their topic pair, so matching the second topic isolates late-fee events
/// from the repayment/draw/accrual events emitted around them.
fn late_fee_event_count(env: &Env) -> usize {
    let events = env.events().all();
    let expected = symbol_short!("late_fee");
    let mut count = 0usize;
    for i in 0..events.len() {
        let topics = events.get(i).unwrap().1;
        if let Some(topic) = topics.get(1) {
            if let Ok(symbol) = Symbol::try_from_val(env, &topic) {
                if symbol == expected {
                    count += 1;
                }
            }
        }
    }
    count
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
            "amount_per_period={}, period_seconds={}, repay_amount={}",
            amount_per_period,
            period_seconds,
            repay_amount,
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
                "amount_per_period={}, period_seconds={}, requested_repay={}, effective_repay={}, outstanding={}",
                amount_per_period,
                period_seconds,
                requested_repay,
                effective_repay,
                outstanding,
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
            // Build the diagnostic message eagerly: proptest's `prop_assert_eq!`
            // macro expands the literal through `concat!`, which does not resolve
            // inline format captures.
            let detail = format!(
                "amount_per_period={amount_per_period}, period_seconds={period_seconds}, elapsed_seconds={elapsed_seconds}, requested_repay={requested_repay}, effective_repay={effective_repay}, interest_repaid={interest_repaid}, principal_repaid={principal_repaid}, outstanding={outstanding}"
            );
            prop_assert_eq!(schedule.next_due_ts, expected_due, "{}", detail);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Extreme-ratio late-fee aggregation
// ─────────────────────────────────────────────────────────────────────────────

proptest! {
    // The 10^12-installment harness is comparatively expensive to build, so a
    // smaller case count keeps the suite fast while still exploring the
    // schedule/overdue space.
    #![proptest_config(ProptestConfig::with_cases(16))]

    /// A repayment that settles `10^12` installments still charges exactly the
    /// per-installment late-fee sum, emits a single event, and stays O(1).
    ///
    /// `amount_per_period = 1` makes `installments_paid` equal to the principal
    /// repaid, so the removed loop would have iterated `10^12` times. The
    /// arithmetic implementation must instead:
    ///   * advance `next_due_ts` by `installments_paid * period_seconds`;
    ///   * credit `late_fee * overdue_periods` to the treasury — the aggregate
    ///     the old loop produced;
    ///   * emit exactly one `late_fee` event; and
    ///   * stay well inside the per-transaction CPU budget.
    #[test]
    fn extreme_ratio_repay_aggregates_late_fee(
        period_seconds in 1_u64..=86_400_u64,
        overdue_periods in 1_u64..=50_u64,
        late_fee in 1_i128..=1_000_i128,
    ) {
        let ctx = setup_extreme_env(EXTREME_DEBT);
        let client = ctx.client();

        // First due date strictly in the past: `overdue_periods` full periods
        // have elapsed, so exactly that many installments are overdue.
        let first_due = EXTREME_NOW - overdue_periods * period_seconds;
        client.set_repayment_schedule(&ctx.borrower, &1_i128, &period_seconds, &first_due);
        client.set_late_fee_flat(&late_fee);

        let treasury_before = client.get_protocol_summary().treasury_balance;

        ctx.env.cost_estimate().budget().reset_unlimited();
        client.repay_credit(&ctx.borrower, &EXTREME_DEBT);
        let cpu = ctx.env.cost_estimate().budget().cpu_instruction_cost();

        let schedule = client.get_repayment_schedule(&ctx.borrower).unwrap();
        let expected_due = first_due
            .saturating_add((EXTREME_DEBT as u64).saturating_mul(period_seconds));
        prop_assert_eq!(
            schedule.next_due_ts,
            expected_due,
            "next_due_ts must advance by installments_paid * period_seconds",
        );

        let treasury_after = client.get_protocol_summary().treasury_balance;
        prop_assert_eq!(
            treasury_after - treasury_before,
            late_fee * overdue_periods as i128,
            "aggregate late fee must equal the per-installment sum",
        );

        prop_assert_eq!(
            late_fee_event_count(&ctx.env),
            1,
            "exactly one late-fee event per repayment",
        );

        prop_assert!(
            cpu < EXTREME_REPAY_CPU_CEILING,
            "extreme repayment consumed {cpu} CPU instructions",
        );
    }
}

#[cfg(test)]
mod edge_cases {
    use super::*;

    #[test]
    fn partial_repay_does_not_advance() {
        let ctx = setup_env();
        ctx.client().set_repayment_schedule(
            &ctx.borrower,
            &100_i128,
            &86_400_u64,
            &INITIAL_NEXT_DUE,
        );

        fund_repayment(&ctx, 99);
        ctx.client().repay_credit(&ctx.borrower, &99);

        let schedule = ctx.client().get_repayment_schedule(&ctx.borrower).unwrap();
        assert_eq!(schedule.next_due_ts, INITIAL_NEXT_DUE);
    }

    #[test]
    fn exact_installment_advances_one_period() {
        let ctx = setup_env();
        ctx.client().set_repayment_schedule(
            &ctx.borrower,
            &100_i128,
            &86_400_u64,
            &INITIAL_NEXT_DUE,
        );

        fund_repayment(&ctx, 100);
        ctx.client().repay_credit(&ctx.borrower, &100);

        let schedule = ctx.client().get_repayment_schedule(&ctx.borrower).unwrap();
        assert_eq!(schedule.next_due_ts, INITIAL_NEXT_DUE + 86_400);
    }

    #[test]
    fn multiple_installments_advance_multiple_periods() {
        let ctx = setup_env();
        ctx.client().set_repayment_schedule(
            &ctx.borrower,
            &200_i128,
            &3_600_u64,
            &INITIAL_NEXT_DUE,
        );

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
        ctx.client().set_repayment_schedule(
            &ctx.borrower,
            &100_i128,
            &86_400_u64,
            &INITIAL_NEXT_DUE,
        );

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
        ctx.client().set_repayment_schedule(
            &ctx.borrower,
            &100_i128,
            &86_400_u64,
            &INITIAL_NEXT_DUE,
        );

        let repay = accrued_interest(DRAW_AMOUNT, elapsed_seconds) + 100;
        fund_repayment(&ctx, repay);
        ctx.client().repay_credit(&ctx.borrower, &repay);

        let schedule = ctx.client().get_repayment_schedule(&ctx.borrower).unwrap();
        assert_eq!(schedule.next_due_ts, INITIAL_NEXT_DUE + 86_400);
    }

    // ── Late-fee aggregation: exact period boundary ───────────────────────

    /// A due date exactly equal to `now` is **not** overdue (the contract uses a
    /// strict `now > due_ts` comparison). With `period_seconds = 100` and
    /// `next_due_ts = now - 200`, exactly two of the five settled installments
    /// are overdue, so the aggregate fee is `2 * late_fee` — not `3 * late_fee`,
    /// which a naive `elapsed / period_seconds + 1` would charge.
    #[test]
    fn late_fee_exact_period_boundary_uses_strict_comparison() {
        let ctx = setup_env();
        let client = ctx.client();

        let period_seconds: u64 = 100;
        let late_fee: i128 = 30;
        // The draw happened at INITIAL_TIMESTAMP; keep the ledger there so the
        // repayment carries no interest, and place the first due date two full
        // periods earlier.
        let now = INITIAL_TIMESTAMP;
        let first_due = now - 2 * period_seconds;

        // amount_per_period = 1_000 and a 5_000 principal repayment settle five
        // installments, of which `ceil(200 / 100) = 2` are overdue.
        client.set_repayment_schedule(&ctx.borrower, &1_000_i128, &period_seconds, &first_due);
        client.set_late_fee_flat(&late_fee);

        let treasury_before = client.get_protocol_summary().treasury_balance;
        fund_repayment(&ctx, 5_000);
        client.repay_credit(&ctx.borrower, &5_000);
        let treasury_after = client.get_protocol_summary().treasury_balance;

        assert_eq!(
            treasury_after - treasury_before,
            2 * late_fee,
            "only installments with due_ts < now are overdue",
        );
        assert_eq!(
            late_fee_event_count(&ctx.env),
            1,
            "the aggregate is reported in one event",
        );
    }

    // ── Extreme ratio: 10^12 installments in one repayment ────────────────

    /// Acceptance criterion: repaying `10^12` principal against
    /// `amount_per_period = 1` settles `10^12` installments, charges the
    /// aggregate of the per-installment late fees, emits a single event, and
    /// completes well within the Soroban CPU budget.
    #[test]
    fn extreme_ratio_repay_of_10_pow_12_completes_within_budget() {
        let ctx = setup_extreme_env(EXTREME_DEBT);
        let client = ctx.client();

        let period_seconds: u64 = 3_600;
        let overdue_periods: u64 = 3;
        let late_fee: i128 = 7;
        let first_due = EXTREME_NOW - overdue_periods * period_seconds;

        client.set_repayment_schedule(&ctx.borrower, &1_i128, &period_seconds, &first_due);
        client.set_late_fee_flat(&late_fee);

        let treasury_before = client.get_protocol_summary().treasury_balance;

        ctx.env.cost_estimate().budget().reset_unlimited();
        client.repay_credit(&ctx.borrower, &EXTREME_DEBT);
        let cpu = ctx.env.cost_estimate().budget().cpu_instruction_cost();

        let schedule = client.get_repayment_schedule(&ctx.borrower).unwrap();
        let expected_due =
            first_due.saturating_add((EXTREME_DEBT as u64).saturating_mul(period_seconds));
        assert_eq!(schedule.next_due_ts, expected_due);

        let treasury_after = client.get_protocol_summary().treasury_balance;
        assert_eq!(
            treasury_after - treasury_before,
            late_fee * overdue_periods as i128,
            "aggregate fee must equal the per-installment sum",
        );
        assert_eq!(
            late_fee_event_count(&ctx.env),
            1,
            "one late-fee event per repayment",
        );
        assert!(
            cpu < EXTREME_REPAY_CPU_CEILING,
            "extreme repayment consumed {cpu} CPU instructions",
        );
    }
}
