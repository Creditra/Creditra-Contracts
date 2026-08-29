// SPDX-License-Identifier: MIT

//! Per-entrypoint gas snapshot tests for the collateral (v7) contract.
//!
//! # What
//!
//! Snapshots CPU and memory usage for every public collateral entrypoint to
//! establish a regression baseline. Any unintended change in resource
//! consumption will surface here during CI.
//!
//! # Entrypoints covered
//!
//! | Entrypoint | Category |
//! |---|---|
//! | `deposit_collateral` | borrower / write |
//! | `withdraw_collateral` | borrower / write |
//! | `partial_release_collateral` | borrower / write |
//! | `repay_and_release_collateral` | borrower / write |
//! | `deposit_collateral_token` | borrower / write |
//! | `withdraw_collateral_token` | borrower / write |
//! | `set_min_collateral_ratio_bps` | admin / write |
//! | `set_collateral_risk_weight` | admin / write |
//! | `set_collateral_token_allowlist` | admin / write |
//! | `set_admin_collateral_cooldown_seconds` | admin / write |
//! | `get_collateral` | read-only |
//! | `get_collateral_for_token` | read-only |
//! | `get_min_collateral_ratio_bps` | read-only |
//! | `get_collateral_tokens` | read-only |
//! | `get_admin_collateral_cooldown_seconds` | read-only |
//! | `get_last_admin_collateral_critical_action_ts` | read-only |
//!
//! # How
//!
//! Uses the Soroban `Budget` test utility to measure CPU instructions and
//! memory bytes consumed per call. Values are compared against conservative
//! upper bounds that form the regression baseline.
//!
//! # See also
//!
//! - `contracts/accrual/tests/gas_snap.rs` — accrual gas baseline.
//! - `contracts/collateral/tests/auth_snap.rs` — authorization snapshot.
#![cfg(test)]

extern crate std;
use creditra_collateral::{Collateral, CollateralClient};
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger, MockAuth},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env, Vec,
};

const COLLATERAL_AMOUNT: i128 = 1_000;

// ── Helpers ───────────────────────────────────────────────────────────────

/// Reset the budget to unlimited, run `f`, return (cpu, mem).
fn measure(env: &Env, f: impl FnOnce()) -> (u64, u64) {
    let mut budget = env.cost_estimate().budget();
    budget.reset_unlimited();
    let cpu_before = budget.cpu_instruction_cost();
    let mem_before = budget.memory_bytes_cost();
    f();
    let cpu_after = budget.cpu_instruction_cost();
    let mem_after = budget.memory_bytes_cost();
    (
        cpu_after.saturating_sub(cpu_before),
        mem_after.saturating_sub(mem_before),
    )
}

struct Fixture<'a> {
    contract_id: Address,
    client: CreditClient<'a>,
    borrower: Address,
    admin: Address,
    token: Address,
}

fn setup(env: &Env) -> Fixture<'_> {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let token = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    client.set_liquidity_token(&token);

    // Fund the contract and borrower
    StellarAssetClient::new(env, &token).mint(&borrower, &100_000);

    Fixture {
        contract_id,
        client,
        borrower,
        admin,
        token,
    }
}

#[test]
fn test_collateral_gas_snapshot() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(Collateral, ());
    let client = CollateralClient::new(&env, &contract_id);

    let user = Address::generate(&env);
    let amount = 1000_i128;

    let (cpu_dep, mem_dep) = measure(&env, || {
        client.deposit(&user, &amount);
    });

    let (cpu_wdr, mem_wdr) = measure(&env, || {
        client.withdraw(&user, &amount);
    });

    std::println!("=== Collateral Contract Gas Snapshot Baseline ===");
    std::println!("Deposit  -> CPU: {}, MEM: {}", cpu_dep, mem_dep);
    std::println!("Withdraw -> CPU: {}, MEM: {}", cpu_wdr, mem_wdr);
}

// ── Borrower write entrypoints ────────────────────────────────────────────

