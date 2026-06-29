// SPDX-License-Identifier: MIT

//! Freeze draws tests for the Credit contract.
//!
//! # Coverage
//! - is_draws_frozen returns false on freshly initialized contract
//! - repay_credit succeeds while draws are frozen (critical safety feature)
//! - freeze_draws/unfreeze_draws toggle the flag correctly
//! - draw_credit is blocked when draws are frozen

use creditra_credit::{Credit, CreditClient, FreezeReason};
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

// ── is_draws_frozen default behavior ──────────────────────────────────────────

#[test]
fn is_draws_frozen_returns_false_on_freshly_initialized_contract() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    // On a freshly initialized contract, is_draws_frozen should return false
    assert!(
        !client.is_draws_frozen(),
        "is_draws_frozen should return false by default before any freeze_draws call"
    );
}

// ── freeze_draws/unfreeze_draws toggle ────────────────────────────────────────

#[test]
fn freeze_draws_sets_flag_to_true() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    assert!(!client.is_draws_frozen(), "should start unfrozen");

    client.freeze_draws(&FreezeReason::LiquidityReserve);
    assert!(
        client.is_draws_frozen(),
        "should be frozen after freeze_draws"
    );
}

#[test]
fn unfreeze_draws_sets_flag_to_false() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.freeze_draws(&FreezeReason::LiquidityReserve);
    assert!(client.is_draws_frozen());

    client.unfreeze_draws();
    assert!(
        !client.is_draws_frozen(),
        "should be unfrozen after unfreeze_draws"
    );
}

// ── draw_credit blocked when frozen ───────────────────────────────────────────

#[test]
fn draw_credit_blocked_when_draws_frozen() {
    let (env, _admin, contract_id, token_address) = setup_with_token();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    // Open line while unfrozen
    client.open_credit_line(&borrower, &1_000, &300, &50);

    // Mint tokens to contract for liquidity
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &1_000);

    // Freeze draws
    client.freeze_draws(&FreezeReason::LiquidityReserve);
    assert!(client.is_draws_frozen());

    // Draw should fail
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.draw_credit(&borrower, &500);
    }));

    assert!(
        result.is_err(),
        "draw_credit must fail when draws are frozen"
    );
}

// ── repay_credit succeeds while frozen (critical safety feature) ───────────────

#[test]
fn repay_credit_succeeds_while_draws_frozen() {
    let (env, _admin, contract_id, token_address) = setup_with_token();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    // Setup: open line, draw, then freeze draws
    client.open_credit_line(&borrower, &1_000, &300, &50);
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &1_000);
    client.draw_credit(&borrower, &500);

    let before = client.get_credit_line(&borrower).unwrap();
    assert_eq!(before.utilized_amount, 500);

    // Freeze draws
    client.freeze_draws(&FreezeReason::LiquidityReserve);
    assert!(client.is_draws_frozen());

    // Mint tokens to borrower and approve contract
    let sac = token::StellarAssetClient::new(&env, &token_address);
    sac.mint(&borrower, &200);
    token::Client::new(&env, &token_address).approve(&borrower, &contract_id, &200, &1_000);

    // Repay should succeed even when draws are frozen
    client.repay_credit(&borrower, &200);

    let after = client.get_credit_line(&borrower).unwrap();
    assert_eq!(
        after.utilized_amount, 300,
        "repayment must succeed when draws are frozen"
    );
}

#[test]
fn repay_credit_full_repayment_while_draws_frozen() {
    let (env, _admin, contract_id, token_address) = setup_with_token();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    // Setup
    client.open_credit_line(&borrower, &1_000, &300, &50);
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &1_000);
    client.draw_credit(&borrower, &800);

    // Freeze draws
    client.freeze_draws(&FreezeReason::LiquidityReserve);
    assert!(client.is_draws_frozen());

    // Full repayment
    let sac = token::StellarAssetClient::new(&env, &token_address);
    sac.mint(&borrower, &800);
    token::Client::new(&env, &token_address).approve(&borrower, &contract_id, &800, &1_000);

    client.repay_credit(&borrower, &800);

    let after = client.get_credit_line(&borrower).unwrap();
    assert_eq!(
        after.utilized_amount, 0,
        "full repayment must work when draws are frozen"
    );
}

// ── event emission ───────────────────────────────────────────────────────────

#[test]
fn freeze_draws_emits_event() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    let _ = env.events().all(); // clear setup events

    client.freeze_draws(&FreezeReason::LiquidityReserve);

    let events = env.events().all();
    assert_eq!(events.len(), 1, "should emit exactly one event");

    let (_contract, topics, _data) = events.last().unwrap();
    assert_eq!(
        Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
        Symbol::new(&env, "drw_freeze")
    );
}

#[test]
fn unfreeze_draws_emits_event() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.freeze_draws(&FreezeReason::LiquidityReserve);
    let _ = env.events().all(); // clear

    client.unfreeze_draws();

    let events = env.events().all();
    assert_eq!(events.len(), 1);

    let (_contract, topics, _data) = events.last().unwrap();
    assert_eq!(
        Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
        Symbol::new(&env, "drw_freeze")
    );
}
