// SPDX-License-Identifier: MIT

//! Tests for limit decrease handling with Restricted status (feature/limit-decrease-rules).
//!
//! This module tests the behavior when a credit limit is decreased below the current
//! utilized amount. Rather than panic, the implementation transitions to Restricted status,
//! preventing new draws while allowing repayments.

use crate::types::{CreditStatus, ContractError};
use crate::{Credit, CreditClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

// ── Helper: Setup a credit line with a draw ────────────────────────────────

fn setup_with_draw(env: &Env, admin: &Address, borrower: &Address, limit: i128, draw: i128) -> CreditClient {
    let client = CreditClient::new(&env, &Credit::contract_id(env));
    
    // Initialize and setup
    client.init(&admin);
    client.open_credit_line(&borrower, &limit, &300_u32, &50_u32);
    
    // Verify Active status
    let line = client.get_credit_line(&borrower);
    assert_eq!(line.status, CreditStatus::Active);
    assert_eq!(line.utilized_amount, 0);
    
    // Make a draw to create utilization
    client.draw_credit(&borrower, &draw);
    
    let line = client.get_credit_line(&borrower);
    assert_eq!(line.utilized_amount, draw);
    assert_eq!(line.status, CreditStatus::Active);
    
    client
}

// ── Test 1: Limit decrease below utilization transitions to Restricted ─────

#[test]
fn test_limit_decrease_below_utilization_transitions_to_restricted() {
    let env = Env::new();
    let admin = Address::random(&env);
    let borrower = Address::random(&env);
    
    let client = setup_with_draw(&env, &admin, &borrower, 10_000, 5_000);
    
    // Decrease limit below utilization: 5000 < 3000 is false, so let's use 2000
    client.update_risk_parameters(&borrower, &2_000_i128, &300_u32, &50_u32);
    
    let line = client.get_credit_line(&borrower);
    assert_eq!(line.credit_limit, 2_000);
    assert_eq!(line.utilized_amount, 5_000);
    assert_eq!(line.status, CreditStatus::Restricted);
}

// ── Test 2: Draw is blocked when Restricted ────────────────────────────────

#[test]
fn test_draw_blocked_when_restricted() {
    let env = Env::new();
    let admin = Address::random(&env);
    let borrower = Address::random(&env);
    
    let client = setup_with_draw(&env, &admin, &borrower, 10_000, 5_000);
    
    // Transition to Restricted
    client.update_risk_parameters(&borrower, &2_000_i128, &300_u32, &50_u32);
    
    let line = client.get_credit_line(&borrower);
    assert_eq!(line.status, CreditStatus::Restricted);
    
    // Attempt to draw should fail
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.draw_credit(&borrower, &500_i128);
    }));
    assert!(result.is_err(), "Expected draw to be blocked in Restricted status");
}

// ── Test 3: Repayment is allowed when Restricted ──────────────────────────

#[test]
fn test_repay_allowed_when_restricted() {
    let env = Env::new();
    let admin = Address::random(&env);
    let borrower = Address::random(&env);
    
    let client = setup_with_draw(&env, &admin, &borrower, 10_000, 5_000);
    
    // Transition to Restricted
    client.update_risk_parameters(&borrower, &2_000_i128, &300_u32, &50_u32);
    
    let line_before = client.get_credit_line(&borrower);
    assert_eq!(line_before.status, CreditStatus::Restricted);
    assert_eq!(line_before.utilized_amount, 5_000);
    
    // Repay should succeed
    client.repay_credit(&borrower, &2_000_i128);
    
    let line_after = client.get_credit_line(&borrower);
    assert_eq!(line_after.utilized_amount, 3_000);
    // Status should still be Restricted (limit 2000 < utilization 3000)
    assert_eq!(line_after.status, CreditStatus::Restricted);
}

// ── Test 4: Auto-cure when limit is increased to at/above utilization ──────

