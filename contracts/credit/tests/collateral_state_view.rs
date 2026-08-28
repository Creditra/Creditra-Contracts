// SPDX-License-Identifier: MIT
#![cfg(test)]

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, Address, Env};

fn setup(env: &Env) -> (CreditClient, Address, Address, Address) {
    env.mock_all_auths_allowing_non_root_auth();
    let admin = Address::generate(env);
    let borrower = Address::generate(env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&token);

    let token_admin = StellarAssetClient::new(env, &token);
    token_admin.mint(&borrower, &100_000_i128);
    token_admin.mint(&token, &100_000_i128);

    soroban_sdk::token::Client::new(env, &token).approve(
        &borrower,
        &contract_id,
        &1_000_000_i128,
        &1_000_000_u32,
    );

    (client, admin, borrower, token)
}

/// No credit line, no deposit: balance = 0, health = u32::MAX.
#[test]
fn get_collateral_state_no_line_no_deposit() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env);

    let state = client.get_collateral_state(&borrower);

    assert_eq!(state.borrower, borrower);
    assert_eq!(state.balance, 0);
    assert_eq!(state.min_ratio_bps, 15_000);
    assert_eq!(state.health_factor_bps, u32::MAX);
}

/// After deposit with no debt, health = u32::MAX.
#[test]
fn get_collateral_state_after_deposit_no_debt() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env);

    client.deposit_collateral(&borrower, &5_000);
    let state = client.get_collateral_state(&borrower);

    assert_eq!(state.balance, 5_000);
    assert_eq!(state.health_factor_bps, u32::MAX);
}

/// With active debt: health_factor_bps = balance * 10_000 / utilized_amount.
#[test]
fn get_collateral_state_with_active_debt() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env);

    client.open_credit_line(&borrower, &10_000, &0, &0);
    // 1_500 satisfies the default 150% (15_000 bps) ratio for a 1_000 draw.
    client.deposit_collateral(&borrower, &1_500);
    client.draw_credit(&borrower, &1_000);

    let state = client.get_collateral_state(&borrower);

    assert_eq!(state.balance, 1_500);
    // health = 1_500 * 10_000 / 1_000 = 15_000
    assert_eq!(state.health_factor_bps, 15_000_u32);
}

/// After full repayment, health returns to u32::MAX.
#[test]
fn get_collateral_state_after_full_repay() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env);

    client.open_credit_line(&borrower, &10_000, &0, &0);
    client.deposit_collateral(&borrower, &1_500);
    client.draw_credit(&borrower, &1_000);
    client.repay_credit(&borrower, &1_000);

    let state = client.get_collateral_state(&borrower);

    assert_eq!(state.balance, 1_500);
    assert_eq!(state.health_factor_bps, u32::MAX);
}

/// collateral_token field matches the configured token address.
#[test]
fn get_collateral_state_token_field() {
    let env = Env::default();
    let (client, _, borrower, token) = setup(&env);

    let state = client.get_collateral_state(&borrower);
    assert_eq!(state.collateral_token, Some(token));
}
