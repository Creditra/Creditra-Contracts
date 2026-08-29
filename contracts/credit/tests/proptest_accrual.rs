// SPDX-License-Identifier: MIT

//! Property test: `accrued_interest <= utilized_amount` invariant.
//!
//! Generates random sequences of draw/repay/default/reopen operations across
//! multiple borrowers with random time advances, and asserts after every
//! mutation that `accrued_interest <= utilized_amount` for every active line.
//!
//! # Invariant
//!
//! After `apply_accrual` capitalizes interest into `utilized_amount`, the
//! cumulative capitalized component must always satisfy:
//!
//! ```text
//! 0 <= accrued_interest <= utilized_amount
//! ```
//!
//! The lower bound holds because interest is computed with
//! `Rounding::Floor` (never negative). The upper bound holds because
//! `accrued_interest` is a sub-component of `utilized_amount`: both are
//! incremented by the same `accrued_i` on every accrual, and repayments
//! first reduce `accrued_interest` before touching principal.
//!
//! # Covered paths
//!
//! | Path              | Why it matters                                   |
//! |-------------------|--------------------------------------------------|
//! | `draw_credit`     | Triggers `apply_accrual`; increases principal    |
//! | `repay_credit`    | Interest-first allocation; partial + over-repay  |
//! | `default_credit_line` | Status change; accrual runs at entry         |
//! | `close_credit_line`   | Requires zero utilization                    |
//! | Time advancement  | Drives interest accumulation between mutations   |
//! | Multiple borrowers | Ensures invariant holds across all lines        |

use creditra_credit::types::CreditStatus;
use creditra_credit::{Credit, CreditClient};
use proptest::prelude::*;
use proptest::test_runner::Config as ProptestConfig;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{Address, Env};

const BORROWER_COUNT: usize = 3;
const MAX_STEPS: usize = 32;
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
    let mut cursor = 0_u32;
    loop {
        let (lines, next_cursor) = client.enumerate_credit_lines(&cursor, &8, &false);
        if lines.is_empty() {
            break;
        }
        for i in 0..lines.len() {
            let line = lines.get(i).unwrap();
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
        match next_cursor {
            Some(c) => cursor = c,
            None => break,
        }
    }
}

/// Generate a random sequence of operations with amounts and time advances.
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
    Default,
    Reopen,
}

fn op_strategy() -> impl Strategy<Value = std::vec::Vec<OpStep>> {
    proptest::collection::vec(
        (
            0usize..BORROWER_COUNT,
            (0u64..=3u64),
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
                    2 => OpKind::Default,
                    _ => OpKind::Reopen,
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

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        .. ProptestConfig::default()
    })]

    /// After every draw, repay, default, or reopen the invariant
    /// `accrued_interest <= utilized_amount` must hold for every line.
    #[test]
    fn prop_accrued_interest_never_exceeds_utilized(
        steps in op_strategy(),
    ) {
        let (env, client, borrowers) = setup_env();

        assert_accrued_le_utilized(&client, "initial");

        for (step_idx, step) in steps.iter().enumerate() {
            let borrower = &borrowers[step.borrower_index];

            env.ledger().with_mut(|l| l.timestamp += step.time_advance);

            match step.op {
                OpKind::Draw => {
                    if let Some(line) = client.get_credit_line(borrower) {
                        let headroom = (line.credit_limit - line.utilized_amount).max(1);
                        let amount = step.amount.min(headroom.min(10_000));
                        let _ = client.try_draw_credit(borrower, &amount);
                    }
                }
                OpKind::Repay => {
                    if let Some(line) = client.get_credit_line(borrower) {
                        let amount = step.amount.min(line.utilized_amount + 5_000);
                        let _ = client.try_repay_credit(borrower, &amount);
                    }
                }
                OpKind::Default => {
                    let _ = client.try_default_credit_line(borrower);
                }
                OpKind::Reopen => {
                    let new_limit = 60_000_i128 + (step.borrower_index as i128 * 15_000_i128);
                    let new_rate = 1_200_u32 + (step.borrower_index as u32 * 400_u32);
                    let new_score = 35_u32 + (step.borrower_index as u32 * 8_u32);
                    let _ = client.try_open_credit_line(borrower, &new_limit, &new_rate, &new_score);
                }
            }

            let label = std::format!("step={} op={:?}", step_idx, step.op);
            assert_accrued_le_utilized(&client, &label);
        }
    }
}

/// Edge case: draw then wait a full year, verify invariant holds after accrual.
#[test]
fn accrual_bound_draw_then_wait_one_year() {
    let (env, client, borrowers) = setup_env();
    let borrower = &borrowers[0];

    client.draw_credit(borrower, &10_000_i128);
    assert_accrued_le_utilized(&client, "after_draw");

    env.ledger().with_mut(|l| l.timestamp += 31_536_000);
    client.update_risk_parameters(borrower, &50_000_i128, &1_000_u32, &30_u32);
    assert_accrued_le_utilized(&client, "after_one_year_accrual");
}

