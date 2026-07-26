// SPDX-License-Identifier: MIT

//! TTL bump regression tests for persistent per-borrower state.
//!
//! The credit contract stores live per-borrower records in persistent storage.
//! These entries must have their TTL extended on frequently-invoked read/write
//! paths so that active credit lines are not silently archived by the network.

use creditra_credit::storage::{
    DataKey, CREDIT_LINE_TTL_EXTEND_TO, CREDIT_LINE_TTL_THRESHOLD, LEDGER_BUMP_AMOUNT,
    LEDGER_BUMP_THRESHOLD,
};
use creditra_credit::types::{CreditLineData, CreditStatus, GracePeriodConfig, GraceWaiverMode};
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::storage::{Instance as _, Persistent as _};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env};

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

#[test]
fn already_closed_credit_line_read_bumps_ttl() {
    let env = Env::default();
    let (contract_id, client, admin) = setup(&env);

    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);
    client.close_credit_line(&borrower, &admin);

    let initial_ttl = ttl_for_key(&env, &contract_id, &borrower);
    let target_remaining = CREDIT_LINE_TTL_THRESHOLD.saturating_sub(1);
    advance_ledgers(&env, initial_ttl.saturating_sub(target_remaining));

    // The already-closed path returns early, but its storage read must still
    // refresh the persistent credit-line TTL.
    client.close_credit_line(&borrower, &admin);

    let ttl_after = ttl_for_key(&env, &contract_id, &borrower);
    assert!(
        ttl_after >= CREDIT_LINE_TTL_EXTEND_TO,
        "credit-line TTL not bumped on idempotent close: initial={initial_ttl} after={ttl_after}"
    );
}
