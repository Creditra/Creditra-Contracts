#![cfg(test)]

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, Address, Env};

fn setup(env: &Env) -> (CreditClient, Address, Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let borrower = Address::generate(env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    client.set_min_collateral_ratio_bps(&15000);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&contract_id);

    let token_admin = StellarAssetClient::new(env, &token);
    token_admin.mint(&borrower, &100_000_i128); // borrower funds
    token_admin.mint(&contract_id, &100_000_i128); // reserve funds

    (client, admin, borrower, token)
}

#[test]
fn test_deposit_and_withdraw_collateral() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env);

    client.deposit_collateral(&borrower, &5000);
    assert_eq!(client.get_collateral(&borrower), 5000);

    // Borrower doesn't have an active credit line, can withdraw all
    client.withdraw_collateral(&borrower, &5000);
    assert_eq!(client.get_collateral(&borrower), 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #35)")] // CollateralRatioBelowMinimum
fn test_withdraw_breaches_min_ratio() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env);

    client.open_credit_line(&borrower, &10000, &0, &0);
    client.deposit_collateral(&borrower, &2000); // Deposited 2000

    client.draw_credit(&borrower, &1000); // Drew 1000. Required collateral = 1000 * 1.5 = 1500

    client.withdraw_collateral(&borrower, &1000); // Attempt to withdraw 1000, leaving 1000. 1000 < 1500 => PANIC
}

#[test]
#[should_panic(expected = "Error(Contract, #35)")] // CollateralRatioBelowMinimum
fn test_draw_credit_breaches_min_ratio() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env);

    client.open_credit_line(&borrower, &10000, &0, &0);
    client.deposit_collateral(&borrower, &1000); // Deposited 1000

    // Attempt to draw 1000. Required collateral = 1000 * 1.5 = 1500. Have 1000. 1000 < 1500 => PANIC
    client.draw_credit(&borrower, &1000);
}

#[test]
fn test_draw_credit_succeeds_with_sufficient_collateral() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env);

    client.open_credit_line(&borrower, &10000, &0, &0);
    client.deposit_collateral(&borrower, &1500); // Deposited 1500

    // Attempt to draw 1000. Required collateral = 1500. Have 1500. OK
    client.draw_credit(&borrower, &1000);
    assert_eq!(client.get_collateral(&borrower), 1500);
}

#[test]
fn test_withdraw_with_open_credit_line_zero_utilized() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env);

    client.open_credit_line(&borrower, &10000, &0, &0);
    client.deposit_collateral(&borrower, &5000);
    assert_eq!(client.get_collateral(&borrower), 5000);

    // Credit line exists but utilized_amount = 0 → no ratio check → can withdraw all.
    client.withdraw_collateral(&borrower, &5000);
    assert_eq!(client.get_collateral(&borrower), 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #22)")] // MissingLiquidityToken
fn test_deposit_without_collateral_token_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    // No liquidity token set → deposit should fail with MissingLiquidityToken.
    client.deposit_collateral(&borrower, &1000);
}

#[test]
#[should_panic(expected = "Error(Contract, #22)")] // MissingLiquidityToken
fn test_withdraw_without_collateral_token_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    // Set collateral balance directly so amount <= cur_balance check passes before MissingLiquidityToken check
    env.as_contract(&contract_id, || {
        creditra_credit::storage::set_collateral_balance(&env, &borrower, 1000);
    });
    // No liquidity token set → withdraw should fail with MissingLiquidityToken.
    client.withdraw_collateral(&borrower, &1000);
}

#[test]
#[should_panic(expected = "Error(Contract, #39)")] // InsufficientCollateralBalance
fn test_withdraw_collateral_insufficient_balance() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env);

    client.deposit_collateral(&borrower, &1000);
    assert_eq!(client.get_collateral(&borrower), 1000);

    // Attempt to withdraw 1001, which is more than the deposited 1000.
    // Should panic with InsufficientCollateralBalance (39).
    client.withdraw_collateral(&borrower, &1001);
}

#[test]
#[should_panic(expected = "Error(Contract, #39)")]
fn test_withdraw_zero_collateral_balance_reverts_with_error_39() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env);

    // Borrower has 0 collateral balance; attempting to withdraw 100 fails with InsufficientCollateralBalance (#39)
    client.withdraw_collateral(&borrower, &100);
}

#[test]
fn test_withdraw_exact_collateral_balance_succeeds() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env);

    client.deposit_collateral(&borrower, &1000);
    assert_eq!(client.get_collateral(&borrower), 1000);

    // Exact balance withdrawal leaves 0 balance
    client.withdraw_collateral(&borrower, &1000);
    assert_eq!(client.get_collateral(&borrower), 0);
}
