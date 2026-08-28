// SPDX-License-Identifier: MIT

//! Property tests: borrow state invariants for the v7 borrow subsystem.
//!
//! # What
//!
//! Generates arbitrary sequences of [`draw_credit`] and [`repay_credit`] actions
//! across multiple borrowers and verifies that fundamental accounting and state
//! invariants hold after every successful state change.
//!
//! # Invariants verified
//!
//! | ID  | Invariant                                                       | Scope        |
//! |-----|-----------------------------------------------------------------|--------------|
//! | I1  | `0 ≤ utilized_amount ≤ credit_limit`                           | per-borrower |
//! | I2  | `accrued_interest ≥ 0`                                         | per-borrower |
//! | I3  | `interest_rate_bps ≤ 10_000`                                   | per-borrower |
//! | I4  | `get_total_utilized() == Σ utilized_amount` across open lines  | global       |
//! | I5  | After repay: `utilized_amount ≤ utilized_before`               | per-borrower |
//! | I6  | After draw: `utilized_amount ≥ utilized_before + amount`       | per-borrower |
//! | I7  | Borrower A's operations never affect Borrower B's utilization  | cross-borrower |
//!
//! # Edge cases covered (deterministic)
//!
//! - Zero-amount draw & repay → `InvalidAmount` (#5)
//! - Draw on non-existent line → `CreditLineNotFound` (#3)
//! - Draw exceeding credit limit → `OverLimit` (#6)
//! - Repay on non-existent line → `CreditLineNotFound` (#3)
//! - Repay on closed credit line → `CreditLineClosed` (#4)
//! - Overpayment is capped and does not make utilization negative
//! - Draw with insufficient collateral → `CollateralRatioBelowMinimum` (#35)
//! - Full repayment zeros out utilization and `TotalUtilized`
//!
//! # References
//!
//! - [`creditra_credit::Credit::draw_credit`]
//! - [`creditra_credit::Credit::repay_credit`]
//! - Issue #845

use creditra_credit::types::CreditLineData;
use creditra_credit::{Credit, CreditClient};
use proptest::collection::vec as proptest_vec;
use proptest::prelude::*;
use proptest::test_runner::{Config as ProptestConfig, TestCaseError, TestCaseResult};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Env};

// ── Constants ──────────────────────────────────────────────────────────────

const BORROWER_COUNT: usize = 3;
const MAX_STEPS: usize = 48;
const CREDIT_LIMITS: [i128; BORROWER_COUNT] = [20_000, 35_000, 50_000];
const COLLATERAL_AMOUNTS: [i128; BORROWER_COUNT] = [30_000, 52_500, 75_000];
const INTEREST_RATE_BPS: u32 = 500;
const RISK_SCORE: u32 = 50;
const INITIAL_TOKEN_BALANCE: i128 = 2_000_000;
const INITIAL_TIMESTAMP: u64 = 1_000;

// ── Types ──────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
enum Action {
    Draw { borrower_index: usize, amount: i128 },
    Repay { borrower_index: usize, amount: i128 },
}

#[derive(Clone, Debug)]
struct RawStep {
    borrower_index: usize,
    wants_draw: bool,
    requested_amount: i128,
}

struct TestCtx {
    env: Env,
    _contract_id: Address,
    borrowers: Vec<Address>,
    credit_limits: [i128; BORROWER_COUNT],
}

impl TestCtx {
    fn client(&self) -> CreditClient<'_> {
        CreditClient::new(&self.env, &self._contract_id)
    }
}

// ── Setup ──────────────────────────────────────────────────────────────────

fn setup() -> TestCtx {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    env.ledger().set_timestamp(INITIAL_TIMESTAMP);

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&contract_id);

    let asset = StellarAssetClient::new(&env, &token);
    asset.mint(&contract_id, &INITIAL_TOKEN_BALANCE);

    let token_client = TokenClient::new(&env, &token);
    let mut borrowers = Vec::with_capacity(BORROWER_COUNT);

    for i in 0..BORROWER_COUNT {
        let borrower = Address::generate(&env);
        asset.mint(&borrower, &INITIAL_TOKEN_BALANCE);
        let approve_expiration = env.ledger().sequence() + 100_000;
        token_client.approve(
            &borrower,
            &contract_id,
            &INITIAL_TOKEN_BALANCE,
            &approve_expiration,
        );

        client.deposit_collateral(&borrower, &COLLATERAL_AMOUNTS[i]);
        client.open_credit_line(
            &borrower,
            &CREDIT_LIMITS[i],
            &INTEREST_RATE_BPS,
            &RISK_SCORE,
        );

        borrowers.push(borrower);
    }

    TestCtx {
        env,
        _contract_id: contract_id,
        borrowers,
        credit_limits: CREDIT_LIMITS,
    }
}

