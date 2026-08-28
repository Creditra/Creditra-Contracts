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

fn assert_last_event_topic(env: &Env, expected: Symbol) {
    let events = env.events().all();
    let (_contract_id, topics, _data) = events.last().unwrap();

    let namespace = Symbol::try_from_val(env, &topics.get(0).unwrap()).unwrap();
    let event_name = Symbol::try_from_val(env, &topics.get(1).unwrap()).unwrap();

    assert_eq!(namespace, symbol_short!("credit"));
    assert_eq!(event_name, expected);
}

fn find_borrower_index(client: &CreditClient<'_>, target: &Address) -> usize {
    let mut cursor = 0_u32;
    let mut idx = 0_usize;
    loop {
        let (lines, next_cursor) = client.enumerate_credit_lines(&cursor, &10, &false);
        if lines.is_empty() {
            break;
        }
        for i in 0..lines.len() {
            let line = lines.get(i).unwrap();
            if line.borrower == *target {
                return idx;
            }
            idx += 1;
        }
        match next_cursor {
            Some(c) => cursor = c,
            None => break,
        }
    }
    panic!("borrower not found");
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

    let original_count = client.get_credit_line_count();
    assert_eq!(original_count, 2);
    // The borrower is the second line opened → index 1
    assert_eq!(find_borrower_index(&client, &borrower), 1);

    client.close_credit_line(&borrower, &admin);
    assert_last_event_topic(&env, symbol_short!("closed"));
    assert_eq!(client.get_credit_line_count(), original_count);

    client.open_credit_line(&borrower, &2_000_i128, &425_u32, &70_u32);
    assert_last_event_topic(&env, symbol_short!("opened"));

    let reopened = client.get_credit_line(&borrower).unwrap();
    assert_eq!(reopened.status, CreditStatus::Active);
    assert_eq!(reopened.credit_limit, 2_000);
    // Count unchanged → ID reused (no new ID allocated)
    assert_eq!(client.get_credit_line_count(), original_count);
    // Borrower is still at index 1 in enumeration
    assert_eq!(find_borrower_index(&client, &borrower), 1);
}
