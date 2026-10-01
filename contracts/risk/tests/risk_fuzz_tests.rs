// SPDX-License-Identifier: MIT

//! Integration test verifying risk fuzzing invariants and edge cases.

use creditra_risk::{RiskContract, RiskContractClient};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env};

#[test]
fn test_risk_fuzz_sequence() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(RiskContract, ());
    let client = RiskContractClient::new(&env, &contract_id);

    // Initialize
    client.init(&admin);
    assert_eq!(client.get_admin(), admin);
    assert_eq!(client.get_risk_admin_cooldown(), 0);

    // Configure cooldown to 3600 seconds
    client.set_risk_admin_cooldown(&3600);
    assert_eq!(client.get_risk_admin_cooldown(), 3600);

    // Initial action at timestamp 1000
    env.ledger().with_mut(|li| li.timestamp = 1000);
    client.record_risk_admin_action();

    // Rapid second action within cooldown (timestamp 2000 < 1000 + 3600) should fail
    env.ledger().with_mut(|li| li.timestamp = 2000);
    assert!(client.try_record_risk_admin_action().is_err());

    // Action after cooldown elapses (timestamp 4600 >= 1000 + 3600) should succeed
    env.ledger().with_mut(|li| li.timestamp = 4600);
    client.record_risk_admin_action();

    // Pause contract
    client.set_paused(&true);
    assert!(client.try_record_risk_admin_action().is_err());
    assert!(client.try_set_risk_admin_cooldown(&1800).is_err());

    // Unpause contract
    client.set_paused(&false);
    assert_eq!(client.get_risk_admin_cooldown(), 3600);
}
