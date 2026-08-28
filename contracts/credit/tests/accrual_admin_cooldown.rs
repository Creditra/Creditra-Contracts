// SPDX-License-Identifier: MIT

//! Regression tests for the per-borrower cooldown on accrual-critical admin actions.

use creditra_credit::types::{ContractError, CreditStatus};
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env};

const START_TS: u64 = 10_000;
const COOLDOWN_SECONDS: u64 = 300;

fn setup(start_ts: u64) -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = start_ts);

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    (env, contract_id, admin)
}

fn set_timestamp(env: &Env, timestamp: u64) {
    env.ledger().with_mut(|li| li.timestamp = timestamp);
}

#[test]
fn accrual_admin_cooldown_rejects_second_action_until_boundary() {
    let (env, contract_id, _admin) = setup(START_TS);
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.set_accrual_admin_cooldown(&COOLDOWN_SECONDS);
    assert_eq!(client.get_accrual_admin_cooldown(), Some(COOLDOWN_SECONDS));

    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);
    set_timestamp(&env, START_TS + 1);
    client.default_credit_line(&borrower);

    set_timestamp(&env, START_TS + COOLDOWN_SECONDS);
    let result = client.try_reinstate_credit_line(&borrower, &CreditStatus::Active);
    assert_eq!(
        result.err().unwrap().unwrap(),
        ContractError::AdminCooldownActive.into()
    );

    set_timestamp(&env, START_TS + COOLDOWN_SECONDS + 1);
    client.reinstate_credit_line(&borrower, &CreditStatus::Active);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Active);
}

#[test]
fn accrual_admin_cooldown_is_per_borrower() {
    let (env, contract_id, _admin) = setup(START_TS);
    let client = CreditClient::new(&env, &contract_id);
    let first = Address::generate(&env);
    let second = Address::generate(&env);

    client.set_accrual_admin_cooldown(&COOLDOWN_SECONDS);
    client.open_credit_line(&first, &1_000_i128, &300_u32, &70_u32);
    client.open_credit_line(&second, &2_000_i128, &300_u32, &70_u32);

    set_timestamp(&env, START_TS + 1);
    client.suspend_credit_line(&first);
    client.suspend_credit_line(&second);

    assert_eq!(
        client.get_credit_line(&first).unwrap().status,
        CreditStatus::Suspended
    );
    assert_eq!(
        client.get_credit_line(&second).unwrap().status,
        CreditStatus::Suspended
    );
}

#[test]
fn accrual_admin_cooldown_zero_disables_guard() {
    let (env, contract_id, _admin) = setup(START_TS);
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.set_accrual_admin_cooldown(&0_u64);
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);

    set_timestamp(&env, START_TS + 1);
    client.default_credit_line(&borrower);
    set_timestamp(&env, START_TS + 1);
    client.reinstate_credit_line(&borrower, &CreditStatus::Active);

    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Active
    );
}

#[test]
fn borrow_admin_cooldown_no_longer_blocks_accrual_actions() {
    let (env, contract_id, _admin) = setup(START_TS);
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.set_borrow_admin_cooldown(&COOLDOWN_SECONDS);
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);

    set_timestamp(&env, START_TS + 1);
    client.suspend_credit_line(&borrower);

    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Suspended
    );
}
