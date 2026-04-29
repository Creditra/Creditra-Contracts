// SPDX-License-Identifier: MIT

//! Token transfer failure and rollback semantics tests for the Credit contract.
//!
//! # Coverage
//! - draw_credit: insufficient reserve balance prevents inconsistent state
//! - repay_credit: insufficient allowance/balance prevents inconsistent state
//! - Reentrancy guard is properly managed
//! - Utilization remains consistent on transfer failures
//! - Soroban atomicity prevents inconsistent states

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Events};
use soroban_sdk::{token, Address, Env, Symbol, TryFromVal};

// ── helpers ──────────────────────────────────────────────────────────────────

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    (env, admin, contract_id)
}

fn setup_with_token() -> (Env, Address, Address, Address) {
    let (env, admin, contract_id) = setup();
    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();
    let client = CreditClient::new(&env, &contract_id);
    client.set_liquidity_token(&token_address);
    (env, admin, contract_id, token_address)
}

// ── draw_credit failure rollback ─────────────────────────────────────────────

#[test]
fn draw_credit_insufficient_reserve_rolls_back() {
    let (env, _admin, contract_id, token_address) = setup_with_token();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    // Open line
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);

    // Mint less than needed to reserve
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &400);

    // Attempt draw - should fail due to insufficient reserve
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.draw_credit(&borrower, &500);
    }));

    assert!(result.is_err(), "draw_credit should fail on insufficient reserve");

    // Verify state is unchanged
    let line = client.get_credit_line(&borrower);
    assert_eq!(line.utilized_amount, 0, "utilized_amount should remain 0");
    assert_eq!(line.status, creditra_credit::types::CreditStatus::Active, "status should remain Active");

    // Verify no drawn event
    let events = env.events().all();
    assert_eq!(events.len(), 1, "should only have open_credit_line event");
}

#[test]
fn repay_credit_insufficient_allowance_rolls_back() {
    let (env, _admin, contract_id, token_address) = setup_with_token();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    // Open line and draw
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &1_000);
    client.draw_credit(&borrower, &500);

    // Mint tokens to borrower but don't approve enough
    token::StellarAssetClient::new(&env, &token_address).mint(&borrower, &500);
    token::Client::new(&env, &token_address).approve(&borrower, &contract_id, &200, &u32::MAX); // only 200 allowance

    // Attempt repay - should fail due to insufficient allowance
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.repay_credit(&borrower, &500);
    }));

    assert!(result.is_err(), "repay_credit should fail on insufficient allowance");

    // Verify state is unchanged
    let line = client.get_credit_line(&borrower);
    assert_eq!(line.utilized_amount, 500, "utilized_amount should remain 500");
    assert_eq!(line.accrued_interest, 0, "accrued_interest should remain 0");

    // Verify no repayment event
    let events = env.events().all();
    assert_eq!(events.len(), 2, "should have open and drawn events only");
}

#[test]
fn repay_credit_insufficient_balance_rolls_back() {
    let (env, _admin, contract_id, token_address) = setup_with_token();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    // Open line and draw
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &1_000);
    client.draw_credit(&borrower, &500);

    // Approve but don't mint enough tokens to borrower
    token::Client::new(&env, &token_address).approve(&borrower, &contract_id, &500, &u32::MAX);

    // Attempt repay - should fail due to insufficient balance
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.repay_credit(&borrower, &500);
    }));

    assert!(result.is_err(), "repay_credit should fail on insufficient balance");

    // Verify state is unchanged
    let line = client.get_credit_line(&borrower);
    assert_eq!(line.utilized_amount, 500, "utilized_amount should remain 500");

    // Verify no repayment event
    let events = env.events().all();
    assert_eq!(events.len(), 2, "should have open and drawn events only");
}

#[test]
fn reentrancy_guard_cleared_on_draw_failure() {
    let (env, _admin, contract_id, token_address) = setup_with_token();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    // Open line
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);

    // Mint insufficient reserve
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &400);

    // Fail the draw
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.draw_credit(&borrower, &500);
    }));

    // Now add enough tokens and try again - should work
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &100);
    client.draw_credit(&borrower, &500);

    let line = client.get_credit_line(&borrower);
    assert_eq!(line.utilized_amount, 500, "should succeed after fixing reserve");
}

#[test]
fn reentrancy_guard_cleared_on_repay_failure() {
    let (env, _admin, contract_id, token_address) = setup_with_token();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    // Open line and draw
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &1_000);
    client.draw_credit(&borrower, &500);

    // Approve insufficient
    token::Client::new(&env, &token_address).approve(&borrower, &contract_id, &200, &u32::MAX);

    // Fail the repay
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.repay_credit(&borrower, &500);
    }));

    // Now approve enough and try again
    token::Client::new(&env, &token_address).approve(&borrower, &contract_id, &500, &u32::MAX);
    token::StellarAssetClient::new(&env, &token_address).mint(&borrower, &500);
    client.repay_credit(&borrower, &500);

    let line = client.get_credit_line(&borrower);
    assert_eq!(line.utilized_amount, 0, "should succeed after fixing allowance");
}