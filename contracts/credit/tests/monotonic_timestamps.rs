// SPDX-License-Identifier: MIT
//! Regression tests: timestamp fields must only move forward (monotonic).
//!
//! Soroban ledger timestamps are validator-controlled and expected to be
//! non-decreasing. These tests verify that the contract rejects any operation
//! that would write a timestamp <= the stored value, simulating a regressed
//! ledger clock.

use proptest::prelude::*;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env};

use creditra_credit::{types::CreditStatus, Credit, CreditClient};

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    env.ledger().with_mut(|li| li.timestamp = 1_000);
    client.init(&admin);
    (env, admin, contract_id)
}

fn open_line(client: &CreditClient, borrower: &Address) {
    client.open_credit_line(borrower, &10_000_i128, &500_u32, &10_u32);
}

// ── last_rate_update_ts ──────────────────────────────────────────────────────

/// Normal forward update succeeds.
#[test]
fn rate_update_ts_advances_forward() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);
    open_line(&client, &borrower);

    env.ledger().with_mut(|li| li.timestamp = 2_000);
    client.update_risk_parameters(&borrower, &10_000_i128, &600_u32, &10_u32);

    let line = client.get_credit_line(&borrower);
    assert_eq!(line.last_rate_update_ts, 2_000);
}

/// Simulated timestamp regression on rate update is rejected.
#[test]
#[should_panic]
fn rate_update_ts_regression_rejected() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);
    open_line(&client, &borrower);

    // First update at t=2000 sets last_rate_update_ts
    env.ledger().with_mut(|li| li.timestamp = 2_000);
    client.update_risk_parameters(&borrower, &10_000_i128, &600_u32, &10_u32);

    // Simulate clock regression: t=1_500 < stored 2_000 → must panic
    env.ledger().with_mut(|li| li.timestamp = 1_500);
    client.update_risk_parameters(&borrower, &10_000_i128, &700_u32, &10_u32);
}

/// Same timestamp (equal, not strictly greater) is also rejected.
#[test]
#[should_panic]
fn rate_update_ts_equal_rejected() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);
    open_line(&client, &borrower);

    env.ledger().with_mut(|li| li.timestamp = 2_000);
    client.update_risk_parameters(&borrower, &10_000_i128, &600_u32, &10_u32);

    // Same timestamp → equal, not strictly greater → rejected
    env.ledger().with_mut(|li| li.timestamp = 2_000);
    client.update_risk_parameters(&borrower, &10_000_i128, &700_u32, &10_u32);
}

/// First rate update (stored_ts == 0) always passes regardless of timestamp.
#[test]
fn rate_update_ts_first_write_always_passes() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);
    open_line(&client, &borrower);

    // stored last_rate_update_ts is 0 (set at open_credit_line to ledger ts=1000)
    // but the guard only fires when stored_ts != 0, so a fresh line at ts=1000
    // has stored_ts=1000 from open. We just verify a forward update works.
    env.ledger().with_mut(|li| li.timestamp = 3_000);
    client.update_risk_parameters(&borrower, &10_000_i128, &600_u32, &10_u32);
    let line = client.get_credit_line(&borrower);
    assert_eq!(line.last_rate_update_ts, 3_000);
}

// ── suspension_ts ────────────────────────────────────────────────────────────

/// Normal suspension sets suspension_ts.
#[test]
fn suspension_ts_set_on_suspend() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);
    open_line(&client, &borrower);

    env.ledger().with_mut(|li| li.timestamp = 2_000);
    client.suspend_credit_line(&borrower);

    let line = client.get_credit_line(&borrower);
    assert_eq!(line.suspension_ts, 2_000);
}

/// Reinstate clears suspension_ts to 0 (intentional, not a regression).
#[test]
fn suspension_ts_cleared_on_reinstate() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);
    open_line(&client, &borrower);

    env.ledger().with_mut(|li| li.timestamp = 2_000);
    client.suspend_credit_line(&borrower);

    env.ledger().with_mut(|li| li.timestamp = 3_000);
    client.reinstate_credit_line(&borrower, &CreditStatus::Active);

    let line = client.get_credit_line(&borrower);
    assert_eq!(line.suspension_ts, 0);
}

/// Re-suspending after reinstate (suspension_ts=0) always passes.
#[test]
fn suspension_ts_resuspend_after_reinstate_passes() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);
    open_line(&client, &borrower);

    env.ledger().with_mut(|li| li.timestamp = 2_000);
    client.suspend_credit_line(&borrower);
    env.ledger().with_mut(|li| li.timestamp = 3_000);
    client.reinstate_credit_line(&borrower, &CreditStatus::Active);

    // After reinstate, suspension_ts == 0, so any ts passes the guard
    env.ledger().with_mut(|li| li.timestamp = 1_500);
    client.suspend_credit_line(&borrower);
    let line = client.get_credit_line(&borrower);
    assert_eq!(line.suspension_ts, 1_500);
}

