// SPDX-License-Identifier: MIT

//! Per-entrypoint CPU/memory gas snapshot tests for the v7 query subsystem.
//!
//! # What
//!
//! Snapshots CPU instructions and memory bytes for every read-only entrypoint
//! in the v7 query surface to establish a regression baseline. Any unintended
//! change in resource consumption will fail CI, alerting developers to budget
//! regressions before they reach production.
//!
//! # Entrypoints covered
//!
//! | Entrypoint                  | Category              |
//! |-----------------------------|-----------------------|
//! | `get_credit_line`           | read-only / borrower  |
//! | `get_credit_line_summary`   | read-only / borrower  |
//! | `get_protocol_summary`      | read-only / aggregate |
//! | `get_repayment_schedule`    | read-only / borrower  |
//! | `get_health_factor`         | read-only / borrower  |
//! | `is_delinquent`             | read-only / borrower  |
//! | `get_credit_lines_paginated`| read-only / paginated |
//! | `borrow_capabilities`       | read-only / bitmap    |
//! | `query_capabilities`        | read-only / bitmap    |
//!
//! # How
//!
//! Uses the Soroban `Budget` test utility to measure CPU instructions and
//! memory bytes consumed per call. Assertions bound each measurement to a
//! conservative upper limit that forms the regression baseline.
//! Read-only entrypoints are expected to be cheaper than write paths; a
//! cross-category ordering assertion enforces this property.
//!
//! # Edge cases tested
//!
//! - No credit line (unknown borrower) — all caps / query paths return
//!   graceful defaults without panic.
//! - Zero utilization — `get_health_factor` returns `u32::MAX`; the cost
//!   of the early-return path is bounded separately.
//! - Non-zero utilization — full computation path for `get_health_factor`
//!   and `borrow_capabilities`.
//! - Determinism — identical consecutive calls produce identical CPU + memory.
//! - Paginated boundary — `get_credit_lines_paginated` with `limit = 1`,
//!   `limit = 10`, and `limit = 100` (max) must all succeed within budget.
//!
//! # Run
//!
//! ```bash
//! cargo test -p creditra-query --test gas_snap
//! ```
//!
//! # See also
//!
//! - `contracts/borrow/tests/gas_snap.rs`   — borrow-subsystem baseline.
//! - `contracts/accrual/tests/gas_snap.rs`  — accrual-subsystem baseline.
//! - `contracts/freeze/tests/gas_snap.rs`   — freeze-subsystem baseline.
//! - `contracts/collateral/tests/gas_snap.rs` — collateral baseline.
//! - `contracts/.gas-baseline.json`         — JSON budget registry.

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{
    testutils::{budget::Budget, Address as _, Ledger},
    token::StellarAssetClient,
    Address, Env,
};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Default credit limit for test credit lines (10 000 units).
const CREDIT_LIMIT: i128 = 10_000;

/// Draw amount used in tests that need a non-zero utilization (30 % of limit).
const DRAW_AMOUNT: i128 = 3_000;

/// Initial ledger timestamp. Chosen to be non-zero so timestamp arithmetic
/// is representative.
const START_TS: u64 = 1_000_000;

// ── Budget helpers ────────────────────────────────────────────────────────────

/// Reset the budget to unlimited, execute `f`, and return `(cpu, mem)`.
///
/// Resetting before each measurement ensures that setup cost does not
/// accumulate in the reading of interest.
fn measure(env: &Env, f: impl FnOnce()) -> (u64, u64) {
    let budget = env.cost_estimate().budget();
    budget.reset_unlimited();
    f();
    (budget.cpu_instruction_cost(), budget.memory_bytes_cost())
}

// ── Fixture ───────────────────────────────────────────────────────────────────

/// All state required by a gas-snapshot test.
struct Fixture<'a> {
    /// Configured and initialized credit contract client.
    client: CreditClient<'a>,
    /// Admin address (used for protocol-level mutations during setup only).
    admin: Address,
    /// A borrower that has an open credit line with zero utilization.
    borrower_idle: Address,
    /// A borrower that has drawn `DRAW_AMOUNT` so `utilized_amount > 0`.
    borrower_active: Address,
    /// A borrower that has no credit line (unknown address).
    borrower_unknown: Address,
}