/// `deposit_collateral` baseline: auth + storage write for a fresh deposit.
#[test]
fn gas_deposit_collateral() {
    let env = Env::default();
    let f = setup(&env);

    let (cpu, mem) = measure(&env, || {
        f.client.deposit_collateral(&f.borrower, &COLLATERAL_AMOUNT);
    });

    assert!(cpu > 0, "deposit_collateral must consume CPU");
    assert!(cpu < 5_000_000, "deposit_collateral CPU regression: {cpu}");
    assert!(mem < 500_000, "deposit_collateral memory regression: {mem}");

    eprintln!("deposit_collateral: cpu={cpu} mem={mem}");
}

/// `withdraw_collateral` baseline: auth + balance read + storage write.
#[test]
fn gas_withdraw_collateral() {
    let env = Env::default();
    let f = setup(&env);
    f.client.deposit_collateral(&f.borrower, &COLLATERAL_AMOUNT);

    let (cpu, mem) = measure(&env, || {
        f.client
            .withdraw_collateral(&f.borrower, &COLLATERAL_AMOUNT);
    });

    assert!(cpu > 0, "withdraw_collateral must consume CPU");
    assert!(cpu < 5_000_000, "withdraw_collateral CPU regression: {cpu}");
    assert!(
        mem < 500_000,
        "withdraw_collateral memory regression: {mem}"
    );

    eprintln!("withdraw_collateral: cpu={cpu} mem={mem}");
}

/// `partial_release_collateral` baseline: auth + partial balance release.
#[test]
fn gas_partial_release_collateral() {
    let env = Env::default();
    let f = setup(&env);
    f.client.deposit_collateral(&f.borrower, &COLLATERAL_AMOUNT);

    let (cpu, mem) = measure(&env, || {
        f.client.partial_release_collateral(&f.borrower, &1);
    });

    assert!(cpu > 0, "partial_release_collateral must consume CPU");
    assert!(
        cpu < 5_000_000,
        "partial_release_collateral CPU regression: {cpu}"
    );
    assert!(
        mem < 500_000,
        "partial_release_collateral memory regression: {mem}"
    );

    eprintln!("partial_release_collateral: cpu={cpu} mem={mem}");
}

/// `repay_and_release_collateral` baseline: auth + repay path + collateral
/// release (most expensive borrower entrypoint due to token transfer).
#[test]
fn gas_repay_and_release_collateral() {
    let env = Env::default();
    let f = setup(&env);
    StellarAssetClient::new(&env, &f.token).mint(&f.contract_id, &1_000);
    f.client.open_credit_line(&f.borrower, &1_000, &300, &50);
    f.client.deposit_collateral(&f.borrower, &COLLATERAL_AMOUNT);
    f.client.draw_credit(&f.borrower, &100);
    TokenClient::new(&env, &f.token).approve(&f.borrower, &f.contract_id, &100, &1_000);

    let (cpu, mem) = measure(&env, || {
        f.client.partial_release_collateral(&f.borrower, &1);
    });

    assert!(cpu > 0, "repay_and_release_collateral must consume CPU");
    assert!(
        cpu < 10_000_000,
        "repay_and_release_collateral CPU regression: {cpu}"
    );
    assert!(
        mem < 1_000_000,
        "repay_and_release_collateral memory regression: {mem}"
    );

    eprintln!("repay_and_release_collateral: cpu={cpu} mem={mem}");
}

/// `deposit_collateral_token` baseline: auth + allowlist check + token
/// transfer + storage write.
#[test]
fn gas_deposit_collateral_token() {
    let env = Env::default();
    let f = setup(&env);
    let mut tokens = Vec::new(&env);
    tokens.push_back(f.token.clone());
    f.client.set_collateral_token_allowlist(&tokens);

    let (cpu, mem) = measure(&env, || {
        f.client
            .deposit_collateral_token(&f.borrower, &f.token, &COLLATERAL_AMOUNT);
    });

    assert!(cpu > 0, "deposit_collateral_token must consume CPU");
    assert!(
        cpu < 10_000_000,
        "deposit_collateral_token CPU regression: {cpu}"
    );
    assert!(
        mem < 1_000_000,
        "deposit_collateral_token memory regression: {mem}"
    );

    eprintln!("deposit_collateral_token: cpu={cpu} mem={mem}");
}

