// SPDX-License-Identifier: MIT

//! Integration tests for the `get_credit_line_snapshot` entrypoint.
//!
//! Covers:
//! - Returns `None` for an unknown borrower.
//! - Returns the correct `CreditLineData` fields after `open_credit_line`.
//! - Reflects collateral balance when collateral is deposited.
//! - Reports `health_factor_bps == u32::MAX` with zero utilization.
//! - Reports correct health factor with outstanding debt and collateral.
//! - Returns the configured `repayment_schedule` when set.
//! - `is_delinquent` is `false` without a schedule.
//! - `is_delinquent` is `true` when past the grace window.
//! - `is_delinquent` is `false` within the grace window.
//! - Snapshot reflects status changes (Suspended, Closed).
//! - Snapshot is `None` after close for a borrower with no line.
//!   (Actually close preserves the record — status is Closed, not None.)

#![cfg(test)]

use creditra_credit::types::{CreditStatus, GraceWaiverMode};
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token::{self, StellarAssetClient},
    Address, Env,
};

// ── Test helpers ──────────────────────────────────────────────────────────────

/// Minimal setup: init contract with a real SAC token (so draws/repays work).
fn setup(env: &Env) -> (CreditClient<'_>, Address, Address, Address) {
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1_000);

    let admin = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    // Real SAC so token transfers actually work.
    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&token);

    // Seed the reserve so draws can succeed.
    StellarAssetClient::new(env, &token).mint(&token, &1_000_000_i128);

    (client, admin, contract_id, token)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[test]
fn returns_none_for_unknown_borrower() {
    let env = Env::default();
    let (client, _, _, _) = setup(&env);
    let borrower = Address::generate(&env);
    assert!(client.get_credit_line_snapshot(&borrower).is_none());
}

#[test]
fn returns_core_line_fields_after_open() {
    let env = Env::default();
    let (client, _, _, _) = setup(&env);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &10_000_i128, &300_u32, &50_u32);

    let snap = client
        .get_credit_line_snapshot(&borrower)
        .expect("snapshot must be Some after open");

    assert_eq!(snap.line.borrower, borrower);
    assert_eq!(snap.line.credit_limit, 10_000);
    assert_eq!(snap.line.utilized_amount, 0);
    assert_eq!(snap.line.interest_rate_bps, 300);
    assert_eq!(snap.line.risk_score, 50);
    assert_eq!(snap.line.status, CreditStatus::Active);
    assert_eq!(snap.line.accrued_interest, 0);
}

#[test]
fn collateral_balance_zero_before_deposit() {
    let env = Env::default();
    let (client, _, _, _) = setup(&env);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &10_000_i128, &0_u32, &0_u32);

    let snap = client
        .get_credit_line_snapshot(&borrower)
        .expect("Some");

    assert_eq!(snap.collateral_balance, 0);
}

#[test]
fn collateral_balance_reflects_deposit() {
    let env = Env::default();
    let (client, _, contract_id, token) = setup(&env);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &10_000_i128, &0_u32, &0_u32);

    // Fund borrower with tokens so deposit_collateral can transfer.
    StellarAssetClient::new(&env, &token).mint(&borrower, &5_000_i128);
    client.deposit_collateral(&borrower, &3_000);

    let _ = contract_id; // used indirectly by client
    let snap = client
        .get_credit_line_snapshot(&borrower)
        .expect("Some");

    assert_eq!(snap.collateral_balance, 3_000);
}

#[test]
fn health_factor_is_max_with_zero_utilization() {
    let env = Env::default();
    let (client, _, _, _) = setup(&env);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &10_000_i128, &0_u32, &0_u32);

    let snap = client
        .get_credit_line_snapshot(&borrower)
        .expect("Some");

    assert_eq!(snap.health_factor_bps, u32::MAX);
}

#[test]
fn health_factor_computed_with_debt_and_collateral() {
    let env = Env::default();
    let (client, _, _, token) = setup(&env);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &10_000_i128, &0_u32, &0_u32);

    // Deposit collateral first so the draw passes the min ratio check.
    StellarAssetClient::new(&env, &token).mint(&borrower, &10_000_i128);
    client.deposit_collateral(&borrower, &3_000);

    // Draw 1_000 → utilized = 1_000, collateral = 3_000, default min_ratio = 15_000 bps
    // health_bps = 3_000 * 100_000_000 / (1_000 * 15_000) = 300_000_000 / 15_000_000 = 20
    client.draw_credit(&borrower, &1_000);

    let snap = client
        .get_credit_line_snapshot(&borrower)
        .expect("Some");

    assert_eq!(snap.line.utilized_amount, 1_000);
    assert_eq!(snap.collateral_balance, 3_000);
    // Health factor should be 20 (well above 10_000 minimum → healthy).
    assert_eq!(snap.health_factor_bps, 20);
}

#[test]
fn repayment_schedule_is_none_when_not_set() {
    let env = Env::default();
    let (client, _, _, _) = setup(&env);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &10_000_i128, &0_u32, &0_u32);

    let snap = client
        .get_credit_line_snapshot(&borrower)
        .expect("Some");

    assert!(snap.repayment_schedule.is_none());
}

