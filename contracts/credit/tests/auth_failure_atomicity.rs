// SPDX-License-Identifier: MIT

//! Adversarial coverage for authorization failure atomicity.
//! 
//! Tests that if a state-changing entrypoint fails its `require_auth` check 
//! (e.g. caller is not authorized), the execution aborts entirely and no 
//! partial state changes (such as accrued interest, nonce increment, or 
//! token transfers) are persisted.

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, Address, Env};

fn setup() -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    
    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);
    (env, admin, contract_id, token_address)
}

#[test]
fn draw_credit_auth_failure_atomicity() {
    let (env, _admin, contract_id, token_address) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &1_000);

    let baseline = client.get_credit_line(&borrower).unwrap();

    // Disable mock auth to simulate an unauthorized call.
    env.mock_auths(&[]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.draw_credit(&borrower, &500);
    }));

    assert!(result.is_err(), "draw_credit must fail when unauthorized");

    // Re-enable mock auth to check state.
    env.mock_all_auths();
    
    let current = client.get_credit_line(&borrower).unwrap();
    assert_eq!(current.utilized_amount, baseline.utilized_amount);
    assert_eq!(current.accrued_interest, baseline.accrued_interest);
    
    let token_client = token::Client::new(&env, &token_address);
    assert_eq!(token_client.balance(&borrower), 0);
}

#[test]
fn repay_credit_auth_failure_atomicity() {
    let (env, _admin, contract_id, token_address) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &1_000);
    client.draw_credit(&borrower, &500);

    token::StellarAssetClient::new(&env, &token_address).mint(&borrower, &500);
    
    let token_client = token::Client::new(&env, &token_address);
    token_client.approve(
        &borrower,
        &contract_id,
        &500,
        &(env.ledger().timestamp().saturating_add(10_000) as u32),
    );

    let baseline = client.get_credit_line(&borrower).unwrap();
    let baseline_bal = token_client.balance(&borrower);

    // Disable mock auth.
    env.mock_auths(&[]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.repay_credit(&borrower, &200);
    }));

    assert!(result.is_err(), "repay_credit must fail when unauthorized");

    // Re-enable mock auth.
    env.mock_all_auths();
    
    let current = client.get_credit_line(&borrower).unwrap();
    assert_eq!(current.utilized_amount, baseline.utilized_amount);
    assert_eq!(token_client.balance(&borrower), baseline_bal);
}
