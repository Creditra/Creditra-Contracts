// SPDX-License-Identifier: MIT
//! Property-based tests for installment-advance semantics — **#758**.
//!
//! These tests complement `proptest_installment.rs` (which focuses on the
//! core `floor(principal / amount_per_period)` advancement formula) by
//! exercising orthogonal invariants:
//!
//! 1. **Monotonicity** — `next_due_ts` must never decrease across any
//!    sequence of random repayments.
//! 2. **Delinquency tracking** — `is_delinquent` must agree with the
//!    model: a borrower is delinquent iff
//!    `ledger_ts > next_due_ts + grace_period` and `utilized_amount > 0`.
//! 3. **Borrower isolation** — repayments for borrower A must not mutate
//!    borrower B's schedule on the same contract instance.
//! 4. **Large-timestamp saturation** — schedules whose arithmetic would
//!    exceed `u64::MAX` must saturate to `u64::MAX` rather than wrapping.
//! 5. **Schedule cleared on close** — `get_repayment_schedule` returns
//!    `None` after `close_credit_line`, regardless of prior schedule state.
//!
//! # Model
//!
//! ```text
//! principal_repaid  = effective_repay - interest_repaid
//! installments_paid = floor(principal_repaid / amount_per_period)
//! next_due_ts′      = next_due_ts + installments_paid × period_seconds
//!                     (saturating, never wraps)
//! ```
//!
//! The helpers and constants are intentionally kept local so this file
//! compiles as a standalone integration test without depending on
//! `proptest_installment`'s internal symbols.

use proptest::prelude::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Env,
};

use creditra_credit::{Credit, CreditClient};

// ─────────────────────────────────────────────────────────────────────────────
// Shared constants
// ─────────────────────────────────────────────────────────────────────────────

/// Ledger timestamp at contract initialisation.
const T0: u64 = 1_000_000;

/// Credit limit used by every test borrower.
const CREDIT_LIMIT: i128 = 50_000;

/// Draw amount (< CREDIT_LIMIT).
const DRAW_AMOUNT: i128 = 20_000;

/// Collateral deposited — must satisfy the default 150 % floor:
/// 20_000 × 1.5 = 30_000.
const COLLATERAL: i128 = 30_000;

/// Annual rate in basis points (5 %).
const RATE_BPS: u32 = 500;

/// Token balance minted to the contract and each borrower (generous headroom).
const TOKEN_BALANCE: i128 = 10_000_000;

/// Seconds per year used by the contract's `prorate_interest` helper.
/// Matches `accrual::SECONDS_PER_YEAR` (non-Julian 365-day year).
const SECONDS_PER_YEAR: u64 = 31_536_000;

// ─────────────────────────────────────────────────────────────────────────────
// Test harness
// ─────────────────────────────────────────────────────────────────────────────

/// Everything a single-borrower test needs.
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

/// Build an initialised credit contract with one drawn credit line.
///
/// Ledger is pinned to `T0` on return; callers advance it as needed.
fn setup() -> Ctx {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    env.ledger().set_timestamp(T0);

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();

    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    client.set_liquidity_token(&token_address);

    let sac = token::StellarAssetClient::new(&env, &token_address);
    sac.mint(&contract_id, &TOKEN_BALANCE);
    sac.mint(&borrower, &TOKEN_BALANCE);

    // Collateral must be in place before draw (150 % LTV floor).
    client.deposit_collateral(&borrower, &COLLATERAL);
    client.open_credit_line(&borrower, &CREDIT_LIMIT, &RATE_BPS, &50_u32);
    client.draw_credit(&borrower, &DRAW_AMOUNT);

    Ctx {
        env,
        contract_id,
        token_address,
        borrower,
    }
}

