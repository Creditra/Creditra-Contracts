// SPDX-License-Identifier: MIT

//! Focused tests for the `collateral_capabilities` view (v7, issue #861).
//!
//! # What
//!
//! Exercises the [`creditra_collateral::views::collateral_capabilities`] helper
//! across all permutations of protocol state that affect each capability flag:
//!
//! - `can_deposit` — blocked by: paused protocol, no credit line, Closed line, blocked borrower.
//! - `can_withdraw` — additionally blocked when collateral balance is zero.
//! - `can_partial_release` — additionally blocked when utilization is zero.
//! - `collateral_required` — true only when `min_ratio_bps > 0` AND utilization > 0.
//! - `collateral_balance` — mirrors `get_collateral` value.
//! - `min_ratio_bps` — mirrors `get_min_collateral_ratio_bps` value.
//!
//! # See also
//! - `contracts/credit/src/types.rs::CollateralCapabilities` — struct definition.
//! - `contracts/collateral/src/views.rs` — implementation.

use creditra_collateral::views::collateral_capabilities;
use creditra_collateral::Credit;
use creditra_collateral::CreditClient;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{token, Address, Env};

const INITIAL_TS: u64 = 1_000;

// ── Setup helpers ────────────────────────────────────────────────────────────

fn setup() -> (Env, Address, CreditClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = INITIAL_TS);

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_addr = token_id.address();
    client.set_liquidity_token(&token_addr);
    client.set_liquidity_source(&contract_id);

    token::StellarAssetClient::new(&env, &token_addr).mint(&contract_id, &5_000_000_i128);

    (env, contract_id, client, admin)
}

// ── Test 1: no credit line ───────────────────────────────────────────────────

/// A phantom borrower with no credit line must get all-false structural flags.
#[test]
fn caps_no_credit_line_all_blocked() {
    let (env, contract_id, _client, _) = setup();
    let phantom = Address::generate(&env);

    let caps = collateral_capabilities(env, contract_id, phantom);

    assert!(!caps.can_deposit, "no line: can_deposit must be false");
    assert!(!caps.can_withdraw, "no line: can_withdraw must be false");
    assert!(!caps.can_partial_release, "no line: can_partial_release must be false");
    assert!(!caps.collateral_required, "no line: collateral_required must be false");
    assert_eq!(caps.collateral_balance, 0);
    assert_eq!(caps.min_ratio_bps, 0);
}

// ── Test 2: active line, no collateral ───────────────────────────────────────

/// An active line with no collateral deposited: can_deposit=true, rest structural flags false.
#[test]
fn caps_active_line_no_collateral() {
    let (env, contract_id, client, _) = setup();
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &50_000_i128, &500_u32, &30_u32);

    let caps = collateral_capabilities(env, contract_id, borrower);

    assert!(caps.can_deposit, "active line: can_deposit must be true");
    assert!(!caps.can_withdraw, "no collateral: can_withdraw must be false");
    assert!(!caps.can_partial_release, "no collateral: can_partial_release must be false");
    assert!(!caps.collateral_required, "no min ratio set: collateral_required must be false");
    assert_eq!(caps.collateral_balance, 0);
    assert_eq!(caps.min_ratio_bps, 0);
}

// ── Test 3: active line, collateral deposited, no utilization ────────────────

/// After depositing collateral: can_withdraw=true, but can_partial_release=false (no draw).
#[test]
fn caps_active_line_with_collateral_no_draw() {
    let (env, contract_id, client, _) = setup();
    let borrower = Address::generate(&env);

    // Mint collateral token for borrower.
    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let col_token = token_id.address();
    token::StellarAssetClient::new(&env, &col_token).mint(&borrower, &100_000_i128);

    client.open_credit_line(&borrower, &50_000_i128, &500_u32, &30_u32);
    client.deposit_collateral(&borrower, &10_000_i128);

    let caps = collateral_capabilities(env, contract_id, borrower);

    assert!(caps.can_deposit, "active line: can_deposit must be true");
    assert!(caps.can_withdraw, "collateral deposited: can_withdraw must be true");
    assert!(!caps.can_partial_release, "no draw: can_partial_release must be false");
    assert_eq!(caps.collateral_balance, 10_000_i128);
}

// ── Test 4: active line, collateral deposited, drawn ─────────────────────────

/// With collateral AND utilization: all three operation flags true.
#[test]
fn caps_active_line_with_collateral_and_draw() {
    let (env, contract_id, client, _) = setup();
    let borrower = Address::generate(&env);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let col_token = token_id.address();
    token::StellarAssetClient::new(&env, &col_token).mint(&borrower, &100_000_i128);

    client.open_credit_line(&borrower, &50_000_i128, &500_u32, &30_u32);
    client.deposit_collateral(&borrower, &20_000_i128);
    client.draw_credit(&borrower, &5_000_i128);

    let caps = collateral_capabilities(env, contract_id, borrower);

    assert!(caps.can_deposit);
    assert!(caps.can_withdraw);
    assert!(caps.can_partial_release, "collateral + draw: can_partial_release must be true");
    assert_eq!(caps.collateral_balance, 20_000_i128);
}

// ── Test 5: paused protocol blocks deposit and withdraw ───────────────────────

