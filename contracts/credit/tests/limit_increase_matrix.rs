// SPDX-License-Identifier: MIT

//! Integration test matrix for credit-limit *increase* logic.
//!
//! Drives `update_risk_parameters` (admin update entrypoint) and validates:
//! 1) Successful increase into a valid range.
//! 2) Fail-soft behavior for limit decreases below current utilization.
//!    Expect `LimitDecreaseRequiresRepayment = 13`.
//! 3) Hard bounds enforcement when attempting to increase above `MaxCreditLimit`.
//!    Expect `LimitOutOfBounds = 34`.

use soroban_sdk::{testutils::Address as _, Address, Env};

use creditra_credit::types::{ContractError, CreditStatus};
use creditra_credit::{Credit, CreditClient};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn setup_contract_with_bounds(
    max_credit_limit: i128,
) -> (Env, CreditClient<'static>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    let contract_id = env.register_contract(None, Credit);
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);

    // Requirement #2: Use `set_credit_limit_bounds` within the Env setup.
    // We configure Min = 0 and Max = fixed value, to make assertions deterministic.
    client.set_credit_limit_bounds(&0_i128, &max_credit_limit);

    (env, client, admin, borrower)
}

fn open_line_and_draw(
    client: &CreditClient<'_>,
    borrower: &Address,
    initial_limit: i128,
    draw: i128,
) {
    // Interest/risk params are chosen as stable defaults; the tests focus on
    // limit range + utilized_amount checks.
    let rate_bps = 300_u32;
    let risk_score = 50_u32;

    client.open_credit_line(borrower, &initial_limit, &rate_bps, &risk_score);
    client.draw_credit(borrower, &draw);

    let line = client.get_credit_line(borrower).unwrap();
    assert_eq!(line.credit_limit, initial_limit);
    assert_eq!(line.utilized_amount, draw);
}

// ── Test Matrix ───────────────────────────────────────────────────────────────

#[test]
fn test_limit_increase_matrix_success_in_range() {
    // Case 1: Success
    // Increase limit to value >= current utilized_amount and <= MaxCreditLimit.
    let max_credit_limit = 10_000_i128;
    let (env, client, _admin, borrower) = setup_contract_with_bounds(max_credit_limit);

    let initial_limit = 6_000_i128;
    let utilized = 4_500_i128;
    open_line_and_draw(&client, &borrower, initial_limit, utilized);

    let new_limit = 8_000_i128; // >= utilized and <= max
    let new_rate_bps = 350_u32;
    let new_risk_score = 60_u32;

    client.update_risk_parameters(&borrower, &new_limit, &new_rate_bps, &new_risk_score);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.credit_limit, new_limit);
    assert_eq!(line.utilized_amount, utilized);
    // When limit >= utilized, the line should be Active.
    assert_eq!(
        line.status,
        CreditStatus::Active,
        "Expected Active when limit >= utilized"
    );

    let _ = env; // silence unused warning in older toolchains
}

#[test]
fn test_limit_increase_matrix_fail_soft_noop_or_repayment_error_below_utilized() {
    // Case 2: Fail-soft no-op / repayment error
    // Attempt to set limit < utilized_amount.
    // Expect `LimitDecreaseRequiresRepayment = 13`.

    let max_credit_limit = 10_000_i128;
    let (_env, client, _admin, borrower) = setup_contract_with_bounds(max_credit_limit);

    let initial_limit = 9_000_i128;
    let utilized = 5_000_i128;
    open_line_and_draw(&client, &borrower, initial_limit, utilized);

    let decreased_below_utilized = utilized - 1; // < utilized
    let new_rate_bps = 300_u32;
    let new_risk_score = 50_u32;

    // Prefer explicit Result-based assertion.
    let result = client.try_update_risk_parameters(
        &borrower,
        &decreased_below_utilized,
        &new_rate_bps,
        &new_risk_score,
    );

    assert!(
        result.is_err(),
        "Expected contract error when decreasing below utilization"
    );
    let err = result.err().unwrap();

    assert_eq!(
        err,
        ContractError::LimitDecreaseRequiresRepayment,
        "Expected LimitDecreaseRequiresRepayment discriminant (13)"
    );
}

#[test]
fn test_limit_increase_matrix_out_of_bounds_increase_above_max() {
    // Case 3: Out of Bounds error
    // Attempt to increase above configured `MaxCreditLimit`.
    // Expect `LimitOutOfBounds = 34`.

    let max_credit_limit = 10_000_i128;
    let (_env, client, _admin, borrower) = setup_contract_with_bounds(max_credit_limit);

    let initial_limit = 9_000_i128;
    let utilized = 4_000_i128;
    open_line_and_draw(&client, &borrower, initial_limit, utilized);

    let out_of_bounds_limit = max_credit_limit + 1; // > max
    let new_rate_bps = 300_u32;
    let new_risk_score = 50_u32;

    let result = client.try_update_risk_parameters(
        &borrower,
        &out_of_bounds_limit,
        &new_rate_bps,
        &new_risk_score,
    );

    assert!(
        result.is_err(),
        "Expected LimitOutOfBounds when increasing above max"
    );
    let err = result.err().unwrap();

    assert_eq!(
        err,
        ContractError::LimitOutOfBounds,
        "Expected LimitOutOfBounds discriminant (34)"
    );
}
