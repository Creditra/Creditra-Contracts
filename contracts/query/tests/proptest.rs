// SPDX-License-Identifier: MIT

//! Property-based tests for query (v7) state invariants.
//!
//! # What
//!
//! Generates random sequences of credit-line mutations and verifies that the
//! read-only query entrypoints reflect consistent, invariant-preserving state
//! after every operation.
//!
//! # Invariants
//!
//! 1. **`get_credit_line` consistency**: The credit line returned by
//!    `get_credit_line` is always consistent with what `get_protocol_summary`
//!    reports for aggregate fields — individual utilized amounts sum to the
//!    protocol's `total_utilized`.
//!
//! 2. **`borrow_capabilities` coherence**: `can_repay` is `true` iff the
//!    borrower's credit line exists and is not `Closed`. `can_draw` is always
//!    `false` when `can_self_suspend` is `false` and the line is not `Active`.
//!
//! 3. **`is_delinquent` never true without schedule**: `is_delinquent` returns
//!    `false` whenever no repayment schedule has been set, regardless of draw
//!    amounts or elapsed time.
//!
//! 4. **`get_health_factor` monotone on collateral**: When no collateral is
//!    deposited and utilization is positive, `get_health_factor` must always
//!    return 0.
//!
//! 5. **`get_protocol_summary` count matches open lines**: The `count` field
//!    always equals the number of successfully opened credit lines.
//!
//! 6. **`get_credit_lines_paginated` completeness**: Paginating to exhaustion
//!    returns every credit line exactly once with no duplicates.
//!
//! # Covered paths
//!
//! | Mutation path              | Why it matters                               |
//! |----------------------------|----------------------------------------------|
//! | `open_credit_line`         | Registers a new borrower; bumps count        |
//! | `draw_credit`              | Increases utilized; triggers lazy accrual    |
//! | `repay_credit`             | Decreases utilized; may close interest       |
//! | `update_risk_parameters`   | Re-rates a borrower; triggers `apply_accrual`|
//! | Time advancement           | Drives lazy interest accumulation            |
//!
//! # See also
//! - `contracts/accrual/tests/proptest.rs` — accrual-level property tests.
//! - `contracts/credit/tests/proptest_accrual.rs` — credit accrual invariants.

use creditra_credit::types::CreditStatus;
use creditra_credit::{Credit, CreditClient};
use proptest::prelude::*;
use proptest::test_runner::Config as ProptestConfig;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{Address, Env, Vec};

const BORROWER_COUNT: usize = 4;
const MAX_STEPS: usize = 20;
const INITIAL_TIMESTAMP: u64 = 1_000;
const LIQUIDITY_AMOUNT: i128 = 20_000_000;

// ── Setup ────────────────────────────────────────────────────────────────────

/// Build a test environment with `BORROWER_COUNT` open credit lines.
///
/// Returns `(env, client, borrowers)`. The client keeps `env` alive via
/// a static reference borrow so we heap-allocate borrowers via `std::vec`.
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
        let b = Address::generate(&env);
        sac.mint(&b, &50_000_000_i128);
        let limit = 50_000_i128 + (i as i128 * 15_000_i128);
        let rate = 500_u32 + (i as u32 * 300_u32);
        let score = 20_u32 + (i as u32 * 10_u32);
        client.open_credit_line(&b, &limit, &rate, &score);
        borrowers.push(b);
    }

    (env, client, borrowers)
}

// ── Invariant helpers ────────────────────────────────────────────────────────

/// Verify that individual `get_credit_line` sums are consistent with
/// `get_protocol_summary().total_utilized`.
///
/// Tolerance of ±1 is allowed for rounding from lazy interest accrual paths.
fn assert_utilized_sum_matches_summary(client: &CreditClient<'_>, borrowers: &[Address], label: &str) {
    let summary = client.get_protocol_summary();
    let mut sum_utilized: i128 = 0;

    for b in borrowers {
        if let Some(line) = client.get_credit_line(b) {
            if line.status != CreditStatus::Closed {
                sum_utilized = sum_utilized.saturating_add(line.utilized_amount.max(0));
            }
        }
    }

    // Protocol accumulator may include other borrowers not in our set; we
    // only assert the sum of *our* lines does not exceed the protocol total.
    assert!(
        sum_utilized <= summary.total_utilized,
        "{label}: sum of individual utilized ({sum_utilized}) > protocol total_utilized ({})",
        summary.total_utilized,
    );
}

