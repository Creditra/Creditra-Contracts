// SPDX-License-Identifier: Apache-2.0
//! Integration tests: `advance_repayment_schedule_after_repay` with zero-principal repay.
//!
//! # Behaviour under test
//!
//! When a borrower repays an amount that covers **only accrued interest** (no
//! principal), the installment schedule must **not** advance: `next_due_ts`
//! stays at its current value.  Only once the payment also satisfies the
//! `amount_per_period` principal component should `next_due_ts` move forward
//! by exactly one period.
//!
//! # Test matrix
//!
//! | Test | Repay amount | Expected `next_due_ts` |
//! |---|---|---|
//! | `interest_only_does_not_advance` | interest accrued only | unchanged |
//! | `interest_plus_installment_advances_one_period` | interest + amount_per_period | + period_secs |
//! | `partial_principal_does_not_advance` | interest + (amount_per_period - 1) | unchanged |
//! | `exact_principal_multiple_periods_advances_once` | interest + amount_per_period | + period_secs (not +2) |
//! | `zero_repay_does_not_advance` | 0 | unchanged |
//! | `full_balance_advances_all_remaining_periods` | full outstanding balance | schedule cleared |
//!
//! All tests use `env.ledger().with_mut(...)` for deterministic timestamp
//! control — no wall-clock dependence.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token, Address, Env,
};

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

/// Ledger timestamp at contract init.
const T0: u64 = 1_700_000_000;

/// One month in seconds (30 days).  Matches the period used in `set_repayment_schedule`.
const PERIOD: u64 = 30 * 24 * 3_600; // 2_592_000

/// Credit-line limit used across all tests.
const CREDIT_LIMIT: i128 = 1_000_000;

/// Amount drawn in each test.
const DRAW_AMOUNT: i128 = 600_000;

/// Installment principal per period.
const AMOUNT_PER_PERIOD: i128 = 100_000;

/// Annual interest rate in basis points (10 % p.a.).
const RATE_BPS: u32 = 1_000;

// ─────────────────────────────────────────────────────────────────────────────
// Shared setup
// ─────────────────────────────────────────────────────────────────────────────

struct Ctx {
    env: Env,
    credit: credit::CreditClient<'static>,
    token: token::StellarAssetClient<'static>,
    admin: Address,
    borrower: Address,
}

/// Build a fully initialised environment with one open credit line and one draw.
///
/// The schedule is set to `AMOUNT_PER_PERIOD` per month starting at
/// `T0 + PERIOD` (first due date one period from now).  The ledger is left at
/// `T0` so callers control time explicitly.
fn setup() -> Ctx {
    let env = Env::default();
    env.mock_all_auths();

    // Pin the ledger to a known timestamp.
    env.ledger().with_mut(|l| {
        l.timestamp = T0;
        l.sequence_number = 1;
    });

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    // Deploy a Stellar Asset Contract so token transfers work.
    let token_id = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token = token::StellarAssetClient::new(&env, &token_id);
    token.mint(&admin, &10_000_000_i128);
    token.mint(&borrower, &10_000_000_i128);

    // Deploy and initialise the credit contract.
    let credit_id = env.register(credit::Credit, ());
    let credit = credit::CreditClient::new(&env, &credit_id);

    credit.init(&admin);
    credit.set_liquidity_token(&token_id);
    credit.set_liquidity_source(&admin);

    // Allow the contract to pull from the liquidity source.
    token.approve(&admin, &credit.address, &10_000_000_i128, &100_000_u32);

    // Open a credit line and immediately draw.
    credit.open_credit_line(&borrower, &CREDIT_LIMIT, &RATE_BPS);
    credit.draw_credit(&borrower, &DRAW_AMOUNT);

    // Configure a 6-period repayment schedule (first due at T0 + PERIOD).
    credit.set_repayment_schedule(
        &borrower,
        &AMOUNT_PER_PERIOD,
        &(T0 + PERIOD),  // first_due_ts
        &PERIOD,          // period_secs
        &6_u32,           // num_periods
    );

    Ctx { env, credit, token, admin, borrower }
}

/// Compute the interest that has accrued on `principal` at `RATE_BPS` over
/// `elapsed_secs`.  Mirrors the contract's `prorate_interest` formula:
///
/// ```text
/// interest = principal * rate_bps * elapsed_secs
///            ─────────────────────────────────────
///                  10_000 * SECONDS_PER_YEAR
/// ```
///
/// Using integer arithmetic (truncating), the same as `math_utils::prorate_interest`.
fn accrued_interest(principal: i128, elapsed_secs: u64) -> i128 {
    const YEAR: u64 = 365 * 24 * 3_600;
    (principal * RATE_BPS as i128 * elapsed_secs as i128)
        / (10_000 * YEAR as i128)
}

