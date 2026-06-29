// SPDX-License-Identifier: MIT

use creditra_credit::types::GraceWaiverMode;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, Address, Env};

fn setup_env() -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();
    CreditClient::new(&env, &contract_id).init(&admin);
    CreditClient::new(&env, &contract_id).set_liquidity_token(&token_address);

    (env, admin, contract_id, token_address)
}

fn setup_borrower_with_draw(
    env: &Env,
    contract_id: &Address,
    token_address: &Address,
    borrower: &Address,
    draw_amount: i128,
) {
    let client = CreditClient::new(env, contract_id);
    client.open_credit_line(borrower, &10_000, &300_u32, &50_u32);
    token::StellarAssetClient::new(env, token_address).mint(contract_id, &(draw_amount * 2));
    client.draw_credit(borrower, &draw_amount);
}

#[test]
fn qualifying_repayment_advances_next_due_timestamp() {
    let (env, admin, contract_id, token_address) = setup_env();
    let _ = admin;
    let borrower = Address::generate(&env);
    let client = CreditClient::new(&env, &contract_id);
    env.ledger().with_mut(|li| li.timestamp = 1_000);
    setup_borrower_with_draw(&env, &contract_id, &token_address, &borrower, 500);

    token::StellarAssetClient::new(&env, &token_address).mint(&borrower, &100);
    token::Client::new(&env, &token_address).approve(&borrower, &contract_id, &100, &u32::MAX);

    client.set_repayment_schedule(&borrower, &100, &86_400, &2_000);
    client.repay_credit(&borrower, &100);

    let schedule = client.get_repayment_schedule(&borrower).unwrap();
    assert_eq!(schedule.next_due_ts, 88_400);
    assert!(!client.is_delinquent(&borrower));
}

#[test]
fn repayment_within_grace_is_not_delinquent() {
    let (env, admin, contract_id, token_address) = setup_env();
    let _ = admin;
    let borrower = Address::generate(&env);
    let client = CreditClient::new(&env, &contract_id);
    env.ledger().with_mut(|li| li.timestamp = 10_000);
    setup_borrower_with_draw(&env, &contract_id, &token_address, &borrower, 500);

    client.set_grace_period_config(&60, &GraceWaiverMode::FullWaiver, &0);
    client.set_repayment_schedule(&borrower, &100, &86_400, &9_970);

    assert!(!client.is_delinquent(&borrower));
}

#[test]
fn delinquency_triggers_after_the_grace_boundary() {
    let (env, admin, contract_id, token_address) = setup_env();
    let _ = admin;
    let borrower = Address::generate(&env);
    let client = CreditClient::new(&env, &contract_id);
    env.ledger().with_mut(|li| li.timestamp = 10_000);
    setup_borrower_with_draw(&env, &contract_id, &token_address, &borrower, 500);

    client.set_grace_period_config(&60, &GraceWaiverMode::FullWaiver, &0);
    client.set_repayment_schedule(&borrower, &100, &86_400, &9_940);

    assert!(!client.is_delinquent(&borrower));

    env.ledger().with_mut(|li| li.timestamp = 10_001);
    assert!(client.is_delinquent(&borrower));
}