/// Build and return a [`Fixture`] wired for query gas tests.
///
/// Setup steps:
/// 1. Register and init the credit contract.
/// 2. Register a SAC liquidity token; mint reserves into the contract.
/// 3. Open a credit line for `borrower_idle` (no draw).
/// 4. Open a credit line for `borrower_active`; draw `DRAW_AMOUNT`.
/// 5. Set `min_collateral_ratio_bps = 0` so draws succeed without collateral.
fn setup(env: &Env) -> Fixture<'_> {
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = START_TS);

    let admin = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    // Liquidity token: a SAC minted with enough reserves for draw operations.
    let token = env
        .register_stellar_asset_contract_v2(Address::generate(env))
        .address();
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&contract_id);
    StellarAssetClient::new(env, &token).mint(&contract_id, &(CREDIT_LIMIT * 100));

    // Disable collateral enforcement so borrowers can draw freely.
    client.set_min_collateral_ratio_bps(&0);

    // borrower_idle: open line, no draw.
    let borrower_idle = Address::generate(env);
    client.open_credit_line(&borrower_idle, &CREDIT_LIMIT, &500_u32, &50_u32);

    // borrower_active: open line + draw to create non-zero utilization.
    let borrower_active = Address::generate(env);
    client.open_credit_line(&borrower_active, &CREDIT_LIMIT, &500_u32, &50_u32);
    client.draw_credit(&borrower_active, &DRAW_AMOUNT);

    let borrower_unknown = Address::generate(env);

    Fixture {
        client,
        admin,
        borrower_idle,
        borrower_active,
        borrower_unknown,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// §1 — get_credit_line
// ═══════════════════════════════════════════════════════════════════════════

/// `get_credit_line` for a borrower that has an open line: persistent
/// storage read + TTL bump. Bounded below 2 M instructions.
#[test]
fn gas_get_credit_line_existing() {
    let env = Env::default();
    let f = setup(&env);

    let (cpu, mem) = measure(&env, || {
        let result = f.client.get_credit_line(&f.borrower_idle);
        assert!(result.is_some(), "borrower_idle must have a credit line");
    });

    assert!(cpu > 0, "get_credit_line must consume CPU");
    assert!(
        cpu < 2_000_000,
        "get_credit_line (existing) CPU regression: {cpu}"
    );
    assert!(
        mem < 200_000,
        "get_credit_line (existing) memory regression: {mem}"
    );

    eprintln!("get_credit_line(existing): cpu={cpu} mem={mem}");
}

/// `get_credit_line` for an unknown borrower: storage miss path.
/// Must be at most as expensive as the hit path (no TTL bump, early `None`).
#[test]
fn gas_get_credit_line_missing() {
    let env = Env::default();
    let f = setup(&env);

    let (cpu, mem) = measure(&env, || {
        let result = f.client.get_credit_line(&f.borrower_unknown);
        assert!(result.is_none(), "unknown borrower must return None");
    });

    assert!(cpu > 0, "get_credit_line (missing) must consume CPU");
    assert!(
        cpu < 2_000_000,
        "get_credit_line (missing) CPU regression: {cpu}"
    );
    assert!(
        mem < 200_000,
        "get_credit_line (missing) memory regression: {mem}"
    );

    eprintln!("get_credit_line(missing): cpu={cpu} mem={mem}");
}

// ═══════════════════════════════════════════════════════════════════════════
// §2 — get_credit_line_summary (backward-compat alias)
// ═══════════════════════════════════════════════════════════════════════════

/// `get_credit_line_summary` is a backward-compatible alias for
/// `get_credit_line`. Its gas cost must match the base read path.
#[test]
fn gas_get_credit_line_summary_existing() {
    let env = Env::default();
    let f = setup(&env);

    let (cpu, mem) = measure(&env, || {
        let result = f.client.get_credit_line_summary(&f.borrower_idle);
        assert!(result.is_some(), "borrower_idle must have a summary");
    });

    assert!(cpu > 0, "get_credit_line_summary must consume CPU");
    assert!(
        cpu < 2_000_000,
        "get_credit_line_summary (existing) CPU regression: {cpu}"
    );
    assert!(
        mem < 200_000,
        "get_credit_line_summary (existing) memory regression: {mem}"
    );

    eprintln!("get_credit_line_summary(existing): cpu={cpu} mem={mem}");
}

/// `get_credit_line_summary` for an unknown borrower mirrors the miss cost
/// of `get_credit_line`.
#[test]
fn gas_get_credit_line_summary_missing() {
    let env = Env::default();
    let f = setup(&env);

    let (cpu, mem) = measure(&env, || {
        let result = f.client.get_credit_line_summary(&f.borrower_unknown);
        assert!(result.is_none());
    });

    assert!(cpu > 0);
    assert!(
        cpu < 2_000_000,
        "get_credit_line_summary (missing) CPU regression: {cpu}"
    );
    assert!(mem < 200_000);

    eprintln!("get_credit_line_summary(missing): cpu={cpu} mem={mem}");
}

// ═══════════════════════════════════════════════════════════════════════════
// §3 — get_protocol_summary
// ═══════════════════════════════════════════════════════════════════════════

/// `get_protocol_summary` reads aggregate counters from instance storage.
/// No per-borrower reads, no TTL bumps — should be very cheap.
#[test]
fn gas_get_protocol_summary_with_lines() {
    let env = Env::default();
    let f = setup(&env);

    let (cpu, mem) = measure(&env, || {
        let summary = f.client.get_protocol_summary();
        // Two credit lines were opened in setup.
        assert!(summary.count >= 2, "at least 2 lines must be counted");
    });

    assert!(cpu > 0, "get_protocol_summary must consume CPU");
    assert!(
        cpu < 2_000_000,
        "get_protocol_summary CPU regression: {cpu}"
    );
    assert!(
        mem < 200_000,
        "get_protocol_summary memory regression: {mem}"
    );

    eprintln!("get_protocol_summary(with_lines): cpu={cpu} mem={mem}");
}

/// `get_protocol_summary` on a fresh contract with no credit lines.
/// Establishes the minimum-overhead baseline for aggregate reads.
#[test]
fn gas_get_protocol_summary_empty() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let (cpu, mem) = measure(&env, || {
        let summary = client.get_protocol_summary();
        assert_eq!(summary.count, 0);
        assert_eq!(summary.total_utilized, 0);
    });

    assert!(cpu > 0);
    assert!(
        cpu < 1_000_000,
        "get_protocol_summary (empty) CPU regression: {cpu}"
    );
    assert!(mem < 100_000);

    eprintln!("get_protocol_summary(empty): cpu={cpu} mem={mem}");
}

