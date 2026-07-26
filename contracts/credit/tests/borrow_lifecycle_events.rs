// SPDX-License-Identifier: MIT
#![cfg(test)]

use creditra_credit::events::{BorrowLifecycleEvent, BorrowLifecyclePhase, DebtForgivenEvent};
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{
    testutils::{Address as _, Events},
    token::StellarAssetClient,
    Address, Env, IntoVal, Symbol,
};

fn setup(env: &Env) -> (CreditClient, Address, Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let borrower = Address::generate(env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&token);

    let token_admin = StellarAssetClient::new(env, &token);
    token_admin.mint(&borrower, &100_000_i128);
    token_admin.mint(&token, &100_000_i128);

    (client, admin, borrower, token)
}

fn find_borrow_lifecycle_events(env: &Env) -> soroban_sdk::Vec<BorrowLifecycleEvent> {
    let mut result = soroban_sdk::Vec::new(env);
    for (_, topics, data) in env.events().all().iter() {
        if topics.len() >= 2 {
            let t1: Result<soroban_sdk::Symbol, _> = topics.get(0).unwrap().try_into_val(env);
            let t2: Result<soroban_sdk::Symbol, _> = topics.get(1).unwrap().try_into_val(env);
            if let (Ok(a), Ok(b)) = (t1, t2) {
                if a == soroban_sdk::symbol_short!("credit") && b == Symbol::new(env, "borrow_lc") {
                    if let Ok(ev) = data.try_into_val(env) {
                        result.push_back(ev);
                    }
                }
            }
        }
    }
    result
}

/// draw_credit emits a BorrowLifecycleEvent with phase Drawn.
#[test]
fn draw_credit_emits_borrow_lifecycle_drawn() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env);

    client.open_credit_line(&borrower, &10_000, &0, &0);
    client.deposit_collateral(&borrower, &1_500);
    client.draw_credit(&borrower, &1_000);

    let events = find_borrow_lifecycle_events(&env);
    assert!(
        !events.is_empty(),
        "expected at least one BorrowLifecycleEvent"
    );

    let ev = events.last().unwrap();
    assert_eq!(ev.borrower, borrower);
    assert!(matches!(ev.phase, BorrowLifecyclePhase::Drawn));
    assert_eq!(ev.utilized_amount, 1_000);
}

/// repay_credit emits a BorrowLifecycleEvent with phase Repaid.
#[test]
fn repay_credit_emits_borrow_lifecycle_repaid() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env);

    client.open_credit_line(&borrower, &10_000, &0, &0);
    client.deposit_collateral(&borrower, &1_500);
    client.draw_credit(&borrower, &1_000);
    client.repay_credit(&borrower, &500);

    let events = find_borrow_lifecycle_events(&env);
    let repaid_events: soroban_sdk::Vec<BorrowLifecycleEvent> = events
        .iter()
        .filter(|e| matches!(e.phase, BorrowLifecyclePhase::Repaid))
        .collect();

    assert!(
        !repaid_events.is_empty(),
        "expected a Repaid lifecycle event"
    );
    let ev = repaid_events.last().unwrap();
    assert_eq!(ev.borrower, borrower);
    assert_eq!(ev.utilized_amount, 500);
}

/// forgive_debt emits DebtForgivenEvent and BorrowLifecycleEvent with phase DebtForgiven.
#[test]
fn forgive_debt_emits_debt_forgiven_and_lifecycle_events() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env);

    client.open_credit_line(&borrower, &10_000, &500, &50);
    client.deposit_collateral(&borrower, &1_500);
    client.draw_credit(&borrower, &1_000);

    // Advance time so interest accrues.
    env.ledger().with_mut(|l| l.timestamp += 365 * 24 * 3600);
    client.accrue_batch(&soroban_sdk::vec![&env, borrower.clone()]);

    client.forgive_debt(&borrower, &100);

    // Check DebtForgivenEvent.
    let mut found_forgiven = false;
    for (_, topics, data) in env.events().all().iter() {
        if topics.len() >= 2 {
            let t2: Result<soroban_sdk::Symbol, _> = topics.get(1).unwrap().try_into_val(&env);
            if let Ok(b) = t2 {
                if b == Symbol::new(&env, "debt_frgv") {
                    let ev: DebtForgivenEvent = data.try_into_val(&env).unwrap();
                    assert_eq!(ev.borrower, borrower);
                    assert!(ev.amount_forgiven <= 100);
                    found_forgiven = true;
                }
            }
        }
    }
    assert!(found_forgiven, "expected DebtForgivenEvent");

    // Check BorrowLifecycleEvent with DebtForgiven phase.
    let lc_events = find_borrow_lifecycle_events(&env);
    let forgiven_lc: soroban_sdk::Vec<BorrowLifecycleEvent> = lc_events
        .iter()
        .filter(|e| matches!(e.phase, BorrowLifecyclePhase::DebtForgiven))
        .collect();
    assert!(
        !forgiven_lc.is_empty(),
        "expected DebtForgiven lifecycle event"
    );
}

/// forgive_debt with zero amount reverts.
#[test]
#[should_panic(expected = "Error(Contract, #")]
fn forgive_debt_zero_amount_reverts() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env);

    client.open_credit_line(&borrower, &10_000, &0, &0);
    client.forgive_debt(&borrower, &0);
}

/// forgive_debt on non-existent line reverts.
#[test]
#[should_panic(expected = "Error(Contract, #")]
fn forgive_debt_no_line_reverts() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env);

    client.forgive_debt(&borrower, &100);
}
