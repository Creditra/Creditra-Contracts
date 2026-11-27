// SPDX-License-Identifier: MIT
#![cfg(test)]

//! Integration tests for multi-collateral support (Issue #599).
//!
//! Covers:
//! - Admin allowlist management (`set_collateral_token_allowlist` / `get_collateral_tokens`)
//! - Deposit and withdraw of an allowlisted token (`deposit_collateral_token` / `withdraw_collateral_token`)
//! - Per-token balance isolation (`get_collateral_for_token`)
//! - Rejection of non-allowlisted tokens
//! - Over-withdrawal reverts with `InsufficientCollateralBalance`
//! - Multiple tokens for the same borrower maintain independent balances
//! - Non-admin cannot mutate the allowlist

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, vec, Address, Env, Vec};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn setup(env: &Env) -> (CreditClient, Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    (client, admin, contract_id)
}

fn mint_token(env: &Env, recipient: &Address, amount: i128) -> Address {
    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token = token_id.address();
    StellarAssetClient::new(env, &token).mint(recipient, &amount);
    // Also mint to token itself so contract can transfer back
    StellarAssetClient::new(env, &token).mint(&token, &amount);
    token
}

// ── Allowlist management ──────────────────────────────────────────────────────

#[test]
fn test_allowlist_starts_empty() {
    let env = Env::default();
    let (client, _, _) = setup(&env);
    assert_eq!(client.get_collateral_tokens(), Vec::<Address>::new(&env));
}

#[test]
fn test_admin_can_set_allowlist() {
    let env = Env::default();
    let (client, _, _) = setup(&env);
    let token_a = Address::generate(&env);
    let token_b = Address::generate(&env);

    client.set_collateral_token_allowlist(&vec![&env, token_a.clone(), token_b.clone()]);
    let list = client.get_collateral_tokens();
    assert_eq!(list.len(), 2);
    assert!(list.contains(token_a));
    assert!(list.contains(token_b));
}

#[test]
fn test_admin_can_clear_allowlist() {
    let env = Env::default();
    let (client, _, _) = setup(&env);
    let token_a = Address::generate(&env);
    client.set_collateral_token_allowlist(&vec![&env, token_a]);
    client.set_collateral_token_allowlist(&Vec::<Address>::new(&env));
    assert_eq!(client.get_collateral_tokens().len(), 0);
}

// ── Deposit and query ─────────────────────────────────────────────────────────

#[test]
fn test_deposit_collateral_token_increments_balance() {
    let env = Env::default();
    let (client, _, _) = setup(&env);
    let borrower = Address::generate(&env);
    let token = mint_token(&env, &borrower, 10_000);

    client.set_collateral_token_allowlist(&vec![&env, token.clone()]);
    assert_eq!(client.get_collateral_for_token(&borrower, &token), 0);

    client.deposit_collateral_token(&borrower, &token, &3_000);
    assert_eq!(client.get_collateral_for_token(&borrower, &token), 3_000);

    client.deposit_collateral_token(&borrower, &token, &2_000);
    assert_eq!(client.get_collateral_for_token(&borrower, &token), 5_000);
}

#[test]
fn test_deposit_two_tokens_independent_balances() {
    let env = Env::default();
    let (client, _, _) = setup(&env);
    let borrower = Address::generate(&env);
    let token_a = mint_token(&env, &borrower, 10_000);
    let token_b = mint_token(&env, &borrower, 10_000);

    client.set_collateral_token_allowlist(&vec![&env, token_a.clone(), token_b.clone()]);

    client.deposit_collateral_token(&borrower, &token_a, &1_000);
    client.deposit_collateral_token(&borrower, &token_b, &4_000);

    assert_eq!(client.get_collateral_for_token(&borrower, &token_a), 1_000);
    assert_eq!(client.get_collateral_for_token(&borrower, &token_b), 4_000);
}

// ── Withdraw ──────────────────────────────────────────────────────────────────

#[test]
fn test_withdraw_collateral_token_decrements_balance() {
    let env = Env::default();
    let (client, _, _) = setup(&env);
    let borrower = Address::generate(&env);
    let token = mint_token(&env, &borrower, 10_000);

    client.set_collateral_token_allowlist(&vec![&env, token.clone()]);
    client.deposit_collateral_token(&borrower, &token, &5_000);
    client.withdraw_collateral_token(&borrower, &token, &2_000);
    assert_eq!(client.get_collateral_for_token(&borrower, &token), 3_000);
}

#[test]
fn test_full_withdrawal_leaves_zero() {
    let env = Env::default();
    let (client, _, _) = setup(&env);
    let borrower = Address::generate(&env);
    let token = mint_token(&env, &borrower, 10_000);

    client.set_collateral_token_allowlist(&vec![&env, token.clone()]);
    client.deposit_collateral_token(&borrower, &token, &5_000);
    client.withdraw_collateral_token(&borrower, &token, &5_000);
    assert_eq!(client.get_collateral_for_token(&borrower, &token), 0);
}