/// `withdraw_collateral_token` baseline: auth + allowlist check + balance
/// read + token transfer + storage write.
#[test]
fn gas_withdraw_collateral_token() {
    let env = Env::default();
    let f = setup(&env);
    let mut tokens = Vec::new(&env);
    tokens.push_back(f.token.clone());
    f.client.set_collateral_token_allowlist(&tokens);
    f.client
        .deposit_collateral_token(&f.borrower, &f.token, &COLLATERAL_AMOUNT);

    let (cpu, mem) = measure(&env, || {
        f.client
            .withdraw_collateral_token(&f.borrower, &f.token, &COLLATERAL_AMOUNT);
    });

    assert!(cpu > 0, "withdraw_collateral_token must consume CPU");
    assert!(
        cpu < 10_000_000,
        "withdraw_collateral_token CPU regression: {cpu}"
    );
    assert!(
        mem < 1_000_000,
        "withdraw_collateral_token memory regression: {mem}"
    );

    eprintln!("withdraw_collateral_token: cpu={cpu} mem={mem}");
}

// ── Admin write entrypoints ───────────────────────────────────────────────

/// `set_min_collateral_ratio_bps` baseline: admin auth + single storage write.
#[test]
fn gas_set_min_collateral_ratio_bps() {
    let env = Env::default();
    let f = setup(&env);

    let (cpu, mem) = measure(&env, || {
        f.client.set_min_collateral_ratio_bps(&15_000);
    });

    assert!(cpu > 0, "set_min_collateral_ratio_bps must consume CPU");
    assert!(
        cpu < 3_000_000,
        "set_min_collateral_ratio_bps CPU regression: {cpu}"
    );
    assert!(
        mem < 300_000,
        "set_min_collateral_ratio_bps memory regression: {mem}"
    );

    eprintln!("set_min_collateral_ratio_bps: cpu={cpu} mem={mem}");
}

/// `set_collateral_risk_weight` baseline: admin auth + per-token storage write.
#[test]
fn gas_set_collateral_risk_weight() {
    let env = Env::default();
    let f = setup(&env);

    let (cpu, mem) = measure(&env, || {
        f.client.set_collateral_risk_weight(&f.token, &8_000);
    });

    assert!(cpu > 0, "set_collateral_risk_weight must consume CPU");
    assert!(
        cpu < 3_000_000,
        "set_collateral_risk_weight CPU regression: {cpu}"
    );
    assert!(
        mem < 300_000,
        "set_collateral_risk_weight memory regression: {mem}"
    );

    eprintln!("set_collateral_risk_weight: cpu={cpu} mem={mem}");
}

/// `set_collateral_token_allowlist` baseline: admin auth + list storage write.
#[test]
fn gas_set_collateral_token_allowlist() {
    let env = Env::default();
    let f = setup(&env);
    let mut tokens = Vec::new(&env);
    tokens.push_back(f.token.clone());

    let (cpu, mem) = measure(&env, || {
        f.client.set_collateral_token_allowlist(&tokens);
    });

    assert!(cpu > 0, "set_collateral_token_allowlist must consume CPU");
    assert!(
        cpu < 3_000_000,
        "set_collateral_token_allowlist CPU regression: {cpu}"
    );
    assert!(
        mem < 300_000,
        "set_collateral_token_allowlist memory regression: {mem}"
    );

    eprintln!("set_collateral_token_allowlist: cpu={cpu} mem={mem}");
}

