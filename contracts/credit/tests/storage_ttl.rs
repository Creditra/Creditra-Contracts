// SPDX-License-Identifier: MIT

//! TTL bump regression tests for persistent per-borrower state.
//!
//! The credit contract stores live per-borrower records in persistent storage.
//! These entries must have their TTL extended on frequently-invoked read/write
//! paths so that active credit lines are not silently archived by the network.

use creditra_credit::storage::{DataKey, LEDGER_BUMP_AMOUNT, LEDGER_BUMP_THRESHOLD};
use creditra_credit::types::{CreditLineData, CreditStatus, GracePeriodConfig, GraceWaiverMode};
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::storage::{Instance as _, Persistent as _};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env, Symbol};

fn setup(env: &Env) -> (Address, CreditClient, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    (contract_id, client, admin)
}

fn advance_ledgers(env: &Env, delta: u32) {
    env.ledger().with_mut(|li| {
        li.sequence_number = li.sequence_number.saturating_add(delta);
    });
}

fn ttl_for_key<K: soroban_sdk::IntoVal<Env, soroban_sdk::Val>>(
    env: &Env,
    contract_id: &Address,
    key: &K,
) -> u32 {
    env.as_contract(contract_id, || env.storage().persistent().get_ttl(key))
}

fn instance_ttl_for_key<K: soroban_sdk::IntoVal<Env, soroban_sdk::Val>>(
    env: &Env,
    contract_id: &Address,
    key: &K,
) -> u32 {
    env.as_contract(contract_id, || env.storage().instance().get_ttl(key))
}

#[test]
fn credit_line_getter_bumps_persistent_ttl() {
    let env = Env::default();
    let (contract_id, client, _admin) = setup(&env);

    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);

    let ttl_initial = ttl_for_key(&env, &contract_id, &borrower);

    // Move just below bump threshold to force the bump to execute.
    let target_remaining = LEDGER_BUMP_THRESHOLD.saturating_sub(1);
    let delta = ttl_initial.saturating_sub(target_remaining);
    advance_ledgers(&env, delta);

    // Read path must bump (and also keep instance storage alive).
    let _ = client.get_credit_line(&borrower).unwrap();

    let ttl_after = ttl_for_key(&env, &contract_id, &borrower);
    assert!(
        ttl_after >= LEDGER_BUMP_AMOUNT,
        "expected TTL to be extended; initial={ttl_initial} after={ttl_after}"
    );
}

#[test]
fn utilization_cap_and_last_draw_keys_bump_persistent_ttl() {
    let env = Env::default();
    let (contract_id, client, admin) = setup(&env);

    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);

    // Set utilization cap (writes persistent key and bumps).
    client.set_utilization_cap(&borrower, &8_000_u32);
    let cap_key = DataKey::UtilizationCapBps(borrower.clone());
    let cap_ttl_initial = ttl_for_key(&env, &contract_id, &cap_key);

    // Advance close to bump threshold, then read via getter which must bump.
    let target_remaining = LEDGER_BUMP_THRESHOLD.saturating_sub(1);
    let delta = cap_ttl_initial.saturating_sub(target_remaining);
    advance_ledgers(&env, delta);
    let _ = client.get_utilization_cap(&borrower);

    let cap_ttl_after = ttl_for_key(&env, &contract_id, &cap_key);
    assert!(
        cap_ttl_after >= LEDGER_BUMP_AMOUNT,
        "cap TTL not extended; initial={cap_ttl_initial} after={cap_ttl_after}"
    );

    // LastDrawTs is bumped on write/read in draw_credit; to avoid requiring token setup,
    // write the key directly as the contract then call a read path.
    let last_draw_key = DataKey::LastDrawTs(borrower.clone());
    env.as_contract(&contract_id, || {
        env.storage().persistent().set(&last_draw_key, &1234_u64);
    });

    let ld_ttl_initial = ttl_for_key(&env, &contract_id, &last_draw_key);
    let delta = ld_ttl_initial.saturating_sub(target_remaining);
    advance_ledgers(&env, delta);

    // Call a path that reads LastDrawTs (draw_credit cooldown check requires borrower auth).
    // We keep it simple: use contract-internal getter via as_contract and expect bump helper
    // to be exercised indirectly by storage accessor.
    env.as_contract(&contract_id, || {
        let _ = creditra_credit::storage::get_last_draw_ts(&env, &borrower);
    });

    let ld_ttl_after = ttl_for_key(&env, &contract_id, &last_draw_key);
    assert!(
        ld_ttl_after >= LEDGER_BUMP_AMOUNT,
        "last_draw TTL not extended; initial={ld_ttl_initial} after={ld_ttl_after}"
    );

    let _ = admin;
}