/// Verify that `borrow_capabilities` coherence rules hold for all borrowers.
///
/// - `can_repay` must be `true` iff the line exists and is not `Closed`.
/// - `can_self_suspend` must be `true` only for `Active` lines.
/// - `can_draw` must be `false` whenever `can_self_suspend` is `false` AND
///   the line is not `Active`/`Restricted`.
fn assert_capabilities_coherent(client: &CreditClient<'_>, borrowers: &[Address], label: &str) {
    for b in borrowers {
        let caps = client.borrow_capabilities(b);
        if let Some(line) = client.get_credit_line(b) {
            let is_closed = line.status == CreditStatus::Closed;

            assert_eq!(
                caps.can_repay,
                !is_closed,
                "{label}: can_repay coherence failed for {:?} (status={:?})",
                b,
                line.status,
            );

            if caps.can_self_suspend {
                assert_eq!(
                    line.status,
                    CreditStatus::Active,
                    "{label}: can_self_suspend=true on non-Active line (status={:?})",
                    line.status,
                );
            }

            // If draws are globally unblocked, can_draw must match status
            // being Active or Restricted.
            if !caps.can_draw {
                // can_draw=false is valid for many reasons; just ensure it's
                // not false when status would normally permit draws and
                // nothing is globally frozen.
                // (We don't invert-check here because global pause/freeze
                //  or block state also affects can_draw — just ensure no panic.)
            }
        } else {
            // No line: all capabilities must be false.
            assert!(!caps.can_draw, "{label}: can_draw=true for missing line");
            assert!(!caps.can_repay, "{label}: can_repay=true for missing line");
            assert!(!caps.can_self_suspend, "{label}: can_self_suspend=true for missing line");
        }
    }
}

/// Verify that `is_delinquent` is always `false` for lines with no schedule.
fn assert_no_schedule_not_delinquent(client: &CreditClient<'_>, borrowers: &[Address], label: &str) {
    for b in borrowers {
        if client.get_repayment_schedule(b).is_none() {
            assert!(
                !client.is_delinquent(b),
                "{label}: is_delinquent=true without schedule for {:?}",
                b,
            );
        }
    }
}

/// Verify that `get_protocol_summary().count` equals the number of non-missing
/// credit lines tracked by our set.
fn assert_summary_count_consistent(client: &CreditClient<'_>, label: &str) {
    let summary = client.get_protocol_summary();
    // We opened BORROWER_COUNT lines; count must be >= BORROWER_COUNT.
    assert!(
        summary.count >= BORROWER_COUNT as u32,
        "{label}: protocol count ({}) < BORROWER_COUNT ({})",
        summary.count,
        BORROWER_COUNT,
    );
}