// ── Strategies ─────────────────────────────────────────────────────────────

fn raw_steps_strategy() -> impl Strategy<Value = Vec<RawStep>> {
    proptest_vec(
        (0usize..BORROWER_COUNT, any::<bool>(), 1_i128..=10_000_i128),
        1..=MAX_STEPS,
    )
    .prop_map(|steps| {
        steps
            .into_iter()
            .map(|(borrower_index, wants_draw, requested_amount)| RawStep {
                borrower_index,
                wants_draw,
                requested_amount,
            })
            .collect()
    })
}

// ── Invariant helpers ──────────────────────────────────────────────────────

fn enforce_per_borrower_invariants(ctx: &TestCtx, modeled: &[i128]) -> TestCaseResult {
    let client = ctx.client();
    for (i, borrower) in ctx.borrowers.iter().enumerate() {
        let line: CreditLineData = client
            .get_credit_line(borrower)
            .expect("credit line must exist for every borrower in the harness");

        // I1: 0 <= utilized_amount <= credit_limit
        prop_assert!(
            line.utilized_amount >= 0,
            "I1 violation — borrower {}: utilized_amount ({}) is negative",
            i,
            line.utilized_amount,
        );
        prop_assert!(
            line.utilized_amount <= line.credit_limit,
            "I1 violation — borrower {}: utilized_amount ({}) exceeds credit_limit ({})",
            i,
            line.utilized_amount,
            line.credit_limit,
        );

        // I2: accrued_interest >= 0
        prop_assert!(
            line.accrued_interest >= 0,
            "I2 violation — borrower {}: accrued_interest ({}) is negative",
            i,
            line.accrued_interest,
        );

        // I3: interest_rate_bps <= 10_000
        prop_assert!(
            line.interest_rate_bps <= 10_000,
            "I3 violation — borrower {}: interest_rate_bps ({}) exceeds 10_000",
            i,
            line.interest_rate_bps,
        );

        // Model consistency: modeled utilization matches on-chain value
        prop_assert_eq!(
            line.utilized_amount,
            modeled[i],
            "model mismatch — borrower {}: on-chain utilized ({}) != modeled ({})",
            i,
            line.utilized_amount,
            modeled[i],
        );
    }
    Ok(())
}

fn assert_total_utilized_invariant(ctx: &TestCtx, modeled: &[i128]) -> TestCaseResult {
    let client = ctx.client();
    let computed_total: i128 = modeled.iter().try_fold(0_i128, |acc, &v| {
        acc.checked_add(v)
            .ok_or_else(|| TestCaseError::fail("I4: computed total overflow"))
    })?;
    let stored_total = client.get_total_utilized();

    prop_assert_eq!(
        stored_total,
        computed_total,
        "I4 violation — stored total_utilized ({}) != sum of modeled utilization ({})",
        stored_total,
        computed_total,
    );

    let mut recomputed_total = 0_i128;
    for (_i, borrower) in ctx.borrowers.iter().enumerate() {
        let line: CreditLineData = client
            .get_credit_line(borrower)
            .expect("credit line must exist");
        recomputed_total = recomputed_total
            .checked_add(line.utilized_amount)
            .ok_or_else(|| TestCaseError::fail("I4: recomputed total overflow"))?;
    }

    prop_assert_eq!(
        stored_total,
        recomputed_total,
        "I4 violation — stored total_utilized ({}) != live sum of credit lines ({})",
        stored_total,
        recomputed_total,
    );

    Ok(())
}

// ── Action materialization ─────────────────────────────────────────────────

fn materialize_draw(
    ctx: &TestCtx,
    modeled: &[i128],
    step: &RawStep,
) -> Result<(Action, i128), TestCaseError> {
    let i = step.borrower_index;
    let utilized = modeled[i];
    let remaining = ctx.credit_limits[i]
        .checked_sub(utilized)
        .ok_or_else(|| TestCaseError::fail("draw: remaining credit underflow"))?;

    if remaining <= 0 {
        return Err(TestCaseError::fail("draw: no remaining credit"));
    }

    let amount = step.requested_amount.min(remaining).max(1);
    Ok((
        Action::Draw {
            borrower_index: i,
            amount,
        },
        amount,
    ))
}