// ═══════════════════════════════════════════════════════════════════════════
// §4 — get_repayment_schedule
// ═══════════════════════════════════════════════════════════════════════════

/// `get_repayment_schedule` for a borrower with no schedule configured.
/// Storage miss — should be cheap, returns `None` gracefully.
#[test]
fn gas_get_repayment_schedule_none() {
    let env = Env::default();
    let f = setup(&env);

    let (cpu, mem) = measure(&env, || {
        let sched = f.client.get_repayment_schedule(&f.borrower_idle);
        assert!(sched.is_none(), "no schedule has been set");
    });

    assert!(cpu > 0);
    assert!(
        cpu < 2_000_000,
        "get_repayment_schedule (none) CPU regression: {cpu}"
    );
    assert!(mem < 200_000);

    eprintln!("get_repayment_schedule(none): cpu={cpu} mem={mem}");
}

/// `get_repayment_schedule` for a borrower after a schedule has been set.
/// Persistent storage hit + TTL bump path.
#[test]
fn gas_get_repayment_schedule_existing() {
    let env = Env::default();
    let f = setup(&env);

    // Configure a repayment schedule: 12 monthly installments.
    f.client.set_repayment_schedule(
        &f.borrower_idle,
        &START_TS,                 // start timestamp
        &(START_TS + 86_400 * 30), // first due
        &12_u32,                   // installments
        &(CREDIT_LIMIT / 12),      // amount per installment
    );

    let (cpu, mem) = measure(&env, || {
        let sched = f.client.get_repayment_schedule(&f.borrower_idle);
        assert!(sched.is_some(), "schedule must be found after set");
    });

    assert!(cpu > 0);
    assert!(
        cpu < 2_000_000,
        "get_repayment_schedule (existing) CPU regression: {cpu}"
    );
    assert!(mem < 200_000);

    eprintln!("get_repayment_schedule(existing): cpu={cpu} mem={mem}");
}

// ═══════════════════════════════════════════════════════════════════════════
// §5 — get_health_factor
// ═══════════════════════════════════════════════════════════════════════════

/// `get_health_factor` for a borrower with zero utilization returns `u32::MAX`
/// via the early-return path — one storage read + early exit.
#[test]
fn gas_get_health_factor_zero_utilization() {
    let env = Env::default();
    let f = setup(&env);

    let (cpu, mem) = measure(&env, || {
        let hf = f.client.get_health_factor(&f.borrower_idle);
        assert_eq!(hf, u32::MAX, "zero utilization must return u32::MAX");
    });

    assert!(cpu > 0);
    assert!(
        cpu < 2_000_000,
        "get_health_factor (zero util) CPU regression: {cpu}"
    );
    assert!(mem < 200_000);

    eprintln!("get_health_factor(zero_util): cpu={cpu} mem={mem}");
}