#[test]
fn repayment_schedule_present_when_set() {
    let env = Env::default();
    let (client, _, _, _) = setup(&env);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &10_000_i128, &0_u32, &0_u32);
    // amount_per_period=500, period_seconds=86_400, first_due_ts=100_000
    client.set_repayment_schedule(&borrower, &500_i128, &86_400_u64, &100_000_u64);

    let snap = client
        .get_credit_line_snapshot(&borrower)
        .expect("Some");
    let sched = snap.repayment_schedule.expect("schedule must be Some");

    assert_eq!(sched.amount_per_period, 500);
    assert_eq!(sched.period_seconds, 86_400);
    assert_eq!(sched.next_due_ts, 100_000);
}

#[test]
fn is_delinquent_false_without_schedule() {
    let env = Env::default();
    let (client, _, _, token) = setup(&env);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &10_000_i128, &0_u32, &0_u32);
    StellarAssetClient::new(&env, &token).mint(&borrower, &3_000_i128);
    client.deposit_collateral(&borrower, &3_000);
    client.draw_credit(&borrower, &1_000);

    let snap = client
        .get_credit_line_snapshot(&borrower)
        .expect("Some");

    assert!(!snap.is_delinquent);
}

#[test]
fn is_delinquent_false_before_grace_window_expires() {
    let env = Env::default();
    let (client, _, _, token) = setup(&env);
    let borrower = Address::generate(&env);

    env.ledger().with_mut(|li| li.timestamp = 10_000);
    client.open_credit_line(&borrower, &10_000_i128, &0_u32, &0_u32);
    StellarAssetClient::new(&env, &token).mint(&borrower, &3_000_i128);
    client.deposit_collateral(&borrower, &3_000);
    client.draw_credit(&borrower, &1_000);

    // Grace window of 120 seconds; due_ts = 9_900 → delinquent_after = 10_020
    client.set_grace_period_config(&120_u64, &GraceWaiverMode::FullWaiver, &0_u32);
    client.set_repayment_schedule(&borrower, &100_i128, &86_400_u64, &9_900_u64);

    // At ts=10_000, delinquent_after=10_020 → not yet delinquent
    let snap = client
        .get_credit_line_snapshot(&borrower)
        .expect("Some");
    assert!(!snap.is_delinquent);
}

#[test]
fn is_delinquent_true_past_grace_window() {
    let env = Env::default();
    let (client, _, _, token) = setup(&env);
    let borrower = Address::generate(&env);

    env.ledger().with_mut(|li| li.timestamp = 10_000);
    client.open_credit_line(&borrower, &10_000_i128, &0_u32, &0_u32);
    StellarAssetClient::new(&env, &token).mint(&borrower, &3_000_i128);
    client.deposit_collateral(&borrower, &3_000);
    client.draw_credit(&borrower, &1_000);

    // Grace window of 60 seconds; due_ts = 9_900 → delinquent_after = 9_960
    client.set_grace_period_config(&60_u64, &GraceWaiverMode::FullWaiver, &0_u32);
    client.set_repayment_schedule(&borrower, &100_i128, &86_400_u64, &9_900_u64);

    // At ts=10_000 > 9_960 → delinquent
    let snap = client
        .get_credit_line_snapshot(&borrower)
        .expect("Some");
    assert!(snap.is_delinquent);
}

#[test]
fn snapshot_reflects_suspended_status() {
    let env = Env::default();
    let (client, _, _, _) = setup(&env);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &10_000_i128, &0_u32, &0_u32);
    client.suspend_credit_line(&borrower);

    let snap = client
        .get_credit_line_snapshot(&borrower)
        .expect("Some after suspend");

    assert_eq!(snap.line.status, CreditStatus::Suspended);
    // No debt → health is max even when suspended.
    assert_eq!(snap.health_factor_bps, u32::MAX);
    // No schedule → not delinquent.
    assert!(!snap.is_delinquent);
}

#[test]
fn snapshot_reflects_closed_status_after_close() {
    let env = Env::default();
    let (client, admin, _, _) = setup(&env);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &10_000_i128, &0_u32, &0_u32);
    client.close_credit_line(&borrower, &admin);

    // Record is preserved after close — status is Closed, not None.
    let snap = client
        .get_credit_line_snapshot(&borrower)
        .expect("Some after close");

    assert_eq!(snap.line.status, CreditStatus::Closed);
    assert_eq!(snap.collateral_balance, 0);
    assert_eq!(snap.health_factor_bps, u32::MAX); // zero utilization
    assert!(snap.repayment_schedule.is_none());
    assert!(!snap.is_delinquent); // Closed → never delinquent
}

#[test]
fn snapshot_is_none_for_borrower_that_never_opened() {
    let env = Env::default();
    let (client, _, _, _) = setup(&env);
    let never_opened = Address::generate(&env);
    assert!(client.get_credit_line_snapshot(&never_opened).is_none());
}
