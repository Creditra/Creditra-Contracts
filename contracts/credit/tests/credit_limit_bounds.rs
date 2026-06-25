// SPDX-License-Identifier: MIT

//! Integration tests for global credit limit bounds enforcement.
//!
//! These tests verify that the admin-configurable min/max credit limit bounds
//! are properly enforced when opening new credit lines and updating existing ones.

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env,
};

use creditra_credit::types::ContractError;
use creditra_credit::{Credit, CreditClient};

// ── Test Helpers ──────────────────────────────────────────────────────────────

fn setup_env() -> (Env, CreditClient<'static>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register_contract(None, Credit);
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);

    (env, client, contract_id, admin)
}

fn setup_with_bounds(min: i128, max: i128) -> (Env, CreditClient<'static>, Address, Address) {
    let (env, client, contract_id, admin) = setup_env();
    client.set_credit_limit_bounds(&min, &max);
    (env, client, contract_id, admin)
}

// ── Test 1: Admin Authorization ───────────────────────────────────────────────

#[test]
fn test_set_bounds_requires_admin_auth() {
    let (env, client, _contract_id, _admin) = setup_env();

    let non_admin = Address::generate(&env);

    // Clear mock_all_auths to test actual authorization
    env.mock_auths(&[]);

    // Try to set bounds as non-admin - should fail with auth error
    let result = client.try_set_credit_limit_bounds(&1_000, &1_000_000);

    assert!(result.is_err(), "Expected error when non-admin sets bounds");
    // Auth errors are handled by Soroban host, not ContractError
}

#[test]
fn test_set_bounds_succeeds_with_admin_auth() {
    let (_env, client, _contract_id, _admin) = setup_env();

    // Admin can set bounds
    client.set_credit_limit_bounds(&1_000, &1_000_000);

    let (min, max) = client.get_credit_limit_bounds();
    assert_eq!(min, Some(1_000));
    assert_eq!(max, Some(1_000_000));
}

// ── Test 2: Validation Safeguards ─────────────────────────────────────────────

#[test]
fn test_set_bounds_rejects_negative_min() {
    let (_env, client, _contract_id, _admin) = setup_env();

    // Try to set negative minimum
    let result = client.try_set_credit_limit_bounds(&-1_000, &1_000_000);

    assert!(result.is_err(), "Expected error for negative minimum");
    let err = result.err().unwrap();
    assert_eq!(
        err.unwrap(),
        ContractError::InvalidAmount,
        "Expected InvalidAmount error for negative min"
    );
}

#[test]
fn test_set_bounds_rejects_max_less_than_min() {
    let (_env, client, _contract_id, _admin) = setup_env();

    // Try to set max < min
    let result = client.try_set_credit_limit_bounds(&1_000_000, &1_000);

    assert!(result.is_err(), "Expected error when max < min");
    let err = result.err().unwrap();
    assert_eq!(
        err.unwrap(),
        ContractError::LimitOutOfBounds,
        "Expected LimitOutOfBounds error when max < min"
    );
}

#[test]
fn test_set_bounds_allows_min_equals_max() {
    let (_env, client, _contract_id, _admin) = setup_env();

    // Setting min == max should be allowed (single valid limit)
    client.set_credit_limit_bounds(&100_000, &100_000);

    let (min, max) = client.get_credit_limit_bounds();
    assert_eq!(min, Some(100_000));
    assert_eq!(max, Some(100_000));
}

#[test]
fn test_set_bounds_allows_zero_min() {
    let (_env, client, _contract_id, _admin) = setup_env();

    // Zero minimum should be allowed
    client.set_credit_limit_bounds(&0, &1_000_000);

    let (min, max) = client.get_credit_limit_bounds();
    assert_eq!(min, Some(0));
    assert_eq!(max, Some(1_000_000));
}

// ── Test 3: Open Credit Line Validation ───────────────────────────────────────

#[test]
fn test_open_credit_line_below_min_fails() {
    let (_env, client, _contract_id, _admin) = setup_with_bounds(10_000, 1_000_000);

    let borrower = Address::generate(&_env);

    // Try to open credit line below minimum
    let result = client.try_open_credit_line(&borrower, &5_000, &500, &50);

    assert!(
        result.is_err(),
        "Expected error when opening line below min"
    );
    let err = result.err().unwrap();
    assert_eq!(
        err.unwrap(),
        ContractError::LimitOutOfBounds,
        "Expected LimitOutOfBounds error"
    );
}

#[test]
fn test_open_credit_line_above_max_fails() {
    let (_env, client, _contract_id, _admin) = setup_with_bounds(10_000, 1_000_000);

    let borrower = Address::generate(&_env);

    // Try to open credit line above maximum
    let result = client.try_open_credit_line(&borrower, &2_000_000, &500, &50);

    assert!(
        result.is_err(),
        "Expected error when opening line above max"
    );
    let err = result.err().unwrap();
    assert_eq!(
        err.unwrap(),
        ContractError::LimitOutOfBounds,
        "Expected LimitOutOfBounds error"
    );
}