fn materialize_repay(modeled: &[i128], step: &RawStep) -> Result<(Action, i128), TestCaseError> {
    let i = step.borrower_index;
    let utilized = modeled[i];

    if utilized <= 0 {
        return Err(TestCaseError::fail("repay: nothing owed"));
    }

    let amount = step.requested_amount.min(utilized).max(1);
    Ok((
        Action::Repay {
            borrower_index: i,
            amount,
        },
        amount,
    ))
}

fn materialize_action(
    ctx: &TestCtx,
    modeled: &[i128],
    step: &RawStep,
) -> Result<(Action, i128), TestCaseError> {
    if modeled[step.borrower_index] <= 0 {
        materialize_draw(ctx, modeled, step)
    } else if modeled[step.borrower_index] >= ctx.credit_limits[step.borrower_index] {
        materialize_repay(modeled, step)
    } else if step.wants_draw {
        materialize_draw(ctx, modeled, step)
    } else {
        materialize_repay(modeled, step)
    }
}

// ── Step application ──────────────────────────────────────────────────────

fn apply_valid_step(ctx: &TestCtx, modeled: &mut [i128], step: &RawStep) -> TestCaseResult {
    let borrower_index = step.borrower_index;
    let borrower = &ctx.borrowers[borrower_index];

    let (action, amount) = materialize_action(ctx, modeled, step)?;
    prop_assert!(amount > 0, "materialized amount must be positive");

    let client = ctx.client();

    match action {
        Action::Draw { .. } => {
            let line_before: CreditLineData = client
                .get_credit_line(borrower)
                .expect("line must exist before draw");
            let utilized_onchain_before = line_before.utilized_amount;

            client.draw_credit(borrower, &amount);

            let line_after: CreditLineData = client
                .get_credit_line(borrower)
                .expect("line must exist after draw");

            // I6: utilized_amount increased by at least amount
            prop_assert!(
                line_after.utilized_amount >= utilized_onchain_before + amount,
                "I6 violation — draw did not increase utilization: \
                 before={}, after={}, amount={}",
                utilized_onchain_before,
                line_after.utilized_amount,
                amount,
            );

            modeled[borrower_index] = modeled[borrower_index]
                .checked_add(amount)
                .ok_or_else(|| TestCaseError::fail("modeled draw overflow"))?;
        }
        Action::Repay { .. } => {
            let line_before: CreditLineData = client
                .get_credit_line(borrower)
                .expect("line must exist before repay");
            let utilized_onchain_before = line_before.utilized_amount;

            client.repay_credit(borrower, &amount);

            let line_after: CreditLineData = client
                .get_credit_line(borrower)
                .expect("line must exist after repay");

            // I5: utilized_amount never increases on repay
            prop_assert!(
                line_after.utilized_amount <= utilized_onchain_before,
                "I5 violation — repay increased utilization: \
                 before={}, after={}",
                utilized_onchain_before,
                line_after.utilized_amount,
            );

            modeled[borrower_index] = modeled[borrower_index]
                .checked_sub(amount)
                .unwrap_or(0)
                .max(0);
        }
    }

    // I7: other borrowers' state is unchanged
    for (j, borrower_j) in ctx.borrowers.iter().enumerate() {
        if j != borrower_index {
            let line_j: CreditLineData =
                client.get_credit_line(borrower_j).expect("line must exist");
            prop_assert_eq!(
                line_j.utilized_amount,
                modeled[j],
                "I7 violation — borrower {}'s utilization changed after operating on borrower {}",
                j,
                borrower_index,
            );
        }
    }

    // Global invariants
    enforce_per_borrower_invariants(ctx, modeled)?;
    assert_total_utilized_invariant(ctx, modeled)?;

    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 1 — Property-based test: borrow state invariants
// ═══════════════════════════════════════════════════════════════════════════

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 128,
        .. ProptestConfig::default()
    })]

    /// Sequences of draw/repay actions across 3 borrowers.
    ///
    /// After every successful mutation, all invariants I1–I7 are checked.
    #[test]
    fn borrow_state_invariants_hold_after_draw_repay_sequences(
        steps in raw_steps_strategy(),
    ) {
        let ctx = setup();
        let mut modeled = vec![0_i128; BORROWER_COUNT];

        // Verify initial state before any operations
        enforce_per_borrower_invariants(&ctx, &modeled)?;
        assert_total_utilized_invariant(&ctx, &modeled)?;

        for step in &steps {
            apply_valid_step(&ctx, &mut modeled, step)?;
        }

        // Final check after all steps
        enforce_per_borrower_invariants(&ctx, &modeled)?;
        assert_total_utilized_invariant(&ctx, &modeled)?;
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 2 — Deterministic edge case tests
// ═══════════════════════════════════════════════════════════════════════════

/// Helper to extract error string from panic payload.
fn extract_error_str(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else {
        String::new()
    }
}

/// Deploy a minimal contract with an open credit line for a single borrower.
fn setup_single_borrower(env: &Env) -> (CreditClient<'_>, Address) {
    env.mock_all_auths();
    env.ledger().set_timestamp(INITIAL_TIMESTAMP);

    let admin = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&contract_id);
    StellarAssetClient::new(env, &token).mint(&contract_id, &INITIAL_TOKEN_BALANCE);

    let borrower = Address::generate(env);
    StellarAssetClient::new(env, &token).mint(&borrower, &INITIAL_TOKEN_BALANCE);
    let approve_expiration = env.ledger().sequence() + 100_000;
    TokenClient::new(env, &token).approve(
        &borrower,
        &contract_id,
        &INITIAL_TOKEN_BALANCE,
        &approve_expiration,
    );

    client.deposit_collateral(&borrower, &100_000);
    client.open_credit_line(&borrower, &10_000, &INTEREST_RATE_BPS, &RISK_SCORE);

    (client, borrower)
}

