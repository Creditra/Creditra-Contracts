// SPDX-License-Identifier: MIT

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, Address, Env};

fn setup<'a>(env: &'a Env) -> (Address, Address, Address, Address, CreditClient<'a>) {
    env.mock_all_auths();

    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let reserve = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);
    client.set_liquidity_source(&reserve);

    (contract_id, token_address, borrower, reserve, client)
}

#[test]
fn protocol_summary_empty_state_returns_zeroes() {
    let env = Env::default();
    let (_contract_id, _token_address, _borrower, _reserve, client) = setup(&env);

    let summary = client.get_protocol_summary();

    assert_eq!(summary.count, 0);
    assert_eq!(summary.total_utilized, 0);
    assert_eq!(summary.total_collateral, 0);
    assert_eq!(summary.treasury_balance, 0);
}

#[test]
fn protocol_summary_returns_aggregate_totals() {
    let env = Env::default();
    let (contract_id, token_address, borrower, _reserve, client) = setup(&env);

    let asset = token::StellarAssetClient::new(&env, &token_address);
    asset.mint(&borrower, &5_000);
    asset.mint(&contract_id, &2_000);

    client.open_credit_line(&borrower, &2_000, &1_000_u32, &50_u32);
    client.deposit_collateral(&borrower, &3_000);
    client.draw_credit(&borrower, &1_000);

    client.set_protocol_fee_bps(&1_000_u32);
    env.ledger()
        .with_mut(|ledger| ledger.timestamp = 31_536_000);
    asset.mint(&borrower, &1_100);
    token::Client::new(&env, &token_address)
        .approve(&borrower, &contract_id, &1_100, &u32::MAX);
    client.repay_credit(&borrower, &1_100);

    let summary = client.get_protocol_summary();

    assert_eq!(summary.count, 1);
    assert_eq!(summary.total_utilized, 0);
    assert_eq!(summary.total_collateral, 3_000);
    assert_eq!(summary.treasury_balance, 10);
}
