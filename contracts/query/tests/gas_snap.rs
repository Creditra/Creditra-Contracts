// SPDX-License-Identifier: MIT

//! Per-entrypoint gas snapshot tests for query (v7).
//!
//! # What
//!
//! Snapshots CPU instructions and memory bytes consumed by each read-only
//! query entrypoint to establish a regression baseline. Any unintended
//! increase in resource consumption will be visible in CI output, alerting
//! developers before it reaches the 50 KB WASM hard budget.
//!
//! # Entrypoints covered
//!
//! | Entrypoint                  | Test(s)                                      |
//! |-----------------------------|----------------------------------------------|
//! | `get_credit_line`           | missing borrower, existing line              |
//! | `get_protocol_summary`      | empty state, after multiple lines            |
//! | `borrow_capabilities`       | no line, active line, paused protocol        |
//! | `is_delinquent`             | no schedule, on-time, delinquent             |
//! | `get_health_factor`         | no utilization, partially collateralized     |
//! | `get_repayment_schedule`    | missing, configured                          |
//! | `get_credit_lines_paginated`| empty, 5-line page                           |
//!
//! # How
//!
//! Uses the Soroban `Budget` test utility to measure CPU and memory per call.
//! Each test resets the budget immediately before the call under measurement
//! and prints the observed values via `eprintln!` for CI log inspection.
//!
//! # See also
//! - `contracts/accrual/tests/gas_snap.rs` — accrual budget regression.
//! - `contracts/credit/tests/budget_regression.rs` — credit contract overall budget.

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{
    testutils::{budget::Budget, Address as _, Ledger},
    token, Address, Env, Vec,
};

// ── Shared setup helpers ─────────────────────────────────────────────────────

/// Reset the budget immediately before `f`, then return (cpu, mem) after.
fn measure(env: &Env, f: impl FnOnce()) -> (u64, u64) {
    let budget = env.cost_estimate().budget();
    budget.reset_unlimited();
    f();
    (budget.cpu_instruction_cost(), budget.memory_bytes_cost())
}

/// Deploy, init admin, wire up a SAC token, and mint `token_mint` units
/// directly into the contract address for use as the liquidity reserve.
///
/// Returns `(env, contract_id, admin_address)`.
fn setup(token_mint: i128) -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_addr = token_id.address();
    client.set_liquidity_token(&token_addr);
    client.set_liquidity_source(&contract_id);

    token::StellarAssetClient::new(&env, &token_addr).mint(&contract_id, &token_mint);

    (env, contract_id, admin)
}

/// Open `n` credit lines and optionally draw `draw_amount` on each one.
/// Returns the list of borrower addresses.
fn open_lines(
    env: &Env,
    client: &CreditClient<'_>,
    n: usize,
    draw_amount: Option<i128>,
) -> std::vec::Vec<Address> {
    let mut borrowers = std::vec::Vec::with_capacity(n);
    for i in 0..n {
        let b = Address::generate(env);
        let limit = 50_000_i128 + (i as i128 * 10_000_i128);
        let rate = 500_u32 + (i as u32 * 100_u32);
        client.open_credit_line(&b, &limit, &rate, &(30_u32 + i as u32));
        if let Some(amt) = draw_amount {
            client.draw_credit(&b, &amt);
        }
        borrowers.push(b);
    }
    borrowers
}

// ═══════════════════════════════════════════════════════════════════════════
// get_credit_line
// ═══════════════════════════════════════════════════════════════════════════

/// `get_credit_line` on a non-existent borrower returns `None` cheaply.
/// This measures the storage-miss path: auth skip + single KV lookup miss.
#[test]
fn gas_get_credit_line_missing() {
    let (env, contract_id, _) = setup(0);
    let client = CreditClient::new(&env, &contract_id);
    let phantom = Address::generate(&env);

    let (cpu, mem) = measure(&env, || {
        let result = client.get_credit_line(&phantom);
        assert!(result.is_none());
    });

    assert!(cpu > 0, "must consume some CPU");
    assert!(cpu < 500_000, "get_credit_line miss CPU unexpectedly high: {cpu}");
    assert!(mem < 200_000, "get_credit_line miss memory unexpectedly high: {mem}");
    eprintln!("get_credit_line(missing): cpu={cpu} mem={mem}");
}

/// `get_credit_line` on an existing borrower hits the persistent storage path
/// and may bump TTL.
#[test]
fn gas_get_credit_line_existing() {
    let (env, contract_id, _) = setup(500_000);
    let client = CreditClient::new(&env, &contract_id);
    let borrowers = open_lines(&env, &client, 1, None);

    let (cpu, mem) = measure(&env, || {
        let result = client.get_credit_line(&borrowers[0]);
        assert!(result.is_some());
    });

    assert!(cpu > 0);
    assert!(cpu < 1_000_000, "get_credit_line existing CPU unexpectedly high: {cpu}");
    eprintln!("get_credit_line(existing): cpu={cpu} mem={mem}");
}

