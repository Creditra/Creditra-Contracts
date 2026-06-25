// SPDX-License-Identifier: MIT

//! Integration tests that read settlement prices from a deployed mock oracle.

use super::{deploy_mock_oracle, setup_credit};
use creditra_credit::types::CreditStatus;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, Address, Env, Symbol};

fn open_and_default(
    client: &creditra_credit::CreditClient<'_>,
    env: &Env,
    contract_id: &Address,
    utilized: i128,
) -> Address {
    let borrower = Address::generate(env);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token_addr = token_id.address();
    client.set_liquidity_token(&token_addr);
    token::StellarAssetClient::new(env, &token_addr).mint(contract_id, &1_000_000_i128);
    token::StellarAssetClient::new(env, &token_addr).mint(&borrower, &1_000_000_i128);
    token::Client::new(env, &token_addr).approve(
        &borrower,
        contract_id,
        &1_000_000_i128,
        &1_000_000_u32,
    );

    client.open_credit_line(&borrower, &10_000_i128, &300_u32, &60_u32);
    if utilized > 0 {
        client.draw_credit(&borrower, &utilized);
    }
    client.default_credit_line(&borrower);
    borrower
}

#[test]
fn settle_reads_price_from_deployed_oracle() {
    let env = Env::default();
    let (client, contract_id, admin) = setup_credit(&env);
    client.set_oracle_config(&500_u32, &3600_u64);

    let oracle = deploy_mock_oracle(&env, &admin, 1_000_i128);
    let borrower = open_and_default(&client, &env, &contract_id, 500);

    let price = oracle.get_price();
    assert_eq!(price, 1_000_i128);

    client.settle_default_liquidation(
        &borrower,
        &500_i128,
        &Symbol::new(&env, "mock_s1"),
        &Some(price),
    );

    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Closed
    );
}

#[test]
fn settle_uses_oracle_price_after_admin_update() {
    let env = Env::default();
    let (client, contract_id, admin) = setup_credit(&env);
    client.set_oracle_config(&500_u32, &3600_u64);

    let oracle = deploy_mock_oracle(&env, &admin, 1_000_i128);
    let b1 = open_and_default(&client, &env, &contract_id, 200);
    client.settle_default_liquidation(
        &b1,
        &200_i128,
        &Symbol::new(&env, "mock_s1"),
        &Some(oracle.get_price()),
    );

    // Admin updates the mock feed; next settlement reads the new price.
    oracle.set_price(&1_040_i128);
    let b2 = open_and_default(&client, &env, &contract_id, 200);
    client.settle_default_liquidation(
        &b2,
        &200_i128,
        &Symbol::new(&env, "mock_s2"),
        &Some(oracle.get_price()),
    );

    assert_eq!(
        client.get_credit_line(&b2).unwrap().status,
        CreditStatus::Closed
    );
}

#[test]
#[should_panic]
fn settle_rejects_zero_price_from_oracle() {
    let env = Env::default();
    let (client, contract_id, admin) = setup_credit(&env);
    client.set_oracle_config(&500_u32, &3600_u64);

    let oracle = deploy_mock_oracle(&env, &admin, 0_i128);
    let borrower = open_and_default(&client, &env, &contract_id, 200);

    client.settle_default_liquidation(
        &borrower,
        &200_i128,
        &Symbol::new(&env, "mock_zero"),
        &Some(oracle.get_price()),
    );
}

#[test]
#[should_panic]
fn non_admin_cannot_set_oracle_price() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let oracle_id = env.register(simple_price_oracle::SimplePriceOracle, ());
    let oracle = simple_price_oracle::SimplePriceOracleClient::new(&env, &oracle_id);
    oracle.init(&admin);

    env.set_auths(&[]);
    let stranger = Address::generate(&env);
    oracle.set_price(&500_i128);
    let _ = stranger;
}
