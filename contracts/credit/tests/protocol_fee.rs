// SPDX-License-Identifier: MIT

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, Address, Env};

fn setup() -> (Env, Address, Address, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let reserve = Address::generate(&env);
    let treasury = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();

    client.set_liquidity_token(&token_address);
    client.set_liquidity_source(&reserve);
    client.set_treasury(&admin, &treasury);

    (env, contract_id, token_address, borrower, reserve, treasury)
}

fn prepare_repay(
    env: &Env,
    contract_id: &Address,
    token_address: &Address,
    borrower: &Address,
    draw_amount: i128,
    repay_amount: i128,
    interest_rate_bps: u32,
    fee_bps: u32,
) -> CreditClient {
    let client = CreditClient::new(env, contract_id);
    client.open_credit_line(borrower, &draw_amount, &interest_rate_bps, &50_u32);

    let asset = token::StellarAssetClient::new(env, token_address);
    asset.mint(contract_id, &draw_amount);
    client.draw_credit(borrower, &draw_amount);

    client.set_protocol_fee_bps(&fee_bps);

    env.ledger()
        .with_mut(|ledger| ledger.timestamp = 31_536_000);

    asset.mint(borrower, &repay_amount);
    token::Client::new(env, token_address).approve(borrower, contract_id, &repay_amount, &u32::MAX);

    client
}

#[test]
fn protocol_fee_zero_fee_keeps_treasury_balance_at_zero() {
    let (env, contract_id, token_address, borrower, reserve, treasury) = setup();
    let client = prepare_repay(
        &env,
        &contract_id,
        &token_address,
        &borrower,
        1_000,
        1_100,
        1_000,
        0,
    );

    assert_eq!(client.get_protocol_fee_bps(), Some(0));
    assert_eq!(client.get_treasury(), Some(treasury.clone()));

    let token_client = token::Client::new(&env, &token_address);
    let contract_balance_before = token_client.balance(&contract_id);
    let reserve_balance_before = token_client.balance(&reserve);
    let treasury_balance_before = token_client.balance(&treasury);

    client.repay_credit(&borrower, &1_100);

    assert_eq!(token_client.balance(&contract_id), contract_balance_before);
    assert_eq!(
        token_client.balance(&reserve),
        reserve_balance_before + 1_100
    );
    assert_eq!(token_client.balance(&treasury), treasury_balance_before);
}

#[test]
fn protocol_fee_max_fee_accrues_expected_fee_amount() {
    let (env, contract_id, token_address, borrower, reserve, treasury) = setup();
    let client = prepare_repay(
        &env,
        &contract_id,
        &token_address,
        &borrower,
        1_000,
        1_100,
        1_000,
        1_000,
    );

    let token_client = token::Client::new(&env, &token_address);
    let contract_balance_before = token_client.balance(&contract_id);
    let reserve_balance_before = token_client.balance(&reserve);

    client.repay_credit(&borrower, &1_100);

    assert_eq!(
        token_client.balance(&contract_id),
        contract_balance_before + 10
    );
    assert_eq!(
        token_client.balance(&reserve),
        reserve_balance_before + 1_090
    );
    assert_eq!(token_client.balance(&treasury), 0);
}

#[test]
fn protocol_fee_rounding_edge_floors_small_fee_to_zero() {
    let (env, contract_id, token_address, borrower, reserve, treasury) = setup();
    let client = prepare_repay(
        &env,
        &contract_id,
        &token_address,
        &borrower,
        10_000,
        10_001,
        1,
        5_000,
    );

    let token_client = token::Client::new(&env, &token_address);
    let contract_balance_before = token_client.balance(&contract_id);
    let reserve_balance_before = token_client.balance(&reserve);

    client.repay_credit(&borrower, &10_001);

    assert_eq!(token_client.balance(&contract_id), contract_balance_before);
    assert_eq!(
        token_client.balance(&reserve),
        reserve_balance_before + 10_001
    );
    assert_eq!(token_client.balance(&treasury), 0);
}