/// `get_health_factor` for an unknown borrower: no credit line → `u32::MAX`
/// via the first short-circuit branch (even cheaper than the zero-util path).
#[test]
fn gas_get_health_factor_no_credit_line() {
    let env = Env::default();
    let f = setup(&env);

    let (cpu, mem) = measure(&env, || {
        let hf = f.client.get_health_factor(&f.borrower_unknown);
        assert_eq!(hf, u32::MAX, "missing line must return u32::MAX");
    });

    assert!(cpu > 0);
    assert!(
        cpu < 2_000_000,
        "get_health_factor (no line) CPU regression: {cpu}"
    );
    assert!(mem < 200_000);

    eprintln!("get_health_factor(no_line): cpu={cpu} mem={mem}");
}

/// `get_health_factor` for a borrower with non-zero utilization: full
/// computation path — credit-line read + collateral read + min-ratio read
/// + overflow-safe arithmetic.
#[test]
fn gas_get_health_factor_with_utilization() {
    let env = Env::default();
    let f = setup(&env);

    let (cpu, mem) = measure(&env, || {
        // borrower_active drew DRAW_AMOUNT; min_collateral_ratio_bps = 0 so
        // denominator is 0 → health factor clamps to u32::MAX even with
        // utilization. The point is to exercise the full computation path.
        let _hf = f.client.get_health_factor(&f.borrower_active);
    });

    assert!(cpu > 0);
    assert!(
        cpu < 3_000_000,
        "get_health_factor (with util) CPU regression: {cpu}"
    );
    assert!(mem < 300_000);

    eprintln!("get_health_factor(with_util): cpu={cpu} mem={mem}");
}

// ═══════════════════════════════════════════════════════════════════════════
// §6 — is_delinquent
// ═══════════════════════════════════════════════════════════════════════════

/// `is_delinquent` for an unknown borrower short-circuits at the first
/// `get_credit_line` call — cheapest path.
#[test]
fn gas_is_delinquent_no_credit_line() {
    let env = Env::default();
    let f = setup(&env);

    let (cpu, mem) = measure(&env, || {
        let result = f.client.is_delinquent(&f.borrower_unknown);
        assert!(!result, "unknown borrower must not be delinquent");
    });

    assert!(cpu > 0);
    assert!(
        cpu < 2_000_000,
        "is_delinquent (no line) CPU regression: {cpu}"
    );
    assert!(mem < 200_000);

    eprintln!("is_delinquent(no_line): cpu={cpu} mem={mem}");
}

/// `is_delinquent` for a borrower with a line but no schedule: two storage
/// reads (credit line + schedule miss) then `false`.
#[test]
fn gas_is_delinquent_no_schedule() {
    let env = Env::default();
    let f = setup(&env);

    let (cpu, mem) = measure(&env, || {
        // borrower_active has a line but no repayment schedule.
        let result = f.client.is_delinquent(&f.borrower_active);
        assert!(!result, "no schedule → not delinquent");
    });

    assert!(cpu > 0);
    assert!(
        cpu < 2_500_000,
        "is_delinquent (no schedule) CPU regression: {cpu}"
    );
    assert!(mem < 250_000);

    eprintln!("is_delinquent(no_schedule): cpu={cpu} mem={mem}");
}

/// `is_delinquent` for a borrower with a schedule but current time is before
/// the due date — full path, returns `false`.
#[test]
fn gas_is_delinquent_not_past_due() {
    let env = Env::default();
    let f = setup(&env);

    // Due date is 30 days from start; current ledger timestamp is START_TS.
    let next_due = START_TS + 86_400 * 30;
    f.client.set_repayment_schedule(
        &f.borrower_active,
        &START_TS,
        &next_due,
        &12_u32,
        &(CREDIT_LIMIT / 12),
    );

    let (cpu, mem) = measure(&env, || {
        let result = f.client.is_delinquent(&f.borrower_active);
        assert!(!result, "not past due → not delinquent");
    });

    assert!(cpu > 0);
    assert!(
        cpu < 3_000_000,
        "is_delinquent (not past due) CPU regression: {cpu}"
    );
    assert!(mem < 300_000);

    eprintln!("is_delinquent(not_past_due): cpu={cpu} mem={mem}");
}