/// Pausing the protocol must set all structural operation flags to false.
#[test]
fn caps_paused_protocol_blocks_all() {
    let (env, contract_id, client, _) = setup();
    let borrower = Address::generate(&env);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let col_token = token_id.address();
    token::StellarAssetClient::new(&env, &col_token).mint(&borrower, &100_000_i128);

    client.open_credit_line(&borrower, &50_000_i128, &500_u32, &30_u32);
    client.deposit_collateral(&borrower, &10_000_i128);
    client.draw_credit(&borrower, &5_000_i128);

    // Pause the protocol.
    client.pause_protocol();

    let caps = collateral_capabilities(env, contract_id, borrower);

    assert!(!caps.can_deposit, "paused: can_deposit must be false");
    assert!(!caps.can_withdraw, "paused: can_withdraw must be false");
    assert!(!caps.can_partial_release, "paused: can_partial_release must be false");
    // Balance and ratio are still reported correctly.
    assert_eq!(caps.collateral_balance, 10_000_i128);
}

// ── Test 6: blocked borrower ─────────────────────────────────────────────────

/// A blocked borrower must get all operation flags set to false.
#[test]
fn caps_blocked_borrower_all_blocked() {
    let (env, contract_id, client, _) = setup();
    let borrower = Address::generate(&env);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let col_token = token_id.address();
    token::StellarAssetClient::new(&env, &col_token).mint(&borrower, &100_000_i128);

    client.open_credit_line(&borrower, &50_000_i128, &500_u32, &30_u32);
    client.deposit_collateral(&borrower, &10_000_i128);

    // Block the borrower.
    client.block_borrower(&borrower);

    let caps = collateral_capabilities(env, contract_id, borrower.clone());

    assert!(!caps.can_deposit, "blocked: can_deposit must be false");
    assert!(!caps.can_withdraw, "blocked: can_withdraw must be false");
    assert!(!caps.can_partial_release, "blocked: can_partial_release must be false");
}

// ── Test 7: min_ratio_bps set and utilized → collateral_required ─────────────

/// When `min_collateral_ratio_bps` is configured and the borrower has drawn,
/// `collateral_required` must be `true`.
#[test]
fn caps_collateral_required_when_ratio_set_and_drawn() {
    let (env, contract_id, client, _) = setup();
    let borrower = Address::generate(&env);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let col_token = token_id.address();
    token::StellarAssetClient::new(&env, &col_token).mint(&borrower, &100_000_i128);

    client.set_min_collateral_ratio_bps(&15_000_u32);
    client.open_credit_line(&borrower, &50_000_i128, &500_u32, &30_u32);
    client.deposit_collateral(&borrower, &20_000_i128);
    client.draw_credit(&borrower, &5_000_i128);

    let caps = collateral_capabilities(env, contract_id, borrower);

    assert!(caps.collateral_required, "ratio set + drawn: collateral_required must be true");
    assert_eq!(caps.min_ratio_bps, 15_000_u32);
}

// ── Test 8: min_ratio_bps set but no draw → collateral_required false ────────

/// When `min_collateral_ratio_bps` is set but the borrower has not drawn,
/// `collateral_required` must be `false` (no debt means no enforcement).
#[test]
fn caps_collateral_required_false_when_no_draw() {
    let (env, contract_id, client, _) = setup();
    let borrower = Address::generate(&env);
    client.set_min_collateral_ratio_bps(&15_000_u32);
    client.open_credit_line(&borrower, &50_000_i128, &500_u32, &30_u32);

    let caps = collateral_capabilities(env, contract_id, borrower);

    assert!(!caps.collateral_required, "no draw: collateral_required must be false even with ratio set");
    assert_eq!(caps.min_ratio_bps, 15_000_u32);
}

// ── Test 9: Closed credit line blocks operations ──────────────────────────────

/// A permanently closed credit line must set all operation flags to false.
#[test]
fn caps_closed_credit_line_all_blocked() {
    let (env, contract_id, client, _) = setup();
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &50_000_i128, &500_u32, &30_u32);
    // Close without drawing (utilization = 0 allows direct close).
    client.close_credit_line(&borrower);

    let caps = collateral_capabilities(env, contract_id, borrower);

    assert!(!caps.can_deposit, "closed line: can_deposit must be false");
    assert!(!caps.can_withdraw, "closed line: can_withdraw must be false");
    assert!(!caps.can_partial_release, "closed line: can_partial_release must be false");
}

// ── Test 10: collateral_balance mirrors get_collateral ───────────────────────

/// The `collateral_balance` field must equal what `get_collateral` returns
/// directly.
#[test]
fn caps_collateral_balance_consistent_with_get_collateral() {
    let (env, contract_id, client, _) = setup();
    let borrower = Address::generate(&env);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let col_token = token_id.address();
    token::StellarAssetClient::new(&env, &col_token).mint(&borrower, &100_000_i128);

    client.open_credit_line(&borrower, &50_000_i128, &500_u32, &30_u32);
    client.deposit_collateral(&borrower, &7_500_i128);

    let direct_balance = client.get_collateral(&borrower);
    let caps = collateral_capabilities(env, contract_id, borrower);

    assert_eq!(
        caps.collateral_balance,
        direct_balance,
        "collateral_balance must match get_collateral"
    );
    assert_eq!(caps.collateral_balance, 7_500_i128);
}
