// SPDX-License-Identifier: MIT

//! Regression coverage for reopening a closed borrower line with an existing
//! non-zero stable credit-line id.

use creditra_credit::types::CreditStatus;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Events};
use soroban_sdk::{symbol_short, Address, Env, Symbol, TryFromVal};

fn setup(env: &Env) -> (CreditClient<'_>, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    (client, admin)
}

fn credit_line_id_for(client: &CreditClient<'_>, borrower: &Address) -> u32 {
    let lines = client.enumerate_credit_lines(&None, &10);

    for index in 0..lines.len() {
        let (id, line) = lines.get(index).unwrap();
        if line.borrower == *borrower {
            return id;
        }
    }

    panic!("borrower must have an enumerated credit line id");
}

fn assert_last_event_topic(env: &Env, expected: Symbol) {
    let events = env.events().all();
    let (_contract_id, topics, _data) = events.last().unwrap();

    let namespace = Symbol::try_from_val(env, &topics.get(0).unwrap()).unwrap();
    let event_name = Symbol::try_from_val(env, &topics.get(1).unwrap()).unwrap();

    assert_eq!(namespace, symbol_short!("credit"));
    assert_eq!(event_name, expected);
}

#[test]
fn reopen_closed_line_reuses_existing_nonzero_id() {
    let env = Env::default();
    let (client, admin) = setup(&env);

    // Address::generate is deterministic for this test Env. Seeding another
    // borrower first makes the target borrower's stable id non-zero.
    let seed_borrower = Address::generate(&env);
    let borrower = Address::generate(&env);

    client.open_credit_line(&seed_borrower, &500_i128, &300_u32, &50_u32);
    client.open_credit_line(&borrower, &1_000_i128, &350_u32, &60_u32);
    assert_last_event_topic(&env, symbol_short!("opened"));

    let original_id = credit_line_id_for(&client, &borrower);
    let original_count = client.get_credit_line_count();
    assert_eq!(
        original_id, 1,
        "target borrower should exercise non-zero id path"
    );
    assert_eq!(original_count, 2);

    client.close_credit_line(&borrower, &admin);
    assert_last_event_topic(&env, symbol_short!("closed"));
    assert_eq!(client.get_credit_line_count(), original_count);

    client.open_credit_line(&borrower, &2_000_i128, &425_u32, &70_u32);
    assert_last_event_topic(&env, symbol_short!("opened"));

    let reopened = client.get_credit_line(&borrower).unwrap();
    assert_eq!(reopened.status, CreditStatus::Active);
    assert_eq!(reopened.credit_limit, 2_000);
    assert_eq!(client.get_credit_line_count(), original_count);
    assert_eq!(credit_line_id_for(&client, &borrower), original_id);
}

#[test]
fn reopen_suspended_line_with_debt_carries_forward() {
    let env = Env::default();
    let (client, _admin) = setup(&env);

    let borrower = Address::generate(&env);
    
    // Open a line
    client.open_credit_line(&borrower, &1_000_i128, &350_u32, &60_u32);
    
    // We mock the storage directly for the test to simulate debt
    let mut line = env.as_contract(&client.address, || {
        creditra_credit::storage::get_credit_line(&env, &borrower).unwrap()
    });
    
    line.utilized_amount = 500;
    line.accrued_interest = 50;
    line.status = CreditStatus::Suspended;
    line.last_accrual_ts = env.ledger().timestamp();
    
    env.as_contract(&client.address, || {
        // manually set TotalUtilized so we can test that it remains unchanged
        creditra_credit::storage::adjust_total_utilized(&env, 0, 500);
        creditra_credit::storage::persist_credit_line(
            &env,
            &borrower,
            &line,
            0,
            Some(CreditStatus::Active)
        );
    });

    let total_before = client.get_total_utilized();
    assert_eq!(total_before, 500);

    // Reopen the line
    client.open_credit_line(&borrower, &2_000_i128, &425_u32, &70_u32);

    let reopened = client.get_credit_line(&borrower).unwrap();
    assert_eq!(reopened.status, CreditStatus::Active);
    assert_eq!(reopened.utilized_amount, 500);
    assert_eq!(reopened.accrued_interest, 50);
    assert_eq!(client.get_total_utilized(), 500, "total utilized should be unchanged");
}
