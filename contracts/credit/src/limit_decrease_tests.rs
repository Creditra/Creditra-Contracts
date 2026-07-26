// SPDX-License-Identifier: MIT

//! Tests for limit decrease handling with Restricted status (feature/limit-decrease-rules).
//!
//! This module tests the behavior when a credit limit is decreased below the current
//! utilized amount. Rather than panic, the implementation transitions to Restricted status,
//! preventing new draws while allowing repayments.

use crate::types::CreditStatus;
use crate::{Credit, CreditClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

// ── Helper: Setup a credit line with a draw ────────────────────────────────

fn setup_with_draw<'a>(env: &'a Env, admin: &Address, borrower: &Address, limit: i128, draw: i128) -> CreditClient<'a> {
    env.mock_all_auths();
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    let liquidity_token = env.register_stellar_asset_contract_v2(Address::generate(env));
    let liquidity_address = liquidity_token.address();
    
    client.init(admin);
    client.set_liquidity_token(&liquidity_address);
    let token_client = soroban_sdk::token::StellarAssetClient::new(env, &liquidity_address);
    token_client.mint(&contract_id, &(limit * 5));
    token_client.mint(borrower, &(limit * 5));
    soroban_sdk::token::Client::new(env, &liquidity_address).approve(borrower, &contract_id, &(limit * 5), &1000_u32);

    client.open_credit_line(borrower, &limit, &300_u32, &50_u32);
    client.deposit_collateral(borrower, &(limit * 2));
    
    if draw > 0 {
        client.draw_credit(borrower, &draw);
    }
    
    client
}

// ── Test 1: Limit decrease below utilization transitions to Restricted ─────

#[test]
fn test_limit_decrease_below_utilization_transitions_to_restricted() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    
    let client = setup_with_draw(&env, &admin, &borrower, 10_000, 5_000);
    
    client.update_risk_parameters(&borrower, &2_000_i128, &300_u32, &50_u32);
    
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.credit_limit, 2_000);
    assert_eq!(line.utilized_amount, 5_000);
    assert_eq!(line.status, CreditStatus::Restricted);
}

// ── Test 2: Draw is blocked when Restricted ────────────────────────────────

#[test]
fn test_draw_blocked_when_restricted() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    
    let client = setup_with_draw(&env, &admin, &borrower, 10_000, 5_000);
    
    client.update_risk_parameters(&borrower, &2_000_i128, &300_u32, &50_u32);
    
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Restricted);
    
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.draw_credit(&borrower, &500_i128);
    }));
    assert!(result.is_err(), "Expected draw to be blocked in Restricted status");
}

// ── Test 3: Repayment is allowed when Restricted ──────────────────────────

#[test]
fn test_repay_allowed_when_restricted() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    
    let client = setup_with_draw(&env, &admin, &borrower, 10_000, 5_000);
    
    client.update_risk_parameters(&borrower, &2_000_i128, &300_u32, &50_u32);
    
    let line_before = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line_before.status, CreditStatus::Restricted);
    assert_eq!(line_before.utilized_amount, 5_000);
    
    client.repay_credit(&borrower, &2_000_i128);
    
    let line_after = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line_after.utilized_amount, 3_000);
    assert_eq!(line_after.status, CreditStatus::Restricted);
}

// ── Test 4: Auto-cure when limit is increased to at/above utilization ──────

#[test]
fn test_auto_cure_when_limit_increased_to_meet_utilization() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    
    let client = setup_with_draw(&env, &admin, &borrower, 10_000, 5_000);
    
    client.update_risk_parameters(&borrower, &2_000_i128, &300_u32, &50_u32);
    
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Restricted);
    
    client.update_risk_parameters(&borrower, &5_000_i128, &300_u32, &50_u32);
    
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.credit_limit, 5_000);
    assert_eq!(line.utilized_amount, 5_000);
    assert_eq!(line.status, CreditStatus::Active, "Status should auto-cure to Active");
}

// ── Test 5: Auto-cure works when limit is increased above utilization ──────

#[test]
fn test_auto_cure_when_limit_increased_above_utilization() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    
    let client = setup_with_draw(&env, &admin, &borrower, 10_000, 5_000);
    
    client.update_risk_parameters(&borrower, &2_000_i128, &300_u32, &50_u32);
    
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Restricted);
    
    client.update_risk_parameters(&borrower, &8_000_i128, &300_u32, &50_u32);
    
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.credit_limit, 8_000);
    assert_eq!(line.status, CreditStatus::Active);
}

// ── Test 6: Multiple cycles of restriction and cure ────────────────────────