#[test]
fn set_repayment_schedule_bumps_schedule_and_credit_line_ttl() {
    let env = Env::default();
    let (contract_id, client, _admin) = setup(&env);

    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);

    // Drain the credit-line TTL to just below the refresh threshold so the
    // next interaction must perform a real bump.
    let line_ttl_initial = ttl_for_key(&env, &contract_id, &borrower);
    let target_remaining = LEDGER_BUMP_THRESHOLD.saturating_sub(1);
    advance_ledgers(&env, line_ttl_initial.saturating_sub(target_remaining));

    // Setting a schedule is a credit-line interaction: it must bump both the
    // credit-line entry and the schedule entry.
    client.set_repayment_schedule(&borrower, &100_i128, &86_400_u64, &1_000_u64);

    let line_ttl_after = ttl_for_key(&env, &contract_id, &borrower);
    assert!(
        line_ttl_after >= LEDGER_BUMP_AMOUNT,
        "credit-line TTL not bumped on set_repayment_schedule: {line_ttl_after}"
    );

    let schedule_key = DataKey::RepaymentSchedule(borrower.clone());
    let schedule_ttl = ttl_for_key(&env, &contract_id, &schedule_key);
    assert!(
        schedule_ttl >= LEDGER_BUMP_AMOUNT,
        "schedule TTL not extended on write: {schedule_ttl}"
    );
}

#[test]
fn get_repayment_schedule_bumps_schedule_ttl() {
    let env = Env::default();
    let (contract_id, client, _admin) = setup(&env);

    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);
    client.set_repayment_schedule(&borrower, &100_i128, &86_400_u64, &1_000_u64);

    let schedule_key = DataKey::RepaymentSchedule(borrower.clone());
    let schedule_ttl_initial = ttl_for_key(&env, &contract_id, &schedule_key);

    // Advance to just below the refresh threshold, then read via the getter.
    let target_remaining = LEDGER_BUMP_THRESHOLD.saturating_sub(1);
    advance_ledgers(&env, schedule_ttl_initial.saturating_sub(target_remaining));

    let schedule = client.get_repayment_schedule(&borrower);
    assert!(schedule.is_some(), "schedule should exist");

    let schedule_ttl_after = ttl_for_key(&env, &contract_id, &schedule_key);
    assert!(
        schedule_ttl_after >= LEDGER_BUMP_AMOUNT,
        "schedule TTL not bumped on read: initial={schedule_ttl_initial} after={schedule_ttl_after}"
    );
}

#[test]
fn accrual_path_bumps_instance_ttl_for_accrual_reads() {
    let env = Env::default();
    let (contract_id, client, _admin) = setup(&env);

    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);
    client.set_penalty_surcharge_bps(&100_u32);

    let grace_cfg = GracePeriodConfig {
        grace_period_seconds: 60,
        waiver_mode: GraceWaiverMode::FullWaiver,
        reduced_rate_bps: 0,
    };
    env.as_contract(&contract_id, || {
        env.storage().instance().set(&creditra_credit::storage::grace_period_key(&env), &grace_cfg);
    });

    let grace_key = creditra_credit::storage::grace_period_key(&env);
    let initial_ttl = instance_ttl_for_key(&env, &contract_id, &grace_key);
    let target_remaining = LEDGER_BUMP_THRESHOLD.saturating_sub(1);
    let delta = initial_ttl.saturating_sub(target_remaining);
    advance_ledgers(&env, delta);

    env.as_contract(&contract_id, || {
        let line = CreditLineData {
            borrower: borrower.clone(),
            credit_limit: 1_000,
            utilized_amount: 100,
            interest_rate_bps: 300,
            risk_score: 70,
            status: CreditStatus::Active,
            last_rate_update_ts: 0,
            accrued_interest: 0,
            last_accrual_ts: 0,
            suspension_ts: 0,
        };
        env.storage().persistent().set(&borrower, &line);
    });

    env.ledger().set_timestamp(100);
    client.update_risk_parameters(&borrower, &1_000_i128, &300_u32, &70_u32);

    let ttl_after = instance_ttl_for_key(&env, &contract_id, &grace_key);
    assert!(
        ttl_after >= LEDGER_BUMP_AMOUNT,
        "instance TTL not extended for accrual reads: initial={initial_ttl} after={ttl_after}"
    );
}