#[test]
fn test_open_credit_line_at_min_succeeds() {
    let (_env, client, _contract_id, _admin) = setup_with_bounds(10_000, 1_000_000);

    let borrower = Address::generate(&_env);

    // Open credit line exactly at minimum
    client.open_credit_line(&borrower, &10_000, &500, &50);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.credit_limit, 10_000);
}

#[test]
fn test_open_credit_line_at_max_succeeds() {
    let (_env, client, _contract_id, _admin) = setup_with_bounds(10_000, 1_000_000);

    let borrower = Address::generate(&_env);

    // Open credit line exactly at maximum
    client.open_credit_line(&borrower, &1_000_000, &500, &50);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.credit_limit, 1_000_000);
}

#[test]
fn test_open_credit_line_within_bounds_succeeds() {
    let (_env, client, _contract_id, _admin) = setup_with_bounds(10_000, 1_000_000);

    let borrower = Address::generate(&_env);

    // Open credit line within bounds
    client.open_credit_line(&borrower, &500_000, &500, &50);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.credit_limit, 500_000);
}

// ── Test 4: Update Risk Parameters Validation ─────────────────────────────────

#[test]
fn test_update_risk_params_increase_above_max_fails() {
    let (env, client, _contract_id, _admin) = setup_with_bounds(10_000, 1_000_000);

    let borrower = Address::generate(&env);

    // Open credit line within bounds
    client.open_credit_line(&borrower, &500_000, &500, &50);

    // Try to increase limit above maximum
    let result = client.try_update_risk_parameters(&borrower, &2_000_000, &600, &60);

    assert!(
        result.is_err(),
        "Expected error when increasing limit above max"
    );
    let err = result.err().unwrap();
    assert_eq!(
        err.unwrap(),
        ContractError::LimitOutOfBounds,
        "Expected LimitOutOfBounds error"
    );
}

#[test]
fn test_update_risk_params_decrease_below_min_fails() {
    let (env, client, _contract_id, _admin) = setup_with_bounds(10_000, 1_000_000);

    let borrower = Address::generate(&env);

    // Open credit line within bounds
    client.open_credit_line(&borrower, &500_000, &500, &50);

    // Try to decrease limit below minimum
    let result = client.try_update_risk_parameters(&borrower, &5_000, &600, &60);

    assert!(
        result.is_err(),
        "Expected error when decreasing limit below min"
    );
    let err = result.err().unwrap();
    assert_eq!(
        err.unwrap(),
        ContractError::LimitOutOfBounds,
        "Expected LimitOutOfBounds error"
    );
}

#[test]
fn test_update_risk_params_within_bounds_succeeds() {
    let (env, client, _contract_id, _admin) = setup_with_bounds(10_000, 1_000_000);

    let borrower = Address::generate(&env);

    // Open credit line
    client.open_credit_line(&borrower, &500_000, &500, &50);

    // Update limit within bounds
    client.update_risk_parameters(&borrower, &750_000, &600, &60);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.credit_limit, 750_000);
}

#[test]
fn test_update_risk_params_to_max_succeeds() {
    let (env, client, _contract_id, _admin) = setup_with_bounds(10_000, 1_000_000);

    let borrower = Address::generate(&env);

    // Open credit line
    client.open_credit_line(&borrower, &500_000, &500, &50);

    // Update to maximum
    client.update_risk_parameters(&borrower, &1_000_000, &600, &60);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.credit_limit, 1_000_000);
}

#[test]
fn test_update_risk_params_to_min_succeeds() {
    let (env, client, _contract_id, _admin) = setup_with_bounds(10_000, 1_000_000);

    let borrower = Address::generate(&env);

    // Open credit line
    client.open_credit_line(&borrower, &500_000, &500, &50);

    // Update to minimum (assuming no utilization)
    client.update_risk_parameters(&borrower, &10_000, &600, &60);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.credit_limit, 10_000);
}

// ── Test 5: Happy Path Scenarios ──────────────────────────────────────────────

#[test]
fn test_no_bounds_configured_allows_any_limit() {
    let (_env, client, _contract_id, _admin) = setup_env();

    let borrower = Address::generate(&_env);

    // Without bounds configured, any positive limit should work
    client.open_credit_line(&borrower, &1, &500, &50);
    let line1 = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line1.credit_limit, 1);

    let borrower2 = Address::generate(&_env);
    client.open_credit_line(&borrower2, &i128::MAX, &500, &50);
    let line2 = client.get_credit_line(&borrower2).unwrap();
    assert_eq!(line2.credit_limit, i128::MAX);
}