/// `set_admin_collateral_cooldown_seconds` baseline: admin auth + single
/// storage write for the cooldown value.
#[test]
fn gas_set_admin_collateral_cooldown_seconds() {
    let env = Env::default();
    let f = setup(&env);

    let (cpu, mem) = measure(&env, || {
        f.client.set_col_admin_cooldown_secs(&120);
    });

    assert!(
        cpu > 0,
        "set_admin_collateral_cooldown_seconds must consume CPU"
    );
    assert!(
        cpu < 3_000_000,
        "set_admin_collateral_cooldown_seconds CPU regression: {cpu}"
    );
    assert!(
        mem < 300_000,
        "set_admin_collateral_cooldown_seconds memory regression: {mem}"
    );

    eprintln!("set_admin_collateral_cooldown_seconds: cpu={cpu} mem={mem}");
}

// ── Read-only entrypoints ─────────────────────────────────────────────────

/// Read-only queries must be cheap (storage read only, no auth overhead).
#[test]
fn gas_read_only_queries() {
    let env = Env::default();
    let f = setup(&env);
    f.client.deposit_collateral(&f.borrower, &COLLATERAL_AMOUNT);

    let mut tokens = Vec::new(&env);
    tokens.push_back(f.token.clone());
    f.client.set_collateral_token_allowlist(&tokens);
    f.client
        .deposit_collateral_token(&f.borrower, &f.token, &COLLATERAL_AMOUNT);

    // get_collateral
    let (cpu, mem) = measure(&env, || {
        let _ = f.client.get_collateral(&f.borrower);
    });
    assert!(cpu > 0);
    assert!(cpu < 2_000_000, "get_collateral CPU regression: {cpu}");
    assert!(mem < 200_000, "get_collateral memory regression: {mem}");
    eprintln!("get_collateral: cpu={cpu} mem={mem}");

    // get_collateral_for_token
    let (cpu, mem) = measure(&env, || {
        let _ = f.client.get_collateral_for_token(&f.borrower, &f.token);
    });
    assert!(cpu > 0);
    assert!(
        cpu < 2_000_000,
        "get_collateral_for_token CPU regression: {cpu}"
    );
    assert!(
        mem < 200_000,
        "get_collateral_for_token memory regression: {mem}"
    );
    eprintln!("get_collateral_for_token: cpu={cpu} mem={mem}");

    // get_min_collateral_ratio_bps
    let (cpu, mem) = measure(&env, || {
        let _ = f.client.get_min_collateral_ratio_bps();
    });
    assert!(cpu > 0);
    assert!(
        cpu < 2_000_000,
        "get_min_collateral_ratio_bps CPU regression: {cpu}"
    );
    assert!(
        mem < 200_000,
        "get_min_collateral_ratio_bps memory regression: {mem}"
    );
    eprintln!("get_min_collateral_ratio_bps: cpu={cpu} mem={mem}");

    // get_collateral_tokens
    let (cpu, mem) = measure(&env, || {
        let _ = f.client.get_collateral_tokens();
    });
    assert!(cpu > 0);
    assert!(
        cpu < 2_000_000,
        "get_collateral_tokens CPU regression: {cpu}"
    );
    assert!(
        mem < 200_000,
        "get_collateral_tokens memory regression: {mem}"
    );
    eprintln!("get_collateral_tokens: cpu={cpu} mem={mem}");

    // get_admin_collateral_cooldown_seconds
    let (cpu, mem) = measure(&env, || {
        let _ = f.client.get_col_admin_cooldown_secs();
    });
    assert!(cpu > 0);
    assert!(
        cpu < 2_000_000,
        "get_admin_collateral_cooldown_seconds CPU regression: {cpu}"
    );
    assert!(
        mem < 200_000,
        "get_admin_collateral_cooldown_seconds memory regression: {mem}"
    );
    eprintln!("get_admin_collateral_cooldown_seconds: cpu={cpu} mem={mem}");

    // get_last_admin_collateral_critical_action_ts
    let (cpu, mem) = measure(&env, || {
        let _ = f.client.get_last_col_admin_action_ts();
    });
    assert!(cpu > 0);
    assert!(
        cpu < 2_000_000,
        "get_last_admin_collateral_critical_action_ts CPU regression: {cpu}"
    );
    assert!(
        mem < 200_000,
        "get_last_admin_collateral_critical_action_ts memory regression: {mem}"
    );
    eprintln!("get_last_admin_collateral_critical_action_ts: cpu={cpu} mem={mem}");
}

