// SPDX-License-Identifier: MIT

use std::panic::{catch_unwind, AssertUnwindSafe};

use creditra_credit::events::InterestAccruedEvent;
use creditra_credit::types::CreditStatus;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::testutils::Events as _;
use soroban_sdk::testutils::Ledger;
use soroban_sdk::{Address, Env, Symbol, TryFromVal, TryIntoVal, Vec};

fn setup_env() -> (Env, Address, CreditClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    (env, admin, client)
}

fn last_accrue_event(env: &Env) -> InterestAccruedEvent {
    let namespace = Symbol::new(env, "credit");
    let kind = Symbol::new(env, "accrue");

    for (_contract, topics, data) in env.events().all().iter().rev() {
        let t0: Symbol = Symbol::try_from_val(env, &topics.get(0).unwrap()).unwrap();
        let t1: Symbol = Symbol::try_from_val(env, &topics.get(1).unwrap()).unwrap();
        if t0 == namespace && t1 == kind {
            return data.try_into_val(env).unwrap();
        }
    }

    panic!("No accrue event found");
}

#[test]
fn accrue_batch_enforces_hard_cap() {
    let (env, _admin, client) = setup_env();

    let mut borrowers = Vec::new(&env);
    for _ in 0..51 {
        borrowers.push_back(Address::generate(&env));
    }

    let result = catch_unwind(AssertUnwindSafe(|| {
        client.accrue_batch(&borrowers);
    }));

    assert!(result.is_err(), "accrue_batch must reject oversized batches");
}

#[test]
fn accrue_batch_skips_missing_and_non_active_lines() {
    let (env, _admin, client) = setup_env();

    let active = Address::generate(&env);
    let suspended = Address::generate(&env);
    let missing = Address::generate(&env);

    client.open_credit_line(&active, &1_000_000_i128, &1_000_u32, &50_u32);
    client.open_credit_line(&suspended, &1_000_000_i128, &1_000_u32, &50_u32);

    env.ledger().set_timestamp(1);
    client.draw_credit(&active, &100_000_i128);
    client.draw_credit(&suspended, &100_000_i128);

    client.suspend_credit_line(&suspended);

    env.ledger().set_timestamp(1 + 31_536_000);

    let before_events = env.events().all().len();

    let mut borrowers = Vec::new(&env);
    borrowers.push_back(active.clone());
    borrowers.push_back(suspended.clone());
    borrowers.push_back(missing.clone());

    client.accrue_batch(&borrowers);

    let active_line = client.get_credit_line(&active).unwrap();
    assert_eq!(active_line.status, CreditStatus::Active);
    assert_eq!(active_line.last_accrual_ts, 1 + 31_536_000);
    assert_eq!(active_line.accrued_interest, 10_000);
    assert_eq!(active_line.utilized_amount, 110_000);

    let suspended_line = client.get_credit_line(&suspended).unwrap();
    assert_eq!(suspended_line.status, CreditStatus::Suspended);
    assert_eq!(suspended_line.last_accrual_ts, 1);
    assert_eq!(suspended_line.accrued_interest, 0);
    assert_eq!(suspended_line.utilized_amount, 100_000);

    assert!(client.get_credit_line(&missing).is_none());

    assert_eq!(env.events().all().len(), before_events + 1);

    let event = last_accrue_event(&env);
    assert_eq!(event.borrower, active);
    assert_eq!(event.accrued_amount, 10_000);
    assert_eq!(event.new_utilized_amount, 110_000);
}

#[test]
fn accrue_batch_respects_pause_guard() {
    let (env, _admin, client) = setup_env();
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &1_000_000_i128, &1_000_u32, &50_u32);
    env.ledger().set_timestamp(1);
    client.draw_credit(&borrower, &100_000_i128);
    env.ledger().set_timestamp(1 + 31_536_000);

    client.set_protocol_paused(&true);

    let mut borrowers = Vec::new(&env);
    borrowers.push_back(borrower);

    let result = catch_unwind(AssertUnwindSafe(|| {
        client.accrue_batch(&borrowers);
    }));

    assert!(result.is_err(), "accrue_batch must fail when paused");
}