/// Mint enough tokens and set an unlimited allowance so `repay_credit` succeeds.
fn fund_repayment(ctx: &Ctx, amount: i128) {
    token::StellarAssetClient::new(&ctx.env, &ctx.token_address).mint(&ctx.borrower, &amount);
    token::Client::new(&ctx.env, &ctx.token_address).approve(
        &ctx.borrower,
        &ctx.contract_id,
        &amount,
        &u32::MAX,
    );
}

/// Model: advance `next_due_ts` by the number of *principal* installments
/// contained in `principal_repaid`.  Uses saturating arithmetic, matching
/// the contract's `saturating_add` / `saturating_mul`.
fn model_advance(
    current_due: u64,
    principal_repaid: i128,
    amount_per_period: i128,
    period_seconds: u64,
) -> u64 {
    if principal_repaid <= 0 || amount_per_period <= 0 {
        return current_due;
    }
    let installments = (principal_repaid / amount_per_period) as u64;
    current_due.saturating_add(installments.saturating_mul(period_seconds))
}

/// Compute the floor interest accrued on `principal` at `RATE_BPS` over
/// `elapsed_secs`.  Mirrors `math_utils::prorate_interest`.
fn floor_interest(principal: i128, elapsed_secs: u64) -> i128 {
    (principal as u128)
        .saturating_mul(RATE_BPS as u128)
        .saturating_mul(elapsed_secs as u128)
        .checked_div((10_000_u128).saturating_mul(SECONDS_PER_YEAR as u128))
        .unwrap_or(0) as i128
}

// ─────────────────────────────────────────────────────────────────────────────
// Property tests
// ─────────────────────────────────────────────────────────────────────────────