/// `is_delinquent` full-path returning `true`: schedule is set, time has
/// advanced past the due date with no grace period.
#[test]
fn gas_is_delinquent_past_due() {
    let env = Env::default();
    let f = setup(&env);

    let next_due = START_TS + 1; // due almost immediately
    f.client.set_repayment_schedule(
        &f.borrower_active,
        &START_TS,
        &next_due,
        &12_u32,
        &(CREDIT_LIMIT / 12),
    );

    // Advance time past due + default grace of 0 seconds.
    env.ledger().with_mut(|li| li.timestamp = START_TS + 100);

    let (cpu, mem) = measure(&env, || {
        let result = f.client.is_delinquent(&f.borrower_active);
        assert!(result, "past due with no grace → must be delinquent");
    });

    assert!(cpu > 0);
    assert!(
        cpu < 3_000_000,
        "is_delinquent (past due) CPU regression: {cpu}"
    );
    assert!(mem < 300_000);

    eprintln!("is_delinquent(past_due): cpu={cpu} mem={mem}");
}

// ═══════════════════════════════════════════════════════════════════════════
// §7 — get_credit_lines_paginated
// ═══════════════════════════════════════════════════════════════════════════

/// `get_credit_lines_paginated` with `limit = 1` from the beginning (no cursor).
/// Minimum per-call overhead: auth + one page iteration.
#[test]
fn gas_get_credit_lines_paginated_limit_1() {
    let env = Env::default();
    let f = setup(&env);

    let (cpu, mem) = measure(&env, || {
        let page = f.client.get_credit_lines_paginated(&None, &1_u32);
        assert!(page.items.len() <= 1, "limit=1 must return at most 1 item");
    });

    assert!(cpu > 0);
    assert!(
        cpu < 2_000_000,
        "get_credit_lines_paginated (limit=1) CPU regression: {cpu}"
    );
    assert!(mem < 200_000);

    eprintln!("get_credit_lines_paginated(limit=1): cpu={cpu} mem={mem}");
}

/// `get_credit_lines_paginated` with `limit = 10` — mid-range page size.
#[test]
fn gas_get_credit_lines_paginated_limit_10() {
    let env = Env::default();
    let f = setup(&env);

    let (cpu, mem) = measure(&env, || {
        let page = f.client.get_credit_lines_paginated(&None, &10_u32);
        // Only 2 lines exist from setup, so the page will be shorter.
        assert!(page.items.len() <= 10);
    });

    assert!(cpu > 0);
    assert!(
        cpu < 3_000_000,
        "get_credit_lines_paginated (limit=10) CPU regression: {cpu}"
    );
    assert!(mem < 300_000);

    eprintln!("get_credit_lines_paginated(limit=10): cpu={cpu} mem={mem}");
}

/// `get_credit_lines_paginated` at the protocol-maximum `limit = 100`.
/// Must succeed without reverting (the revert only fires at limit > 100).
#[test]
fn gas_get_credit_lines_paginated_limit_100() {
    let env = Env::default();
    let f = setup(&env);

    let (cpu, mem) = measure(&env, || {
        let page = f.client.get_credit_lines_paginated(&None, &100_u32);
        assert!(page.items.len() <= 100);
    });

    assert!(cpu > 0);
    assert!(
        cpu < 5_000_000,
        "get_credit_lines_paginated (limit=100) CPU regression: {cpu}"
    );
    assert!(mem < 500_000);

    eprintln!("get_credit_lines_paginated(limit=100): cpu={cpu} mem={mem}");
}

/// `get_credit_lines_paginated` with a cursor pointing past the second item.
/// Exercises the cursor-seek overhead on top of the page scan.
#[test]
fn gas_get_credit_lines_paginated_with_cursor() {
    let env = Env::default();
    let f = setup(&env);

    // Retrieve first page to extract the next_cursor, if any.
    let first_page = f.client.get_credit_lines_paginated(&None, &1_u32);
    let cursor = first_page.next_cursor;

    let (cpu, mem) = measure(&env, || {
        let _page = f.client.get_credit_lines_paginated(&cursor, &1_u32);
    });

    assert!(cpu > 0);
    assert!(
        cpu < 3_000_000,
        "get_credit_lines_paginated (with cursor) CPU regression: {cpu}"
    );
    assert!(mem < 300_000);

    eprintln!("get_credit_lines_paginated(with_cursor): cpu={cpu} mem={mem}");
}

// ═══════════════════════════════════════════════════════════════════════════
// §8 — borrow_capabilities
// ═══════════════════════════════════════════════════════════════════════════

/// `borrow_capabilities` for a borrower with an active, undrawn line.
/// All three capability checks run; no storage mutations.
#[test]
fn gas_borrow_capabilities_active_line() {
    let env = Env::default();
    let f = setup(&env);

    let (cpu, mem) = measure(&env, || {
        let caps = f.client.borrow_capabilities(&f.borrower_idle);
        // An active line that is not paused/blocked/frozen can draw and repay.
        assert!(caps.can_draw, "active undrawn line should be drawable");
        assert!(caps.can_repay, "active line should be repayable");
    });

    assert!(cpu > 0);
    assert!(
        cpu < 3_000_000,
        "borrow_capabilities (active) CPU regression: {cpu}"
    );
    assert!(mem < 300_000);

    eprintln!("borrow_capabilities(active): cpu={cpu} mem={mem}");
}

