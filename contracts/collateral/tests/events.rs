// SPDX-License-Identifier: MIT

//! Focused integration tests for `creditra-collateral` event emission layer.

use creditra_collateral::events::{
    publish_collateral_closed, publish_collateral_liquidated, publish_collateral_released,
    publish_collateral_transferred, publish_collateral_updated, CollateralClosedEvent,
    CollateralDepositedEvent, CollateralLiquidatedEvent, CollateralReleasedEvent,
    CollateralTransferredEvent, CollateralUpdatedEvent, CollateralWithdrawnEvent,
};
use creditra_collateral::{Collateral, CollateralClient, CollateralError};
use soroban_sdk::testutils::{Address as _, Events};
use soroban_sdk::{symbol_short, Address, Env, IntoVal};

fn setup_env<'a>() -> (Env, CollateralClient<'a>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(Collateral, ());
    let client = CollateralClient::new(&env, &contract_id);
    let user = Address::generate(&env);
    (env, client, user)
}

#[test]
fn test_deposit_emits_collateral_deposited_event() {
    let (env, client, user) = setup_env();

    client.deposit(&user, &1_000_i128);

    let events = env.events().all();
    assert_eq!(events.len(), 1);

    let event = events.get(0).unwrap();
    assert_eq!(event.0, client.address);
    assert_eq!(
        event.1,
        (symbol_short!("collat"), symbol_short!("deposit")).into_val(&env)
    );

    let payload: CollateralDepositedEvent = event.2.into_val(&env);
    assert_eq!(payload.user, user);
    assert_eq!(payload.amount, 1_000);
    assert_eq!(payload.new_balance, 1_000);
}

#[test]
fn test_withdraw_emits_collateral_withdrawn_event() {
    let (env, client, user) = setup_env();

    client.deposit(&user, &1_000_i128);
    client.withdraw(&user, &400_i128);

    let events = env.events().all();
    assert_eq!(events.len(), 1);

    let withdraw_event = events.get(0).unwrap();
    assert_eq!(
        withdraw_event.1,
        (symbol_short!("collat"), symbol_short!("withdraw")).into_val(&env)
    );

    let payload: CollateralWithdrawnEvent = withdraw_event.2.into_val(&env);
    assert_eq!(payload.user, user);
    assert_eq!(payload.amount, 400);
    assert_eq!(payload.new_balance, 600);
}

#[test]
fn test_multiple_operations_maintain_event_ordering() {
    let (env, client, user) = setup_env();

    // Operation 1: Deposit 500 -> new_balance 500
    client.deposit(&user, &500_i128);
    let events1 = env.events().all();
    assert_eq!(events1.len(), 1);
    let e1 = events1.get(0).unwrap();
    assert_eq!(
        e1.1,
        (symbol_short!("collat"), symbol_short!("deposit")).into_val(&env)
    );
    let p1: CollateralDepositedEvent = e1.2.into_val(&env);
    assert_eq!(p1.amount, 500);
    assert_eq!(p1.new_balance, 500);

    // Operation 2: Deposit 300 -> new_balance 800
    client.deposit(&user, &300_i128);
    let events2 = env.events().all();
    assert_eq!(events2.len(), 1);
    let e2 = events2.get(0).unwrap();
    assert_eq!(
        e2.1,
        (symbol_short!("collat"), symbol_short!("deposit")).into_val(&env)
    );
    let p2: CollateralDepositedEvent = e2.2.into_val(&env);
    assert_eq!(p2.amount, 300);
    assert_eq!(p2.new_balance, 800);

    // Operation 3: Withdraw 200 -> new_balance 600
    client.withdraw(&user, &200_i128);
    let events3 = env.events().all();
    assert_eq!(events3.len(), 1);
    let e3 = events3.get(0).unwrap();
    assert_eq!(
        e3.1,
        (symbol_short!("collat"), symbol_short!("withdraw")).into_val(&env)
    );
    let p3: CollateralWithdrawnEvent = e3.2.into_val(&env);
    assert_eq!(p3.amount, 200);
    assert_eq!(p3.new_balance, 600);
}

#[test]
fn test_failed_deposit_does_not_emit_event() {
    let (env, client, user) = setup_env();

    let res = client.try_deposit(&user, &0_i128);
    assert_eq!(res, Err(Ok(CollateralError::InvalidAmount)));

    let events = env.events().all();
    assert_eq!(events.len(), 0);
}

#[test]
fn test_failed_withdraw_does_not_emit_event() {
    let (env, client, user) = setup_env();

    client.deposit(&user, &100_i128);

    let res = client.try_withdraw(&user, &500_i128);
    assert_eq!(res, Err(Ok(CollateralError::InsufficientCollateralBalance)));

    let events_after = env.events().all();
    assert_eq!(events_after.len(), 0);
}

#[test]
fn test_helper_publishers_format_and_emit_correctly() {
    let env = Env::default();
    let contract_id = env.register(Collateral, ());
    let user1 = Address::generate(&env);
    let user2 = Address::generate(&env);

    env.as_contract(&contract_id, || {
        // Publish updated
        publish_collateral_updated(&env, &user1, 100, 200);
        // Publish released
        publish_collateral_released(&env, &user1, 50, 150);
        // Publish liquidated
        publish_collateral_liquidated(&env, &user1, &user2, 50, 100);
        // Publish transferred
        publish_collateral_transferred(&env, &user1, &user2, 30);
        // Publish closed
        publish_collateral_closed(&env, &user1, 0);
    });

    let events = env.events().all();
    assert_eq!(events.len(), 5);

    // Updated
    let e0 = events.get(0).unwrap();
    assert_eq!(
        e0.1,
        (symbol_short!("collat"), symbol_short!("updated")).into_val(&env)
    );
    let p0: CollateralUpdatedEvent = e0.2.into_val(&env);
    assert_eq!(p0.user, user1);
    assert_eq!(p0.old_balance, 100);
    assert_eq!(p0.new_balance, 200);

    // Released
    let e1 = events.get(1).unwrap();
    assert_eq!(
        e1.1,
        (symbol_short!("collat"), symbol_short!("release")).into_val(&env)
    );
    let p1: CollateralReleasedEvent = e1.2.into_val(&env);
    assert_eq!(p1.user, user1);
    assert_eq!(p1.amount, 50);
    assert_eq!(p1.new_balance, 150);

    // Liquidated
    let e2 = events.get(2).unwrap();
    assert_eq!(
        e2.1,
        (symbol_short!("collat"), symbol_short!("liquidate")).into_val(&env)
    );
    let p2: CollateralLiquidatedEvent = e2.2.into_val(&env);
    assert_eq!(p2.user, user1);
    assert_eq!(p2.liquidator, user2);
    assert_eq!(p2.amount, 50);
    assert_eq!(p2.new_balance, 100);

    // Transferred
    let e3 = events.get(3).unwrap();
    assert_eq!(
        e3.1,
        (symbol_short!("collat"), symbol_short!("transfer")).into_val(&env)
    );
    let p3: CollateralTransferredEvent = e3.2.into_val(&env);
    assert_eq!(p3.from, user1);
    assert_eq!(p3.to, user2);
    assert_eq!(p3.amount, 30);

    // Closed
    let e4 = events.get(4).unwrap();
    assert_eq!(
        e4.1,
        (symbol_short!("collat"), symbol_short!("closed")).into_val(&env)
    );
    let p4: CollateralClosedEvent = e4.2.into_val(&env);
    assert_eq!(p4.user, user1);
    assert_eq!(p4.final_balance, 0);
}