proptest! {
    /// **Monotonicity invariant** — `next_due_ts` must never decrease.
    ///
    /// For any random schedule and any sequence of random repayment amounts,
    /// each repayment either advances the due date forward or leaves it
    /// unchanged. It must never move backward.
    #[test]
    fn prop_next_due_ts_is_monotonically_non_decreasing(
        amount_per_period in 1_i128..=3_000_i128,
        period_seconds   in 1_u64..=86_400_u64,
        repayments in proptest::collection::vec(1_i128..=5_000_i128, 1..10),
    ) {
        let ctx = setup();

        let first_due = T0 + period_seconds;
        ctx.client().set_repayment_schedule(
            &ctx.borrower,
            &amount_per_period,
            &period_seconds,
            &first_due,
        );

        let mut prev_due = first_due;
        let mut outstanding = DRAW_AMOUNT;

        for repay in repayments {
            if outstanding == 0 {
                break;
            }
            fund_repayment(&ctx, repay);
            ctx.client().repay_credit(&ctx.borrower, &repay);

            let schedule = ctx.client()
                .get_repayment_schedule(&ctx.borrower)
                .unwrap();

            prop_assert!(
                schedule.next_due_ts >= prev_due,
                "next_due_ts regressed: was {prev_due}, now {}",
                schedule.next_due_ts
            );
            prev_due = schedule.next_due_ts;
            outstanding = outstanding.saturating_sub(repay.min(outstanding));
        }
    }

    /// **Delinquency tracking** — `is_delinquent` must agree with the
    /// timestamp model.
    ///
    /// A borrower is delinquent iff the ledger timestamp is strictly past
    /// `next_due_ts` (the default grace period is 0 unless configured).
    /// This test checks the flag at three ledger positions relative to a
    /// random `next_due_ts`:
    ///   * exactly at `next_due_ts`   → NOT delinquent
    ///   * one second before          → NOT delinquent
    ///   * one second after           → delinquent
    #[test]
    fn prop_delinquency_flag_agrees_with_timestamp_model(
        amount_per_period in 1_i128..=1_000_i128,
        period_seconds   in 60_u64..=3_600_u64,
        // offset from T0 for the first due date
        due_offset in 1_u64..=86_400_u64,
    ) {
        let ctx = setup();
        let first_due = T0 + due_offset;

        ctx.client().set_repayment_schedule(
            &ctx.borrower,
            &amount_per_period,
            &period_seconds,
            &first_due,
        );

        // ── One second before the due date: not delinquent ──
        ctx.env.ledger().set_timestamp(first_due.saturating_sub(1).max(T0));
        prop_assert!(
            !ctx.client().is_delinquent(&ctx.borrower),
            "should NOT be delinquent one second before due date"
        );

        // ── Exactly at the due date: not delinquent ──
        ctx.env.ledger().set_timestamp(first_due);
        prop_assert!(
            !ctx.client().is_delinquent(&ctx.borrower),
            "should NOT be delinquent exactly at due date"
        );

        // ── One second past the due date: delinquent ──
        ctx.env.ledger().set_timestamp(first_due + 1);
        prop_assert!(
            ctx.client().is_delinquent(&ctx.borrower),
            "should be delinquent one second past due date"
        );
    }

    /// **Borrower isolation** — repayments for borrower A must not mutate
    /// borrower B's repayment schedule on the same contract instance.
    ///
    /// Both borrowers share the same contract but have independent storage
    /// entries.  This property ensures that the schedule storage key is
    /// properly scoped to the individual borrower address.
    #[test]
    fn prop_borrower_schedules_are_isolated(
        amount_a in 100_i128..=1_000_i128,
        amount_b in 100_i128..=1_000_i128,
        period_a in 300_u64..=3_600_u64,
        period_b in 300_u64..=3_600_u64,
        repay_a in 1_i128..=DRAW_AMOUNT,
    ) {
        // Set up the first borrower via the shared helper.
        let ctx = setup();

        // Register a second borrower on the same contract.
        let borrower_b = Address::generate(&ctx.env);
        let sac = token::StellarAssetClient::new(&ctx.env, &ctx.token_address);
        sac.mint(&borrower_b, &TOKEN_BALANCE);
        // Borrow B needs collateral too.
        let client = ctx.client();
        client.deposit_collateral(&borrower_b, &COLLATERAL);
        client.open_credit_line(&borrower_b, &CREDIT_LIMIT, &RATE_BPS, &50_u32);
        client.draw_credit(&borrower_b, &DRAW_AMOUNT);

        // Give each borrower a different schedule with a fixed, deterministic due date.
        let due_a = T0 + period_a;
        let due_b = T0 + period_b;
        client.set_repayment_schedule(&ctx.borrower, &amount_a, &period_a, &due_a);
        client.set_repayment_schedule(&borrower_b, &amount_b, &period_b, &due_b);

        // Record B's schedule before any A repayment.
        let b_before = client.get_repayment_schedule(&borrower_b).unwrap();

        // Repay on behalf of A.
        fund_repayment(&ctx, repay_a);
        client.repay_credit(&ctx.borrower, &repay_a);

        // B's schedule must be completely unchanged.
        let b_after = client.get_repayment_schedule(&borrower_b).unwrap();
        prop_assert_eq!(
            b_after.next_due_ts,
            b_before.next_due_ts,
            "borrower B's next_due_ts changed after borrower A repaid"
        );
        prop_assert_eq!(
            b_after.amount_per_period,
            b_before.amount_per_period,
            "borrower B's amount_per_period changed after borrower A repaid"
        );
        prop_assert_eq!(
            b_after.period_seconds,
            b_before.period_seconds,
            "borrower B's period_seconds changed after borrower A repaid"
        );
    }

    /// **Schedule–model agreement with random interest** — after accruing
    /// interest for a random elapsed time, the contract's advancement must
    /// equal the model's prediction when interest is allocated first.
    ///
    /// This property picks a random elapsed time, advances the ledger, then
    /// makes a single repayment and checks that the on-chain `next_due_ts`
    /// matches the reference model exactly.
    #[test]
    fn prop_advancement_model_matches_contract_with_accrued_interest(
        amount_per_period in 1_i128..=2_000_i128,
        period_seconds   in 1_u64..=86_400_u64,
        elapsed_secs     in 0_u64..=SECONDS_PER_YEAR,
        repay            in 1_i128..=30_000_i128,
    ) {
        let ctx = setup();
        let first_due = T0 + period_seconds;

        // Advance the ledger so interest accrues.
        ctx.env.ledger().set_timestamp(T0 + elapsed_secs);

        ctx.client().set_repayment_schedule(
            &ctx.borrower,
            &amount_per_period,
            &period_seconds,
            &first_due,
        );

        // Compute accrued interest and outstanding debt the same way repay_credit does.
        let accrued = floor_interest(DRAW_AMOUNT, elapsed_secs);
        let outstanding = DRAW_AMOUNT + accrued;
        let effective_repay = repay.min(outstanding);
        let interest_repaid = effective_repay.min(accrued);
        let principal_repaid = effective_repay - interest_repaid;

        let expected = model_advance(first_due, principal_repaid, amount_per_period, period_seconds);

        fund_repayment(&ctx, repay);
        ctx.client().repay_credit(&ctx.borrower, &repay);

        let schedule = ctx.client()
            .get_repayment_schedule(&ctx.borrower)
            .unwrap();

        prop_assert_eq!(
            schedule.next_due_ts,
            expected,
            "contract={} model={} \
             (amount_per_period={amount_per_period}, period_seconds={period_seconds}, \
              elapsed_secs={elapsed_secs}, repay={repay}, accrued={accrued}, \
              effective_repay={effective_repay}, interest_repaid={interest_repaid}, \
              principal_repaid={principal_repaid})",
            schedule.next_due_ts, expected,
        );
    }

    /// **Saturation safety** — when `amount_per_period` is 1 and
    /// `period_seconds` is near `u64::MAX / DRAW_AMOUNT`, the arithmetic
    /// must saturate to `u64::MAX` rather than wrapping or panicking.
    ///
    /// The contract uses `saturating_mul` and `saturating_add`, so the result
    /// must always be `<= u64::MAX`.
    #[test]
    fn prop_large_period_seconds_saturates_rather_than_wraps(
        // A large but representable period that could overflow when multiplied by
        // the number of installments covered by DRAW_AMOUNT (up to 20_000).
        period_seconds in (u64::MAX / 20_001)..=u64::MAX,
    ) {
        let ctx = setup();
        let first_due = T0;

        // amount_per_period = 1 → up to DRAW_AMOUNT installments per repayment.
        ctx.client().set_repayment_schedule(
            &ctx.borrower,
            &1_i128,
            &period_seconds,
            &first_due,
        );

        fund_repayment(&ctx, DRAW_AMOUNT);
        ctx.client().repay_credit(&ctx.borrower, &DRAW_AMOUNT);

        let schedule = ctx.client()
            .get_repayment_schedule(&ctx.borrower)
            .unwrap();

        // Must not have wrapped below first_due.
        prop_assert!(
            schedule.next_due_ts >= first_due,
            "next_due_ts wrapped below first_due (got {})",
            schedule.next_due_ts
        );
        // The saturating model gives the upper bound.
        let model = model_advance(first_due, DRAW_AMOUNT, 1, period_seconds);
        prop_assert_eq!(
            schedule.next_due_ts,
            model,
            "contract saturated differently from model: contract={} model={}",
            schedule.next_due_ts, model
        );
    }
} // end proptest!