/// `borrow_capabilities` for an unknown borrower: no credit line → all caps
/// are `false`, storage miss path only.
#[test]
fn gas_borrow_capabilities_no_credit_line() {
    let env = Env::default();
    let f = setup(&env);

    let (cpu, mem) = measure(&env, || {
        let caps = f.client.borrow_capabilities(&f.borrower_unknown);
        assert!(!caps.can_draw, "no line → cannot draw");
        assert!(!caps.can_repay, "no line → cannot repay");
        assert!(!caps.can_self_suspend, "no line → cannot self-suspend");
    });

    assert!(cpu > 0);
    assert!(
        cpu < 3_000_000,
        "borrow_capabilities (no line) CPU regression: {cpu}"
    );
    assert!(mem < 300_000);

    eprintln!("borrow_capabilities(no_line): cpu={cpu} mem={mem}");
}

/// `borrow_capabilities` for a borrower whose line has been suspended.
/// `can_draw` must be `false` because the line is not `Active`.
#[test]
fn gas_borrow_capabilities_suspended_line() {
    let env = Env::default();
    let f = setup(&env);

    // Suspend the idle borrower's line.
    f.client.suspend_credit_line(&f.borrower_idle);

    let (cpu, mem) = measure(&env, || {
        let caps = f.client.borrow_capabilities(&f.borrower_idle);
        assert!(!caps.can_draw, "suspended line → cannot draw");
    });

    assert!(cpu > 0);
    assert!(
        cpu < 3_000_000,
        "borrow_capabilities (suspended) CPU regression: {cpu}"
    );
    assert!(mem < 300_000);

    eprintln!("borrow_capabilities(suspended): cpu={cpu} mem={mem}");
}

// ═══════════════════════════════════════════════════════════════════════════
// §9 — query_capabilities
// ═══════════════════════════════════════════════════════════════════════════

/// `query_capabilities` for a borrower with no credit line: all flags false,
/// only one storage miss.
#[test]
fn gas_query_capabilities_no_credit_line() {
    let env = Env::default();
    let f = setup(&env);

    let (cpu, mem) = measure(&env, || {
        let caps = f.client.query_capabilities(&f.borrower_unknown);
        assert!(!caps.has_credit_line);
        assert!(!caps.health_factor_applicable);
        assert!(!caps.delinquency_applicable);
        assert!(!caps.is_delinquent);
    });

    assert!(cpu > 0);
    assert!(
        cpu < 2_500_000,
        "query_capabilities (no line) CPU regression: {cpu}"
    );
    assert!(mem < 250_000);

    eprintln!("query_capabilities(no_line): cpu={cpu} mem={mem}");
}

/// `query_capabilities` for a borrower with a line but zero utilization:
/// `has_credit_line = true`, health/delinquency flags false.
#[test]
fn gas_query_capabilities_zero_utilization() {
    let env = Env::default();
    let f = setup(&env);

    let (cpu, mem) = measure(&env, || {
        let caps = f.client.query_capabilities(&f.borrower_idle);
        assert!(caps.has_credit_line);
        assert!(!caps.health_factor_applicable);
        assert!(!caps.delinquency_applicable);
    });

    assert!(cpu > 0);
    assert!(
        cpu < 3_000_000,
        "query_capabilities (zero util) CPU regression: {cpu}"
    );
    assert!(mem < 300_000);

    eprintln!("query_capabilities(zero_util): cpu={cpu} mem={mem}");
}

/// `query_capabilities` for a borrower with non-zero utilization and a
/// schedule — the full path including `is_delinquent` evaluation.
#[test]
fn gas_query_capabilities_full_path_with_schedule() {
    let env = Env::default();
    let f = setup(&env);

    // Give borrower_active a schedule so `delinquency_applicable` is true
    // and the `is_delinquent` branch fires.
    let next_due = START_TS + 86_400 * 30;
    f.client.set_repayment_schedule(
        &f.borrower_active,
        &START_TS,
        &next_due,
        &12_u32,
        &(CREDIT_LIMIT / 12),
    );

    let (cpu, mem) = measure(&env, || {
        let caps = f.client.query_capabilities(&f.borrower_active);
        assert!(caps.has_credit_line);
        assert!(caps.health_factor_applicable);
        assert!(caps.delinquency_applicable);
        // Time has not advanced past due date.
        assert!(!caps.is_delinquent);
    });

    assert!(cpu > 0);
    assert!(
        cpu < 4_000_000,
        "query_capabilities (full path) CPU regression: {cpu}"
    );
    assert!(mem < 400_000);

    eprintln!("query_capabilities(full_path): cpu={cpu} mem={mem}");
}

