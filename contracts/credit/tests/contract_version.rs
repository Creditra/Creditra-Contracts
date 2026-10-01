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
    // Compared against the constant rather than hard-coded numbers so a
    // deliberate bump (e.g. the breaking change in Issue #1281) cannot leave
    // this assertion silently stale.
    assert_eq!(version.0, CONTRACT_API_VERSION.0);
    assert_eq!(version.1, CONTRACT_API_VERSION.1);
    assert_eq!(version.2, CONTRACT_API_VERSION.2);
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

// ── get_version / get_contract_version are one version, two names ─────────────
//
// `get_version` predates `get_contract_version` and returned a hard-coded
// `(1, 0, 0)` literal, so a `CONTRACT_API_VERSION` bump could silently leave the
// two entrypoints disagreeing. It is now an alias of `CONTRACT_API_VERSION`, and
// these tests are the guard against the literal coming back.

#[test]
fn get_version_is_an_alias_of_get_contract_version() {
    let (env, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    assert_eq!(
        client.get_version(),
        client.get_contract_version(),
        "get_version must report the same version as get_contract_version"
    );
}

#[test]
fn get_version_matches_module_constant() {
    let (env, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    assert_eq!(
        client.get_version(),
        CONTRACT_API_VERSION,
        "get_version must be sourced from CONTRACT_API_VERSION, not a literal"
    );
}

#[test]
fn get_version_reports_the_expected_triple() {
    let (env, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let version = client.get_version();

    assert_eq!((version.0, version.1, version.2), (1, 0, 0));
}