// ─────────────────────────────────────────────────────────────────────────────
// Deterministic edge-case tests
// ─────────────────────────────────────────────────────────────────────────────

/// Focused unit tests that pin specific boundary conditions.  These are plain
/// `#[test]` functions so they run with zero shrinking overhead and produce
/// stable CI output.
#[cfg(test)]
mod edge_cases {
    use super::*;

    // ── Schedule cleared on close ─────────────────────────────────────────

    /// After `close_credit_line`, `get_repayment_schedule` must return `None`.
    ///
    /// The `close_credit_line` path calls `clear_repayment_schedule` in
    /// `lifecycle.rs`.  This test ensures the storage entry is actually removed.
    #[test]
    fn schedule_is_cleared_on_close() {
        let ctx = setup();

        ctx.client()
            .set_repayment_schedule(&ctx.borrower, &500_i128, &86_400_u64, &(T0 + 86_400));
        assert!(
            ctx.client().get_repayment_schedule(&ctx.borrower).is_some(),
            "pre-condition: schedule must exist before close"
        );

        // Fully repay so the borrower can self-close, or use admin close.
        // We hold a drawn balance, so mint and repay it first.
        fund_repayment(&ctx, DRAW_AMOUNT);
        ctx.client().repay_credit(&ctx.borrower, &DRAW_AMOUNT);

        // Admin-close (closer == admin is always allowed regardless of balance).
        // Since mock_all_auths is active the admin address is not tracked; we
        // generate a fresh address and use it as the `closer` argument — the
        // contract will accept it as the admin under `mock_all_auths`.
        let admin_closer = Address::generate(&ctx.env);
        ctx.client().close_credit_line(&ctx.borrower, &admin_closer);

        assert!(
            ctx.client().get_repayment_schedule(&ctx.borrower).is_none(),
            "schedule must be None after close_credit_line"
        );
    }