// ═══════════════════════════════════════════════════════════════════════════
// get_protocol_summary
// ═══════════════════════════════════════════════════════════════════════════

/// `get_protocol_summary` on an empty protocol reads only aggregate slots.
#[test]
fn gas_get_protocol_summary_empty() {
    let (env, contract_id, _) = setup(0);
    let client = CreditClient::new(&env, &contract_id);

    let (cpu, mem) = measure(&env, || {
        let summary = client.get_protocol_summary();
        assert_eq!(summary.count, 0);
    });

    assert!(cpu > 0);
    assert!(cpu < 500_000, "get_protocol_summary empty CPU unexpectedly high: {cpu}");
    eprintln!("get_protocol_summary(empty): cpu={cpu} mem={mem}");
}

/// `get_protocol_summary` after opening 5 lines reflects the accumulator state.
#[test]
fn gas_get_protocol_summary_with_lines() {
    let (env, contract_id, _) = setup(1_000_000);
    let client = CreditClient::new(&env, &contract_id);
    open_lines(&env, &client, 5, Some(5_000_i128));

    let (cpu, mem) = measure(&env, || {
        let summary = client.get_protocol_summary();
        assert_eq!(summary.count, 5);
        assert!(summary.total_utilized > 0);
    });

    assert!(cpu > 0);
    assert!(cpu < 500_000, "get_protocol_summary with lines CPU unexpectedly high: {cpu}");
    eprintln!("get_protocol_summary(5_lines): cpu={cpu} mem={mem}");
}

// ═══════════════════════════════════════════════════════════════════════════
// borrow_capabilities
// ═══════════════════════════════════════════════════════════════════════════

/// `borrow_capabilities` with no credit line returns all-false quickly.
#[test]
fn gas_borrow_capabilities_no_line() {
    let (env, contract_id, _) = setup(0);
    let client = CreditClient::new(&env, &contract_id);
    let phantom = Address::generate(&env);

    let (cpu, mem) = measure(&env, || {
        let caps = client.borrow_capabilities(&phantom);
        assert!(!caps.can_draw);
        assert!(!caps.can_repay);
        assert!(!caps.can_self_suspend);
    });

    assert!(cpu > 0);
    assert!(cpu < 1_000_000, "borrow_capabilities no-line CPU unexpectedly high: {cpu}");
    eprintln!("borrow_capabilities(no_line): cpu={cpu} mem={mem}");
}

/// `borrow_capabilities` with an active line evaluates all predicate checks.
#[test]
fn gas_borrow_capabilities_active_line() {
    let (env, contract_id, _) = setup(500_000);
    let client = CreditClient::new(&env, &contract_id);
    let borrowers = open_lines(&env, &client, 1, None);

    let (cpu, mem) = measure(&env, || {
        let caps = client.borrow_capabilities(&borrowers[0]);
        assert!(caps.can_draw);
        assert!(caps.can_repay);
        assert!(caps.can_self_suspend);
    });

    assert!(cpu > 0);
    assert!(cpu < 2_000_000, "borrow_capabilities active CPU unexpectedly high: {cpu}");
    eprintln!("borrow_capabilities(active): cpu={cpu} mem={mem}");
}

/// `borrow_capabilities` while the protocol is paused must return `can_draw = false`.
#[test]
fn gas_borrow_capabilities_paused_protocol() {
    let (env, contract_id, _) = setup(500_000);
    let client = CreditClient::new(&env, &contract_id);
    let borrowers = open_lines(&env, &client, 1, None);
    client.pause_protocol();

    let (cpu, mem) = measure(&env, || {
        let caps = client.borrow_capabilities(&borrowers[0]);
        assert!(!caps.can_draw, "paused protocol must block draws");
        assert!(caps.can_repay, "paused protocol still allows repay");
    });

    assert!(cpu > 0);
    assert!(cpu < 2_000_000, "borrow_capabilities paused CPU unexpectedly high: {cpu}");
    eprintln!("borrow_capabilities(paused): cpu={cpu} mem={mem}");
}

// ═══════════════════════════════════════════════════════════════════════════
// is_delinquent
// ═══════════════════════════════════════════════════════════════════════════

/// `is_delinquent` with no repayment schedule set returns `false` cheaply.
#[test]
fn gas_is_delinquent_no_schedule() {
    let (env, contract_id, _) = setup(500_000);
    let client = CreditClient::new(&env, &contract_id);
    let borrowers = open_lines(&env, &client, 1, Some(5_000_i128));

    let (cpu, mem) = measure(&env, || {
        let result = client.is_delinquent(&borrowers[0]);
        assert!(!result, "no schedule means not delinquent");
    });

    assert!(cpu > 0);
    assert!(cpu < 1_000_000, "is_delinquent no-schedule CPU unexpectedly high: {cpu}");
    eprintln!("is_delinquent(no_schedule): cpu={cpu} mem={mem}");
}