#[test]
fn test_bounds_can_be_updated() {
    let (_env, client, _contract_id, _admin) = setup_with_bounds(10_000, 1_000_000);

    // Verify initial bounds
    let (min, max) = client.get_credit_limit_bounds();
    assert_eq!(min, Some(10_000));
    assert_eq!(max, Some(1_000_000));

    // Update bounds
    client.set_credit_limit_bounds(&50_000, &5_000_000);

    // Verify updated bounds
    let (min, max) = client.get_credit_limit_bounds();
    assert_eq!(min, Some(50_000));
    assert_eq!(max, Some(5_000_000));
}

#[test]
fn test_existing_lines_not_affected_by_new_bounds() {
    let (env, client, _contract_id, _admin) = setup_with_bounds(10_000, 1_000_000);

    let borrower = Address::generate(&env);

    // Open credit line within original bounds
    client.open_credit_line(&borrower, &500_000, &500, &50);

    // Change bounds to exclude existing limit
    client.set_credit_limit_bounds(&600_000, &2_000_000);

    // Existing line should still exist and be queryable
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.credit_limit, 500_000);

    // But new updates must respect new bounds
    let result = client.try_update_risk_parameters(&borrower, &500_000, &600, &60);
    assert!(result.is_err(), "Existing limit now below new minimum");
}

#[test]
fn test_multiple_borrowers_all_respect_bounds() {
    let (env, client, _contract_id, _admin) = setup_with_bounds(10_000, 1_000_000);

    let borrower1 = Address::generate(&env);
    let borrower2 = Address::generate(&env);
    let borrower3 = Address::generate(&env);

    // All borrowers must respect bounds
    client.open_credit_line(&borrower1, &10_000, &500, &50);
    client.open_credit_line(&borrower2, &500_000, &500, &50);
    client.open_credit_line(&borrower3, &1_000_000, &500, &50);

    // Verify all lines created successfully
    assert!(client.get_credit_line(&borrower1).is_some());
    assert!(client.get_credit_line(&borrower2).is_some());
    assert!(client.get_credit_line(&borrower3).is_some());

    // Try to create one outside bounds
    let borrower4 = Address::generate(&env);
    let result = client.try_open_credit_line(&borrower4, &2_000_000, &500, &50);
    assert!(result.is_err());
}

// ── Test 6: Edge Cases ────────────────────────────────────────────────────────

#[test]
fn test_bounds_with_very_large_values() {
    let (_env, client, _contract_id, _admin) = setup_env();

    // Set bounds with very large values
    client.set_credit_limit_bounds(&1_000_000_000_000, &i128::MAX);

    let (min, max) = client.get_credit_limit_bounds();
    assert_eq!(min, Some(1_000_000_000_000));
    assert_eq!(max, Some(i128::MAX));

    let borrower = Address::generate(&_env);
    client.open_credit_line(&borrower, &i128::MAX, &500, &50);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.credit_limit, i128::MAX);
}

#[test]
fn test_bounds_with_single_valid_value() {
    let (_env, client, _contract_id, _admin) = setup_env();

    // Set min == max (only one valid limit)
    client.set_credit_limit_bounds(&100_000, &100_000);

    let borrower1 = Address::generate(&_env);

    // Only 100_000 should be valid
    client.open_credit_line(&borrower1, &100_000, &500, &50);
    assert!(client.get_credit_line(&borrower1).is_some());

    let borrower2 = Address::generate(&_env);
    let result = client.try_open_credit_line(&borrower2, &100_001, &500, &50);
    assert!(result.is_err());

    let borrower3 = Address::generate(&_env);
    let result = client.try_open_credit_line(&borrower3, &99_999, &500, &50);
    assert!(result.is_err());
}

#[test]
fn test_get_bounds_when_not_configured() {
    let (_env, client, _contract_id, _admin) = setup_env();

    // Bounds should be None when not configured
    let (min, max) = client.get_credit_limit_bounds();
    assert_eq!(min, None);
    assert_eq!(max, None);
}

#[test]
fn test_bounds_enforced_during_protocol_pause() {
    let (_env, client, _contract_id, _admin) = setup_with_bounds(10_000, 1_000_000);

    // Pause protocol
    client.set_paused(&true);

    // Try to set new bounds while paused - should fail
    let result = client.try_set_credit_limit_bounds(&20_000, &2_000_000);
    assert!(
        result.is_err(),
        "Should not be able to set bounds while paused"
    );

    // Unpause
    client.set_paused(&false);

    // Now it should work
    client.set_credit_limit_bounds(&20_000, &2_000_000);
    let (min, max) = client.get_credit_limit_bounds();
    assert_eq!(min, Some(20_000));
    assert_eq!(max, Some(2_000_000));
}
