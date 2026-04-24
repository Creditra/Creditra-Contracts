// SPDX-License-Identifier: MIT

//! Circuit breaker (emergency pause) tests for the Credit contract.
//!
//! # Coverage
//! - Admin can pause/unpause the protocol
//! - Non-admin cannot pause/unpause
//! - When paused, all mutating operations except repay_credit are blocked
//! - repay_credit works even when paused (critical safety feature)
//! - Read-only operations work when paused
//! - Events are emitted on pause/unpause

use creditra_credit::types::{ContractError, CreditStatus};
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Events, Ledger};
use soroban_sdk::{token, Address, Env, Symbol};

// ── helpers ──────────────────────────────────────────────────────────────────

fn setup() -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    (env, admin, contract_id, contract_id.clone())
}

fn setup_with_token() -> (Env, Address, Address, Address, Address) {
    let (env, admin, contract_id, _) = setup();
    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();
    let client = CreditClient::new(&env, &contract_id);
    client.set_liquidity_token(&token_address);
    (env, admin, contract_id, token_address, contract_id.clone())
}

// ── pause/unpause authorization ──────────────────────────────────────────────

#[test]
fn admin_can_pause_protocol() {
    let (env, _admin, contract_id, _) = setup();
    let client = CreditClient::new(&env, &contract_id);

    assert!(!client.is_protocol_paused(), "should start unpaused");

    client.set_protocol_paused(&true);
    assert!(client.is_protocol_paused(), "should be paused after set");
}

#[test]
fn admin_can_unpause_protocol() {
    let (env, _admin, contract_id, _) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_protocol_paused(&true);
    assert!(client.is_protocol_paused());

    client.set_protocol_paused(&false);
    assert!(!client.is_protocol_paused(), "should be unpaused");
}

#[test]
#[should_panic]
fn non_admin_cannot_pause() {
    let (env, _admin, contract_id, _) = setup();
    env.mock_all_auths_allowing_non_root_auth();
    let non_admin = Address::generate(&env);
    let client = CreditClient::new(&env, &contract_id);

    // This should panic with auth error
    non_admin.require_auth();
    client.set_protocol_paused(&true);
}

// ── event emission ───────────────────────────────────────────────────────────

#[test]
fn pause_emits_event() {
    let (env, admin, contract_id, _) = setup();
    let client = CreditClient::new(&env, &contract_id);

    let _ = env.events().all(); // clear setup events

    client.set_protocol_paused(&true);

    let events = env.events().all();
    assert_eq!(events.len(), 1, "should emit exactly one event");

    let (_contract, topics, _data) = events.last().unwrap();
    assert_eq!(
        Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
        Symbol::new(&env, "paused")
    );
}

#[test]
fn unpause_emits_event() {
    let (env, _admin, contract_id, _) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_protocol_paused(&true);
    let _ = env.events().all(); // clear

    client.set_protocol_paused(&false);

    let events = env.events().all();
    assert_eq!(events.len(), 1);

    let (_contract, topics, _data) = events.last().unwrap();
    assert_eq!(
        Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
        Symbol::new(&env, "unpaused")
    );
}

// ── blocked operations when paused ───────────────────────────────────────────

#[test]
fn open_credit_line_blocked_when_paused() {
    let (env, _admin, contract_id, _) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.set_protocol_paused(&true);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.open_credit_line(&borrower, &1_000, &300, &50);
    }));

    assert!(result.is_err(), "open_credit_line must fail when paused");
}

#[test]
fn draw_credit_blocked_when_paused() {
    let (env, _admin, contract_id, token_address, _) = setup_with_token();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    // Open line while unpaused
    client.open_credit_line(&borrower, &1_000, &300, &50);

    // Mint tokens to contract for liquidity
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &1_000);

    // Pause
    client.set_protocol_paused(&true);

    // Draw should fail
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.draw_credit(&borrower, &500);
    }));

    assert!(result.is_err(), "draw_credit must fail when paused");
}

#[test]
fn update_risk_parameters_blocked_when_paused() {
    let (env, _admin, contract_id, _) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &1_000, &300, &50);
    client.set_protocol_paused(&true);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.update_risk_parameters(&borrower, &2_000, &400, &60);
    }));

    assert!(
        result.is_err(),
        "update_risk_parameters must fail when paused"
    );
}

#[test]
fn suspend_credit_line_blocked_when_paused() {
    let (env, _admin, contract_id, _) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &1_000, &300, &50);
    client.set_protocol_paused(&true);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.suspend_credit_line(&borrower);
    }));

    assert!(
        result.is_err(),
        "suspend_credit_line must fail when paused"
    );
}

#[test]
fn close_credit_line_blocked_when_paused() {
    let (env, admin, contract_id, _) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &1_000, &300, &50);
    client.set_protocol_paused(&true);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.close_credit_line(&borrower, &admin);
    }));

    assert!(result.is_err(), "close_credit_line must fail when paused");
}

#[test]
fn default_credit_line_blocked_when_paused() {
    let (env, _admin, contract_id, _) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &1_000, &300, &50);
    client.set_protocol_paused(&true);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.default_credit_line(&borrower);
    }));

    assert!(
        result.is_err(),
        "default_credit_line must fail when paused"
    );
}

#[test]
fn reinstate_credit_line_blocked_when_paused() {
    let (env, _admin, contract_id, _) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &1_000, &300, &50);
    client.default_credit_line(&borrower);
    client.set_protocol_paused(&true);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.reinstate_credit_line(&borrower);
    }));

    assert!(
        result.is_err(),
        "reinstate_credit_line must fail when paused"
    );
}

