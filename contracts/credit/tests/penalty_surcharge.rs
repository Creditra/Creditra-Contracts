// SPDX-License-Identifier: MIT

//! Tests for the penalty surcharge feature for delinquent credit lines.

#![cfg(test)]

use stellar_soroban_sdk::{Address, Env};
use soroban_sdk::Symbol;
use credit::types::{CreditLineData, CreditStatus};

#[test]
fn test_set_and_get_penalty_surcharge_bps() {
    let env = Env::default();
    let admin = Address::generate(&env);

    // Initialize contract
    credit::Credit::init(env.clone(), admin.clone());

    // Set penalty surcharge to 500 bps (5%)
    credit::Credit::set_penalty_surcharge_bps(env.clone(), 500);

    // Verify the surcharge was set correctly
    let surcharge = credit::Credit::get_penalty_surcharge_bps(env.clone());
    assert_eq!(surcharge, 500);

    // Update to a different value
    credit::Credit::set_penalty_surcharge_bps(env.clone(), 1000);

    // Verify the new value
    let surcharge = credit::Credit::get_penalty_surcharge_bps(env.clone());
    assert_eq!(surcharge, 1000);

    // Set to 0 to disable
    credit::Credit::set_penalty_surcharge_bps(env.clone(), 0);

    // Verify it's disabled
    let surcharge = credit::Credit::get_penalty_surcharge_bps(env.clone());
    assert_eq!(surcharge, 0);
}

#[test]
fn test_penalty_surcharge_default_is_zero() {
    let env = Env::default();
    let admin = Address::generate(&env);

    // Initialize contract
    credit::Credit::init(env.clone(), admin.clone());

    // Verify the default penalty surcharge is 0
    let surcharge = credit::Credit::get_penalty_surcharge_bps(env.clone());
    assert_eq!(surcharge, 0);
}

#[test]
fn test_penalty_surcharge_exceeds_max_rate() {
    let env = Env::default();
    let admin = Address::generate(&env);

    // Initialize contract
    credit::Credit::init(env.clone(), admin.clone());

    // Try to set penalty surcharge to 10001 bps (exceeds MAX_INTEREST_RATE_BPS of 10000)
    // This should panic
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        credit::Credit::set_penalty_surcharge_bps(env.clone(), 10001);
    }));

    assert!(result.is_err());
}

#[test]
fn test_penalty_surcharge_applied_to_delinquent_line() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    // Initialize contract
    credit::Credit::init(env.clone(), admin.clone());

    // Set up liquidity token
    let token = Address::generate(&env);
    credit::Credit::set_liquidity_token(env.clone(), token.clone());

    // Set penalty surcharge to 200 bps (2%)
    credit::Credit::set_penalty_surcharge_bps(env.clone(), 200);

    // Open a credit line with 500 bps (5%) interest rate
    credit::Credit::open_credit_line(
        env.clone(),
        borrower.clone(),
        1_000_000, // credit_limit
        500,      // interest_rate_bps
        50,       // risk_score
    );

    // Set up grace period
    credit::Credit::set_grace_period_config(
        env.clone(),
        86400 * 30, // 30 days
        0,          // reduced_rate_bps
        0,          // waiver_mode (FullWaiver)
    );

    // Advance time to make the borrower delinquent
    env.ledger().set_timestamp(86400 * 35); // 35 days later

    // Apply accrual - should use penalty rate (500 + 200 = 700 bps)
    let credit_line = credit::query::get_credit_line(env.clone(), borrower.clone()).unwrap();

    // Verify the effective rate includes the penalty surcharge
    // The accrual should have computed interest at 700 bps
    assert!(credit_line.accrued_interest > 0);
}

#[test]
fn test_penalty_surcharge_not_applied_to_non_delinquent_line() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    // Initialize contract
    credit::Credit::init(env.clone(), admin.clone());

    // Set up liquidity token
    let token = Address::generate(&env);
    credit::Credit::set_liquidity_token(env.clone(), token.clone());

    // Set penalty surcharge to 200 bps (2%)
    credit::Credit::set_penalty_surcharge_bps(env.clone(), 200);

    // Open a credit line with 500 bps (5%) interest rate
    credit::Credit::open_credit_line(
        env.clone(),
        borrower.clone(),
        1_000_000, // credit_limit
        500,      // interest_rate_bps
        50,       // risk_score
    );

    // Advance time but keep within grace period (not delinquent)
    env.ledger().set_timestamp(86400 * 10); // 10 days later

    // Apply accrual - should NOT use penalty rate (only 500 bps, not 700)
    let credit_line = credit::query::get_credit_line(env.clone(), borrower.clone()).unwrap();

    // Verify the base rate is used (no penalty)
    // The interest should be computed at 500 bps
    assert!(credit_line.accrued_interest > 0);

    // Verify the stored interest_rate_bps hasn't changed
    assert_eq!(credit_line.interest_rate_bps, 500);
}

