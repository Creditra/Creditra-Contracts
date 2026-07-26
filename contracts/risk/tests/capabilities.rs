// SPDX-License-Identifier: MIT

//! Integration tests for the risk v7 capabilities view (`contracts/risk/src/views.rs`).

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

#[test]
fn risk_capabilities_view_matches_credit_entrypoint() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &5_000_i128, &400_u32, &60_u32);

    let caps = client.risk_capabilities(&borrower);
    assert!(caps.can_update_risk_parameters);
    assert!(caps.can_change_rate);
    assert!(caps.can_commit_vrf);
}
