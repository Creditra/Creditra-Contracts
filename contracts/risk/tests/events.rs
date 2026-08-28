// SPDX-License-Identifier: MIT
#![cfg(test)]

use creditra_risk::{
    RiskAdminActionRecordedEvent, RiskAdminCooldownConfiguredEvent, RiskContract,
    RiskContractClient, RiskInitializedEvent, RiskPausedEvent,
};
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events},
    Address, Env, Symbol, TryFromVal, TryIntoVal,
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

/// Convert a `Val` back to a `Symbol` for topic comparison.
fn val_to_symbol(env: &Env, val: soroban_sdk::Val) -> Symbol {
    Symbol::try_from_val(env, &val).expect("topic must be a symbol")
}

#[test]
fn test_events() {
    let (env, admin, _contract_id, client) = setup();

    // Check initialization event (emitted by init)
    let events = env.events().all();
    let event = events.last().unwrap();
    // event is a tuple (contract_id: Address, topics: Vec<Val>, data: Val)
    let topics = &event.1;
    let data = event.2.clone();
    assert_eq!(
        val_to_symbol(&env, topics.get(0).unwrap()),
        symbol_short!("risk")
    );
    assert_eq!(
        val_to_symbol(&env, topics.get(1).unwrap()),
        symbol_short!("init")
    );
    let ev: RiskInitializedEvent = data.try_into_val(&env).unwrap();
    assert_eq!(ev.admin, admin);

    // Set cooldown
    client.set_risk_admin_cooldown(&3600);
    let events = env.events().all();
    let event = events.last().unwrap();
    let topics = &event.1;
    let data = event.2.clone();
    assert_eq!(
        val_to_symbol(&env, topics.get(0).unwrap()),
        symbol_short!("risk")
    );
    assert_eq!(
        val_to_symbol(&env, topics.get(1).unwrap()),
        symbol_short!("rad_cool")
    );
    let ev: RiskAdminCooldownConfiguredEvent = data.try_into_val(&env).unwrap();
    assert_eq!(ev.cooldown_seconds, 3600);

    // Set paused
    client.set_paused(&true);
    let events = env.events().all();
    let event = events.last().unwrap();
    let topics = &event.1;
    let data = event.2.clone();
    assert_eq!(
        val_to_symbol(&env, topics.get(0).unwrap()),
        symbol_short!("risk")
    );
    assert_eq!(
        val_to_symbol(&env, topics.get(1).unwrap()),
        symbol_short!("paused")
    );
    let ev: RiskPausedEvent = data.try_into_val(&env).unwrap();
    assert_eq!(ev.paused, true);

    // Unpause then record action
    client.set_paused(&false);
    client.record_risk_admin_action();
    let events = env.events().all();
    let event = events.last().unwrap();
    let topics = &event.1;
    let data = event.2.clone();
    assert_eq!(
        val_to_symbol(&env, topics.get(0).unwrap()),
        symbol_short!("risk")
    );
    assert_eq!(
        val_to_symbol(&env, topics.get(1).unwrap()),
        symbol_short!("rad_act")
    );
    let ev: RiskAdminActionRecordedEvent = data.try_into_val(&env).unwrap();
    // In the test environment timestamp defaults to 0.
    let _ = ev.timestamp;
}