// ── Zero-amount draw → InvalidAmount (#5) ─────────────────────────────────

#[test]
fn draw_zero_amount_fails_with_invalid_amount() {
    let env = Env::default();
    let (client, borrower) = setup_single_borrower(&env);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.draw_credit(&borrower, &0_i128);
    }));
    assert!(result.is_err(), "draw with zero amount must revert");
    let err_str = extract_error_str(&result.unwrap_err());
    assert!(
        err_str.contains("#5"),
        "expected InvalidAmount (#5), got: {err_str:?}"
    );
}

// ── Zero-amount repay → InvalidAmount (#5) ────────────────────────────────

#[test]
fn repay_zero_amount_fails_with_invalid_amount() {
    let env = Env::default();
    let (client, borrower) = setup_single_borrower(&env);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.repay_credit(&borrower, &0_i128);
    }));
    assert!(result.is_err(), "repay with zero amount must revert");
    let err_str = extract_error_str(&result.unwrap_err());
    assert!(
        err_str.contains("#5"),
        "expected InvalidAmount (#5), got: {err_str:?}"
    );
}

// ── Draw on non-existent line → CreditLineNotFound (#3) ───────────────────

#[test]
fn draw_nonexistent_line_fails_with_not_found() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(INITIAL_TIMESTAMP);

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let random_borrower = Address::generate(&env);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.draw_credit(&random_borrower, &100_i128);
    }));
    assert!(result.is_err(), "draw on non-existent line must revert");
    let err_str = extract_error_str(&result.unwrap_err());
    assert!(
        err_str.contains("#3"),
        "expected CreditLineNotFound (#3), got: {err_str:?}"
    );
}

// ── Repay on non-existent line → CreditLineNotFound (#3) ──────────────────

#[test]
fn repay_nonexistent_line_fails_with_not_found() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let random_borrower = Address::generate(&env);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.repay_credit(&random_borrower, &100_i128);
    }));
    assert!(result.is_err(), "repay on non-existent line must revert");
    let err_str = extract_error_str(&result.unwrap_err());
    assert!(
        err_str.contains("#3"),
        "expected CreditLineNotFound (#3), got: {err_str:?}"
    );
}

// ── Draw exceeding credit limit → OverLimit (#6) ──────────────────────────

#[test]
fn draw_over_limit_fails_with_over_limit() {
    let env = Env::default();
    let (client, borrower) = setup_single_borrower(&env);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.draw_credit(&borrower, &10_001_i128);
    }));
    assert!(result.is_err(), "draw over limit must revert");
    let err_str = extract_error_str(&result.unwrap_err());
    assert!(
        err_str.contains("#6"),
        "expected OverLimit (#6), got: {err_str:?}"
    );
}

// ── Insufficient collateral → CollateralRatioBelowMinimum (#35) ───────────

#[test]
fn draw_with_insufficient_collateral_fails() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(INITIAL_TIMESTAMP);

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&contract_id);
    let asset = StellarAssetClient::new(&env, &token);
    asset.mint(&contract_id, &INITIAL_TOKEN_BALANCE);
    asset.mint(&borrower, &10_000_i128);

    // Deposit very little collateral relative to the credit limit
    client.deposit_collateral(&borrower, &100_i128);
    client.open_credit_line(&borrower, &10_000, &INTEREST_RATE_BPS, &RISK_SCORE);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.draw_credit(&borrower, &1_000_i128);
    }));
    assert!(
        result.is_err(),
        "draw with insufficient collateral must revert"
    );
    let err_str = extract_error_str(&result.unwrap_err());
    assert!(
        err_str.contains("#35"),
        "expected CollateralRatioBelowMinimum (#35), got: {err_str:?}"
    );
}

