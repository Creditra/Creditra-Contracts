// SPDX-License-Identifier: MIT
#![cfg(test)]

use creditra_risk::events::{
    RiskAdminActionRecordedEvent, RiskAdminCooldownConfiguredEvent, RiskInitializedEvent,
    RiskPausedEvent,
};
use creditra_risk::{RiskContract, RiskContractClient};
use soroban_sdk::{
    testutils::{Address as _, Events},
    Address, Env, IntoVal, Symbol,
};

fn setup() -> (Env, Address, Address, RiskContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(RiskContract, ());
    let client = RiskContractClient::new(&env, &contract_id);
    client.init(&admin);
    (env, admin, contract_id, client)
}

#[test]
fn test_events() {
    let (env, admin, _contract_id, client) = setup();

    // Check initialization event
    let event = env.events().all().last().unwrap();
    let topics = event.1;
    let data = event.2;
    assert_eq!(topics.get(0).unwrap(), symbol_short!("risk").into_val(&env));
    assert_eq!(topics.get(1).unwrap(), symbol_short!("init").into_val(&env));
    let ev: RiskInitializedEvent = data.try_into_val(&env).unwrap();
    assert_eq!(ev.admin, admin);

    // Set cooldown
    client.set_risk_admin_cooldown(&3600);
    let event = env.events().all().last().unwrap();
    let topics = event.1;
    let data = event.2;
    assert_eq!(topics.get(0).unwrap(), symbol_short!("risk").into_val(&env));
    assert_eq!(topics.get(1).unwrap(), symbol_short!("rad_cool").into_val(&env));
    let ev: RiskAdminCooldownConfiguredEvent = data.try_into_val(&env).unwrap();
    assert_eq!(ev.cooldown_seconds, 3600);

    // Set paused
    client.set_paused(&true);
    let event = env.events().all().last().unwrap();
    let topics = event.1;
    let data = event.2;
    assert_eq!(topics.get(0).unwrap(), symbol_short!("risk").into_val(&env));
    assert_eq!(topics.get(1).unwrap(), symbol_short!("paused").into_val(&env));
    let ev: RiskPausedEvent = data.try_into_val(&env).unwrap();
    assert_eq!(ev.paused, true);

    // Record action
    client.record_risk_admin_action();
    let event = env.events().all().last().unwrap();
    let topics = event.1;
    let data = event.2;
    assert_eq!(topics.get(0).unwrap(), symbol_short!("risk").into_val(&env));
    assert_eq!(topics.get(1).unwrap(), symbol_short!("rad_act").into_val(&env));
    let ev: RiskAdminActionRecordedEvent = data.try_into_val(&env).unwrap();
    assert!(ev.timestamp > 0);
}