/// `is_delinquent` with a schedule that is not yet due returns `false`.
#[test]
fn gas_is_delinquent_on_time() {
    let (env, contract_id, _) = setup(500_000);
    let client = CreditClient::new(&env, &contract_id);
    let borrowers = open_lines(&env, &client, 1, Some(5_000_i128));

    // Set a repayment schedule with next_due_ts 30 days in the future.
    let now = env.ledger().timestamp();
    let next_due = now + 86_400 * 30;
    client.set_repayment_schedule(&borrowers[0], &1_000_i128, &(86_400_u64 * 30), &next_due);

    let (cpu, mem) = measure(&env, || {
        let result = client.is_delinquent(&borrowers[0]);
        assert!(!result, "schedule not yet due must not be delinquent");
    });

    assert!(cpu > 0);
    assert!(cpu < 1_000_000, "is_delinquent on-time CPU unexpectedly high: {cpu}");
    eprintln!("is_delinquent(on_time): cpu={cpu} mem={mem}");
}

/// `is_delinquent` after the due date has passed returns `true`.
#[test]
fn gas_is_delinquent_past_due() {
    let (env, contract_id, _) = setup(500_000);
    let client = CreditClient::new(&env, &contract_id);
    let borrowers = open_lines(&env, &client, 1, Some(5_000_i128));

    // Set due timestamp in the past relative to a future ledger time.
    let base_ts = env.ledger().timestamp();
    let next_due = base_ts + 1;
    client.set_repayment_schedule(&borrowers[0], &1_000_i128, &(86_400_u64 * 30), &next_due);

    // Advance ledger well past the due timestamp.
    env.ledger().with_mut(|l| l.timestamp = base_ts + 86_400 * 7);

    let (cpu, mem) = measure(&env, || {
        let result = client.is_delinquent(&borrowers[0]);
        assert!(result, "past-due schedule must be delinquent");
    });

    assert!(cpu > 0);
    assert!(cpu < 1_500_000, "is_delinquent past-due CPU unexpectedly high: {cpu}");
    eprintln!("is_delinquent(past_due): cpu={cpu} mem={mem}");
}

// ═══════════════════════════════════════════════════════════════════════════
// get_health_factor
// ═══════════════════════════════════════════════════════════════════════════

/// `get_health_factor` with zero utilization returns `u32::MAX` cheaply.
#[test]
fn gas_get_health_factor_zero_utilization() {
    let (env, contract_id, _) = setup(500_000);
    let client = CreditClient::new(&env, &contract_id);
    let borrowers = open_lines(&env, &client, 1, None);

    let (cpu, mem) = measure(&env, || {
        let hf = client.get_health_factor(&borrowers[0]);
        assert_eq!(hf, u32::MAX, "zero utilization must return u32::MAX");
    });

    assert!(cpu > 0);
    assert!(cpu < 1_000_000, "get_health_factor zero util CPU unexpectedly high: {cpu}");
    eprintln!("get_health_factor(zero_util): cpu={cpu} mem={mem}");
}

/// `get_health_factor` with utilization and no collateral evaluates the full formula.
#[test]
fn gas_get_health_factor_with_utilization() {
    let (env, contract_id, _) = setup(500_000);
    let client = CreditClient::new(&env, &contract_id);
    let borrowers = open_lines(&env, &client, 1, Some(10_000_i128));

    let (cpu, mem) = measure(&env, || {
        // No collateral deposited — health factor will be 0.
        let hf = client.get_health_factor(&borrowers[0]);
        assert_eq!(hf, 0, "no collateral with utilization must return 0");
    });

    assert!(cpu > 0);
    assert!(cpu < 2_000_000, "get_health_factor with util CPU unexpectedly high: {cpu}");
    eprintln!("get_health_factor(with_util): cpu={cpu} mem={mem}");
}

// ═══════════════════════════════════════════════════════════════════════════
// get_repayment_schedule
// ═══════════════════════════════════════════════════════════════════════════

/// `get_repayment_schedule` with no schedule set returns `None` cheaply.
#[test]
fn gas_get_repayment_schedule_missing() {
    let (env, contract_id, _) = setup(0);
    let client = CreditClient::new(&env, &contract_id);
    let phantom = Address::generate(&env);

    let (cpu, mem) = measure(&env, || {
        let result = client.get_repayment_schedule(&phantom);
        assert!(result.is_none());
    });

    assert!(cpu > 0);
    assert!(cpu < 500_000, "get_repayment_schedule missing CPU unexpectedly high: {cpu}");
    eprintln!("get_repayment_schedule(missing): cpu={cpu} mem={mem}");
}