#[test]
fn test_multiple_restriction_and_cure_cycles() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    
    let client = setup_with_draw(&env, &admin, &borrower, 10_000, 5_000);
    
    client.update_risk_parameters(&borrower, &3_000_i128, &300_u32, &50_u32);
    assert_eq!(client.get_credit_line(&borrower).unwrap().status, CreditStatus::Restricted);
    
    client.update_risk_parameters(&borrower, &5_000_i128, &300_u32, &50_u32);
    assert_eq!(client.get_credit_line(&borrower).unwrap().status, CreditStatus::Active);
    
    client.update_risk_parameters(&borrower, &2_000_i128, &300_u32, &50_u32);
    assert_eq!(client.get_credit_line(&borrower).unwrap().status, CreditStatus::Restricted);
    
    client.update_risk_parameters(&borrower, &6_000_i128, &300_u32, &50_u32);
    assert_eq!(client.get_credit_line(&borrower).unwrap().status, CreditStatus::Active);
}

// ── Test 7: Restriction with partial repayment ─────────────────────────────

#[test]
fn test_restriction_partial_repay_still_restricted() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    
    let client = setup_with_draw(&env, &admin, &borrower, 10_000, 8_000);
    
    client.update_risk_parameters(&borrower, &5_000_i128, &300_u32, &50_u32);
    
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Restricted);
    assert_eq!(line.utilized_amount, 8_000);
    
    client.repay_credit(&borrower, &2_000_i128);
    
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 6_000);
    assert_eq!(line.status, CreditStatus::Restricted, "Still restricted since 6000 > 5000");
    
    client.repay_credit(&borrower, &1_000_i128);
    
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 5_000);
}

// ── Test 8: Non-Active status is not auto-cured ─────────────────────────────

#[test]
fn test_suspended_line_not_auto_cured_on_limit_increase() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    
    let client = setup_with_draw(&env, &admin, &borrower, 10_000, 5_000);
    
    client.suspend_credit_line(&borrower);
    assert_eq!(client.get_credit_line(&borrower).unwrap().status, CreditStatus::Suspended);
    
    client.update_risk_parameters(&borrower, &2_000_i128, &300_u32, &50_u32);
    
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Suspended, "Suspended status should persist");
    assert_eq!(line.credit_limit, 2_000);
    
    client.update_risk_parameters(&borrower, &10_000_i128, &300_u32, &50_u32);
    
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Suspended);
}

// ── Test 9: Interest rate update works during restriction ──────────────────

#[test]
fn test_interest_rate_update_during_restriction() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    
    let client = setup_with_draw(&env, &admin, &borrower, 10_000, 5_000);
    
    client.update_risk_parameters(&borrower, &2_000_i128, &500_u32, &50_u32);
    
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Restricted);
    assert_eq!(line.interest_rate_bps, 500);
    assert_eq!(line.risk_score, 50);
}

// ── Test 10: Exact boundary: limit == utilization ────────────────────────

#[test]
fn test_limit_equals_utilization_not_restricted() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    
    let client = setup_with_draw(&env, &admin, &borrower, 10_000, 5_000);
    
    client.update_risk_parameters(&borrower, &5_000_i128, &300_u32, &50_u32);
    
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.credit_limit, 5_000);
    assert_eq!(line.utilized_amount, 5_000);
    assert_eq!(line.status, CreditStatus::Active);
}

// ── Test 11: Full cure through repayment to zero ──────────────────────────

#[test]
fn test_full_cure_through_complete_repayment() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    
    let client = setup_with_draw(&env, &admin, &borrower, 10_000, 5_000);
    
    client.update_risk_parameters(&borrower, &2_000_i128, &300_u32, &50_u32);
    assert_eq!(client.get_credit_line(&borrower).unwrap().status, CreditStatus::Restricted);
    
    client.repay_credit(&borrower, &5_000_i128);
    
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 0);
    assert_eq!(line.status, CreditStatus::Restricted);
    
    client.update_risk_parameters(&borrower, &10_000_i128, &300_u32, &50_u32);
    assert_eq!(client.get_credit_line(&borrower).unwrap().status, CreditStatus::Active);
}

// ── Test 12: Decreasing limit while already Restricted ────────────────────

#[test]
fn test_further_decrease_while_restricted() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    
    let client = setup_with_draw(&env, &admin, &borrower, 10_000, 7_000);
    
    client.update_risk_parameters(&borrower, &5_000_i128, &300_u32, &50_u32);
    assert_eq!(client.get_credit_line(&borrower).unwrap().status, CreditStatus::Restricted);
    
    client.update_risk_parameters(&borrower, &3_000_i128, &300_u32, &50_u32);
    
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.credit_limit, 3_000);
    assert_eq!(line.status, CreditStatus::Restricted);
}