// ── Lifecycle entrypoints that load a credit line ahead of `apply_accrual` ──
//
// Each of these previously read the credit line via a raw
// `env.storage().persistent().get(&borrower)` call, bypassing
// `storage::get_credit_line`'s TTL bump. A borrower who only ever interacts
// through one of these paths (e.g. is suspended, defaulted, or reinstated but
// never draws or repays) would have their entry silently drift toward
// archival. These regression tests drain the entry's TTL below the refresh
// threshold and assert that invoking each path bumps it back up.

/// Drain `borrower`'s credit-line TTL down to just below the refresh
/// threshold so the next accrual read must perform a real bump.
fn drain_credit_line_ttl(env: &Env, contract_id: &Address, borrower: &Address) {
    let ttl_initial = ttl_for_key(env, contract_id, borrower);
    let target_remaining = LEDGER_BUMP_THRESHOLD.saturating_sub(1);
    advance_ledgers(env, ttl_initial.saturating_sub(target_remaining));
}

#[test]
fn self_suspend_bumps_credit_line_ttl_on_accrual_read() {
    let env = Env::default();
    let (contract_id, client, _admin) = setup(&env);

    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);

    drain_credit_line_ttl(&env, &contract_id, &borrower);

    client.self_suspend_credit_line(&borrower);

    let ttl_after = ttl_for_key(&env, &contract_id, &borrower);
    assert!(
        ttl_after >= LEDGER_BUMP_AMOUNT,
        "credit-line TTL not bumped on self_suspend_credit_line: {ttl_after}"
    );
}

#[test]
fn default_credit_line_bumps_credit_line_ttl_on_accrual_read() {
    let env = Env::default();
    let (contract_id, client, _admin) = setup(&env);

    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);

    drain_credit_line_ttl(&env, &contract_id, &borrower);

    client.default_credit_line(&borrower);

    let ttl_after = ttl_for_key(&env, &contract_id, &borrower);
    assert!(
        ttl_after >= LEDGER_BUMP_AMOUNT,
        "credit-line TTL not bumped on default_credit_line: {ttl_after}"
    );
}

#[test]
fn reinstate_credit_line_bumps_credit_line_ttl_on_accrual_read() {
    let env = Env::default();
    let (contract_id, client, _admin) = setup(&env);

    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);
    client.default_credit_line(&borrower);

    drain_credit_line_ttl(&env, &contract_id, &borrower);

    client.reinstate_credit_line(&borrower, &CreditStatus::Active);

    let ttl_after = ttl_for_key(&env, &contract_id, &borrower);
    assert!(
        ttl_after >= LEDGER_BUMP_AMOUNT,
        "credit-line TTL not bumped on reinstate_credit_line: {ttl_after}"
    );
}

#[test]
fn settle_default_liquidation_bumps_credit_line_ttl_on_accrual_read() {
    let env = Env::default();
    let (contract_id, client, _admin) = setup(&env);

    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);
    client.default_credit_line(&borrower);

    drain_credit_line_ttl(&env, &contract_id, &borrower);

    let settlement_id = Symbol::new(&env, "settle1");
    client.settle_default_liquidation(&borrower, &500_i128, &settlement_id, &10_000_u32, &None);

    let ttl_after = ttl_for_key(&env, &contract_id, &borrower);
    assert!(
        ttl_after >= LEDGER_BUMP_AMOUNT,
        "credit-line TTL not bumped on settle_default_liquidation: {ttl_after}"
    );
}

#[test]
fn reverse_draw_bumps_credit_line_ttl_on_accrual_read() {
    let env = Env::default();
    let (contract_id, client, _admin) = setup(&env);

    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);

    // Record a draw-audit entry directly (bypassing token transfer machinery,
    // which is irrelevant to this TTL regression test) so `reverse_draw` can
    // find an original draw to reverse.
    let original_ts = env.ledger().timestamp();
    env.as_contract(&contract_id, || {
        env.storage()
            .persistent()
            .set(&DataKey::DrawAudit(borrower.clone(), original_ts), &200_i128);
    });

    drain_credit_line_ttl(&env, &contract_id, &borrower);

    client.reverse_draw(&borrower, &100_i128, &original_ts, &1_u32);

    let ttl_after = ttl_for_key(&env, &contract_id, &borrower);
    assert!(
        ttl_after >= LEDGER_BUMP_AMOUNT,
        "credit-line TTL not bumped on reverse_draw: {ttl_after}"
    );
}