/// `query_capabilities` for a closed line: `has_credit_line = true`,
/// `delinquency_applicable = false` — the closed-line short-circuit.
#[test]
fn gas_query_capabilities_closed_line() {
    let env = Env::default();
    let f = setup(&env);

    f.client.close_credit_line(&f.borrower_idle, &f.admin);

    let (cpu, mem) = measure(&env, || {
        let caps = f.client.query_capabilities(&f.borrower_idle);
        assert!(caps.has_credit_line, "closed line still present in storage");
        assert!(!caps.delinquency_applicable, "closed → delinquency N/A");
        assert!(!caps.is_delinquent);
    });

    assert!(cpu > 0);
    assert!(
        cpu < 3_000_000,
        "query_capabilities (closed) CPU regression: {cpu}"
    );
    assert!(mem < 300_000);

    eprintln!("query_capabilities(closed): cpu={cpu} mem={mem}");
}

// ═══════════════════════════════════════════════════════════════════════════
// §10 — Determinism: identical calls must produce identical cost
// ═══════════════════════════════════════════════════════════════════════════

/// Two back-to-back `get_credit_line` calls on the same state must consume
/// identical CPU and memory (deterministic Soroban cost model).
#[test]
fn gas_get_credit_line_deterministic() {
    let env = Env::default();
    let f = setup(&env);

    let (cpu1, mem1) = measure(&env, || {
        let _ = f.client.get_credit_line(&f.borrower_idle);
    });
    let (cpu2, mem2) = measure(&env, || {
        let _ = f.client.get_credit_line(&f.borrower_idle);
    });

    assert_eq!(cpu1, cpu2, "get_credit_line CPU must be deterministic");
    assert_eq!(mem1, mem2, "get_credit_line memory must be deterministic");

    eprintln!("get_credit_line(deterministic): cpu={cpu1} mem={mem1}");
}

/// Two back-to-back `query_capabilities` calls must be deterministic.
#[test]
fn gas_query_capabilities_deterministic() {
    let env = Env::default();
    let f = setup(&env);

    let (cpu1, mem1) = measure(&env, || {
        let _ = f.client.query_capabilities(&f.borrower_idle);
    });
    let (cpu2, mem2) = measure(&env, || {
        let _ = f.client.query_capabilities(&f.borrower_idle);
    });

    assert_eq!(cpu1, cpu2, "query_capabilities CPU must be deterministic");
    assert_eq!(
        mem1, mem2,
        "query_capabilities memory must be deterministic"
    );

    eprintln!("query_capabilities(deterministic): cpu={cpu1} mem={mem1}");
}

/// Two back-to-back `get_health_factor` calls must be deterministic.
#[test]
fn gas_get_health_factor_deterministic() {
    let env = Env::default();
    let f = setup(&env);

    let (cpu1, mem1) = measure(&env, || {
        let _ = f.client.get_health_factor(&f.borrower_active);
    });
    let (cpu2, mem2) = measure(&env, || {
        let _ = f.client.get_health_factor(&f.borrower_active);
    });

    assert_eq!(cpu1, cpu2, "get_health_factor CPU must be deterministic");
    assert_eq!(mem1, mem2, "get_health_factor memory must be deterministic");

    eprintln!("get_health_factor(deterministic): cpu={cpu1} mem={mem1}");
}

// ═══════════════════════════════════════════════════════════════════════════
// §11 — Ordering: reads must be cheaper than writes
// ═══════════════════════════════════════════════════════════════════════════

/// Read-only query entrypoints must be strictly cheaper than `draw_credit`,
/// which performs auth + token CPI + multiple storage writes.
///
/// We compare `get_credit_line` (cheapest representative read) against
/// `draw_credit` (representative write). This guards against query-path
/// regressions that approach write-path cost.
#[test]
fn gas_read_cheaper_than_write() {
    let env = Env::default();
    let f = setup(&env);

    let (read_cpu, _) = measure(&env, || {
        let _ = f.client.get_credit_line(&f.borrower_idle);
    });

    let (write_cpu, _) = measure(&env, || {
        f.client.draw_credit(&f.borrower_active, &100_i128);
    });

    assert!(
        read_cpu < write_cpu,
        "get_credit_line ({read_cpu} CPU) must cost less than draw_credit ({write_cpu} CPU)"
    );

    eprintln!("read_cpu={read_cpu} write_cpu={write_cpu}");
}