/// Edge case: draw, accrue, then repay partially.
#[test]
fn accrual_bound_draw_accrue_repay_partial() {
    let (env, client, borrowers) = setup_env();
    let borrower = &borrowers[0];

    client.draw_credit(borrower, &10_000_i128);
    assert_accrued_le_utilized(&client, "after_draw");

    env.ledger().with_mut(|l| l.timestamp += 15_768_000);
    client.update_risk_parameters(borrower, &50_000_i128, &1_000_u32, &30_u32);
    assert_accrued_le_utilized(&client, "after_accrual");

    let line = client.get_credit_line(borrower).unwrap();
    let partial = line.utilized_amount / 3;
    client.repay_credit(borrower, &partial);
    assert_accrued_le_utilized(&client, "after_partial_repay");
}

/// Edge case: draw, accrue, then over-repay (repay more than balance).
#[test]
fn accrual_bound_draw_accrue_over_repay() {
    let (env, client, borrowers) = setup_env();
    let borrower = &borrowers[0];

    client.draw_credit(borrower, &5_000_i128);
    assert_accrued_le_utilized(&client, "after_draw");

    env.ledger().with_mut(|l| l.timestamp += 15_768_000);
    client.update_risk_parameters(borrower, &50_000_i128, &1_000_u32, &30_u32);
    assert_accrued_le_utilized(&client, "after_accrual");

    let line = client.get_credit_line(borrower).unwrap();
    let overpay = line.utilized_amount + 1_000;
    client.repay_credit(borrower, &overpay);
    assert_accrued_le_utilized(&client, "after_over_repay");
}

/// Edge case: draw, default, then reopen.
#[test]
fn accrual_bound_draw_default_reopen() {
    let (env, client, borrowers) = setup_env();
    let borrower = &borrowers[0];

    client.draw_credit(borrower, &10_000_i128);
    assert_accrued_le_utilized(&client, "after_draw");

    env.ledger().with_mut(|l| l.timestamp += 15_768_000);
    client.default_credit_line(borrower);
    assert_accrued_le_utilized(&client, "after_default");

    client.open_credit_line(borrower, &80_000_i128, &1_500_u32, &40_u32);
    assert_accrued_le_utilized(&client, "after_reopen");
}

/// Edge case: multiple draws and repayments across several borrowers.
#[test]
fn accrual_bound_multi_borrower_sequence() {
    let (env, client, borrowers) = setup_env();

    client.draw_credit(&borrowers[0], &10_000_i128);
    env.ledger().with_mut(|l| l.timestamp += 7_884_000);
    client.update_risk_parameters(&borrowers[0], &50_000_i128, &1_000_u32, &30_u32);
    assert_accrued_le_utilized(&client, "b0_after_accrual");

    client.draw_credit(&borrowers[1], &15_000_i128);
    env.ledger().with_mut(|l| l.timestamp += 15_768_000);
    client.default_credit_line(&borrowers[1]);
    assert_accrued_le_utilized(&client, "b1_after_default");

    let line = client.get_credit_line(&borrowers[0]).unwrap();
    let partial = line.utilized_amount / 2;
    client.repay_credit(&borrowers[0], &partial);
    assert_accrued_le_utilized(&client, "b0_after_partial_repay");

    client.draw_credit(&borrowers[2], &8_000_i128);
    env.ledger().with_mut(|l| l.timestamp += 7_884_000);
    client.update_risk_parameters(&borrowers[2], &50_000_i128, &1_000_u32, &30_u32);
    assert_accrued_le_utilized(&client, "b2_after_accrual");

    let line = client.get_credit_line(&borrowers[2]).unwrap();
    let overpay = line.utilized_amount + 500;
    client.repay_credit(&borrowers[2], &overpay);
    assert_accrued_le_utilized(&client, "b2_after_over_repay");
}

/// Edge case: zero utilization should never accrue interest.
#[test]
fn accrual_bound_zero_utilization_no_accrual() {
    let (env, client, borrowers) = setup_env();
    let borrower = &borrowers[0];

    env.ledger().with_mut(|l| l.timestamp += 31_536_000);
    client.update_risk_parameters(borrower, &50_000_i128, &1_000_u32, &30_u32);

    let line = client.get_credit_line(borrower).unwrap();
    assert_eq!(line.accrued_interest, 0);
    assert_eq!(line.utilized_amount, 0);
    assert_accrued_le_utilized(&client, "zero_utilization");
}

/// Edge case: max rate (100%) with large principal.
#[test]
fn accrual_bound_max_rate_large_principal() {
    let (env, client, borrowers) = setup_env();
    let borrower = &borrowers[0];

    client.open_credit_line(borrower, &1_000_000_i128, &10_000_u32, &50_u32);
    client.draw_credit(borrower, &100_000_i128);
    assert_accrued_le_utilized(&client, "after_draw_max_rate");

    env.ledger().with_mut(|l| l.timestamp += 31_536_000);
    client.update_risk_parameters(borrower, &1_000_000_i128, &10_000_u32, &50_u32);
    assert_accrued_le_utilized(&client, "after_max_rate_accrual");

    let line = client.get_credit_line(borrower).unwrap();
    client.repay_credit(borrower, &line.utilized_amount);
    assert_accrued_le_utilized(&client, "after_max_rate_repay");
}