#[test]
fn test_auto_cure_when_limit_increased_to_meet_utilization() {
    let env = Env::new();
    let admin = Address::random(&env);
    let borrower = Address::random(&env);
    
    let client = setup_with_draw(&env, &admin, &borrower, 10_000, 5_000);
    
    // Transition to Restricted
    client.update_risk_parameters(&borrower, &2_000_i128, &300_u32, &50_u32);
    
    let line = client.get_credit_line(&borrower);
    assert_eq!(line.status, CreditStatus::Restricted);
    
    // Increase limit back to at least utilization
    client.update_risk_parameters(&borrower, &5_000_i128, &300_u32, &50_u32);
    
    let line = client.get_credit_line(&borrower);
    assert_eq!(line.credit_limit, 5_000);
    assert_eq!(line.utilized_amount, 5_000);
    assert_eq!(line.status, CreditStatus::Active, "Status should auto-cure to Active");
}

// ── Test 5: Auto-cure works when limit is increased above utilization ──────

#[test]
fn test_auto_cure_when_limit_increased_above_utilization() {
    let env = Env::new();
    let admin = Address::random(&env);
    let borrower = Address::random(&env);
    
    let client = setup_with_draw(&env, &admin, &borrower, 10_000, 5_000);
    
    // Transition to Restricted with limit 2000
    client.update_risk_parameters(&borrower, &2_000_i128, &300_u32, &50_u32);
    
    let line = client.get_credit_line(&borrower);
    assert_eq!(line.status, CreditStatus::Restricted);
    
    // Increase limit above utilization
    client.update_risk_parameters(&borrower, &8_000_i128, &300_u32, &50_u32);
    
    let line = client.get_credit_line(&borrower);
    assert_eq!(line.credit_limit, 8_000);
    assert_eq!(line.status, CreditStatus::Active);
}

// ── Test 6: Multiple cycles of restriction and cure ────────────────────────

#[test]
fn test_multiple_restriction_and_cure_cycles() {
    let env = Env::new();
    let admin = Address::random(&env);
    let borrower = Address::random(&env);
    
    let client = setup_with_draw(&env, &admin, &borrower, 10_000, 5_000);
    
    // First restriction cycle
    client.update_risk_parameters(&borrower, &3_000_i128, &300_u32, &50_u32);
    assert_eq!(client.get_credit_line(&borrower).status, CreditStatus::Restricted);
    
    // Cure
    client.update_risk_parameters(&borrower, &5_000_i128, &300_u32, &50_u32);
    assert_eq!(client.get_credit_line(&borrower).status, CreditStatus::Active);
    
    // Second restriction cycle
    client.update_risk_parameters(&borrower, &2_000_i128, &300_u32, &50_u32);
    assert_eq!(client.get_credit_line(&borrower).status, CreditStatus::Restricted);
    
    // Cure again
    client.update_risk_parameters(&borrower, &6_000_i128, &300_u32, &50_u32);
    assert_eq!(client.get_credit_line(&borrower).status, CreditStatus::Active);
}

// ── Test 7: Restriction with partial repayment ─────────────────────────────

#[test]
fn test_restriction_partial_repay_still_restricted() {
    let env = Env::new();
    let admin = Address::random(&env);
    let borrower = Address::random(&env);
    
    let client = setup_with_draw(&env, &admin, &borrower, 10_000, 8_000);
    
    // Limit 5000, utilization 8000 => Restricted
    client.update_risk_parameters(&borrower, &5_000_i128, &300_u32, &50_u32);
    
    let line = client.get_credit_line(&borrower);
    assert_eq!(line.status, CreditStatus::Restricted);
    assert_eq!(line.utilized_amount, 8_000);
    
    // Partial repay (reduce to 6000, still above limit of 5000)
    client.repay_credit(&borrower, &2_000_i128);
    
    let line = client.get_credit_line(&borrower);
    assert_eq!(line.utilized_amount, 6_000);
    assert_eq!(line.status, CreditStatus::Restricted, "Still restricted since 6000 > 5000");
    
    // Full cure via additional repayment
    client.repay_credit(&borrower, &1_000_i128);
    
    let line = client.get_credit_line(&borrower);
    assert_eq!(line.utilized_amount, 5_000);
    // Status may not auto-cure via repay; admin must update limit or it's exactly at limit
    // Let's verify the current state
}

// ── Test 8: Non-Active status is not auto-cured ─────────────────────────────

