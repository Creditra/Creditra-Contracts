// SPDX-License-Identifier: MIT

//! Property-based tests for accrual (v7) state invariants.
//!
//! # What
//!
//! Generates random sequences of draw, repay, and time-advance operations across
//! multiple borrowers and verifies that key accrual invariants hold after every
//! mutation.
//!
//! # Invariants
//!
//! 1. **Accrued ≤ Utilized**: `0 <= accrued_interest <= utilized_amount` for
//!    every line after any operation that triggers `apply_accrual`.
//! 2. **Monotonic total utilized**: `total_utilized` never decreases when only
//!    draws and accruals occur (repays may decrease it, but draw + accrual
//!    must increase monotonically).
//! 3. **Batch consistency**: Running `accrue_batch` on a list of borrowers
//!    produces the same per-line results as calling `update_risk_parameters`
//!    (which triggers `apply_accrual`) on each borrower individually with
//!    the same parameters.
//! 4. **Zero utilization = zero accrual**: A line with `utilized_amount == 0`
//!    must never accrue interest regardless of elapsed time or rate.
//!
//! # Covered paths
//!
//! | Path                  | Why it matters                                    |
//! |-----------------------|---------------------------------------------------|
//! | `draw_credit`         | Triggers `apply_accrual`; increases principal     |
//! | `repay_credit`        | Interest-first allocation; partial + over-repay   |
//! | `update_risk_parameters` | Triggers `apply_accrual`; may alter limits      |
//! | `accrue_batch`        | Batched accrual across multiple borrowers         |
//! | Time advancement      | Drives interest accumulation between mutations    |
//! | Multiple borrowers    | Ensures invariant holds across all lines          |
//!
//! # See also
//! - `contracts/credit/tests/proptest_accrual.rs` — credit-level accrual proptest.
//! - `contracts/credit/tests/proptest_accrual_monotonic.rs` — monotonicity invariants.

use creditra_credit::types::CreditStatus;
use creditra_credit::{Credit, CreditClient};
use proptest::prelude::*;
use proptest::test_runner::Config as ProptestConfig;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{Address, Env, Vec};

const BORROWER_COUNT: usize = 3;
const MAX_STEPS: usize = 24;
const INITIAL_TIMESTAMP: u64 = 1_000;
const LIQUIDITY_AMOUNT: i128 = 10_000_000;

fn setup_env() -> (Env, CreditClient<'static>, std::vec::Vec<Address>) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(INITIAL_TIMESTAMP);

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&contract_id);

    let sac = StellarAssetClient::new(&env, &token);
    sac.mint(&contract_id, &LIQUIDITY_AMOUNT);

    let mut borrowers = std::vec::Vec::with_capacity(BORROWER_COUNT);
    for i in 0..BORROWER_COUNT {
        let borrower = Address::generate(&env);
        sac.mint(&borrower, &50_000_000_i128);
        let credit_limit = 50_000_i128 + (i as i128 * 20_000_i128);
        let rate_bps = 1_000_u32 + (i as u32 * 500_u32);
        let score = 30_u32 + (i as u32 * 10_u32);
        client.open_credit_line(&borrower, &credit_limit, &rate_bps, &score);
        borrowers.push(borrower);
    }

    (env, client, borrowers)
}

/// Assert `0 <= accrued_interest <= utilized_amount` for every active line.
fn assert_accrued_le_utilized(client: &CreditClient<'_>, label: &str) {
    let mut cursor = None;
    loop {
        let page = client.enumerate_credit_lines(&cursor, &8);
        if page.is_empty() {
            break;
        }
        for item in page.iter() {
            let (_, line) = item;
            assert!(
                line.accrued_interest >= 0,
                "{label}: accrued_interest is negative ({}) for borrower {:?}",
                line.accrued_interest,
                line.borrower,
            );
            assert!(
                line.accrued_interest <= line.utilized_amount,
                "{label}: accrued_interest ({}) > utilized_amount ({}) for borrower {:?}",
                line.accrued_interest,
                line.utilized_amount,
                line.borrower,
            );
        }
    }
}

/// Assert `total_utilized` >= 0 (sanity check).
fn assert_total_utilized_non_negative(client: &CreditClient<'_>, label: &str) {
    let total = client.total_utilized();
    assert!(total >= 0, "{label}: total_utilized is negative: {total}");
}

// ── Operation types for random sequences ──────────────────────────────────

