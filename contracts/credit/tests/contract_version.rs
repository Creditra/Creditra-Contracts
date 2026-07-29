// SPDX-License-Identifier: MIT

use creditra_credit::{Credit, CreditClient, CONTRACT_API_VERSION};
use soroban_sdk::{testutils::Address as _, Address, Env};

fn setup() -> (Env, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    (env, contract_id)
}

#[test]
fn get_contract_version_returns_expected_value() {
    let (env, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let version = client.get_contract_version();
    assert_eq!(version.0, 1);
    assert_eq!(version.1, 0);
    assert_eq!(version.2, 0);
}

#[test]
fn get_contract_version_format_is_stable() {
    let (env, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let version = client.get_contract_version();
    assert!(version.0 >= 1, "major version must be at least 1");
}

#[test]
fn get_contract_version_matches_module_constant() {
    let (env, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let version = client.get_contract_version();
    assert_eq!(
        version.0, CONTRACT_API_VERSION.0,
        "major must match CONTRACT_API_VERSION"
    );
    assert_eq!(
        version.1, CONTRACT_API_VERSION.1,
        "minor must match CONTRACT_API_VERSION"
    );
    assert_eq!(
        version.2, CONTRACT_API_VERSION.2,
        "patch must match CONTRACT_API_VERSION"
    );
}