// ── Repay on closed line → CreditLineClosed (#4) ─────────────────────────

#[test]
fn repay_closed_line_fails_with_closed() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(INITIAL_TIMESTAMP);

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&contract_id);
    let asset = StellarAssetClient::new(&env, &token);
    asset.mint(&contract_id, &INITIAL_TOKEN_BALANCE);
    asset.mint(&borrower, &200_000_i128);
    client.deposit_collateral(&borrower, &100_000);
    client.open_credit_line(&borrower, &10_000, &INTEREST_RATE_BPS, &RISK_SCORE);
    client.close_credit_line(&borrower, &admin);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.repay_credit(&borrower, &100_i128);
    }));
    assert!(result.is_err(), "repay on closed line must revert");
    let err_str = extract_error_str(&result.unwrap_err());
    assert!(
        err_str.contains("#4"),
        "expected CreditLineClosed (#4), got: {err_str:?}"
    );
}

// ── Overpayment is capped — utilization goes to zero, not negative ───────

#[test]
fn overpayment_caps_at_zero_utilization() {
    let env = Env::default();
    let (client, borrower) = setup_single_borrower(&env);

    client.draw_credit(&borrower, &5_000);

    let line_before: CreditLineData = client.get_credit_line(&borrower).expect("line must exist");
    assert_eq!(line_before.utilized_amount, 5_000);

    // Repay way more than owed
    client.repay_credit(&borrower, &100_000);

    let line_after: CreditLineData = client
        .get_credit_line(&borrower)
        .expect("line must exist after repay");
    assert_eq!(
        line_after.utilized_amount, 0,
        "overpayment must leave utilization at 0"
    );
    assert_eq!(
        client.get_total_utilized(),
        0,
        "total_utilized must be 0 after full repayment"
    );
}

// ── Full repay after draw zeros out utilization and total_utilized ───────

#[test]
fn full_repay_zeros_utilization_and_total() {
    let env = Env::default();
    let (client, borrower) = setup_single_borrower(&env);

    client.draw_credit(&borrower, &3_000);

    let line_before: CreditLineData = client.get_credit_line(&borrower).expect("line must exist");
    assert_eq!(line_before.utilized_amount, 3_000);
    assert_eq!(client.get_total_utilized(), 3_000);

    client.repay_credit(&borrower, &3_000);

    let line_after: CreditLineData = client
        .get_credit_line(&borrower)
        .expect("line must exist after repay");
    assert_eq!(line_after.utilized_amount, 0);
    assert_eq!(client.get_total_utilized(), 0);
}

// ── Multi-borrower deterministic sequence ────────────────────────────────

#[test]
fn multi_borrower_sequence_preserves_invariants() {
    let ctx = setup();
    let client = ctx.client();
    let mut modeled = vec![0_i128; BORROWER_COUNT];

    // Initial state
    enforce_per_borrower_invariants(&ctx, &modeled).unwrap();
    assert_total_utilized_invariant(&ctx, &modeled).unwrap();

    // Draw on each borrower
    for i in 0..BORROWER_COUNT {
        let amount = ctx.credit_limits[i] / 2;
        client.draw_credit(&ctx.borrowers[i], &amount);
        modeled[i] = amount;

        enforce_per_borrower_invariants(&ctx, &modeled).unwrap();
        assert_total_utilized_invariant(&ctx, &modeled).unwrap();
    }

    // Partial repay on each borrower
    for i in 0..BORROWER_COUNT {
        let repay_amount = modeled[i] / 3;
        client.repay_credit(&ctx.borrowers[i], &repay_amount);
        modeled[i] = modeled[i].saturating_sub(repay_amount).max(0);

        enforce_per_borrower_invariants(&ctx, &modeled).unwrap();
        assert_total_utilized_invariant(&ctx, &modeled).unwrap();
    }

    // Full repay on first borrower
    client.repay_credit(&ctx.borrowers[0], &modeled[0]);
    modeled[0] = 0;

    enforce_per_borrower_invariants(&ctx, &modeled).unwrap();
    assert_total_utilized_invariant(&ctx, &modeled).unwrap();
}
