// SPDX-License-Identifier: MIT
#![cfg(test)]

use creditra_risk::{RiskContract, RiskContractClient};
use soroban_sdk::{
    testutils::{Address as _, MockAuth, MockAuthInvoke},
    Address, Env, IntoVal,
};

#[test]
#[should_panic(expected = "admin not initialized")]
fn test_get_admin_pre_init_panics() {
    let env = Env::default();
    let contract_id = env.register(RiskContract, ());
    let client = RiskContractClient::new(&env, &contract_id);
    
    // Should panic because it's not initialized
    client.get_admin();
}

#[test]
fn test_re_init_overwrites_admin() {
    let env = Env::default();
    env.mock_all_auths();
    
    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);
    
    let contract_id = env.register(RiskContract, ());
    let client = RiskContractClient::new(&env, &contract_id);
    
    // First init
    client.init(&admin1);
    assert_eq!(client.get_admin(), admin1);
    
    // Second init (re-init) - currently allowed because there is no initialization guard,
    // only the `require_auth` on the passed `admin` argument.
    client.init(&admin2);
    assert_eq!(client.get_admin(), admin2);
}
