// SPDX-License-Identifier: MIT

use std::panic::{catch_unwind, AssertUnwindSafe};

use creditra_credit::events::InterestAccruedEvent;
use creditra_credit::types::CreditStatus;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::testutils::Events as _;
use soroban_sdk::testutils::Ledger as _;
use soroban_sdk::{symbol_short, token, Address, Env, Symbol, TryFromVal, TryIntoVal, Vec};

fn setup_env() -> (Env, Address, CreditClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    let token_admin = Address::generate(&env);
    let token = env.register_stellar_asset_contract_v2(token_admin);
    client.set_liquidity_token(&token.address());
    token::StellarAssetClient::new(&env, &token.address()).mint(&contract_id, &1_000_000_000_i128);

    (env, admin, client)
}

fn last_accrue_event(env: &Env) -> InterestAccruedEvent {
    let all_events = env.events().all();
    let namespace = symbol_short!("credit");
    let kind = symbol_short!("accrue");

    for (_contract, topics, data) in all_events.iter().rev() {
        if topics.len() >= 2 {
            if let (Ok(t0), Ok(t1)) = (
                Symbol::try_from_val(env, &topics.get(0).unwrap()),
                Symbol::try_from_val(env, &topics.get(1).unwrap()),
            ) {
                if t0 == namespace && t1 == kind {
                    return data.try_into_val(env).unwrap();
                }
            }
        }
    }

    panic!("No accrue event found among {} events", all_events.len());
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

    assert!(
        result.is_err(),
        "accrue_batch must reject oversized batches"
    );
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

    env.ledger().set_timestamp(1 + 31_557_600);

    let _ = env.events().all(); // Clear setup events

    let mut borrowers = Vec::new(&env);
    borrowers.push_back(active.clone());
    borrowers.push_back(suspended.clone());
    borrowers.push_back(missing.clone());

    client.accrue_batch(&borrowers);

    let event = last_accrue_event(&env);
    assert_eq!(event.borrower, active);
    assert_eq!(event.accrued_amount, 10_000);
    assert_eq!(event.new_utilized_amount, 110_000);

    let active_line = client.get_credit_line(&active).unwrap();
    assert_eq!(active_line.status, CreditStatus::Active);
    assert_eq!(active_line.last_accrual_ts, 1 + 31_557_600);
    assert_eq!(active_line.accrued_interest, 10_000);
    assert_eq!(active_line.utilized_amount, 110_000);

    let suspended_line = client.get_credit_line(&suspended).unwrap();
    assert_eq!(suspended_line.status, CreditStatus::Suspended);
    assert_eq!(suspended_line.last_accrual_ts, 1);
    assert_eq!(suspended_line.accrued_interest, 0);
    assert_eq!(suspended_line.utilized_amount, 100_000);

    assert!(client.get_credit_line(&missing).is_none());
}