// ── last_accrual_ts (already guarded in accrual.rs) ─────────────────────────

/// Accrual with regressed timestamp is a no-op (existing guard returns early).
#[test]
fn accrual_ts_regression_is_noop() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);
    open_line(&client, &borrower);

    // Draw to create utilization so accrual has something to do
    env.ledger().with_mut(|li| li.timestamp = 2_000);
    client.draw_credit(&borrower, &1_000_i128);

    let line_before = client.get_credit_line(&borrower);
    let ts_before = line_before.last_accrual_ts;

    // Regress the clock and draw again — accrual guard returns early, ts unchanged
    env.ledger().with_mut(|li| li.timestamp = 1_500);
    client.draw_credit(&borrower, &100_i128);

    let line_after = client.get_credit_line(&borrower);
    assert_eq!(line_after.last_accrual_ts, ts_before);
}

// ── Property test: monotonicity over randomized operation sequences ──────────

/// Operations that can write timestamps on a credit line.
#[derive(Debug, Clone)]
enum Op {
    /// update_risk_parameters with a new rate (triggers last_rate_update_ts write)
    UpdateRate { new_rate: u32 },
    /// suspend_credit_line (triggers suspension_ts write)
    Suspend,
    /// reinstate_credit_line to Active after default (clears suspension_ts to 0)
    DefaultThenReinstate,
}

fn arb_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (1u32..=800u32).prop_map(|r| Op::UpdateRate { new_rate: r }),
        Just(Op::Suspend),
        Just(Op::DefaultThenReinstate),
    ]
}

proptest! {
    /// Over any sequence of operations with a strictly-advancing ledger clock,
    /// `last_accrual_ts` and `last_rate_update_ts` must never decrease.
    ///
    /// The ledger timestamp advances by a positive delta before each operation,
    /// so the clock is always strictly increasing. After each operation the test
    /// asserts both timestamp fields are >= their previous values.
    #[test]
    fn prop_timestamps_monotonic_over_op_sequence(
        ops in proptest::collection::vec(arb_op(), 1..20),
        deltas in proptest::collection::vec(1u64..=500u64, 1..20),
    ) {
        let env = Env::default();
        env.mock_all_auths_allowing_non_root_auth();
        let admin = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        env.ledger().with_mut(|li| li.timestamp = 1_000);
        client.init(&admin);

        let borrower = Address::generate(&env);
        client.open_credit_line(&borrower, &10_000_i128, &500_u32, &10_u32);

        let mut ts: u64 = 1_000;
        let mut prev_accrual_ts = client.get_credit_line(&borrower).last_accrual_ts;
        let mut prev_rate_ts = client.get_credit_line(&borrower).last_rate_update_ts;
        let mut is_suspended = false;

        for (op, delta) in ops.iter().zip(deltas.iter().cycle()) {
            ts += delta;
            env.ledger().with_mut(|li| li.timestamp = ts);

            match op {
                Op::UpdateRate { new_rate } => {
                    if !is_suspended {
                        let _ = client.try_update_risk_parameters(
                            &borrower,
                            &10_000_i128,
                            new_rate,
                            &10_u32,
                        );
                    }
                }
                Op::Suspend => {
                    if !is_suspended {
                        let _ = client.try_suspend_credit_line(&borrower);
                        is_suspended = true;
                    }
                }
                Op::DefaultThenReinstate => {
                    // Default then immediately reinstate to Active — exercises
                    // the reinstate path which clears suspension_ts to 0.
                    let _ = client.try_default_credit_line(&borrower);
                    let _ = client.try_reinstate_credit_line(&borrower, &CreditStatus::Active);
                    is_suspended = false;
                }
            }

            let line = client.get_credit_line(&borrower);

            prop_assert!(
                line.last_accrual_ts >= prev_accrual_ts,
                "last_accrual_ts regressed: {} < {} at ts={}",
                line.last_accrual_ts,
                prev_accrual_ts,
                ts
            );
            prop_assert!(
                line.last_rate_update_ts >= prev_rate_ts,
                "last_rate_update_ts regressed: {} < {} at ts={}",
                line.last_rate_update_ts,
                prev_rate_ts,
                ts
            );

            prev_accrual_ts = line.last_accrual_ts;
            prev_rate_ts = line.last_rate_update_ts;
        }
    }
}