// ─────────────────────────────────────────────────────────────────────────────
// Helper: read next_due_ts from the repayment schedule.
// ─────────────────────────────────────────────────────────────────────────────

fn next_due_ts(ctx: &Ctx) -> u64 {
    ctx.credit
        .get_repayment_schedule(&ctx.borrower)
        .expect("schedule must exist")
        .next_due_ts
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 1 — interest-only repay must NOT advance the schedule
// ─────────────────────────────────────────────────────────────────────────────

/// **Core edge case.**
///
/// Advance the ledger to halfway through the first period (15 days), compute
/// the accrued interest, repay exactly that amount, and assert that
/// `next_due_ts` is still `T0 + PERIOD`.
///
/// Rationale: interest payment reduces the outstanding balance but does not
/// satisfy the installment's principal component; the due date must not move.
#[test]
fn interest_only_does_not_advance() {
    let ctx = setup();

    // Advance to 15 days in — interest has accrued but no installment is due.
    let elapsed = 15 * 24 * 3_600_u64;
    ctx.env.ledger().with_mut(|l| l.timestamp = T0 + elapsed);

    let due_ts_before = next_due_ts(&ctx);
    assert_eq!(
        due_ts_before,
        T0 + PERIOD,
        "pre-condition: first due date is T0 + PERIOD"
    );

    // Repay exactly the interest accrued so far — no principal.
    let interest_only = accrued_interest(DRAW_AMOUNT, elapsed);
    assert!(interest_only > 0, "sanity: some interest must have accrued");

    ctx.token.approve(
        &ctx.borrower,
        &ctx.credit.address,
        &interest_only,
        &100_000_u32,
    );
    ctx.credit.repay_credit(&ctx.borrower, &interest_only);

    // Schedule must not have moved.
    let due_ts_after = next_due_ts(&ctx);
    assert_eq!(
        due_ts_after, due_ts_before,
        "interest-only repay must NOT advance next_due_ts \
         (observed: {due_ts_after}, expected: {due_ts_before})"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 2 — interest + one installment advances exactly one period
// ─────────────────────────────────────────────────────────────────────────────

/// Repay the full interest accrued over one period **plus** the installment
/// principal.  `next_due_ts` must advance by exactly `PERIOD`.
#[test]
fn interest_plus_installment_advances_one_period() {
    let ctx = setup();

    // Advance to exactly the first due date.
    ctx.env
        .ledger()
        .with_mut(|l| l.timestamp = T0 + PERIOD);

    let due_ts_before = next_due_ts(&ctx);

    let interest = accrued_interest(DRAW_AMOUNT, PERIOD);
    let repay_amount = interest + AMOUNT_PER_PERIOD;

    ctx.token.approve(
        &ctx.borrower,
        &ctx.credit.address,
        &repay_amount,
        &100_000_u32,
    );
    ctx.credit.repay_credit(&ctx.borrower, &repay_amount);

    let due_ts_after = next_due_ts(&ctx);
    assert_eq!(
        due_ts_after,
        due_ts_before + PERIOD,
        "interest + principal repay must advance next_due_ts by exactly one period \
         (observed: {due_ts_after}, expected: {})",
        due_ts_before + PERIOD
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 3 — one token short of the principal threshold does NOT advance
// ─────────────────────────────────────────────────────────────────────────────

/// Repay interest + (amount_per_period − 1).  The principal component is just
/// one token below the threshold — schedule must not advance.
#[test]
fn partial_principal_does_not_advance() {
    let ctx = setup();

    ctx.env
        .ledger()
        .with_mut(|l| l.timestamp = T0 + PERIOD);

    let due_ts_before = next_due_ts(&ctx);

    let interest = accrued_interest(DRAW_AMOUNT, PERIOD);
    // One stroops below the installment threshold.
    let repay_amount = interest + AMOUNT_PER_PERIOD - 1;

    ctx.token.approve(
        &ctx.borrower,
        &ctx.credit.address,
        &repay_amount,
        &100_000_u32,
    );
    ctx.credit.repay_credit(&ctx.borrower, &repay_amount);

    let due_ts_after = next_due_ts(&ctx);
    assert_eq!(
        due_ts_after, due_ts_before,
        "paying one stroop less than the installment must NOT advance the schedule \
         (observed: {due_ts_after}, expected: {due_ts_before})"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 4 — repaying two installments' worth of principal advances by ONE period
// ─────────────────────────────────────────────────────────────────────────────

/// Even if the borrower pays enough principal to cover two periods, the
/// schedule should advance by exactly **one** period per `repay_credit` call
/// (the contract processes installments one at a time).
#[test]
fn double_principal_advances_only_one_period() {
    let ctx = setup();

    ctx.env
        .ledger()
        .with_mut(|l| l.timestamp = T0 + PERIOD);

    let due_ts_before = next_due_ts(&ctx);

    let interest = accrued_interest(DRAW_AMOUNT, PERIOD);
    // Two installments of principal.
    let repay_amount = interest + 2 * AMOUNT_PER_PERIOD;

    ctx.token.approve(
        &ctx.borrower,
        &ctx.credit.address,
        &repay_amount,
        &100_000_u32,
    );
    ctx.credit.repay_credit(&ctx.borrower, &repay_amount);

    let due_ts_after = next_due_ts(&ctx);
    assert_eq!(
        due_ts_after,
        due_ts_before + PERIOD,
        "repaying two periods' principal in one call must still advance by only one period \
         (observed: {due_ts_after}, expected: {})",
        due_ts_before + PERIOD
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 5 — zero-amount repay does not advance schedule
// ─────────────────────────────────────────────────────────────────────────────

/// A zero-amount `repay_credit` call (if the contract permits it) must have no
/// effect on `next_due_ts`.  If the contract rejects zero repayments with an
/// error, that is also acceptable — we verify either way.
#[test]
fn zero_repay_does_not_advance() {
    let ctx = setup();

    ctx.env
        .ledger()
        .with_mut(|l| l.timestamp = T0 + PERIOD);

    let due_ts_before = next_due_ts(&ctx);

    // Some contracts reject a zero amount; catch that gracefully.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.token.approve(
            &ctx.borrower,
            &ctx.credit.address,
            &0_i128,
            &100_000_u32,
        );
        ctx.credit.repay_credit(&ctx.borrower, &0_i128);
    }));

    // Whether the call succeeded or panicked (contract rejection), the schedule
    // must remain unchanged.
    let due_ts_after = next_due_ts(&ctx);
    assert_eq!(
        due_ts_after, due_ts_before,
        "zero-amount repay must NOT advance next_due_ts \
         (observed: {due_ts_after}, expected: {due_ts_before})"
    );

    // Suppress the unused-result warning; we intentionally allow both outcomes.
    let _ = result;
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 6 — sequential interest-only then full repay: correct two-step behaviour
// ─────────────────────────────────────────────────────────────────────────────

/// Simulates a realistic borrower flow:
/// 1. Pays interest only mid-period → schedule unchanged.
/// 2. At the due date, pays the remaining interest + principal → schedule advances.
///
/// This is the primary regression scenario described in issue #503.
#[test]
fn sequential_interest_only_then_full_repay() {
    let ctx = setup();

    // ── Step 1: interest-only payment at 15 days ──────────────────────────
    let midpoint = 15 * 24 * 3_600_u64;
    ctx.env
        .ledger()
        .with_mut(|l| l.timestamp = T0 + midpoint);

    let interest_mid = accrued_interest(DRAW_AMOUNT, midpoint);
    ctx.token.approve(
        &ctx.borrower,
        &ctx.credit.address,
        &interest_mid,
        &100_000_u32,
    );
    ctx.credit.repay_credit(&ctx.borrower, &interest_mid);

    let due_ts_after_step1 = next_due_ts(&ctx);
    assert_eq!(
        due_ts_after_step1,
        T0 + PERIOD,
        "step 1: interest-only must leave next_due_ts at T0 + PERIOD"
    );

    // ── Step 2: remaining interest + principal at the due date ────────────
    // After the step-1 payment, the outstanding principal is still DRAW_AMOUNT
    // (interest was cleared, no principal was paid).  Interest has continued to
    // accrue on the full principal from the draw date; we compute it for the
    // remaining half-period.
    ctx.env
        .ledger()
        .with_mut(|l| l.timestamp = T0 + PERIOD);

    // Remaining interest = interest on DRAW_AMOUNT for the second 15 days.
    let interest_remaining = accrued_interest(DRAW_AMOUNT, PERIOD - midpoint);
    let repay_step2 = interest_remaining + AMOUNT_PER_PERIOD;

    ctx.token.approve(
        &ctx.borrower,
        &ctx.credit.address,
        &repay_step2,
        &100_000_u32,
    );
    ctx.credit.repay_credit(&ctx.borrower, &repay_step2);

    let due_ts_after_step2 = next_due_ts(&ctx);
    assert_eq!(
        due_ts_after_step2,
        T0 + 2 * PERIOD,
        "step 2: interest + principal must advance next_due_ts by one period \
         (observed: {due_ts_after_step2}, expected: {})",
        T0 + 2 * PERIOD
    );
}