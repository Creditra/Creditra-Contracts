// SPDX-License-Identifier: MIT

//! Focused tests for `creditra-accrual` public entrypoints and structured events.

use creditra_accrual::events::{
    publish_accrual_batch_completed, publish_interest_accrued, AccrualBatchCompletedEvent,
    InterestAccruedEvent,
};
use creditra_accrual::{Credit, CreditClient};
use soroban_sdk::{
    testutils::{Address as _, Events},
    vec, Address, Env, IntoVal, Symbol,
};

#[test]
fn test_publish_accrual_batch_completed_event() {
    let env = Env::default();
    let event_payload = AccrualBatchCompletedEvent {
        borrowers_processed: 5,
        lines_accrued: 3,
        total_interest_accrued: 10_500,
        timestamp: 1_700_000_000,
    };

    publish_accrual_batch_completed(&env, event_payload.clone());

    let events = env.events().all();
    assert_eq!(events.len(), 1);
    let event = events.get(0).unwrap();
    assert_eq!(
        event.topics,
        (Symbol::new(&env, "accrual"), Symbol::new(&env, "batch")).into_val(&env)
    );
}

#[test]
fn test_publish_interest_accrued_event() {
    let env = Env::default();
    let borrower = Address::generate(&env);
    let event_payload = InterestAccruedEvent {
        borrower: borrower.clone(),
        accrued_amount: 500,
        new_utilized_amount: 10_500,
        new_accrued_interest: 500,
        elapsed_seconds: 86_400,
        timestamp: 1_700_000_000,
    };

    publish_interest_accrued(&env, event_payload.clone());

    let events = env.events().all();
    assert_eq!(events.len(), 1);
    let event = events.get(0).unwrap();
    assert_eq!(
        event.topics,
        (Symbol::new(&env, "accrual"), Symbol::new(&env, "accrue")).into_val(&env)
    );
}

#[test]
fn test_accrue_batch_public_entrypoint() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let borrower1 = Address::generate(&env);
    let borrower2 = Address::generate(&env);

    client.open_credit_line(&borrower1, &100_000_i128, &1_000_u32, &50_u32);
    client.open_credit_line(&borrower2, &100_000_i128, &1_000_u32, &50_u32);

    let batch = vec![&env, borrower1.clone(), borrower2.clone()];
    // Should run smoothly without errors on active credit lines.
    client.accrue_batch(&batch);
}