    // ── Zero outstanding: repayment is a no-op for the schedule ──────────

    /// When `utilized_amount` is already zero, `repay_credit` caps
    /// `effective_repay` to 0, so the schedule must not advance.
    #[test]
    fn schedule_unchanged_when_nothing_outstanding() {
        let ctx = setup();
        let first_due = T0 + 3_600;

        ctx.client()
            .set_repayment_schedule(&ctx.borrower, &100_i128, &3_600_u64, &first_due);

        // Repay the full drawn amount.
        fund_repayment(&ctx, DRAW_AMOUNT);
        ctx.client().repay_credit(&ctx.borrower, &DRAW_AMOUNT);

        let due_after_full_repay = ctx
            .client()
            .get_repayment_schedule(&ctx.borrower)
            .unwrap()
            .next_due_ts;

        // Attempt another repayment on a zeroed-out balance.
        fund_repayment(&ctx, 500);
        ctx.client().repay_credit(&ctx.borrower, &500);

        let due_after_second = ctx
            .client()
            .get_repayment_schedule(&ctx.borrower)
            .unwrap()
            .next_due_ts;

        assert_eq!(
            due_after_second, due_after_full_repay,
            "schedule must not advance when outstanding balance is zero"
        );
    }

    // ── Exact boundary: amount_per_period - 1 does not advance ───────────

    /// Paying one stroop less than `amount_per_period` in pure principal must
    /// leave `next_due_ts` unchanged (floor division rounds down).
    #[test]
    fn one_stroop_short_of_installment_does_not_advance() {
        let ctx = setup();
        let amount_per_period: i128 = 1_000;
        let period_seconds: u64 = 3_600;
        let first_due = T0 + period_seconds;

        ctx.client().set_repayment_schedule(
            &ctx.borrower,
            &amount_per_period,
            &period_seconds,
            &first_due,
        );

        // Repay exactly (amount_per_period - 1) in principal (no interest accrued).
        let repay = amount_per_period - 1;
        fund_repayment(&ctx, repay);
        ctx.client().repay_credit(&ctx.borrower, &repay);

        let schedule = ctx.client().get_repayment_schedule(&ctx.borrower).unwrap();
        assert_eq!(
            schedule.next_due_ts, first_due,
            "next_due_ts must not advance for (amount_per_period - 1) repayment"
        );
    }

    // ── Exact boundary: amount_per_period advances exactly one period ─────

