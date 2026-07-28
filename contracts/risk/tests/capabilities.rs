// SPDX-License-Identifier: MIT

//! Integration test verifying contract initialisation and admin retrieval
//! (replaces the original cross-crate capabilities test which referenced
//! `creditra_credit`, a crate that is not a dependency of `creditra-risk`).

use creditra_risk::{RiskContract, RiskContractClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

#[test]
fn risk_contract_initialises_and_exposes_admin() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(RiskContract, ());
    let client = RiskContractClient::new(&env, &contract_id);
    client.init(&admin);

    let returned = client.get_admin();
    assert_eq!(
        returned, admin,
        "get_admin must return the address passed to init"
    );
}

#[test]
fn cooldown_defaults_to_zero_after_init() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(RiskContract, ());
    let client = RiskContractClient::new(&env, &contract_id);
    client.init(&admin);

    assert_eq!(
        client.get_risk_admin_cooldown(),
        0,
        "cooldown must default to 0 (disabled) after init"
    );
}
