// SPDX-License-Identifier: MIT

//! Focused tests for credit-line freeze with structured reason taxonomy (#629).

use creditra_credit::events::{CreditLineFreezeEvent, DrawsFrozenEvent};
use creditra_credit::{Credit, CreditClient, FreezeReason};
use soroban_sdk::testutils::{Address as _, Events};
use soroban_sdk::{token, Address, Env, Symbol, TryFromVal, TryIntoVal};

fn setup_with_token() -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);
    (env, admin, contract_id, token_address)
}

#[test]
fn freeze_draws_records_and_returns_reason() {
    let (env, _admin, contract_id, _) = setup_with_token();
    let client = CreditClient::new(&env, &contract_id);

    assert!(client.get_draws_freeze_reason().is_none());

    client.freeze_draws(&FreezeReason::Compliance);
    assert!(client.is_draws_frozen());
    assert_eq!(
        client.get_draws_freeze_reason(),
        Some(FreezeReason::Compliance)
    );
}

#[test]
fn unfreeze_draws_clears_active_reason() {
    let (env, _admin, contract_id, _) = setup_with_token();
    let client = CreditClient::new(&env, &contract_id);

    client.freeze_draws(&FreezeReason::RiskInvestigation);
    client.unfreeze_draws();

    assert!(!client.is_draws_frozen());
    assert!(client.get_draws_freeze_reason().is_none());
}

#[test]
fn freeze_credit_line_blocks_draws_with_reason() {
    let (env, _admin, contract_id, token_address) = setup_with_token();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &1_000, &300, &50);
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &1_000);

    client.freeze_credit_line(&borrower, &FreezeReason::Compliance);
    assert!(client.is_credit_line_frozen(&borrower));
    assert_eq!(
        client.get_credit_line_freeze_reason(&borrower),
        Some(FreezeReason::Compliance)
    );

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.draw_credit(&borrower, &100);
    }));
    assert!(
        result.is_err(),
        "draw must fail while credit line is frozen"
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #41)")]
fn draw_credit_reverts_with_credit_line_frozen_error() {
    let (env, _admin, contract_id, token_address) = setup_with_token();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &1_000, &300, &50);
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &1_000);
    client.freeze_credit_line(&borrower, &FreezeReason::RiskInvestigation);
    client.draw_credit(&borrower, &100);
}

#[test]
fn unfreeze_credit_line_restores_draw_access() {
    let (env, _admin, contract_id, token_address) = setup_with_token();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &1_000, &300, &50);
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &1_000);
    client.freeze_credit_line(&borrower, &FreezeReason::OperationalMaintenance);
    client.unfreeze_credit_line(&borrower);

    assert!(!client.is_credit_line_frozen(&borrower));
    assert!(client.get_credit_line_freeze_reason(&borrower).is_none());
    client.draw_credit(&borrower, &100);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().utilized_amount,
        100
    );
}

#[test]
fn repay_credit_allowed_while_credit_line_frozen() {
    let (env, _admin, contract_id, token_address) = setup_with_token();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &1_000, &300, &50);
    let sac = token::StellarAssetClient::new(&env, &token_address);
    sac.mint(&contract_id, &1_000);
    client.draw_credit(&borrower, &400);
    client.freeze_credit_line(&borrower, &FreezeReason::Compliance);

    sac.mint(&borrower, &100);
    token::Client::new(&env, &token_address).approve(&borrower, &contract_id, &100, &1_000);
    client.repay_credit(&borrower, &100);

    assert_eq!(
        client.get_credit_line(&borrower).unwrap().utilized_amount,
        300
    );
}

#[test]
#[should_panic]
fn freeze_credit_line_requires_admin_auth() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    client.open_credit_line(&borrower, &1_000, &300, &50);
    client.freeze_credit_line(&borrower, &FreezeReason::Compliance);
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn freeze_credit_line_unknown_borrower_reverts() {
    let (env, _admin, contract_id, _) = setup_with_token();
    let client = CreditClient::new(&env, &contract_id);
    let missing = Address::generate(&env);
    client.freeze_credit_line(&missing, &FreezeReason::Compliance);
}

#[test]
fn freeze_credit_line_emits_event_with_reason() {
    let (env, _admin, contract_id, _) = setup_with_token();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000, &300, &50);

    let _ = env.events().all();
    client.freeze_credit_line(&borrower, &FreezeReason::BorrowerRequest);

    let events = env.events().all();
    assert_eq!(events.len(), 1);
    let (_contract, topics, data) = events.last().unwrap();
    assert_eq!(
        Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
        Symbol::new(&env, "line_frz")
    );
    let event: CreditLineFreezeEvent = data.try_into_val(&env).unwrap();
    assert!(event.frozen);
    assert_eq!(event.reason, FreezeReason::BorrowerRequest);
    assert_eq!(event.borrower, borrower);
}

#[test]
fn freeze_draws_emits_reason_in_event() {
    let (env, _admin, contract_id, _) = setup_with_token();
    let client = CreditClient::new(&env, &contract_id);

    let _ = env.events().all();
    client.freeze_draws(&FreezeReason::LiquidityReserve);

    let events = env.events().all();
    let (_contract, topics, data) = events.last().unwrap();
    assert_eq!(
        Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
        Symbol::new(&env, "drw_freeze")
    );
    let event: DrawsFrozenEvent = data.try_into_val(&env).unwrap();
    assert!(event.frozen);
    assert_eq!(event.reason, FreezeReason::LiquidityReserve);
}

#[test]
fn updating_credit_line_freeze_reason_overwrites_storage() {
    let (env, _admin, contract_id, _) = setup_with_token();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000, &300, &50);

    client.freeze_credit_line(&borrower, &FreezeReason::Compliance);
    client.freeze_credit_line(&borrower, &FreezeReason::RiskInvestigation);

    assert_eq!(
        client.get_credit_line_freeze_reason(&borrower),
        Some(FreezeReason::RiskInvestigation)
    );
}