#[test]
fn set_liquidity_token_blocked_when_paused() {
    let (env, _admin, contract_id, _) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let token = Address::generate(&env);

    client.set_protocol_paused(&true);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.set_liquidity_token(&token);
    }));

    assert!(
        result.is_err(),
        "set_liquidity_token must fail when paused"
    );
}

#[test]
fn set_liquidity_source_blocked_when_paused() {
    let (env, _admin, contract_id, _) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let source = Address::generate(&env);

    client.set_protocol_paused(&true);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.set_liquidity_source(&source);
    }));

    assert!(
        result.is_err(),
        "set_liquidity_source must fail when paused"
    );
}

#[test]
fn set_rate_change_limits_blocked_when_paused() {
    let (env, _admin, contract_id, _) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_protocol_paused(&true);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.set_rate_change_limits(&500, &3600);
    }));

    assert!(
        result.is_err(),
        "set_rate_change_limits must fail when paused"
    );
}

#[test]
fn set_max_draw_amount_blocked_when_paused() {
    let (env, _admin, contract_id, _) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_protocol_paused(&true);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.set_max_draw_amount(&10_000);
    }));

    assert!(
        result.is_err(),
        "set_max_draw_amount must fail when paused"
    );
}

// ── repay_credit exception (critical safety feature) ─────────────────────────

#[test]
fn repay_credit_works_when_paused() {
    let (env, _admin, contract_id, token_address, _) = setup_with_token();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    // Setup: open line, draw, then pause
    client.open_credit_line(&borrower, &1_000, &300, &50);
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &1_000);
    client.draw_credit(&borrower, &500);

    let before = client.get_credit_line(&borrower).unwrap();
    assert_eq!(before.utilized_amount, 500);

    // Pause the protocol
    client.set_protocol_paused(&true);
    assert!(client.is_protocol_paused());

    // Mint tokens to borrower and approve contract
    let sac = token::StellarAssetClient::new(&env, &token_address);
    sac.mint(&borrower, &200);
    token::Client::new(&env, &token_address).approve(&borrower, &contract_id, &200, &1_000);

    // Repay should succeed even when paused
    client.repay_credit(&borrower, &200);

    let after = client.get_credit_line(&borrower).unwrap();
    assert_eq!(after.utilized_amount, 300, "repayment must succeed when paused");
}

#[test]
fn repay_credit_full_repayment_when_paused() {
    let (env, _admin, contract_id, token_address, _) = setup_with_token();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    // Setup
    client.open_credit_line(&borrower, &1_000, &300, &50);
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &1_000);
    client.draw_credit(&borrower, &800);

    // Pause
    client.set_protocol_paused(&true);

    // Full repayment
    let sac = token::StellarAssetClient::new(&env, &token_address);
    sac.mint(&borrower, &800);
    token::Client::new(&env, &token_address).approve(&borrower, &contract_id, &800, &1_000);

    client.repay_credit(&borrower, &800);

    let after = client.get_credit_line(&borrower).unwrap();
    assert_eq!(after.utilized_amount, 0, "full repayment must work when paused");
}

// ── read-only operations work when paused ────────────────────────────────────

#[test]
fn get_credit_line_works_when_paused() {
    let (env, _admin, contract_id, _) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &1_000, &300, &50);
    client.set_protocol_paused(&true);

    // Read should work
    let line = client.get_credit_line(&borrower);
    assert!(line.is_some(), "get_credit_line must work when paused");
    assert_eq!(line.unwrap().credit_limit, 1_000);
}

#[test]
fn is_protocol_paused_always_works() {
    let (env, _admin, contract_id, _) = setup();
    let client = CreditClient::new(&env, &contract_id);

    assert!(!client.is_protocol_paused());

    client.set_protocol_paused(&true);
    assert!(client.is_protocol_paused());

    client.set_protocol_paused(&false);
    assert!(!client.is_protocol_paused());
}

#[test]
fn get_rate_change_limits_works_when_paused() {
    let (env, _admin, contract_id, _) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_rate_change_limits(&500, &3600);
    client.set_protocol_paused(&true);

    let limits = client.get_rate_change_limits();
    assert!(limits.is_some(), "get_rate_change_limits must work when paused");
}

#[test]
fn get_max_draw_amount_works_when_paused() {
    let (env, _admin, contract_id, _) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_max_draw_amount(&5_000);
    client.set_protocol_paused(&true);

    let max = client.get_max_draw_amount();
    assert!(max.is_some(), "get_max_draw_amount must work when paused");
}

// ── pause/unpause idempotency ────────────────────────────────────────────────

#[test]
fn pause_when_already_paused_is_idempotent() {
    let (env, _admin, contract_id, _) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_protocol_paused(&true);
    assert!(client.is_protocol_paused());

    // Pause again
    client.set_protocol_paused(&true);
    assert!(client.is_protocol_paused());
}

#[test]
fn unpause_when_already_unpaused_is_idempotent() {
    let (env, _admin, contract_id, _) = setup();
    let client = CreditClient::new(&env, &contract_id);

    assert!(!client.is_protocol_paused());

    // Unpause again
    client.set_protocol_paused(&false);
    assert!(!client.is_protocol_paused());
}

// ── operations resume after unpause ──────────────────────────────────────────

#[test]
fn operations_resume_after_unpause() {
    let (env, _admin, contract_id, _) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    // Pause
    client.set_protocol_paused(&true);

    // Verify open fails
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.open_credit_line(&borrower, &1_000, &300, &50);
    }));
    assert!(result.is_err());

    // Unpause
    client.set_protocol_paused(&false);

    // Now open should succeed
    client.open_credit_line(&borrower, &1_000, &300, &50);
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.credit_limit, 1_000);
}
