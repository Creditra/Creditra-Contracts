// SPDX-License-Identifier: MIT

//! CI-pinned integration test for the credit liquidation request (`liq_req`) event.
//!
//! This test ensures that the event `("credit", "liq_req")` remains a 2-tuple of `(Address, i128)`
//! (specifically `(borrower, utilized_amount)`) and does not silently change or grow extra fields.
//! The stability of this payload is critical for downstream indexers consuming this event.

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::testutils::Events as _;
use soroban_sdk::{symbol_short, token, Address, Env, IntoVal, Symbol, TryFromVal, Val, Vec};

fn setup_defaulted_line(env: &Env, utilized_amount: i128) -> (Address, Address, Address) {
    env.mock_all_auths_allowing_non_root_auth();

    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let contract_id = env.register(Credit, ());

    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);
    
    // Mint liquidity tokens to fund credit lines and draws
    token::StellarAssetClient::new(env, &token_address).mint(&contract_id, &1_000_000_i128);
    token::StellarAssetClient::new(env, &token_address).mint(&borrower, &1_000_000_i128);
    token::Client::new(env, &token_address).approve(
        &borrower,
        &contract_id,
        &1_000_000_i128,
        &1_000_000_u32,
    );

    // Open a credit line and draw some amount to establish utilized debt
    client.open_credit_line(&borrower, &10_000_i128, &300_u32, &60_u32);

    if utilized_amount > 0 {
        client.draw_credit(&borrower, &utilized_amount);
    }

    // Default the credit line to trigger the liquidation request event
    client.default_credit_line(&borrower);

    (contract_id, borrower, admin)
}

#[test]
fn test_event_liq_req_payload_tuple_pin() {
    let env = Env::default();
    let utilized_amount = 1234_i128;
    let (contract_id, borrower, _admin) = setup_defaulted_line(&env, utilized_amount);

    let all_events = env.events().all();
    let namespace = symbol_short!("credit");
    let kind = Symbol::new(&env, "liq_req");

    // Extract the ("credit", "liq_req") event
    let mut found_event = None;
    for (event_contract, topics, data) in all_events.iter() {
        if event_contract == contract_id && topics.len() >= 2 {
            let t0: Symbol = Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap();
            let t1: Symbol = Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap();
            if t0 == namespace && t1 == kind {
                found_event = Some((topics, data));
                break;
            }
        }
    }

    let (topics, data) = found_event.expect("Expected a ('credit', 'liq_req') event to be published");

    // 1. Assert topic symbols
    assert_eq!(topics.len(), 2, "Topic list must contain exactly 2 elements");
    assert_eq!(
        Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap(),
        symbol_short!("credit"),
        "First topic must be 'credit'"
    );
    assert_eq!(
        Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
        Symbol::new(&env, "liq_req"),
        "Second topic must be 'liq_req'"
    );

    // 2. Assert payload shape and tuple arity using Vec-equality.
    // In Soroban, a tuple like (Address, i128) is serialized on-chain as a Vector of Vals.
    // We convert the event payload data to a Vec<Val>.
    let payload_vec = Vec::<Val>::try_from_val(&env, &data)
        .expect("Payload must be convertible to a Soroban Vec<Val>");

    // Check arity/length explicitly (must be exactly 2 elements: borrower and utilized_amount)
    assert_eq!(
        payload_vec.len(),
        2,
        "Payload tuple arity must be exactly 2. If this fails, the event payload shape has changed!"
    );

    // Construct the expected payload vector.
    // If the contract payload ever changes (e.g. grows a third element, or elements change),
    // this assertion will fail.
    let expected_payload = soroban_sdk::vec![
        &env,
        borrower.into_val(&env),
        utilized_amount.into_val(&env),
    ];

    // Assert exact Vec equality to pin the exact structure and prevent silent growth/changes
    assert_eq!(
        payload_vec, expected_payload,
        "Payload shape does not match expected (Address, i128) format exactly"
    );
}
