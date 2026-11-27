// SPDX-License-Identifier: MIT
#![cfg(test)]

use creditra_risk::{ContractError, RiskContract, RiskContractClient, RiskPausedEvent};
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events},
    Address, Env, Symbol, TryFromVal, TryIntoVal,
};

fn setup() -> (Env, Address, RiskContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(RiskContract, ());
    let client = RiskContractClient::new(&env, &contract_id);
    client.init(&admin);
    (env, admin, client)
}

fn val_to_symbol(env: &Env, val: soroban_sdk::Val) -> Symbol {
    Symbol::try_from_val(env, &val).expect("topic must be a symbol")
}

#[test]
fn test_paused_blocks_set_cooldown() {
    let (env, _admin, client) = setup();
    client.set_paused(&true);

    let res = client.try_set_risk_admin_cooldown(&3600);
    assert_eq!(res.unwrap_err().unwrap(), ContractError::Paused);
}

#[test]
fn test_paused_blocks_record_action() {
    let (env, _admin, client) = setup();
    client.set_paused(&true);

    let res = client.try_record_risk_admin_action();
    assert_eq!(res.unwrap_err().unwrap(), ContractError::Paused);
}

#[test]
fn test_unpause_restores_functionality() {
    let (env, _admin, client) = setup();
    client.set_paused(&true);

    // Blocked
    assert_eq!(
        client.try_set_risk_admin_cooldown(&3600).unwrap_err().unwrap(),
        ContractError::Paused
    );

    // Unpause
    client.set_paused(&false);

    // Should work now
    client.set_risk_admin_cooldown(&3600);
    client.record_risk_admin_action();
}

#[test]
fn test_set_paused_emits_events() {
    let (env, _admin, client) = setup();

    // Pause
    client.set_paused(&true);
    let events = env.events().all();
    let event = events.last().unwrap();
    let topics = &event.1;
    let data = event.2.clone();

    assert_eq!(val_to_symbol(&env, topics.get(0).unwrap()), symbol_short!("risk"));
    assert_eq!(val_to_symbol(&env, topics.get(1).unwrap()), symbol_short!("paused"));
    let ev: RiskPausedEvent = data.try_into_val(&env).unwrap();
    assert_eq!(ev.paused, true);

    // Unpause
    client.set_paused(&false);
    let events = env.events().all();
    let event = events.last().unwrap();
    let topics = &event.1;
    let data = event.2.clone();

    assert_eq!(val_to_symbol(&env, topics.get(0).unwrap()), symbol_short!("risk"));
    assert_eq!(val_to_symbol(&env, topics.get(1).unwrap()), symbol_short!("paused"));
    let ev: RiskPausedEvent = data.try_into_val(&env).unwrap();
    assert_eq!(ev.paused, false);
}
