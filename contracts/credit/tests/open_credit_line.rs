// SPDX-License-Identifier: MIT

use creditra_credit::types::CreditStatus;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Events};
use soroban_sdk::{symbol_short, Address, Env, TryFromVal};

fn setup(env: &Env) -> (CreditClient, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    (client, admin)
}

// ── valid open ────────────────────────────────────────────────────────────────

#[test]
fn open_stores_correct_fields() {
    let env = Env::default();
    let (client, _) = setup(&env);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &5_000_i128, &300_u32, &50_u32);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.borrower, borrower);
    assert_eq!(line.credit_limit, 5_000);
    assert_eq!(line.utilized_amount, 0);
    assert_eq!(line.interest_rate_bps, 300);
    assert_eq!(line.risk_score, 50);
    assert_eq!(line.status, CreditStatus::Active);
    assert_eq!(line.last_rate_update_ts, 0);
    assert_eq!(line.accrued_interest, 0);
    assert_eq!(line.suspension_ts, 0);
}

#[test]
fn open_at_max_rate_and_score_succeeds() {
    let env = Env::default();
    let (client, _) = setup(&env);
    let borrower = Address::generate(&env);

    // MAX_INTEREST_RATE_BPS = 10_000, MAX_RISK_SCORE = 100
    client.open_credit_line(&borrower, &1_i128, &10_000_u32, &100_u32);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.interest_rate_bps, 10_000);
    assert_eq!(line.risk_score, 100);
    assert_eq!(line.status, CreditStatus::Active);
}

// ── event payload ─────────────────────────────────────────────────────────────

#[test]
fn open_emits_opened_event_with_correct_topics() {
    let env = Env::default();
    let (client, _) = setup(&env);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &2_000_i128, &500_u32, &60_u32);

    let events = env.events().all();
    let ev = events.last().unwrap();
    let topics = ev.1;

    let t0 = soroban_sdk::Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap();
    let t1 = soroban_sdk::Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap();
    assert_eq!(t0, symbol_short!("credit"));
    assert_eq!(t1, symbol_short!("opened"));
}

// ── invalid parameters ────────────────────────────────────────────────────────

#[test]
#[should_panic(expected = "credit_limit must be greater than zero")]
fn open_rejects_zero_credit_limit() {
    let env = Env::default();
    let (client, _) = setup(&env);
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &0_i128, &300_u32, &50_u32);
}

#[test]
#[should_panic(expected = "credit_limit must be greater than zero")]
fn open_rejects_negative_credit_limit() {
    let env = Env::default();
    let (client, _) = setup(&env);
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &-1_i128, &300_u32, &50_u32);
}

#[test]
fn open_rejects_rate_above_max() {
    let env = Env::default();
    let (client, _) = setup(&env);
    let borrower = Address::generate(&env);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.open_credit_line(&borrower, &1_000_i128, &10_001_u32, &50_u32);
    }));
    assert!(result.is_err());
    // No line should have been stored
    assert!(client.get_credit_line(&borrower).is_none());
}

#[test]
fn open_rejects_score_above_max() {
    let env = Env::default();
    let (client, _) = setup(&env);
    let borrower = Address::generate(&env);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.open_credit_line(&borrower, &1_000_i128, &300_u32, &101_u32);
    }));
    assert!(result.is_err());
    assert!(client.get_credit_line(&borrower).is_none());
}

// ── duplicate / reopen policy ─────────────────────────────────────────────────

#[test]
#[should_panic(expected = "borrower already has an active credit line")]
fn open_rejects_duplicate_active_line() {
    let env = Env::default();
    let (client, _) = setup(&env);
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &50_u32);
    client.open_credit_line(&borrower, &2_000_i128, &400_u32, &60_u32);
}

#[test]
fn open_allows_reopen_after_close_and_resets_fields() {
    let env = Env::default();
    let (client, admin) = setup(&env);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &50_u32);
    client.close_credit_line(&borrower, &admin);

    client.open_credit_line(&borrower, &3_000_i128, &200_u32, &40_u32);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Active);
    assert_eq!(line.credit_limit, 3_000);
    assert_eq!(line.utilized_amount, 0);
    assert_eq!(line.last_rate_update_ts, 0);
    assert_eq!(line.accrued_interest, 0);
}

#[test]
fn open_allows_reopen_after_default() {
    let env = Env::default();
    let (client, _) = setup(&env);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &50_u32);
    client.default_credit_line(&borrower);

    client.open_credit_line(&borrower, &1_500_i128, &350_u32, &65_u32);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Active);
    assert_eq!(line.credit_limit, 1_500);
}

#[test]
fn open_allows_reopen_after_suspend() {
    let env = Env::default();
    let (client, _) = setup(&env);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &50_u32);
    client.suspend_credit_line(&borrower);

    client.open_credit_line(&borrower, &2_000_i128, &400_u32, &70_u32);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Active);
    assert_eq!(line.credit_limit, 2_000);
    assert_eq!(line.suspension_ts, 0);
}
