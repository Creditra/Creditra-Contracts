// SPDX-License-Identifier: MIT

use creditra_credit::events::{BountyWithdrawnEvent, TreasuryWithdrawnEvent};
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Events, Ledger};
use soroban_sdk::{symbol_short, token, Address, Env, FromVal, Symbol, TryFromVal};

fn setup_test_env<'a>(
    env: &'a Env,
) -> (
    Address,
    Address,
    Address,
    Address,
    Address,
    Address,
    CreditClient<'a>,
) {
    env.mock_all_auths_allowing_non_root_auth();

    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let treasury = Address::generate(env);
    let bounty = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token_address = token_id.address();

    client.set_liquidity_token(&token_address);
    client.set_liquidity_source(&contract_id);
    client.set_treasury(&admin, &treasury);
    client.set_bounty(&admin, &bounty);

    (
        contract_id,
        token_address,
        admin,
        borrower,
        treasury,
        bounty,
        client,
    )
}

fn accrue_fees(
    env: &Env,
    contract_id: &Address,
    token_address: &Address,
    borrower: &Address,
    client: &CreditClient<'_>,
) {
    let draw_amount: i128 = 1_000;
    let repay_amount: i128 = 1_100;

    client.open_credit_line(borrower, &draw_amount, &1_000_u32, &50_u32);

    let asset = token::StellarAssetClient::new(env, token_address);
    let collateral = draw_amount * 3;
    asset.mint(borrower, &collateral);
    client.deposit_collateral(borrower, &collateral);

    asset.mint(contract_id, &draw_amount);
    client.draw_credit(borrower, &draw_amount);
    client.set_protocol_fee_bps(&1_000_u32);

    env.ledger().with_mut(|ledger| ledger.timestamp = 31_557_600);

    asset.mint(borrower, &repay_amount);
    token::Client::new(env, token_address).approve(
        borrower,
        contract_id,
        &repay_amount,
        &6_000_000_u32,
    );

    client.repay_credit(borrower, &repay_amount);
}

#[test]
fn withdraw_treasury_zero_balance_emits_no_events() {
    let env = Env::default();
    let (_contract_id, _token, admin, _borrower, _treasury, _bounty, client) =
        setup_test_env(&env);

    let initial_events = env.events().all().len();
    client.withdraw_treasury(&admin);
    let after_events = env.events().all().len();

    assert_eq!(initial_events, after_events);
}

#[test]
fn withdraw_treasury_non_zero_balance_emits_event_and_transfers() {
    let env = Env::default();
    let (contract_id, token_address, admin, borrower, treasury, _bounty, client) =
        setup_test_env(&env);

    accrue_fees(&env, &contract_id, &token_address, &borrower, &client);

    let initial_balance = client.get_protocol_summary().treasury_balance;
    assert!(initial_balance > 0);

    let token_client = token::Client::new(&env, &token_address);
    assert_eq!(token_client.balance(&treasury), 0);

    let initial_credit_events = env
        .events()
        .all()
        .iter()
        .filter(|e| e.0 == contract_id)
        .count();

    client.withdraw_treasury(&admin);

    let current_credit_events: std::vec::Vec<_> = env
        .events()
        .all()
        .into_iter()
        .filter(|e| e.0 == contract_id)
        .collect();

    assert_eq!(current_credit_events.len(), initial_credit_events + 1);
    assert_eq!(token_client.balance(&treasury), initial_balance);
    assert_eq!(client.get_protocol_summary().treasury_balance, 0);

    let last_ev = current_credit_events.last().unwrap();
    let topics = &last_ev.1;
    let t0 = Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap();
    let t1 = Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap();

    assert_eq!(t0, symbol_short!("credit"));
    assert_eq!(t1, Symbol::new(&env, "tre_wdrn"));

    let data = TreasuryWithdrawnEvent::from_val(&env, &last_ev.2);
    assert_eq!(data.recipient, treasury);
    assert_eq!(data.amount, initial_balance);
    assert_eq!(data.executor, admin);

    let events_count_before_second = env.events().all().len();
    client.withdraw_treasury(&admin);
    assert_eq!(env.events().all().len(), events_count_before_second);
}

#[test]
fn withdraw_bounty_zero_balance_emits_no_events() {
    let env = Env::default();
    let (_contract_id, _token, admin, _borrower, _treasury, _bounty, client) =
        setup_test_env(&env);

    let initial_events = env.events().all().len();
    client.withdraw_bounty(&admin);
    let after_events = env.events().all().len();

    assert_eq!(initial_events, after_events);
}

#[test]
fn withdraw_bounty_non_zero_balance_emits_event_and_transfers() {
    let env = Env::default();
    let (contract_id, token_address, admin, borrower, _treasury, bounty, client) =
        setup_test_env(&env);

    client.set_treasury_fee_share_bps(&0_u32);
    accrue_fees(&env, &contract_id, &token_address, &borrower, &client);

    let initial_balance = client.get_protocol_summary().bounty_balance;
    assert!(initial_balance > 0);

    let token_client = token::Client::new(&env, &token_address);
    assert_eq!(token_client.balance(&bounty), 0);

    let initial_credit_events = env
        .events()
        .all()
        .iter()
        .filter(|e| e.0 == contract_id)
        .count();

    client.withdraw_bounty(&admin);

    let current_credit_events: std::vec::Vec<_> = env
        .events()
        .all()
        .into_iter()
        .filter(|e| e.0 == contract_id)
        .collect();

    assert_eq!(current_credit_events.len(), initial_credit_events + 1);
    assert_eq!(token_client.balance(&bounty), initial_balance);
    assert_eq!(client.get_protocol_summary().bounty_balance, 0);

    let last_ev = current_credit_events.last().unwrap();
    let topics = &last_ev.1;
    let t0 = Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap();
    let t1 = Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap();

    assert_eq!(t0, symbol_short!("credit"));
    assert_eq!(t1, Symbol::new(&env, "bty_wdrn"));

    let data = BountyWithdrawnEvent::from_val(&env, &last_ev.2);
    assert_eq!(data.recipient, bounty);
    assert_eq!(data.amount, initial_balance);
    assert_eq!(data.executor, admin);

    let events_count_before_second = env.events().all().len();
    client.withdraw_bounty(&admin);
    assert_eq!(env.events().all().len(), events_count_before_second);
}
