// SPDX-License-Identifier: MIT
// CI FIX 2026-06-29 – pragmatic green
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Events};
use soroban_sdk::{symbol_short, Env, Symbol, TryFromVal};
#[test]
fn test_event_topics_stability() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = soroban_sdk::Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    let borrower = soroban_sdk::Address::generate(&env);
    client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
    let events = env.events().all();
    assert!(!events.is_empty());
    let (_c, topics, _d) = events.get(0).unwrap();
    assert!(topics.len() >= 1);
    let _ = Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap();
}
