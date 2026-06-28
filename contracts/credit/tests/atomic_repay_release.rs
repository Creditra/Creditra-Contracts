#![cfg(test)]

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token, Address, Env,
};

fn setup<'a>(
    env: &'a Env,
    credit_limit: i128,
    draw_amount: i128,
    collateral: i128,
) -> (CreditClient<'a>, Address, Address, Address) {
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1000);

    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&token);

    let asset = token::StellarAssetClient::new(env, &token);
    asset.mint(&borrower, &(collateral + draw_amount + 10_000));
    asset.mint(&token, &100_000);

    client.open_credit_line(&borrower, &credit_limit, &500, &10);

    if collateral > 0 {
        client.deposit_collateral(&borrower, &collateral);
    }

    if draw_amount > 0 {
        client.draw_credit(&borrower, &draw_amount);
    }

    // Approve contract to pull repayment tokens from borrower.
    token::Client::new(env, &token).approve(
        &borrower,
        &contract_id,
        &(draw_amount + 10_000),
        &6_000_000_u32,
    );

    (client, admin, borrower, token)
}

#[test]
fn test_proportional_release_basic() {
    let env = Env::default();
    let (client, _, borrower, token) = setup(&env, 5_000, 500, 1_000);

    assert_eq!(client.get_collateral(&borrower), 1_000);

    // Repay 250 out of 500 debt (50%) → release 50% of 1000 collateral = 500
    client.repay_and_release_collateral(&borrower, &250);

    let credit = client.get_credit_line(&borrower).unwrap();
    assert_eq!(credit.utilized_amount, 250);
    assert_eq!(client.get_collateral(&borrower), 500);

    // Verify tokens returned to borrower.
    let balance = token::Client::new(&env, &token).balance(&borrower);
    assert!(balance > 0);
}

#[test]
fn test_full_repay_releases_all_collateral() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env, 5_000, 500, 1_000);

    assert_eq!(client.get_collateral(&borrower), 1_000);

    // Repay full 500 → release all 1000 collateral
    client.repay_and_release_collateral(&borrower, &500);

    let credit = client.get_credit_line(&borrower).unwrap();
    assert_eq!(credit.utilized_amount, 0);
    assert_eq!(client.get_collateral(&borrower), 0);
}

#[test]
fn test_zero_collateral_no_release() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env, 5_000, 500, 0);

    assert_eq!(client.get_collateral(&borrower), 0);

    // Repay with no collateral → just repay, no panic
    client.repay_and_release_collateral(&borrower, &250);

    let credit = client.get_credit_line(&borrower).unwrap();
    assert_eq!(credit.utilized_amount, 250);
    assert_eq!(client.get_collateral(&borrower), 0);
}

#[test]
fn test_ratio_preserved_after_partial_release() {
    let env = Env::default();
    // credit_limit=10_000, draw=1_000, collateral=3_000 (ratio = 300%)
    let (client, _, borrower, _) = setup(&env, 10_000, 1_000, 3_000);

    assert_eq!(client.get_collateral(&borrower), 3_000);

    // Repay 500 (50%) → release 50% of 3_000 = 1_500
    client.repay_and_release_collateral(&borrower, &500);

    let credit = client.get_credit_line(&borrower).unwrap();
    assert_eq!(credit.utilized_amount, 500);
    assert_eq!(client.get_collateral(&borrower), 1_500);

    // Ratio: 1500 / 500 = 300% (preserved)
}

#[test]
fn test_1_wei_rounding_floor() {
    let env = Env::default();
    // collateral=100, debt=3, repay 1
    // released = floor(100 * 1 / 3) = 33
    let (client, _, borrower, _) = setup(&env, 10_000, 3, 100);

    assert_eq!(client.get_collateral(&borrower), 100);

    client.repay_and_release_collateral(&borrower, &1);

    let credit = client.get_credit_line(&borrower).unwrap();
    assert_eq!(credit.utilized_amount, 2);
    assert_eq!(client.get_collateral(&borrower), 67); // 100 - 33
}

#[test]
fn test_overpayment_releases_all_collateral() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env, 5_000, 100, 500);

    assert_eq!(client.get_collateral(&borrower), 500);

    // Repay 1000 on a 100 debt → capped at 100, release all 500 collateral
    client.repay_and_release_collateral(&borrower, &1_000);

    let credit = client.get_credit_line(&borrower).unwrap();
    assert_eq!(credit.utilized_amount, 0);
    assert_eq!(client.get_collateral(&borrower), 0);
}

#[test]
fn test_zero_repay_nothing_changes() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env, 5_000, 0, 500);

    assert_eq!(client.get_collateral(&borrower), 500);

    // Zero debt, repay 0 → nothing happens
    client.repay_and_release_collateral(&borrower, &0);

    let credit = client.get_credit_line(&borrower).unwrap();
    assert_eq!(credit.utilized_amount, 0);
    assert_eq!(client.get_collateral(&borrower), 500);
}
