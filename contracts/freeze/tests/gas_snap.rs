// SPDX-License-Identifier: MIT

//! Per-entrypoint CPU/memory gas snapshots for every freeze-related entrypoint.
//!
//! These snapshots establish a regression baseline so that future changes to
//! the freeze module are flagged when they shift CPU or memory consumption
//! beyond the configured tolerance.
//!
//! Run with:
//! ```bash
//! cargo test -p creditra-freeze --test gas_snap
//! ```
//!
//! To accept updated baselines after an intentional change:
//! ```bash
//! cargo test -p creditra-freeze --test gas_snap -- --accept
//! ```

use creditra_freeze::{Credit, CreditClient, FreezeReason};
use soroban_sdk::{
    testutils::{budget::Budget, Address as _, Ledger},
    Address, Env,
};

// ── Snapshot type ────────────────────────────────────────────────────────────

/// Single entrypoint gas snapshot, serialised by `insta` for regression tracking.
#[derive(Debug)]
struct FreezeGasSample {
    entrypoint: &'static str,
    cpu_instructions: u64,
    memory_bytes: u64,
}

fn budget(env: &Env) -> Budget {
    env.cost_estimate().budget()
}

fn measure(env: &Env, f: impl FnOnce()) -> (u64, u64) {
    budget(env).reset_unlimited();
    f();
    let cpu = budget(env).cpu_instruction_cost();
    let mem = budget(env).memory_bytes_cost();
    (cpu, mem)
}

fn snap(entrypoint: &'static str, env: &Env, f: impl FnOnce()) {
    let (cpu, mem) = measure(env, f);
    let sample = FreezeGasSample {
        entrypoint,
        cpu_instructions: cpu,
        memory_bytes: mem,
    };
    insta::assert_debug_snapshot!(entrypoint, sample);
}

// ── Harness ──────────────────────────────────────────────────────────────────

fn setup() -> (Env, CreditClient<'static>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    client.open_credit_line(&borrower, &1_000_000_i128, &300_u32, &50_u32);
    (env, client, admin, borrower)
}

// ═════════════════════════════════════════════════════════════════════════════
//  Gas Snapshots
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn gas_freeze_draws() {
    let (env, client, _admin, _borrower) = setup();
    snap("freeze_draws", &env, || {
        client.freeze_draws();
    });
}

#[test]
fn gas_unfreeze_draws() {
    let (env, client, _admin, _borrower) = setup();
    client.freeze_draws();
    snap("unfreeze_draws", &env, || {
        client.unfreeze_draws();
    });
}

#[test]
fn gas_freeze_credit_line() {
    let (env, client, _admin, borrower) = setup();
    snap("freeze_credit_line", &env, || {
        client.freeze_credit_line(&borrower, &FreezeReason::RiskInvestigation);
    });
}

#[test]
fn gas_unfreeze_credit_line() {
    let (env, client, _admin, borrower) = setup();
    client.freeze_credit_line(&borrower, &FreezeReason::RiskInvestigation);
    snap("unfreeze_credit_line", &env, || {
        client.unfreeze_credit_line(&borrower);
    });
}

#[test]
fn gas_freeze_borrower_until() {
    let (env, client, admin, borrower) = setup();
    let expiry = env.ledger().timestamp() + 3600;
    snap("freeze_borrower_until", &env, || {
        client.freeze_borrower_until(&admin, &borrower, &expiry);
    });
}

#[test]
fn gas_unfreeze_borrower() {
    let (env, client, admin, borrower) = setup();
    let expiry = env.ledger().timestamp() + 3600;
    client.freeze_borrower_until(&admin, &borrower, &expiry);
    snap("unfreeze_borrower", &env, || {
        client.unfreeze_borrower(&admin, &borrower);
    });
}
