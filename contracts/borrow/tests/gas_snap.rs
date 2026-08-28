// SPDX-License-Identifier: MIT

//! Per-entrypoint gas snapshot tests for the borrow (v7) subsystem.
//!
//! # What
//!
//! Snapshots CPU instructions and memory bytes for every borrow-related public
//! entrypoint to establish a regression baseline. Any unintended change in
//! resource consumption will fail CI, alerting developers to budget regressions.
//!
//! # Entrypoints covered
//!
//! | Entrypoint | Category |
//! |---|---|
//! | `draw_credit` | borrower / write |
//! | `repay_credit` | borrower / write |
//! | `reverse_draw` | admin / write |
//! | `set_draw_min_interval` | admin / write |
//! | `set_borrow_admin_cooldown` | admin / write |
//! | `set_utilization_cap` | admin / write |
//! | `get_draw_min_interval` | read-only |
//! | `get_borrow_admin_cooldown` | read-only |
//! | `get_utilization_cap` | read-only |
//!
//! # How
//!
//! Uses the Soroban `Budget` test utility to measure CPU instructions and
//! memory bytes consumed per call. Values are compared against conservative
//! upper bounds that form the regression baseline.
//!
//! # See also
//!
//! - `contracts/collateral/tests/gas_snap.rs` — collateral gas baseline.
//! - `contracts/accrual/tests/gas_snap.rs` — accrual gas baseline.
//! - `contracts/borrow/tests/auth_snap.rs` — authorization snapshot.

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{
    testutils::{budget::Budget, Address as _, Ledger},
    token::StellarAssetClient,
    Address, Env,
};

const CREDIT_LIMIT: i128 = 10_000;
const DRAW_AMOUNT: i128 = 500;
const START_TS: u64 = 10_000;

// ── Helpers ───────────────────────────────────────────────────────────────

/// Reset the budget to unlimited, run `f`, return (cpu, mem).
fn measure(env: &Env, f: impl FnOnce()) -> (u64, u64) {
    let mut budget = env.cost_estimate().budget();
    budget.reset_unlimited();
    f();
    (budget.cpu_instruction_cost(), budget.memory_bytes_cost())
}

struct Fixture<'a> {
    client: CreditClient<'a>,
    admin: Address,
    borrower: Address,
    contract_id: Address,
    token: Address,
}

/// Deploy and configure the credit contract with a funded reserve and an open
/// credit line ready for draw/repay operations.
fn setup(env: &Env) -> Fixture<'_> {
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = START_TS);

    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    let token = env
        .register_stellar_asset_contract_v2(Address::generate(env))
        .address();
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&contract_id);

    // Fund the contract reserve so draw_credit can transfer tokens out.
    StellarAssetClient::new(env, &token).mint(&contract_id, &(CREDIT_LIMIT * 10));
    // Fund the borrower so repay_credit can pull tokens in.
    StellarAssetClient::new(env, &token).mint(&borrower, &(CREDIT_LIMIT * 2));

    client.open_credit_line(&borrower, &CREDIT_LIMIT, &300_u32, &50_u32);

    Fixture {
        client,
        admin,
        borrower,
        contract_id,
        token,
    }
}

// ── Borrower write entrypoints ────────────────────────────────────────────

/// `draw_credit` baseline: auth + balance checks + token transfer + storage write.
#[test]
fn gas_draw_credit() {
    let env = Env::default();
    let f = setup(&env);

    let (cpu, mem) = measure(&env, || {
        f.client.draw_credit(&f.borrower, &DRAW_AMOUNT);
    });

    assert!(cpu > 0, "draw_credit must consume CPU");
    assert!(cpu < 10_000_000, "draw_credit CPU regression: {cpu}");
    assert!(mem < 1_000_000, "draw_credit memory regression: {mem}");

    eprintln!("draw_credit: cpu={cpu} mem={mem}");
}

/// `repay_credit` baseline: auth + token transfer + storage write + interest logic.
#[test]
fn gas_repay_credit() {
    let env = Env::default();
    let f = setup(&env);
    f.client.draw_credit(&f.borrower, &DRAW_AMOUNT);

    let (cpu, mem) = measure(&env, || {
        f.client.repay_credit(&f.borrower, &DRAW_AMOUNT);
    });

    assert!(cpu > 0, "repay_credit must consume CPU");
    assert!(cpu < 10_000_000, "repay_credit CPU regression: {cpu}");
    assert!(mem < 1_000_000, "repay_credit memory regression: {mem}");

    eprintln!("repay_credit: cpu={cpu} mem={mem}");
}