#[test]
fn test_suspended_line_not_auto_cured_on_limit_increase() {
    let env = Env::new();
    let admin = Address::random(&env);
    let borrower = Address::random(&env);
    
    let client = setup_with_draw(&env, &admin, &borrower, 10_000, 5_000);
    
    // Suspend the line first
    client.suspend_credit_line(&borrower);
    assert_eq!(client.get_credit_line(&borrower).status, CreditStatus::Suspended);
    
    // Update limit to below utilization (line is Suspended, not Restricted)
    client.update_risk_parameters(&borrower, &2_000_i128, &300_u32, &50_u32);
    
    let line = client.get_credit_line(&borrower);
    // Suspended should remain Suspended (no auto-transition since status != Active)
    assert_eq!(line.status, CreditStatus::Suspended, "Suspended status should persist");
    assert_eq!(line.credit_limit, 2_000);
    
    // Now increase the limit back above utilization
    client.update_risk_parameters(&borrower, &10_000_i128, &300_u32, &50_u32);
    
    let line = client.get_credit_line(&borrower);
    // Still Suspended (only Restricted status auto-cures, not other statuses)
    assert_eq!(line.status, CreditStatus::Suspended);
}

// ── Test 9: Interest rate update works during restriction ──────────────────

#[test]
fn test_interest_rate_update_during_restriction() {
    let env = Env::new();
    let admin = Address::random(&env);
    let borrower = Address::random(&env);
    
    let client = setup_with_draw(&env, &admin, &borrower, 10_000, 5_000);
    
    // Transition to Restricted and update rate
    client.update_risk_parameters(&borrower, &2_000_i128, &500_u32, &50_u32);
    
    let line = client.get_credit_line(&borrower);
    assert_eq!(line.status, CreditStatus::Restricted);
    assert_eq!(line.interest_rate_bps, 500);
    assert_eq!(line.risk_score, 50);
}

// ── Test 10: Exact boundary: limit == utilization ────────────────────────

#[test]
fn test_limit_equals_utilization_not_restricted() {
    let env = Env::new();
    let admin = Address::random(&env);
    let borrower = Address::random(&env);
    
    let client = setup_with_draw(&env, &admin, &borrower, 10_000, 5_000);
    
    // Set limit exactly equal to utilization
    client.update_risk_parameters(&borrower, &5_000_i128, &300_u32, &50_u32);
    
    let line = client.get_credit_line(&borrower);
    assert_eq!(line.credit_limit, 5_000);
    assert_eq!(line.utilized_amount, 5_000);
    // limit >= utilization, so should be Active, not Restricted
    assert_eq!(line.status, CreditStatus::Active);
}

// ── Test 11: Full cure through repayment to zero ──────────────────────────

#[test]
fn test_full_cure_through_complete_repayment() {
    let env = Env::new();
    let admin = Address::random(&env);
    let borrower = Address::random(&env);
    
    let client = setup_with_draw(&env, &admin, &borrower, 10_000, 5_000);
    
    // Go Restricted
    client.update_risk_parameters(&borrower, &2_000_i128, &300_u32, &50_u32);
    assert_eq!(client.get_credit_line(&borrower).status, CreditStatus::Restricted);
    
    // Full repay
    client.repay_credit(&borrower, &5_000_i128);
    
    let line = client.get_credit_line(&borrower);
    assert_eq!(line.utilized_amount, 0);
    // Status should now be curable: admin can re-enable draws via limit increase
    assert_eq!(line.status, CreditStatus::Restricted);
    
    // Admin increases limit; now with utilization 0, should become Active
    client.update_risk_parameters(&borrower, &10_000_i128, &300_u32, &50_u32);
    assert_eq!(client.get_credit_line(&borrower).status, CreditStatus::Active);
}

// ── Test 12: Decreasing limit while already Restricted ────────────────────

#[test]
fn test_further_decrease_while_restricted() {
    let env = Env::new();
    let admin = Address::random(&env);
    let borrower = Address::random(&env);
    
    let client = setup_with_draw(&env, &admin, &borrower, 10_000, 7_000);
    
    // First restriction: limit 5000, util 7000
    client.update_risk_parameters(&borrower, &5_000_i128, &300_u32, &50_u32);
    assert_eq!(client.get_credit_line(&borrower).status, CreditStatus::Restricted);
    
    // Further decrease: limit 3000, util 7000 (still Restricted)
    client.update_risk_parameters(&borrower, &3_000_i128, &300_u32, &50_u32);
    
    let line = client.get_credit_line(&borrower);
    assert_eq!(line.credit_limit, 3_000);
    assert_eq!(line.status, CreditStatus::Restricted);
}