#[derive(Debug, Clone)]
struct OpStep {
    borrower_index: usize,
    op: OpKind,
    amount: i128,
    time_advance: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum OpKind {
    Draw,
    Repay,
    UpdateRisk,
    AccrueBatch,
    Noop, // no-op / time advance only
}

fn op_strategy() -> impl Strategy<Value = std::vec::Vec<OpStep>> {
    proptest::collection::vec(
        (
            0usize..BORROWER_COUNT,
            (0u64..=4u64),
            1_i128..=10_000_i128,
            1u64..=31_536_000u64,
        ),
        1..=MAX_STEPS,
    )
    .prop_map(|steps| {
        steps
            .into_iter()
            .map(|(borrower_index, op, amount, time_advance)| {
                let op = match op {
                    0 => OpKind::Draw,
                    1 => OpKind::Repay,
                    2 => OpKind::UpdateRisk,
                    3 => OpKind::AccrueBatch,
                    _ => OpKind::Noop,
                };
                OpStep {
                    borrower_index,
                    op,
                    amount,
                    time_advance,
                }
            })
            .collect()
    })
}

// ═══════════════════════════════════════════════════════════════════════════
// Proptest 1 — accrued_interest ≤ utilized_amount invariant
// ═══════════════════════════════════════════════════════════════════════════

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 128,
        .. ProptestConfig::default()
    })]

    /// After every draw, repay, risk-update, or batch-accrue operation, the
    /// invariant `accrued_interest <= utilized_amount` must hold for every
    /// credit line.
    #[test]
    fn prop_accrued_never_exceeds_utilized(
        steps in op_strategy(),
    ) {
        let (env, client, borrowers) = setup_env();

        assert_accrued_le_utilized(&client, "initial");
        assert_total_utilized_non_negative(&client, "initial");

        for (step_idx, step) in steps.iter().enumerate() {
            let borrower = &borrowers[step.borrower_index];

            env.ledger().with_mut(|l| l.timestamp += step.time_advance);

            match step.op {
                OpKind::Draw => {
                    if let Some(line) = client.get_credit_line(borrower) {
                        if line.status == CreditStatus::Active {
                            let headroom = (line.credit_limit - line.utilized_amount).max(1);
                            let amount = step.amount.min(headroom.min(10_000));
                            let _ = client.try_draw_credit(borrower, &amount);
                        }
                    }
                }
                OpKind::Repay => {
                    if let Some(line) = client.get_credit_line(borrower) {
                        if line.utilized_amount > 0 {
                            let amount = step.amount.min(line.utilized_amount + 5_000);
                            let _ = client.try_repay_credit(borrower, &amount);
                        }
                    }
                }
                OpKind::UpdateRisk => {
                    let _ = client.try_update_risk_parameters(
                        borrower,
                        &(50_000_i128 + (step.borrower_index as i128 * 10_000_i128)),
                        &(500_u32 + (step.borrower_index as u32 * 200_u32)),
                        &(30_u32 + (step.borrower_index as u32 * 5_u32)),
                    );
                }
                OpKind::AccrueBatch => {
                    let mut batch: Vec<Address> = Vec::new(&env);
                    for b in &borrowers {
                        batch.push_back(b.clone());
                    }
                    let _ = client.try_accrue_batch(&batch);
                }
                OpKind::Noop => {
                    // time-only step, no operation.
                }
            }

            let label = std::format!("step={} op={:?}", step_idx, step.op);
            assert_accrued_le_utilized(&client, &label);
            assert_total_utilized_non_negative(&client, &label);
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Proptest 2 — batch accrual consistency
// ═══════════════════════════════════════════════════════════════════════════

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        .. ProptestConfig::default()
    })]

    /// Batch accrual via `accrue_batch` must produce the same per-borrower
    /// state as individual `update_risk_parameters` calls that trigger
    /// `apply_accrual` with identical parameters.
    #[test]
    fn prop_batch_consistent_with_individual_accrual(
        time_advance in 86_400u64..=31_536_000u64,
    ) {
        // Use two identical environments.
        let (env_a, client_a, borrowers_a) = setup_env();
        let (env_b, client_b, borrowers_b) = setup_env();

        // Draw on all borrowers in both environments.
        for i in 0..BORROWER_COUNT {
            client_a.draw_credit(&borrowers_a[i], &(5_000_i128 + (i as i128 * 2_000_i128)));
            client_b.draw_credit(&borrowers_b[i], &(5_000_i128 + (i as i128 * 2_000_i128)));
        }

        // Advance time identically.
        env_a.ledger().with_mut(|l| l.timestamp += time_advance);
        env_b.ledger().with_mut(|l| l.timestamp += time_advance);

        // Env A: batch accrual.
        let mut batch: Vec<Address> = Vec::new(&env_a);
        for b in &borrowers_a {
            batch.push_back(b.clone());
        }
        client_a.accrue_batch(&batch);

        // Env B: individual update_risk_parameters (triggers apply_accrual).
        for i in 0..BORROWER_COUNT {
            let line = client_b.get_credit_line(&borrowers_b[i]).unwrap();
            let _ = client_b.try_update_risk_parameters(
                &borrowers_b[i],
                &line.credit_limit,
                &line.interest_rate_bps,
                &line.risk_score,
            );
        }

        // Verify identical per-borrower state.
        for i in 0..BORROWER_COUNT {
            let line_a = client_a.get_credit_line(&borrowers_a[i]).unwrap();
            let line_b = client_b.get_credit_line(&borrowers_b[i]).unwrap();
            assert_eq!(
                line_a.utilized_amount, line_b.utilized_amount,
                "batch vs individual: borrower {i} utilized_amount mismatch (advance={time_advance}s)"
            );
            assert_eq!(
                line_a.accrued_interest, line_b.accrued_interest,
                "batch vs individual: borrower {i} accrued_interest mismatch (advance={time_advance}s)"
            );
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Edge case tests
// ═══════════════════════════════════════════════════════════════════════════

/// Zero utilization must never accrue interest regardless of elapsed time.
#[test]
fn zero_utilization_zero_accrual() {
    let (env, client, borrowers) = setup_env();
    let borrower = &borrowers[0];

    // Advance 1 year without any draw.
    env.ledger().with_mut(|l| l.timestamp += 31_536_000);
    client.update_risk_parameters(borrower, &50_000_i128, &10_000_u32, &50_u32);

    let line = client.get_credit_line(borrower).unwrap();
    assert_eq!(line.accrued_interest, 0, "zero utilization must produce zero interest");
    assert_eq!(line.utilized_amount, 0);
}

/// Draw, wait a full year at max rate, verify accrued ≤ utilized.
#[test]
fn max_rate_one_year_bound() {
    let (env, client, borrowers) = setup_env();
    let borrower = &borrowers[0];

    // Reopen with max rate (100% APR).
    client.open_credit_line(borrower, &1_000_000_i128, &10_000_u32, &50_u32);
    client.draw_credit(borrower, &100_000_i128);

    env.ledger().with_mut(|l| l.timestamp += 31_536_000);
    client.update_risk_parameters(borrower, &1_000_000_i128, &10_000_u32, &50_u32);

    let line = client.get_credit_line(borrower).unwrap();
    assert!(line.accrued_interest > 0, "should have accrued interest at max rate");
    assert!(line.accrued_interest <= line.utilized_amount,
        "accrued_interest ({}) must not exceed utilized_amount ({})",
        line.accrued_interest, line.utilized_amount);
}

/// Multiple accruals without repayment must accumulate monotonically.
#[test]
fn accrual_monotonic_without_repayment() {
    let (env, client, borrowers) = setup_env();
    let borrower = &borrowers[0];

    client.draw_credit(borrower, &10_000_i128);

    let mut prev_utilized = 0_i128;
    for quarter in 1..=4 {
        env.ledger().with_mut(|l| l.timestamp += 7_884_000); // ~1 quarter
        client.update_risk_parameters(borrower, &50_000_i128, &500_u32, &30_u32);

        let line = client.get_credit_line(borrower).unwrap();
        assert!(line.utilized_amount >= prev_utilized,
            "quarter {quarter}: utilized_amount ({}) < previous ({})",
            line.utilized_amount, prev_utilized);
        assert!(line.accrued_interest <= line.utilized_amount,
            "quarter {quarter}: accrued > utilized");
        prev_utilized = line.utilized_amount;
    }
}

/// Over-repay must bring utilization to zero and reset accrued_interest.
#[test]
fn over_repay_resets_accrued_interest() {
    let (env, client, borrowers) = setup_env();
    let borrower = &borrowers[0];

    client.draw_credit(borrower, &10_000_i128);
    env.ledger().with_mut(|l| l.timestamp += 15_768_000);
    client.update_risk_parameters(borrower, &50_000_i128, &1_000_u32, &30_u32);

    let line = client.get_credit_line(borrower).unwrap();
    let overpay = line.utilized_amount + 1_000;
    client.repay_credit(borrower, &overpay);

    let line = client.get_credit_line(borrower).unwrap();
    assert_eq!(line.utilized_amount, 0, "over-repay must zero utilization");
    assert_eq!(line.accrued_interest, 0, "over-repay must zero accrued_interest");
}

/// Batch accrual on empty batch must succeed without reverting.
#[test]
fn batch_empty_succeeds() {
    let (env, client, _borrowers) = setup_env();
    let empty: Vec<Address> = Vec::new(&env);
    client.accrue_batch(&empty);
    assert_total_utilized_non_negative(&client, "empty_batch");
}

/// Batch accrual with mixed existing/non-existing borrowers.
#[test]
fn batch_mixed_existing_and_missing() {
    let (env, client, borrowers) = setup_env();

    client.draw_credit(&borrowers[0], &5_000_i128);
    env.ledger().with_mut(|l| l.timestamp += 86_400 * 15);

    let mut batch: Vec<Address> = Vec::new(&env);
    batch.push_back(borrowers[0].clone());
    // Push a non-existent borrower.
    batch.push_back(Address::generate(&env));

    // Must not revert; missing lines are silently skipped.
    client.accrue_batch(&batch);
    assert_accrued_le_utilized(&client, "mixed_batch");
}