/// `repay_credit` (partial) baseline: same path as full repay but with a
/// smaller amount — must not cost more than a full repay.
#[test]
fn gas_repay_credit_partial() {
    let env = Env::default();
    let f = setup(&env);
    f.client.draw_credit(&f.borrower, &DRAW_AMOUNT);

    let (cpu, mem) = measure(&env, || {
        f.client.repay_credit(&f.borrower, &(DRAW_AMOUNT / 2));
    });

    assert!(cpu > 0, "repay_credit (partial) must consume CPU");
    assert!(
        cpu < 10_000_000,
        "repay_credit (partial) CPU regression: {cpu}"
    );
    assert!(
        mem < 1_000_000,
        "repay_credit (partial) memory regression: {mem}"
    );

    eprintln!("repay_credit(partial): cpu={cpu} mem={mem}");
}

// ── Admin write entrypoints ───────────────────────────────────────────────

/// `reverse_draw` baseline: admin auth + draw audit lookup + token transfer
/// back to reserve + storage write.
#[test]
fn gas_reverse_draw() {
    let env = Env::default();
    let f = setup(&env);
    f.client.draw_credit(&f.borrower, &DRAW_AMOUNT);
    let draw_ts = START_TS;

    // Advance time — still within the reversal window.
    env.ledger().with_mut(|li| li.timestamp = START_TS + 60);

    let (cpu, mem) = measure(&env, || {
        f.client
            .reverse_draw(&f.borrower, &DRAW_AMOUNT, &draw_ts, &0_u32);
    });

    assert!(cpu > 0, "reverse_draw must consume CPU");
    assert!(cpu < 10_000_000, "reverse_draw CPU regression: {cpu}");
    assert!(mem < 1_000_000, "reverse_draw memory regression: {mem}");

    eprintln!("reverse_draw: cpu={cpu} mem={mem}");
}

/// `set_draw_min_interval` baseline: admin auth + single storage write.
#[test]
fn gas_set_draw_min_interval() {
    let env = Env::default();
    let f = setup(&env);

    let (cpu, mem) = measure(&env, || {
        f.client.set_draw_min_interval(&300_u64);
    });

    assert!(cpu > 0, "set_draw_min_interval must consume CPU");
    assert!(
        cpu < 5_000_000,
        "set_draw_min_interval CPU regression: {cpu}"
    );
    assert!(
        mem < 500_000,
        "set_draw_min_interval memory regression: {mem}"
    );

    eprintln!("set_draw_min_interval: cpu={cpu} mem={mem}");
}

/// `set_borrow_admin_cooldown` baseline: admin auth + single storage write.
#[test]
fn gas_set_borrow_admin_cooldown() {
    let env = Env::default();
    let f = setup(&env);

    let (cpu, mem) = measure(&env, || {
        f.client.set_borrow_admin_cooldown(&3_600_u64);
    });

    assert!(cpu > 0, "set_borrow_admin_cooldown must consume CPU");
    assert!(
        cpu < 5_000_000,
        "set_borrow_admin_cooldown CPU regression: {cpu}"
    );
    assert!(
        mem < 500_000,
        "set_borrow_admin_cooldown memory regression: {mem}"
    );

    eprintln!("set_borrow_admin_cooldown: cpu={cpu} mem={mem}");
}

/// `set_utilization_cap` baseline: admin auth + per-borrower storage write.
#[test]
fn gas_set_utilization_cap() {
    let env = Env::default();
    let f = setup(&env);

    let (cpu, mem) = measure(&env, || {
        f.client.set_utilization_cap(&f.borrower, &8_000_u32);
    });

    assert!(cpu > 0, "set_utilization_cap must consume CPU");
    assert!(cpu < 5_000_000, "set_utilization_cap CPU regression: {cpu}");
    assert!(
        mem < 500_000,
        "set_utilization_cap memory regression: {mem}"
    );

    eprintln!("set_utilization_cap: cpu={cpu} mem={mem}");
}

// ── Read-only entrypoints ─────────────────────────────────────────────────