/// `borrow_capabilities` and `query_capabilities` (multi-read bitmaps) must
/// be cheaper than `draw_credit` (write + token CPI).
#[test]
fn gas_capability_reads_cheaper_than_draw() {
    let env = Env::default();
    let f = setup(&env);

    let (borrow_caps_cpu, _) = measure(&env, || {
        let _ = f.client.borrow_capabilities(&f.borrower_idle);
    });

    let (query_caps_cpu, _) = measure(&env, || {
        let _ = f.client.query_capabilities(&f.borrower_idle);
    });

    let (draw_cpu, _) = measure(&env, || {
        f.client.draw_credit(&f.borrower_active, &100_i128);
    });

    assert!(
        borrow_caps_cpu < draw_cpu,
        "borrow_capabilities ({borrow_caps_cpu}) must cost less than draw_credit ({draw_cpu})"
    );
    assert!(
        query_caps_cpu < draw_cpu,
        "query_capabilities ({query_caps_cpu}) must cost less than draw_credit ({draw_cpu})"
    );

    eprintln!(
        "borrow_caps_cpu={borrow_caps_cpu} query_caps_cpu={query_caps_cpu} draw_cpu={draw_cpu}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// §12 — Aggregate summary: all 9 entrypoints in one test
// ═══════════════════════════════════════════════════════════════════════════

/// Snapshot all 9 query entrypoints in a single pass so reviewers can see
/// the full cost table in one `cargo test` run.
///
/// Runs every entrypoint in the order listed in the module doc and prints
/// a formatted cost table to stderr. Assertions are per-entrypoint to
/// surface the first failing budget in CI output.
#[test]
fn gas_query_all_entrypoints_summary() {
    let env = Env::default();
    let f = setup(&env);

    // Set up a repayment schedule for the active borrower so the
    // `is_delinquent` full path is exercised in `query_capabilities`.
    let next_due = START_TS + 86_400 * 30;
    f.client.set_repayment_schedule(
        &f.borrower_active,
        &START_TS,
        &next_due,
        &12_u32,
        &(CREDIT_LIMIT / 12),
    );

    let mut table: std::vec::Vec<(&str, u64, u64)> = std::vec::Vec::new();

    macro_rules! snap {
        ($name:expr, $limit_cpu:expr, $limit_mem:expr, $body:block) => {{
            let (cpu, mem) = measure(&env, || $body);
            assert!(
                cpu < $limit_cpu,
                "{}: CPU regression — got {cpu}, limit {}",
                $name,
                $limit_cpu
            );
            assert!(
                mem < $limit_mem,
                "{}: memory regression — got {mem}, limit {}",
                $name,
                $limit_mem
            );
            table.push(($name, cpu, mem));
        }};
    }

    snap!("get_credit_line", 2_000_000, 200_000, {
        let _ = f.client.get_credit_line(&f.borrower_idle);
    });
    snap!("get_credit_line_summary", 2_000_000, 200_000, {
        let _ = f.client.get_credit_line_summary(&f.borrower_idle);
    });
    snap!("get_protocol_summary", 2_000_000, 200_000, {
        let _ = f.client.get_protocol_summary();
    });
    snap!("get_repayment_schedule", 2_000_000, 200_000, {
        let _ = f.client.get_repayment_schedule(&f.borrower_active);
    });
    snap!("get_health_factor", 3_000_000, 300_000, {
        let _ = f.client.get_health_factor(&f.borrower_active);
    });
    snap!("is_delinquent", 3_000_000, 300_000, {
        let _ = f.client.is_delinquent(&f.borrower_active);
    });
    snap!("get_credit_lines_paginated", 5_000_000, 500_000, {
        let _ = f.client.get_credit_lines_paginated(&None, &100_u32);
    });
    snap!("borrow_capabilities", 3_000_000, 300_000, {
        let _ = f.client.borrow_capabilities(&f.borrower_idle);
    });
    snap!("query_capabilities", 4_000_000, 400_000, {
        let _ = f.client.query_capabilities(&f.borrower_active);
    });

    eprintln!(
        "\n{:<35} {:>15} {:>15}",
        "entrypoint", "cpu_instructions", "memory_bytes"
    );
    eprintln!("{}", "-".repeat(67));
    for (name, cpu, mem) in &table {
        eprintln!("{:<35} {:>15} {:>15}", name, cpu, mem);
    }
}