// ── Error cases ───────────────────────────────────────────────────────────────

#[test]
#[should_panic(expected = "Error(Contract, #22)")] // MissingLiquidityToken – token not allowlisted
fn test_deposit_non_allowlisted_token_fails() {
    let env = Env::default();
    let (client, _, _) = setup(&env);
    let borrower = Address::generate(&env);
    let token = mint_token(&env, &borrower, 10_000);
    // allowlist is empty → deposit should panic
    client.deposit_collateral_token(&borrower, &token, &1_000);
}

#[test]
#[should_panic(expected = "Error(Contract, #22)")] // MissingLiquidityToken – token not allowlisted
fn test_withdraw_non_allowlisted_token_fails() {
    let env = Env::default();
    let (client, _, _) = setup(&env);
    let borrower = Address::generate(&env);
    let token = mint_token(&env, &borrower, 10_000);
    client.withdraw_collateral_token(&borrower, &token, &1_000);
}

#[test]
#[should_panic(expected = "Error(Contract, #39)")] // InsufficientCollateralBalance
fn test_over_withdrawal_fails() {
    let env = Env::default();
    let (client, _, _) = setup(&env);
    let borrower = Address::generate(&env);
    let token = mint_token(&env, &borrower, 10_000);

    client.set_collateral_token_allowlist(&vec![&env, token.clone()]);
    client.deposit_collateral_token(&borrower, &token, &500);
    client.withdraw_collateral_token(&borrower, &token, &1_000); // 1000 > 500
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")] // InvalidAmount
fn test_deposit_zero_amount_fails() {
    let env = Env::default();
    let (client, _, _) = setup(&env);
    let borrower = Address::generate(&env);
    let token = mint_token(&env, &borrower, 10_000);

    client.set_collateral_token_allowlist(&vec![&env, token.clone()]);
    client.deposit_collateral_token(&borrower, &token, &0);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")] // InvalidAmount
fn test_withdraw_zero_amount_fails() {
    let env = Env::default();
    let (client, _, _) = setup(&env);
    let borrower = Address::generate(&env);
    let token = mint_token(&env, &borrower, 10_000);

    client.set_collateral_token_allowlist(&vec![&env, token.clone()]);
    client.deposit_collateral_token(&borrower, &token, &1_000);
    client.withdraw_collateral_token(&borrower, &token, &0);
}

// ── Isolation: multi-token does not affect single-token balance ───────────────

#[test]
fn test_multi_token_deposit_does_not_affect_legacy_collateral_balance() {
    let env = Env::default();
    let (client, _, contract_id) = setup(&env);
    let borrower = Address::generate(&env);

    // Set up the legacy liquidity token (used by deposit_collateral)
    let liquidity_token = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let liq_token_addr = liquidity_token.address();
    client.set_liquidity_token(&liq_token_addr);
    client.set_liquidity_source(&liq_token_addr);
    StellarAssetClient::new(&env, &liq_token_addr).mint(&borrower, &10_000);
    StellarAssetClient::new(&env, &liq_token_addr).mint(&liq_token_addr, &10_000);
    StellarAssetClient::new(&env, &liq_token_addr).mint(&contract_id, &10_000);

    // Set up a separate collateral token on the allowlist
    let col_token = mint_token(&env, &borrower, 10_000);
    // also fund contract_id for transfers back
    StellarAssetClient::new(&env, &col_token).mint(&contract_id, &10_000);
    client.set_collateral_token_allowlist(&vec![&env, col_token.clone()]);

    // Deposit via legacy single-token path
    client.deposit_collateral(&borrower, &3_000);
    // Deposit via multi-token path
    client.deposit_collateral_token(&borrower, &col_token, &2_000);

    // Each balance is independent
    assert_eq!(client.get_collateral(&borrower), 3_000);
    assert_eq!(
        client.get_collateral_for_token(&borrower, &col_token),
        2_000
    );
}

// ── Health-aware multi-token withdrawals (Issue #1222) ────────────────────────

/// Contract with a liquidity token plus one allowlisted collateral token,
/// funded for the borrower. `CreditClient` is returned by value; `contract_id`
/// doubles as the liquidity source so draws can settle.
fn setup_drawable(env: &Env) -> (CreditClient, Address, Address, Address) {
    let (client, _admin, contract_id) = setup(env);
    let borrower = Address::generate(env);

    let liquidity = env.register_stellar_asset_contract_v2(Address::generate(env));
    let liquidity_addr = liquidity.address();
    client.set_liquidity_token(&liquidity_addr);
    client.set_liquidity_source(&contract_id);
    StellarAssetClient::new(env, &liquidity_addr).mint(&contract_id, &1_000_000);
    StellarAssetClient::new(env, &liquidity_addr).mint(&borrower, &1_000_000);

    let col = mint_token(env, &borrower, 100_000);
    StellarAssetClient::new(env, &col).mint(&contract_id, &100_000);
    client.set_collateral_token_allowlist(&vec![&env, col.clone()]);

    (client, contract_id, borrower, col)
}

/// A balance deposited through `deposit_collateral_token` must back a draw:
/// a 1_500 token balance covers exactly a 1_000 draw at the 150 % floor.
#[test]
fn test_multi_token_deposit_backs_draw_ratio() {
    let env = Env::default();
    let (client, _contract_id, borrower, col) = setup_drawable(&env);

    client.open_credit_line(&borrower, &10_000, &0, &0);
    client.deposit_collateral_token(&borrower, &col, &1_500);

    client.draw_credit(&borrower, &1_000);
    assert_eq!(client.get_total_utilized(), 1_000);
}

/// The same deposit below the floor must NOT back the draw.
#[test]
#[should_panic(expected = "Error(Contract, #35)")] // CollateralRatioBelowMinimum
fn test_multi_token_deposit_below_floor_rejects_draw() {
    let env = Env::default();
    let (client, _contract_id, borrower, col) = setup_drawable(&env);

    client.open_credit_line(&borrower, &10_000, &0, &0);
    client.deposit_collateral_token(&borrower, &col, &1_000);

    // Required for a 1_000 draw at 150 % is 1_500.
    client.draw_credit(&borrower, &1_000);
}

/// Withdrawing an allowlisted token that drops the position below the floor
/// must revert.
#[test]
#[should_panic(expected = "Error(Contract, #35)")] // CollateralRatioBelowMinimum
fn test_withdraw_collateral_token_breaching_ratio_reverts() {
    let env = Env::default();
    let (client, _contract_id, borrower, col) = setup_drawable(&env);

    client.open_credit_line(&borrower, &10_000, &0, &0);
    client.deposit_collateral_token(&borrower, &col, &2_000);
    client.draw_credit(&borrower, &1_000); // requires 1_500 of collateral

    // Leaving 1_000 behind is below the 1_500 requirement.
    client.withdraw_collateral_token(&borrower, &col, &1_000);
}

/// A zero-debt borrower keeps full freedom to withdraw any token.
#[test]
fn test_zero_debt_borrower_withdraws_token_freely() {
    let env = Env::default();
    let (client, _contract_id, borrower, col) = setup_drawable(&env);

    client.open_credit_line(&borrower, &10_000, &0, &0);
    client.deposit_collateral_token(&borrower, &col, &2_000);

    client.withdraw_collateral_token(&borrower, &col, &2_000);
    assert_eq!(client.get_collateral_for_token(&borrower, &col), 0);
}

/// `get_health_factor` must see multi-token balances, not just the legacy one.
#[test]
fn test_health_factor_reflects_multi_token_balances() {
    let env = Env::default();
    let (client, _contract_id, borrower, col) = setup_drawable(&env);

    client.open_credit_line(&borrower, &10_000, &0, &0);
    client.deposit_collateral_token(&borrower, &col, &3_000);
    client.draw_credit(&borrower, &1_000);

    // 3_000 * 100_000_000 / (1_000 * 15_000) = 20_000
    assert_eq!(client.get_health_factor(&borrower), 20_000);
}

#[test]
fn test_mixed_legacy_and_allowlisted_collateral_conserves_total() {
    let env = Env::default();
    let (client, _, contract_id) = setup(&env);
    let borrower_a = Address::generate(&env);
    let borrower_b = Address::generate(&env);

    let legacy = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let legacy_addr = legacy.address();
    client.set_liquidity_token(&legacy_addr);
    client.set_liquidity_source(&legacy_addr);
    StellarAssetClient::new(&env, &legacy_addr).mint(&borrower_a, &10_000);
    StellarAssetClient::new(&env, &legacy_addr).mint(&borrower_b, &10_000);
    StellarAssetClient::new(&env, &legacy_addr).mint(&contract_id, &10_000);

    let token = mint_token(&env, &borrower_a, 10_000);
    StellarAssetClient::new(&env, &token).mint(&borrower_b, &10_000);
    StellarAssetClient::new(&env, &token).mint(&contract_id, &10_000);
    client.set_collateral_token_allowlist(&vec![&env, token.clone()]);

    client.deposit_collateral(&borrower_a, &3_000);
    client.deposit_collateral(&borrower_b, &2_000);
    client.deposit_collateral_token(&borrower_a, &token, &4_000);
    client.deposit_collateral_token(&borrower_b, &token, &1_000);

    assert_eq!(client.get_protocol_summary_view().total_collateral, 10_000);
    client.withdraw_collateral(&borrower_a, &1_000);
    client.withdraw_collateral_token(&borrower_a, &token, &2_000);
    client.withdraw_collateral(&borrower_b, &2_000);
    client.withdraw_collateral_token(&borrower_b, &token, &1_000);
    assert_eq!(client.get_protocol_summary_view().total_collateral, 4_000);
}
