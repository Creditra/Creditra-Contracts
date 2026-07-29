// SPDX-License-Identifier: MIT

//! Regression tests for the per-borrower cooldown on critical admin actions.

use creditra_credit::types::ContractError;
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
fn borrow_admin_cooldown_rejects_second_action_until_boundary() {
    let (env, contract_id, _admin) = setup(START_TS);
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.set_borrow_admin_cooldown(&COOLDOWN_SECONDS);
    assert_eq!(client.get_borrow_admin_cooldown(), Some(COOLDOWN_SECONDS));

    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);

    set_timestamp(&env, START_TS + COOLDOWN_SECONDS - 1);
    let result = client.try_update_risk_parameters(&borrower, &1_100_i128, &350_u32, &71_u32);
    assert_eq!(
        result.err().unwrap().unwrap(),
        ContractError::AdminCooldownActive.into()
    );

    set_timestamp(&env, START_TS + COOLDOWN_SECONDS);
    client.update_risk_parameters(&borrower, &1_100_i128, &350_u32, &71_u32);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.credit_limit, 1_100);
    assert_eq!(line.interest_rate_bps, 350);
}

#[test]
fn borrow_admin_cooldown_is_per_borrower() {
    let (env, contract_id, _admin) = setup(START_TS);
    let client = CreditClient::new(&env, &contract_id);
    let first = Address::generate(&env);
    let second = Address::generate(&env);

    client.set_borrow_admin_cooldown(&COOLDOWN_SECONDS);
    client.open_credit_line(&first, &1_000_i128, &300_u32, &70_u32);

    client.open_credit_line(&second, &2_000_i128, &300_u32, &70_u32);

    assert_eq!(client.get_credit_line(&first).unwrap().credit_limit, 1_000);
    assert_eq!(client.get_credit_line(&second).unwrap().credit_limit, 2_000);
}

#[test]
fn borrow_admin_cooldown_zero_disables_guard() {
    let (env, contract_id, _admin) = setup(START_TS);
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.set_borrow_admin_cooldown(&0_u64);
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);

    set_timestamp(&env, START_TS);
    client.update_risk_parameters(&borrower, &1_200_i128, &350_u32, &71_u32);

    assert_eq!(
        client.get_credit_line(&borrower).unwrap().credit_limit,
        1_200
    );
}
