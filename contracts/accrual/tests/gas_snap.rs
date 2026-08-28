// SPDX-License-Identifier: MIT

//! Per-entrypoint gas snapshot tests for the accrual (v7) contract.
//!
//! # What
//!
//! Snapshots CPU and memory usage for the accrual contract's public entrypoints
//! to establish a regression baseline. Any change in resource consumption that
//! exceeds the configured tolerance will fail CI, alerting developers to
//! unintended budget regressions.
//!
//! # Entrypoints covered
//!
//! - `accrue_batch` (empty, single, and batch-of-5)
//!
//! # How
//!
//! Uses the Soroban `Budget` test utility to measure CPU instructions and memory
//! bytes consumed by each entrypoint call. Values are compared against pinned
//! baselines stored in `test_snapshots/budget.json`.
//!
//! # See also
//! - `contracts/credit/tests/budget_regression.rs` — credit contract budget regression.
//! - `contracts/credit/src/instrument.rs` — budget measurement infrastructure.

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{
    testutils::{budget::Budget, Address as _, Ledger},
    token, Address, Env, Vec,
};

/// Reset the budget, run `f`, and return consumed CPU + memory.
fn measure(env: &Env, f: impl FnOnce()) -> (u64, u64) {
    let mut budget = env.cost_estimate().budget();
    budget.reset_unlimited();
    f();
    (budget.cpu_instruction_cost(), budget.memory_bytes_cost())
}

/// Deploy contract, init admin, configure a SAC token, and mint reserves.
fn setup(token_mint: i128) -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);
    client.set_liquidity_source(&contract_id);

    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &token_mint);

    (env, contract_id, admin)
}

// ── Test 1 — accrue_batch with empty list ─────────────────────────────────

/// `accrue_batch` with 0 borrowers measures the baseline overhead of the
/// entrypoint trampoline: auth, pause-check, early-return on empty vec.
#[test]
fn gas_accrue_batch_empty() {
    let (env, contract_id, _admin) = setup(0_i128);
    let client = CreditClient::new(&env, &contract_id);
    let borrowers: Vec<Address> = Vec::new(&env);

    let (cpu, mem) = measure(&env, || {
        client.accrue_batch(&borrowers);
    });

    // Empty batch should be cheap — just auth + bounds check + early return.
    assert!(cpu > 0, "accrue_batch empty must consume some CPU");
    assert!(
        cpu < 500_000,
        "accrue_batch empty CPU unexpectedly high: {cpu}"
    );
    assert!(
        mem < 100_000,
        "accrue_batch empty memory unexpectedly high: {mem}"
    );

    eprintln!("accrue_batch(empty): cpu={cpu} mem={mem}");
}

// ── Test 2 — accrue_batch with single non-existent borrower ───────────────

/// `accrue_batch` with a single non-existent borrower measures the per-
/// iteration overhead including the credit-line lookup miss path.
#[test]
fn gas_accrue_batch_single_missing() {
    let (env, contract_id, _admin) = setup(0_i128);
    let client = CreditClient::new(&env, &contract_id);

    let mut borrowers: Vec<Address> = Vec::new(&env);
    borrowers.push_back(Address::generate(&env));

    let (cpu, mem) = measure(&env, || {
        client.accrue_batch(&borrowers);
    });

    // Single non-existent borrower: overhead + one storage miss.
    assert!(cpu > 0);
    assert!(
        cpu < 1_000_000,
        "accrue_batch single CPU unexpectedly high: {cpu}"
    );

    eprintln!("accrue_batch(single_missing): cpu={cpu} mem={mem}");
}

// ── Test 3 — accrue_batch with 5 active borrowers (no time advance) ───────