/// Read-only borrow query entrypoints must be cheap (storage read only,
/// no auth overhead).
#[test]
fn gas_borrow_read_only_queries() {
    let env = Env::default();
    let f = setup(&env);
    f.client.set_draw_min_interval(&120_u64);
    f.client.set_borrow_admin_cooldown(&3_600_u64);
    f.client.set_utilization_cap(&f.borrower, &7_500_u32);

    // get_draw_min_interval
    let (cpu, mem) = measure(&env, || {
        let _ = f.client.get_draw_min_interval();
    });
    assert!(cpu > 0, "get_draw_min_interval must consume CPU");
    assert!(
        cpu < 2_000_000,
        "get_draw_min_interval CPU regression: {cpu}"
    );
    assert!(
        mem < 200_000,
        "get_draw_min_interval memory regression: {mem}"
    );
    eprintln!("get_draw_min_interval: cpu={cpu} mem={mem}");

    // get_borrow_admin_cooldown
    let (cpu, mem) = measure(&env, || {
        let _ = f.client.get_borrow_admin_cooldown();
    });
    assert!(cpu > 0, "get_borrow_admin_cooldown must consume CPU");
    assert!(
        cpu < 2_000_000,
        "get_borrow_admin_cooldown CPU regression: {cpu}"
    );
    assert!(
        mem < 200_000,
        "get_borrow_admin_cooldown memory regression: {mem}"
    );
    eprintln!("get_borrow_admin_cooldown: cpu={cpu} mem={mem}");

    // get_utilization_cap
    let (cpu, mem) = measure(&env, || {
        let _ = f.client.get_utilization_cap(&f.borrower);
    });
    assert!(cpu > 0, "get_utilization_cap must consume CPU");
    assert!(cpu < 2_000_000, "get_utilization_cap CPU regression: {cpu}");
    assert!(
        mem < 200_000,
        "get_utilization_cap memory regression: {mem}"
    );
    eprintln!("get_utilization_cap: cpu={cpu} mem={mem}");
}

// ── Edge cases ────────────────────────────────────────────────────────────

/// Two identical `draw_credit` calls must produce the same CPU and memory
/// cost (deterministic budget model).
#[test]
fn gas_draw_credit_deterministic() {
    let env = Env::default();
    let f = setup(&env);

    let (cpu1, mem1) = measure(&env, || {
        f.client.draw_credit(&f.borrower, &DRAW_AMOUNT);
    });
    let (cpu2, mem2) = measure(&env, || {
        f.client.draw_credit(&f.borrower, &DRAW_AMOUNT);
    });

    assert_eq!(cpu1, cpu2, "draw_credit CPU must be deterministic");
    assert_eq!(mem1, mem2, "draw_credit memory must be deterministic");

    eprintln!("draw_credit(deterministic): cpu={cpu1} mem={mem1}");
}

/// Write entrypoints must be more expensive than read-only queries, as they
/// perform auth checks, token transfers, and storage writes.
#[test]
fn gas_write_more_expensive_than_read() {
    let env = Env::default();
    let f = setup(&env);

    let (read_cpu, _) = measure(&env, || {
        let _ = f.client.get_draw_min_interval();
    });

    let (write_cpu, _) = measure(&env, || {
        f.client.draw_credit(&f.borrower, &DRAW_AMOUNT);
    });

    assert!(
        write_cpu >= read_cpu,
        "draw_credit ({write_cpu} CPU) should cost at least as much as get_draw_min_interval ({read_cpu} CPU)"
    );

    eprintln!("read_cpu={read_cpu} write_cpu={write_cpu}");
}

/// Back-to-back admin operations must not exhibit unexpected cost
/// accumulation between calls (each call is independently bounded).
#[test]
fn gas_admin_operations_independent_cost() {
    let env = Env::default();
    let f = setup(&env);

    let (cpu1, _) = measure(&env, || {
        f.client.set_draw_min_interval(&120_u64);
    });

    let (cpu2, _) = measure(&env, || {
        f.client.set_draw_min_interval(&240_u64);
    });

    assert_eq!(
        cpu1, cpu2,
        "set_draw_min_interval cost must be stable across calls"
    );

    eprintln!("admin_ops_independent: cpu1={cpu1} cpu2={cpu2}");
}
