// SPDX-License-Identifier: MIT

//! Freeze draws tests for the Credit contract.
//!
//! # Coverage
//! - is_draws_frozen returns false on freshly initialized contract
//! - repay_credit succeeds while draws are frozen (critical safety feature)
//! - freeze_draws/unfreeze_draws toggle the flag correctly
//! - draw_credit is blocked when draws are frozen
//! - freeze_draws/unfreeze_draws are idempotent

use creditra_credit::types::ContractError;
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

    // Setup: open line, collateralize, draw, then freeze draws
    client.open_credit_line(&borrower, &1_000, &300, &50);
    // `draw_credit` enforces the collateral-ratio floor (ContractError #35), so
    // a collateralized line is required. The borrower must hold the collateral
    // token before depositing it.
    let sac = token::StellarAssetClient::new(&env, &token_address);
    sac.mint(&borrower, &100_000);
    sac.mint(&contract_id, &1_000);
    client.deposit_collateral(&borrower, &100_000_i128);
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
    let sac = token::StellarAssetClient::new(&env, &token_address);
    sac.mint(&borrower, &100_000);
    sac.mint(&contract_id, &1_000);
    client.deposit_collateral(&borrower, &100_000_i128);
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

// ── idempotent behavior ────────────────────────────────────────────────────────

#[test]
fn freeze_draws_idempotent() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.freeze_draws(&FreezeReason::LiquidityReserve);
    assert!(client.is_draws_frozen());

    // Freeze again - should succeed and remain frozen
    client.freeze_draws(&FreezeReason::LiquidityReserve);
    assert!(client.is_draws_frozen(), "should remain frozen after redundant freeze");
}

#[test]
fn unfreeze_draws_idempotent() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    // Unfreeze when already unfrozen - should succeed
    client.unfreeze_draws();
    assert!(!client.is_draws_frozen(), "should remain unfrozen after redundant unfreeze");
}

// ── Draw-error precedence across every draw-blocking layer (Issue #1358) ─────
//
// `draw_credit` evaluates its guards in a fixed order:
//
//   paused (#18) > global freeze (#19) > borrower freeze (#40) > line freeze (#46) > status
//
// `repay_credit` is deliberately not gated by any of them, so repayment remains
// available in every combination. These tests pin the precedence so a future
// reordering of the guard chain is caught immediately, and confirm that
// self-suspension composes with each freeze layer instead of replacing it.

fn setup_active_line_with_liquidity() -> (Env, Address, Address, Address, Address) {
    let (env, admin, contract_id, token_address) = setup_with_token();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &10_000_i128, &300_u32, &50_u32);
    let sac = token::StellarAssetClient::new(&env, &token_address);
    sac.mint(&borrower, &1_000_000_i128);
    sac.mint(&contract_id, &1_000_000_i128);
    client.deposit_collateral(&borrower, &100_000_i128);

    (env, admin, borrower, contract_id, token_address)
}

fn assert_draw_error(
    client: &CreditClient<'_>,
    borrower: &Address,
    amount: i128,
    expected: ContractError,
) {
    let result = client.try_draw_credit(borrower, &amount);
    let err = result
        .err()
        .expect("draw_credit should have failed")
        .expect("expected a typed contract error");
    assert_eq!(err, expected.into(), "unexpected draw error");
}

#[test]
fn draw_error_precedence_runs_paused_global_borrower_line() {
    let (env, admin, borrower, contract_id, _token) = setup_active_line_with_liquidity();
    let client = CreditClient::new(&env, &contract_id);

    // Stack all four layers, then peel them off in precedence order.
    client.freeze_credit_line(&borrower, &FreezeReason::Compliance);
    client.freeze_borrower_until(&admin, &borrower, &(env.ledger().timestamp() + 10_000));
    client.freeze_draws(&FreezeReason::LiquidityReserve);
    client.set_protocol_paused(&true);

    assert_draw_error(&client, &borrower, 100, ContractError::Paused);

    client.set_protocol_paused(&false);
    assert_draw_error(&client, &borrower, 100, ContractError::DrawsFrozen);

    client.unfreeze_draws();
    assert_draw_error(&client, &borrower, 100, ContractError::BorrowerFrozen);

    client.unfreeze_borrower(&admin, &borrower);
    assert_draw_error(&client, &borrower, 100, ContractError::CreditLineFrozen);

    // Every layer lifted: the draw now goes through.
    client.unfreeze_credit_line(&borrower);
    client.draw_credit(&borrower, &100);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().utilized_amount,
        100
    );
}

#[test]
fn draw_error_precedence_line_freeze_beats_suspended_status() {
    let (env, _admin, borrower, contract_id, _token) = setup_active_line_with_liquidity();
    let client = CreditClient::new(&env, &contract_id);

    client.freeze_credit_line(&borrower, &FreezeReason::Compliance);
    client.self_suspend_credit_line(&borrower);

    // Freeze layers are evaluated before the status check, so the freeze error
    // is the one callers observe even after the line is self-suspended.
    assert_draw_error(&client, &borrower, 100, ContractError::CreditLineFrozen);
}

#[test]
fn repayment_is_available_with_every_layer_stacked() {
    let (env, admin, borrower, contract_id, token) = setup_active_line_with_liquidity();
    let client = CreditClient::new(&env, &contract_id);

    client.draw_credit(&borrower, &2_000_i128);
    client.freeze_draws(&FreezeReason::LiquidityReserve);
    client.freeze_borrower_until(&admin, &borrower, &(env.ledger().timestamp() + 10_000));
    client.freeze_credit_line(&borrower, &FreezeReason::Compliance);
    client.self_suspend_credit_line(&borrower);

    assert_draw_error(&client, &borrower, 100, ContractError::DrawsFrozen);

    token::Client::new(&env, &token).approve(&borrower, &contract_id, &500_i128, &1_000_u32);
    client.repay_credit(&borrower, &500_i128);

    assert_eq!(
        client.get_credit_line(&borrower).unwrap().utilized_amount,
        1_500
    );
}