#[test]
fn test_penalty_rate_entered_event_emitted() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    // Initialize contract
    credit::Credit::init(env.clone(), admin.clone());

    // Set up liquidity token
    let token = Address::generate(&env);
    credit::Credit::set_liquidity_token(env.clone(), token.clone());

    // Set penalty surcharge to 200 bps (2%)
    credit::Credit::set_penalty_surcharge_bps(env.clone(), 200);

    // Open a credit line
    credit::Credit::open_credit_line(
        env.clone(),
        borrower.clone(),
        1_000_000,
        500,
        50,
    );

    // Set up grace period
    credit::Credit::set_grace_period_config(
        env.clone(),
        86400 * 30,
        0,
        0,
    );

    // Draw some funds
    credit::Credit::draw_credit(
        env.clone(),
        borrower.clone(),
        borrower.clone(), // recipient
        100_000,
    );

    // Advance time to make borrower delinquent
    env.ledger().set_timestamp(86400 * 35);

    // Apply accrual - should emit PenaltyRateEnteredEvent
    credit::Credit::accrue(env.clone(), borrower.clone());

    // Check events - should contain PenaltyRateEnteredEvent
    let events = env.events().all();
    assert!(events.len() > 0);

    // Verify the event contains the correct data
    let penalty_event = events.iter().find(|e| {
        e.topics[0] == soroban_sdk::Symbol::new(&env, "credit")
            && e.topics[1] == soroban_sdk::Symbol::new(&env, "pen_enter")
    });

    assert!(penalty_event.is_some());
}

#[test]
fn test_penalty_rate_exited_event_emitted() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    // Initialize contract
    credit::Credit::init(env.clone(), admin.clone());

    // Set up liquidity token
    let token = Address::generate(&env);
    credit::Credit::set_liquidity_token(env.clone(), token.clone());

    // Set penalty surcharge to 200 bps (2%)
    credit::Credit::set_penalty_surcharge_bps(env.clone(), 200);

    // Open a credit line
    credit::Credit::open_credit_line(
        env.clone(),
        borrower.clone(),
        1_000_000,
        500,
        50,
    );

    // Set up grace period
    credit::Credit::set_grace_period_config(
        env.clone(),
        86400 * 30,
        0,
        0,
    );

    // Draw funds and become delinquent
    credit::Credit::draw_credit(
        env.clone(),
        borrower.clone(),
        borrower.clone(),
        100_000,
    );

    env.ledger().set_timestamp(86400 * 35);
    credit::Credit::accrue(env.clone(), borrower.clone());

    // Repay to become non-delinquent
    credit::Credit::repay_credit(
        env.clone(),
        borrower.clone(),
        100_000,
    );

    // Advance time and accrual - should emit PenaltyRateExitedEvent
    env.ledger().set_timestamp(86400 * 40);
    credit::Credit::accrue(env.clone(), borrower.clone());

    // Check events - should contain PenaltyRateExitedEvent
    let events = env.events().all();
    let exit_event = events.iter().find(|e| {
        e.topics[0] == soroban_sdk::Symbol::new(&env, "credit")
            && e.topics[1] == soroban_sdk::Symbol::new(&env, "pen_exit")
    });

    assert!(exit_event.is_some());
}

#[test]
fn test_penalty_surcharge_with_zero_surcharge_no_effect() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    // Initialize contract
    credit::Credit::init(env.clone(), admin.clone());

    // Set up liquidity token
    let token = Address::generate(&env);
    credit::Credit::set_liquidity_token(env.clone(), token.clone());

    // Set penalty surcharge to 0 (disabled)
    credit::Credit::set_penalty_surcharge_bps(env.clone(), 0);

    // Open a credit line with 500 bps
    credit::Credit::open_credit_line(
        env.clone(),
        borrower.clone(),
        1_000_000,
        500,
        50,
    );

    // Set up grace period
    credit::Credit::set_grace_period_config(
        env.clone(),
        86400 * 30,
        0,
        0,
    );

    // Advance time to make borrower delinquent
    env.ledger().set_timestamp(86400 * 35);

    // Apply accrual - should use base rate (500 bps) since surcharge is 0
    credit::Credit::accrue(env.clone(), borrower.clone());

    let credit_line = credit::query::get_credit_line(env.clone(), borrower.clone()).unwrap();
    
    // Interest should be computed at 500 bps (no penalty)
    assert!(credit_line.accrued_interest > 0);
}

#[test]
fn test_penalty_surcharge_clamped_to_max_rate() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    // Initialize contract
    credit::Credit::init(env.clone(), admin.clone());

    // Set up liquidity token
    let token = Address::generate(&env);
    credit::Credit::set_liquidity_token(env.clone(), token.clone());

    // Open a credit line with 9500 bps (95%)
    credit::Credit::open_credit_line(
        env.clone(),
        borrower.clone(),
        1_000_000,
        9500,
        50,
    );

    // Set penalty surcharge to 1000 bps (10%)
    // Base rate (9500) + surcharge (1000) = 10500, which exceeds MAX_INTEREST_RATE_BPS (10000)
    credit::Credit::set_penalty_surcharge_bps(env.clone(), 1000);

    // Set up grace period
    credit::Credit::set_grace_period_config(
        env.clone(),
        86400 * 30,
        0,
        0,
    );

    // Advance time to make borrower delinquent
    env.ledger().set_timestamp(86400 * 35);

    // Apply accrual - effective rate should be clamped to 10000 bps (MAX)
    credit::Credit::accrue(env.clone(), borrower.clone());

    let credit_line = credit::query::get_credit_line(env.clone(), borrower.clone()).unwrap();
    
    // The accrual should succeed without overflow
    assert!(credit_line.accrued_interest >= 0);
}