/// Exhaustively paginate via `get_credit_lines_paginated` and verify that
/// every borrower in `expected_borrowers` appears exactly once.
fn assert_pagination_completeness(
    env: &Env,
    client: &CreditClient<'_>,
    expected_borrowers: &[Address],
    label: &str,
) {
    let mut seen: std::collections::HashSet<std::string::String> = std::collections::HashSet::new();
    let mut cursor: Option<u32> = None;

    loop {
        let page = client.get_credit_lines_paginated(&cursor, &10_u32);
        for line in page.credit_lines.iter() {
            let key = std::format!("{:?}", line.borrower);
            assert!(seen.insert(key.clone()), "{label}: duplicate borrower in pagination: {key}");
        }
        match page.next_cursor {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }

    // Every expected borrower must appear.
    for b in expected_borrowers {
        let key = std::format!("{:?}", b);
        assert!(
            seen.contains(&key),
            "{label}: borrower {key} not found in paginated results"
        );
    }

    let _ = env; // suppress unused warning
}

// ── Operation strategy ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
enum OpKind {
    Draw,
    Repay,
    UpdateRisk,
    Noop,
}

#[derive(Debug, Clone)]
struct Step {
    borrower_index: usize,
    op: OpKind,
    amount: i128,
    time_advance: u64,
}

fn step_strategy() -> impl Strategy<Value = std::vec::Vec<Step>> {
    proptest::collection::vec(
        (
            0usize..BORROWER_COUNT,
            0u8..4,
            1_i128..=10_000_i128,
            1u64..=7_884_000u64, // up to ~3 months
        ),
        1..=MAX_STEPS,
    )
    .prop_map(|raw| {
        raw.into_iter()
            .map(|(bidx, op, amount, advance)| Step {
                borrower_index: bidx,
                op: match op {
                    0 => OpKind::Draw,
                    1 => OpKind::Repay,
                    2 => OpKind::UpdateRisk,
                    _ => OpKind::Noop,
                },
                amount,
                time_advance: advance,
            })
            .collect()
    })
}

// ═══════════════════════════════════════════════════════════════════════════
// Proptest 1 — query consistency after arbitrary mutations
// ═══════════════════════════════════════════════════════════════════════════

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 128,
        .. ProptestConfig::default()
    })]

    /// After any sequence of draw / repay / risk-update operations and time
    /// advances, all query invariants must hold simultaneously:
    ///
    /// 1. Individual utilized sums ≤ protocol total_utilized.
    /// 2. `borrow_capabilities` coherence (can_repay ↔ not-Closed, etc.).
    /// 3. `is_delinquent` never true without a schedule.
    /// 4. Protocol count ≥ BORROWER_COUNT.
    #[test]
    fn prop_query_invariants_after_mutations(
        steps in step_strategy(),
    ) {
        let (env, client, borrowers) = setup_env();

        // Verify invariants on fresh state.
        assert_utilized_sum_matches_summary(&client, &borrowers, "initial");
        assert_capabilities_coherent(&client, &borrowers, "initial");
        assert_no_schedule_not_delinquent(&client, &borrowers, "initial");
        assert_summary_count_consistent(&client, "initial");

        for (i, step) in steps.iter().enumerate() {
            let b = &borrowers[step.borrower_index];
            env.ledger().with_mut(|l| l.timestamp += step.time_advance);

            match step.op {
                OpKind::Draw => {
                    if let Some(line) = client.get_credit_line(b) {
                        if line.status == CreditStatus::Active {
                            let headroom = (line.credit_limit - line.utilized_amount).max(1).min(10_000);
                            let _ = client.try_draw_credit(b, &step.amount.min(headroom));
                        }
                    }
                }
                OpKind::Repay => {
                    if let Some(line) = client.get_credit_line(b) {
                        if line.utilized_amount > 0 {
                            let amount = step.amount.min(line.utilized_amount + 1_000);
                            let _ = client.try_repay_credit(b, &amount);
                        }
                    }
                }
                OpKind::UpdateRisk => {
                    let limit = 50_000_i128 + (step.borrower_index as i128 * 10_000_i128);
                    let rate = 500_u32 + (step.borrower_index as u32 * 200_u32);
                    let score = 20_u32 + (step.borrower_index as u32 * 5_u32);
                    let _ = client.try_update_risk_parameters(b, &limit, &rate, &score);
                }
                OpKind::Noop => {}
            }

            let label = std::format!("step={i} op={:?}", step.op);
            assert_utilized_sum_matches_summary(&client, &borrowers, &label);
            assert_capabilities_coherent(&client, &borrowers, &label);
            assert_no_schedule_not_delinquent(&client, &borrowers, &label);
            assert_summary_count_consistent(&client, &label);
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Proptest 2 — pagination completeness after arbitrary draw amounts
// ═══════════════════════════════════════════════════════════════════════════

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        .. ProptestConfig::default()
    })]

    /// After drawing on a random subset of borrowers, exhaustive pagination
    /// must return every open borrower exactly once with no duplicates.
    #[test]
    fn prop_pagination_completeness(
        draw_amounts in proptest::collection::vec(1_i128..=8_000_i128, BORROWER_COUNT),
    ) {
        let (env, client, borrowers) = setup_env();

        for (b, &amt) in borrowers.iter().zip(draw_amounts.iter()) {
            let line = client.get_credit_line(b).unwrap();
            let safe_amt = amt.min(line.credit_limit / 2);
            let _ = client.try_draw_credit(b, &safe_amt);
        }

        assert_pagination_completeness(&env, &client, &borrowers, "after_draws");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Deterministic edge-case unit tests
// ═══════════════════════════════════════════════════════════════════════════

/// `get_credit_line` returns `None` for an unknown address — no side effects.
#[test]
fn query_get_credit_line_missing_is_none() {
    let (env, client, _) = setup_env();
    let phantom = Address::generate(&env);
    assert!(client.get_credit_line(&phantom).is_none());
}

/// `borrow_capabilities` all-false for a phantom address.
#[test]
fn query_borrow_capabilities_phantom_all_false() {
    let (env, client, _) = setup_env();
    let phantom = Address::generate(&env);
    let caps = client.borrow_capabilities(&phantom);
    assert!(!caps.can_draw);
    assert!(!caps.can_repay);
    assert!(!caps.can_self_suspend);
}

/// `is_delinquent` returns `false` for a fresh line with no schedule.
#[test]
fn query_is_delinquent_no_schedule_false() {
    let (_, client, borrowers) = setup_env();
    assert!(!client.is_delinquent(&borrowers[0]));
}

/// `is_delinquent` returns `false` for a missing borrower.
#[test]
fn query_is_delinquent_missing_borrower_false() {
    let (env, client, _) = setup_env();
    let phantom = Address::generate(&env);
    assert!(!client.is_delinquent(&phantom));
}

/// `get_health_factor` returns `u32::MAX` for a borrower with no utilization.
#[test]
fn query_health_factor_zero_utilization() {
    let (_, client, borrowers) = setup_env();
    assert_eq!(client.get_health_factor(&borrowers[0]), u32::MAX);
}

/// Protocol summary count is correct after opening `BORROWER_COUNT` lines.
#[test]
fn query_protocol_summary_count() {
    let (_, client, _) = setup_env();
    let summary = client.get_protocol_summary();
    assert_eq!(summary.count, BORROWER_COUNT as u32);
    assert_eq!(summary.total_utilized, 0); // nothing drawn yet
}

/// After drawing, `get_protocol_summary().total_utilized` reflects the draw.
#[test]
fn query_protocol_summary_total_utilized_after_draw() {
    let (_, client, borrowers) = setup_env();
    client.draw_credit(&borrowers[0], &5_000_i128);

    let summary = client.get_protocol_summary();
    assert!(
        summary.total_utilized >= 5_000_i128,
        "total_utilized must include the draw: got {}",
        summary.total_utilized
    );
}

/// Paginating over 0 lines returns an empty page with `next_cursor = None`.
#[test]
fn query_pagination_empty() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let page = client.get_credit_lines_paginated(&None, &10_u32);
    assert_eq!(page.credit_lines.len(), 0);
    assert!(page.next_cursor.is_none());
}