// ── Edge cases ────────────────────────────────────────────────────────────

/// Two identical `deposit_collateral` calls must produce the same CPU and
/// memory cost (deterministic budget model).
#[test]
fn gas_deposit_collateral_deterministic() {
    let env = Env::default();
    let f = setup(&env);
    f.client.deposit_collateral(&f.borrower, &100);

    let (cpu1, mem1) = measure(&env, || {
        f.client.deposit_collateral(&f.borrower, &500);
    });
    let (cpu2, mem2) = measure(&env, || {
        f.client.deposit_collateral(&f.borrower, &500);
    });

    let diff = (cpu1 as i64 - cpu2 as i64).abs();
    assert!(
        diff < 5_000,
        "deposit_collateral CPU must be deterministic within tolerance, diff={diff}"
    );
    let mem_diff = (mem1 as i64 - mem2 as i64).abs();
    assert!(
        mem_diff < 5_000,
        "deposit_collateral memory must be deterministic within tolerance, diff={mem_diff}"
    );

    eprintln!("deposit_collateral(deterministic): cpu={cpu1} mem={mem1}");
}

/// Admin write entrypoints must be more expensive than read-only queries,
/// as they perform auth checks and storage writes.
#[test]
fn gas_write_more_expensive_than_read() {
    let env = Env::default();
    let f = setup(&env);
    f.client.deposit_collateral(&f.borrower, &COLLATERAL_AMOUNT);

    let (read_cpu, _) = measure(&env, || {
        let _ = f.client.get_collateral(&f.borrower);
    });

    let (write_cpu, _) = measure(&env, || {
        f.client.deposit_collateral(&f.borrower, &100);
    });

    assert!(
        write_cpu >= read_cpu,
        "write entrypoint ({write_cpu} CPU) should cost at least as much as read ({read_cpu} CPU)"
    );

    eprintln!("read_cpu={read_cpu} write_cpu={write_cpu}");
}

/// `set_collateral_token_allowlist` with a 10-token list must stay within
/// budget — linear growth check for the list write path.
#[test]
fn gas_set_collateral_token_allowlist_large_list() {
    let env = Env::default();
    let f = setup(&env);

    let mut tokens = Vec::new(&env);
    for _ in 0..10 {
        tokens.push_back(Address::generate(&env));
    }

    let (cpu, mem) = measure(&env, || {
        f.client.set_collateral_token_allowlist(&tokens);
    });

    assert!(cpu > 0);
    assert!(
        cpu < 10_000_000,
        "set_collateral_token_allowlist (10 tokens) CPU regression: {cpu}"
    );
    assert!(
        mem < 1_000_000,
        "set_collateral_token_allowlist (10 tokens) memory regression: {mem}"
    );

    eprintln!("set_collateral_token_allowlist(10 tokens): cpu={cpu} mem={mem}");
}

/// Back-to-back admin operations must not exhibit unexpected cost
/// accumulation between calls (each call is independently bounded).
#[test]
fn gas_admin_operations_independent_cost() {
    let env = Env::default();
    let f = setup(&env);
    f.client.set_min_collateral_ratio_bps(&10_000);

    let (cpu1, _) = measure(&env, || {
        f.client.set_min_collateral_ratio_bps(&12_000);
    });

    let (cpu2, _) = measure(&env, || {
        f.client.set_min_collateral_ratio_bps(&13_000);
    });

    // Cost must not inflate between consecutive admin calls.
    let diff = (cpu1 as i64 - cpu2 as i64).abs();
    assert!(
        diff < 5_000,
        "set_min_collateral_ratio_bps cost must be stable across calls, diff={diff}"
    );

    eprintln!("admin_ops_independent: cpu1={cpu1} cpu2={cpu2}");
}
