// SPDX-License-Identifier: MIT

use std::panic::{catch_unwind, AssertUnwindSafe};

use creditra_credit::events::InterestAccruedEvent;
use creditra_credit::types::CreditStatus;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::testutils::Events as _;
use soroban_sdk::{token::StellarAssetClient, Address, Env, Symbol, TryFromVal, TryIntoVal, Vec};

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

// ── Keeper batch skipping and entry limit (Issue #1332) ──────────────────────

/// Deploy a contract that can actually settle draws: a funded liquidity token
/// plus an explicitly disabled collateral floor (these tests only exercise the
/// keeper batch, not the ratio guard). Returns the admin for `close_credit_line`.
fn setup_keeper_env(env: &Env) -> (Address, CreditClient<'_>) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    client.set_min_collateral_ratio_bps(&0);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&contract_id);
    StellarAssetClient::new(env, &token).mint(&contract_id, &1_000_000_000_i128);

    (admin, client)
}

/// Exactly `ACCRUE_BATCH_MAX` (50) borrowers is accepted; every address is
/// unknown, so the batch is a no-op rather than a revert.
#[test]
fn accrue_batch_accepts_exactly_fifty_borrowers() {
    let env = Env::default();
    let (_admin, client) = setup_keeper_env(&env);

    let mut borrowers = Vec::new(&env);
    for _ in 0..50 {
        borrowers.push_back(Address::generate(&env));
    }

    client.accrue_batch(&borrowers);
}

/// One borrower over the cap must revert before any line is touched.
#[test]
#[should_panic(expected = "Error(Contract, #5)")] // InvalidAmount (cap guard)
fn accrue_batch_rejects_fifty_one_borrowers() {
    let env = Env::default();
    let (_admin, client) = setup_keeper_env(&env);

    let mut borrowers = Vec::new(&env);
    for _ in 0..51 {
        borrowers.push_back(Address::generate(&env));
    }

    client.accrue_batch(&borrowers);
}

/// A batch mixing unknown, zero-debt, Suspended and Closed lines must accrue
/// only the Active line that carries debt — and emit exactly one event.
#[test]
fn accrue_batch_accrues_only_active_lines_with_debt() {
    let env = Env::default();
    let (admin, client) = setup_keeper_env(&env);

    let active = Address::generate(&env);
    let zero_debt = Address::generate(&env);
    let suspended = Address::generate(&env);
    let closed = Address::generate(&env);
    let missing = Address::generate(&env);

    for borrower in [&active, &zero_debt, &suspended, &closed] {
        client.open_credit_line(borrower, &1_000_000_i128, &1_000_u32, &50_u32);
    }

    env.ledger().set_timestamp(1);
    client.draw_credit(&active, &100_000_i128);
    client.draw_credit(&suspended, &100_000_i128);
    client.draw_credit(&closed, &100_000_i128);

    client.suspend_credit_line(&suspended);
    client.close_credit_line(&closed, &admin);

    env.ledger().set_timestamp(1 + 31_536_000);

    let mut borrowers = Vec::new(&env);
    borrowers.push_back(missing.clone());
    borrowers.push_back(zero_debt.clone());
    borrowers.push_back(suspended.clone());
    borrowers.push_back(closed.clone());
    borrowers.push_back(active.clone());

    let before_events = env.events().all().len();
    client.accrue_batch(&borrowers);

    // The eligible line accrued one year of interest at 1_000 bps.
    let active_line = client.get_credit_line(&active).unwrap();
    assert_eq!(active_line.last_accrual_ts, 1 + 31_536_000);
    assert_eq!(active_line.accrued_interest, 10_000);
    assert_eq!(active_line.utilized_amount, 110_000);

    // Every skipped line is untouched.
    let zero_debt_line = client.get_credit_line(&zero_debt).unwrap();
    assert_eq!(zero_debt_line.status, CreditStatus::Active);
    assert_eq!(zero_debt_line.accrued_interest, 0);

    let suspended_line = client.get_credit_line(&suspended).unwrap();
    assert_eq!(suspended_line.status, CreditStatus::Suspended);
    assert_eq!(suspended_line.last_accrual_ts, 1);
    assert_eq!(suspended_line.accrued_interest, 0);

    let closed_line = client.get_credit_line(&closed).unwrap();
    assert_eq!(closed_line.status, CreditStatus::Closed);
    assert_eq!(closed_line.last_accrual_ts, 1);

    assert!(client.get_credit_line(&missing).is_none());

    // Exactly one event: the zero-accrual lines stayed quiet.
    assert_eq!(env.events().all().len(), before_events + 1);
    let event = last_accrue_event(&env);
    assert_eq!(event.borrower, active);
    assert_eq!(event.accrued_amount, 10_000);
    assert_eq!(event.new_utilized_amount, 110_000);
}

/// A batch that produces no interest emits no event at all.
#[test]
fn accrue_batch_emits_no_event_for_zero_accrual() {
    let env = Env::default();
    let (_admin, client) = setup_keeper_env(&env);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &1_000_000_i128, &1_000_u32, &50_u32);
    env.ledger().set_timestamp(1);
    client.draw_credit(&borrower, &100_000_i128);

    let before_events = env.events().all().len();

    let mut borrowers = Vec::new(&env);
    borrowers.push_back(borrower.clone());
    client.accrue_batch(&borrowers);

    assert_eq!(env.events().all().len(), before_events);
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.last_accrual_ts, 1);
    assert_eq!(line.accrued_interest, 0);
}

/// The keeper hook is pause-gated: a paused protocol rejects the whole batch.
#[test]
#[should_panic(expected = "Error(Contract, #18)")] // Paused
fn accrue_batch_reverts_when_paused() {
    let env = Env::default();
    let (_admin, client) = setup_keeper_env(&env);
    client.set_protocol_paused(&true);

    let mut borrowers = Vec::new(&env);
    borrowers.push_back(Address::generate(&env));
    client.accrue_batch(&borrowers);
}