    /// Paying exactly `amount_per_period` in principal must advance
    /// `next_due_ts` by exactly `period_seconds`.
    #[test]
    fn exact_installment_advances_exactly_one_period() {
        let ctx = setup();
        let amount_per_period: i128 = 1_000;
        let period_seconds: u64 = 3_600;
        let first_due = T0 + period_seconds;

        ctx.client().set_repayment_schedule(
            &ctx.borrower,
            &amount_per_period,
            &period_seconds,
            &first_due,
        );

        fund_repayment(&ctx, amount_per_period);
        ctx.client().repay_credit(&ctx.borrower, &amount_per_period);

        let schedule = ctx.client().get_repayment_schedule(&ctx.borrower).unwrap();
        assert_eq!(
            schedule.next_due_ts,
            first_due + period_seconds,
            "next_due_ts must advance by exactly one period for exact installment"
        );
    }

    // ── Schedule is not auto-created when none was set ────────────────────

    /// `get_repayment_schedule` returns `None` for a borrower that has an open
    /// drawn credit line but for whom `set_repayment_schedule` was never called.
    #[test]
    fn no_schedule_returns_none() {
        let ctx = setup();
        assert!(
            ctx.client().get_repayment_schedule(&ctx.borrower).is_none(),
            "expected None when no schedule has been set"
        );
    }

    // ── Delinquency: no schedule → never delinquent ───────────────────────

    /// A borrower with no repayment schedule must not be flagged as delinquent
    /// regardless of the ledger timestamp.
    #[test]
    fn no_schedule_is_not_delinquent() {
        let ctx = setup();

        // Jump far into the future.
        ctx.env.ledger().set_timestamp(T0 + 10 * SECONDS_PER_YEAR);

        assert!(
            !ctx.client().is_delinquent(&ctx.borrower),
            "a borrower without a schedule must never be delinquent"
        );
    }

    // ── Delinquency: fully repaid borrower is not delinquent ─────────────

    /// Once `utilized_amount` reaches 0 the borrower has no outstanding debt,
    /// so `is_delinquent` must return `false` even if the due date has passed.
    #[test]
    fn fully_repaid_is_not_delinquent_after_due_date() {
        let ctx = setup();
        let first_due = T0 + 1_000;

        ctx.client()
            .set_repayment_schedule(&ctx.borrower, &100_i128, &1_000_u64, &first_due);

        // Repay everything before the due date.
        fund_repayment(&ctx, DRAW_AMOUNT);
        ctx.client().repay_credit(&ctx.borrower, &DRAW_AMOUNT);

        // Advance past the due date.
        ctx.env.ledger().set_timestamp(first_due + 5_000);

        assert!(
            !ctx.client().is_delinquent(&ctx.borrower),
            "fully repaid borrower must not be delinquent after due date"
        );
    }

    // ── Monotonicity: sequential repayments each advance or hold ─────────

    /// Ten sequential repayments of varying sizes — the due date must be
    /// non-decreasing after every step.
    #[test]
    fn sequential_repayments_preserve_monotonicity() {
        let ctx = setup();
        let first_due = T0 + 600;

        ctx.client()
            .set_repayment_schedule(&ctx.borrower, &500_i128, &600_u64, &first_due);

        // Repayment amounts chosen to exercise partial, exact, and multi-period advances.
        let repayments: &[i128] = &[499, 1, 500, 501, 1_000, 3_000, 5_000, 2_000, 500, 999];
        let mut prev_due = first_due;
        let mut outstanding = DRAW_AMOUNT;

        for &r in repayments {
            if outstanding == 0 {
                break;
            }
            fund_repayment(&ctx, r);
            ctx.client().repay_credit(&ctx.borrower, &r);

            let schedule = ctx.client().get_repayment_schedule(&ctx.borrower).unwrap();
            assert!(
                schedule.next_due_ts >= prev_due,
                "monotonicity violated: prev={prev_due} new={}",
                schedule.next_due_ts
            );
            prev_due = schedule.next_due_ts;
            outstanding = outstanding.saturating_sub(r.min(outstanding));
        }
    }
}
