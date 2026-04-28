// SPDX-License-Identifier: MIT

use std::panic::{catch_unwind, AssertUnwindSafe};

use creditra_credit::types::CreditStatus;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::testutils::Events as _;
use soroban_sdk::{Address, Env, Symbol, TryFromVal};

fn setup_defaulted_line(utilized_amount: i128) -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());

    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    client.open_credit_line(&borrower, &10_000, &300_u32, &60_u32);

    if utilized_amount > 0 {
        client.draw_credit(&borrower, &utilized_amount);
    }

    client.default_credit_line(&borrower);

    (env, contract_id, borrower)
}

fn has_event_topic(env: &Env, event_kind: &str) -> bool {
    let namespace = Symbol::new(env, "credit");
    let kind = Symbol::new(env, event_kind);

    for (_contract, topics, _data) in env.events().all().iter() {
        let t0: Symbol = Symbol::try_from_val(env, &topics.get(0).unwrap()).unwrap();
        let t1: Symbol = Symbol::try_from_val(env, &topics.get(1).unwrap()).unwrap();
        if t0 == namespace && t1 == kind {
            return true;
        }
    }

    false
}

#[test]
fn default_emits_liquidation_request_event() {
    let (env, _contract_id, _borrower) = setup_defaulted_line(500);

    assert!(has_event_topic(&env, "liq_req"));
}

#[test]
fn settle_partial_default_liquidation_and_block_replay() {
    let (env, contract_id, borrower) = setup_defaulted_line(1_000);
    let client = CreditClient::new(&env, &contract_id);
    let settlement_id = Symbol::new(&env, "auc_001");

    client.settle_default_liquidation(&borrower, &300_i128, &settlement_id);
    assert!(has_event_topic(&env, "liq_setl"));

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Defaulted);
    assert_eq!(line.utilized_amount, 700);

    let replay = catch_unwind(AssertUnwindSafe(|| {
        client.settle_default_liquidation(&borrower, &50_i128, &settlement_id);
    }));
    assert!(replay.is_err(), "replay settlement should panic");
}

#[test]
fn settle_full_default_liquidation_closes_credit_line() {
    let (env, contract_id, borrower) = setup_defaulted_line(450);
    let client = CreditClient::new(&env, &contract_id);

    client.settle_default_liquidation(&borrower, &450_i128, &Symbol::new(&env, "auc_fin"));
    assert!(has_event_topic(&env, "closed"));
    assert!(has_event_topic(&env, "liq_setl"));

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Closed);
    assert_eq!(line.utilized_amount, 0);
}

#[test]
fn settle_default_liquidation_requires_defaulted_status() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);
    client.open_credit_line(&borrower, &5_000, &200_u32, &40_u32);
    client.draw_credit(&borrower, &500_i128);

    let result = catch_unwind(AssertUnwindSafe(|| {
        client.settle_default_liquidation(&borrower, &100_i128, &Symbol::new(&env, "auc_bad"));
    }));

    assert!(result.is_err(), "non-defaulted settlement should panic");
}
