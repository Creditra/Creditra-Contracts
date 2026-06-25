// SPDX-License-Identifier: MIT

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

fn setup(env: &Env) -> (CreditClient, Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    // Initialize credit line
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &50_u32);
    (client, admin, borrower)
}

#[test]
fn test_rate_floor_overrides_formula() {
    let env = Env::default();
    let (client, _admin, borrower) = setup(&env);

    // Set floor to 400 bps
    client.set_borrower_rate_floor(&borrower, &Some(400_u32));

    // Update risk params with 300 bps rate (below floor)
    client.update_risk_parameters(&borrower, &1_000_i128, &300_u32, &50_u32);

    // Effective rate should be 400 (the floor)
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.interest_rate_bps, 400);
}

#[test]
fn test_rate_floor_does_not_override_higher_rate() {
    let env = Env::default();
    let (client, _admin, borrower) = setup(&env);

    // Set floor to 400 bps
    client.set_borrower_rate_floor(&borrower, &Some(400_u32));

    // Update risk params with 500 bps rate (above floor)
    client.update_risk_parameters(&borrower, &1_000_i128, &500_u32, &50_u32);

    // Effective rate should be 500
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.interest_rate_bps, 500);
}

#[test]
fn test_removing_rate_floor() {
    let env = Env::default();
    let (client, _admin, borrower) = setup(&env);

    // Set floor to 400 bps
    client.set_borrower_rate_floor(&borrower, &Some(400_u32));

    // Remove floor
    client.set_borrower_rate_floor(&borrower, &None);

    // Update risk params with 300 bps rate
    client.update_risk_parameters(&borrower, &1_000_i128, &300_u32, &50_u32);

    // Effective rate should be 300
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.interest_rate_bps, 300);
}