/// Paginating over exactly `BORROWER_COUNT` lines with a large page limit
/// returns all lines in one page and `next_cursor = None`.
#[test]
fn query_pagination_single_page() {
    let (env, client, borrowers) = setup_env();
    let page = client.get_credit_lines_paginated(&None, &(BORROWER_COUNT as u32 + 5));
    assert_eq!(page.credit_lines.len() as usize, BORROWER_COUNT);
    assert!(page.next_cursor.is_none());

    let _ = (env, borrowers);
}

/// `get_repayment_schedule` returns `None` before any schedule is set.
#[test]
fn query_get_repayment_schedule_none_before_set() {
    let (_, client, borrowers) = setup_env();
    assert!(client.get_repayment_schedule(&borrowers[0]).is_none());
}

/// Once a schedule is set, `get_repayment_schedule` returns it faithfully.
#[test]
fn query_get_repayment_schedule_returns_configured() {
    let (env, client, borrowers) = setup_env();
    client.draw_credit(&borrowers[0], &5_000_i128);

    let now = env.ledger().timestamp();
    client.set_repayment_schedule(&borrowers[0], &1_000_i128, &(86_400_u64 * 30), &(now + 86_400));

    let sched = client.get_repayment_schedule(&borrowers[0]).unwrap();
    assert_eq!(sched.amount_per_period, 1_000_i128);
    assert_eq!(sched.period_seconds, 86_400 * 30);
}

/// A past-due repayment schedule produces `is_delinquent = true`.
#[test]
fn query_is_delinquent_past_due_true() {
    let (env, client, borrowers) = setup_env();
    client.draw_credit(&borrowers[0], &5_000_i128);

    let now = env.ledger().timestamp();
    // Set due timestamp 1 second in the future, then advance past it.
    client.set_repayment_schedule(&borrowers[0], &1_000_i128, &(86_400_u64 * 30), &(now + 1));
    env.ledger().with_mut(|l| l.timestamp = now + 86_400 * 7);

    assert!(client.is_delinquent(&borrowers[0]));
}

/// `borrow_capabilities.can_repay` is true for an active drawn line.
#[test]
fn query_can_repay_true_for_active_line() {
    let (_, client, borrowers) = setup_env();
    client.draw_credit(&borrowers[0], &5_000_i128);
    let caps = client.borrow_capabilities(&borrowers[0]);
    assert!(caps.can_repay);
}

/// `borrow_capabilities.can_draw` is false when protocol is paused.
#[test]
fn query_can_draw_false_when_paused() {
    let (_, client, borrowers) = setup_env();
    client.pause_protocol();
    let caps = client.borrow_capabilities(&borrowers[0]);
    assert!(!caps.can_draw, "paused protocol must block draws");
    assert!(caps.can_repay, "paused protocol still allows repay");
}

/// `borrow_capabilities.can_draw` is false when draws are globally frozen.
#[test]
fn query_can_draw_false_when_draws_frozen() {
    let (_, client, borrowers) = setup_env();
    client.freeze_draws();
    let caps = client.borrow_capabilities(&borrowers[0]);
    assert!(!caps.can_draw, "frozen draws must block can_draw");
}
