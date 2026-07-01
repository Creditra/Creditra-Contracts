// SPDX-License-Identifier: MIT

use creditra_credit::types::CreditStatus;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token::StellarAssetClient, Address, Env};

fn setup_restricted_line() -> (Env, Address, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);
    client.set_liquidity_source(&contract_id);

    StellarAssetClient::new(&env, &token_address).mint(&contract_id, &10_000_i128);

    client.open_credit_line(&borrower, &10_000_i128, &300_u32, &50_u32);
    client.draw_credit(&borrower, &5_000_i128);
    client.update_risk_parameters(&borrower, &2_000_i128, &300_u32, &50_u32);

    (env, admin, borrower, contract_id, token_address)
}

#[test]
fn restricted_rejects_new_draws_and_allows_repayment() {
    let (env, _admin, borrower, contract_id, token_address) = setup_restricted_line();
    let client = CreditClient::new(&env, &contract_id);

    let line = client
        .get_credit_line(&borrower)
        .expect("credit line exists");
    assert_eq!(line.status, CreditStatus::Restricted);
    assert_eq!(line.utilized_amount, 5_000);

    let draw_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.draw_credit(&borrower, &1_i128);
    }));
    assert!(draw_result.is_err(), "Restricted should reject new draws");

    let line_after_failed_draw = client
        .get_credit_line(&borrower)
        .expect("credit line exists");
    assert_eq!(line_after_failed_draw.status, CreditStatus::Restricted);
    assert_eq!(line_after_failed_draw.utilized_amount, 5_000);

    StellarAssetClient::new(&env, &token_address).mint(&borrower, &2_000_i128);
    soroban_sdk::token::Client::new(&env, &token_address).approve(
        &borrower,
        &contract_id,
        &2_000_i128,
        &1_000_u32,
    );

    client.repay_credit(&borrower, &2_000_i128);

    let line_after_repay = client
        .get_credit_line(&borrower)
        .expect("credit line exists");
    assert_eq!(line_after_repay.status, CreditStatus::Restricted);
    assert_eq!(line_after_repay.utilized_amount, 3_000);
}
