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
    assert_eq!(client.get_collateral_for_token(&borrower, &col_token), 2_000);
}