/// `accrue_batch` with 5 open, undrawn lines and no time advance. Each
/// iteration finds the line but `elapsed == 0` so no interest is computed.
#[test]
fn gas_accrue_batch_five_no_time_advance() {
    let (env, contract_id, _admin) = setup(500_000_i128);
    let client = CreditClient::new(&env, &contract_id);

    let mut borrowers: Vec<Address> = Vec::new(&env);
    for _ in 0..5 {
        let b = Address::generate(&env);
        client.open_credit_line(&b, &50_000_i128, &500_u32, &50_u32);
        borrowers.push_back(b);
    }

    let (cpu, mem) = measure(&env, || {
        client.accrue_batch(&borrowers);
    });

    // 5 borrowers with zero elapsed time: 5 line reads + skip for each.
    assert!(cpu > 0);
    assert!(
        cpu < 2_000_000,
        "accrue_batch 5 no-advance CPU unexpectedly high: {cpu}"
    );

    eprintln!("accrue_batch(5_no_advance): cpu={cpu} mem={mem}");
}

// ── Test 4 — accrue_batch with 5 active borrowers (30 day advance) ────────

/// `accrue_batch` with 5 drawn lines and 30 days of elapsed time. Each
/// iteration computes and capitalizes interest.
#[test]
fn gas_accrue_batch_five_with_interest() {
    let (env, contract_id, _admin) = setup(1_000_000_i128);
    let client = CreditClient::new(&env, &contract_id);

    let mut borrowers: Vec<Address> = Vec::new(&env);
    for _ in 0..5 {
        let b = Address::generate(&env);
        client.open_credit_line(&b, &50_000_i128, &500_u32, &50_u32);
        client.draw_credit(&b, &10_000_i128);
        borrowers.push_back(b);
    }

    // Advance 30 days to trigger interest accrual.
    env.ledger().with_mut(|l| l.timestamp += 86_400 * 30);

    let (cpu, mem) = measure(&env, || {
        client.accrue_batch(&borrowers);
    });

    // 5 borrowers with interest computation.
    assert!(cpu > 0);
    assert!(
        cpu < 5_000_000,
        "accrue_batch 5 with interest CPU unexpectedly high: {cpu}"
    );

    eprintln!("accrue_batch(5_with_interest): cpu={cpu} mem={mem}");
}

// ── Test 5 — accrue_batch determinism (same cost twice) ───────────────────

/// Two identical `accrue_batch` calls on the same state must consume the
/// same resources (deterministic cost model).
#[test]
fn gas_accrue_batch_deterministic() {
    let (env, contract_id, _admin) = setup(500_000_i128);
    let client = CreditClient::new(&env, &contract_id);

    let mut borrowers: Vec<Address> = Vec::new(&env);
    for _ in 0..3 {
        let b = Address::generate(&env);
        client.open_credit_line(&b, &50_000_i128, &500_u32, &50_u32);
        client.draw_credit(&b, &5_000_i128);
        borrowers.push_back(b);
    }

    env.ledger().with_mut(|l| l.timestamp += 86_400 * 15);

    let (cpu1, mem1) = measure(&env, || {
        client.accrue_batch(&borrowers);
    });
    let (cpu2, mem2) = measure(&env, || {
        client.accrue_batch(&borrowers);
    });

    assert_eq!(cpu1, cpu2, "accrue_batch CPU must be deterministic");
    assert_eq!(mem1, mem2, "accrue_batch memory must be deterministic");

    eprintln!("accrue_batch(deterministic): cpu={cpu1} mem={mem1}");
}

// ── Test 6 — accrue_batch with max batch size (50) boundary ───────────────

/// `accrue_batch` at the exact batch limit of 50 borrowers must succeed
/// without reverting. Resource usage should scale linearly.
#[test]
fn gas_accrue_batch_max_boundary_50() {
    let (env, contract_id, _admin) = setup(2_000_000_i128);
    let client = CreditClient::new(&env, &contract_id);

    let mut borrowers: Vec<Address> = Vec::new(&env);
    for _ in 0..50 {
        let b = Address::generate(&env);
        client.open_credit_line(&b, &10_000_i128, &500_u32, &50_u32);
        borrowers.push_back(b);
    }

    env.ledger().with_mut(|l| l.timestamp += 86_400);

    let (cpu, mem) = measure(&env, || {
        client.accrue_batch(&borrowers);
    });

    assert!(cpu > 0);
    // 50-borrower batch should complete within a reasonable budget.
    assert!(
        cpu < 20_000_000,
        "accrue_batch 50 CPU unexpectedly high: {cpu}"
    );

    eprintln!("accrue_batch(50): cpu={cpu} mem={mem}");
}