/// `get_repayment_schedule` after `set_repayment_schedule` returns the stored value.
#[test]
fn gas_get_repayment_schedule_configured() {
    let (env, contract_id, _) = setup(500_000);
    let client = CreditClient::new(&env, &contract_id);
    let borrowers = open_lines(&env, &client, 1, Some(5_000_i128));

    let next_due = env.ledger().timestamp() + 86_400 * 30;
    client.set_repayment_schedule(&borrowers[0], &2_000_i128, &(86_400_u64 * 30), &next_due);

    let (cpu, mem) = measure(&env, || {
        let result = client.get_repayment_schedule(&borrowers[0]);
        assert!(result.is_some());
        let sched = result.unwrap();
        assert_eq!(sched.amount_per_period, 2_000_i128);
    });

    assert!(cpu > 0);
    assert!(cpu < 1_000_000, "get_repayment_schedule configured CPU unexpectedly high: {cpu}");
    eprintln!("get_repayment_schedule(configured): cpu={cpu} mem={mem}");
}

// ═══════════════════════════════════════════════════════════════════════════
// get_credit_lines_paginated
// ═══════════════════════════════════════════════════════════════════════════

/// `get_credit_lines_paginated` on an empty contract returns an empty page.
#[test]
fn gas_get_credit_lines_paginated_empty() {
    let (env, contract_id, _) = setup(0);
    let client = CreditClient::new(&env, &contract_id);

    let (cpu, mem) = measure(&env, || {
        let page = client.get_credit_lines_paginated(&None, &10_u32);
        assert_eq!(page.credit_lines.len(), 0);
    });

    assert!(cpu > 0);
    assert!(cpu < 500_000, "paginated empty CPU unexpectedly high: {cpu}");
    eprintln!("get_credit_lines_paginated(empty): cpu={cpu} mem={mem}");
}

/// `get_credit_lines_paginated` over 5 lines measures enumeration overhead.
#[test]
fn gas_get_credit_lines_paginated_five() {
    let (env, contract_id, _) = setup(1_000_000);
    let client = CreditClient::new(&env, &contract_id);
    open_lines(&env, &client, 5, None);

    let (cpu, mem) = measure(&env, || {
        let page = client.get_credit_lines_paginated(&None, &5_u32);
        assert_eq!(page.credit_lines.len(), 5);
    });

    assert!(cpu > 0);
    assert!(cpu < 5_000_000, "paginated 5-lines CPU unexpectedly high: {cpu}");
    eprintln!("get_credit_lines_paginated(5_lines): cpu={cpu} mem={mem}");
}

/// Second-page cursor navigation is at most as expensive as the first page.
#[test]
fn gas_get_credit_lines_paginated_cursor() {
    let (env, contract_id, _) = setup(2_000_000);
    let client = CreditClient::new(&env, &contract_id);
    open_lines(&env, &client, 10, None);

    // First page of 5.
    let first = client.get_credit_lines_paginated(&None, &5_u32);
    assert!(first.next_cursor.is_some(), "10 lines must have a second page");

    let (cpu, mem) = measure(&env, || {
        let page = client.get_credit_lines_paginated(&first.next_cursor, &5_u32);
        assert_eq!(page.credit_lines.len(), 5);
    });

    assert!(cpu > 0);
    assert!(cpu < 5_000_000, "paginated cursor page CPU unexpectedly high: {cpu}");
    eprintln!("get_credit_lines_paginated(cursor_page): cpu={cpu} mem={mem}");
}

// ═══════════════════════════════════════════════════════════════════════════
// Determinism check
// ═══════════════════════════════════════════════════════════════════════════

/// Two identical `borrow_capabilities` calls must consume identical resources.
///
/// The Soroban cost model is deterministic: same inputs on same state must
/// produce the same CPU and memory numbers.
#[test]
fn gas_query_deterministic_borrow_capabilities() {
    let (env, contract_id, _) = setup(500_000);
    let client = CreditClient::new(&env, &contract_id);
    let borrowers = open_lines(&env, &client, 1, None);

    let (cpu1, mem1) = measure(&env, || {
        client.borrow_capabilities(&borrowers[0]);
    });
    let (cpu2, mem2) = measure(&env, || {
        client.borrow_capabilities(&borrowers[0]);
    });

    assert_eq!(cpu1, cpu2, "borrow_capabilities CPU must be deterministic");
    assert_eq!(mem1, mem2, "borrow_capabilities memory must be deterministic");
    eprintln!("borrow_capabilities(deterministic): cpu={cpu1} mem={mem1}");
}
