// SPDX-License-Identifier: MIT

//! Per-entrypoint CPU/memory gas snapshots for every freeze-related entrypoint.
//!
//! These snapshots establish a regression baseline so that future changes to
//! the freeze module are flagged when they shift CPU or memory consumption
//! beyond the configured tolerance.
//!
//! # Entrypoints covered
//!
//! ## State-changing (admin-only)
//! | Entrypoint              | Scenario(s)                       |
//! |-------------------------|-----------------------------------|
//! | `freeze_draws`          | baseline, each `FreezeReason`     |
//! | `unfreeze_draws`        | baseline                          |
//! | `freeze_credit_line`    | baseline, each `FreezeReason`     |
//! | `unfreeze_credit_line`  | baseline                          |
//! | `freeze_borrower_until` | baseline                          |
//! | `unfreeze_borrower`     | baseline                          |
//!
//! ## Read-only (no auth required)
//! | Entrypoint                    | Scenario(s)                   |
//! |-------------------------------|-------------------------------|
//! | `is_draws_frozen`             | when frozen, when not frozen  |
//! | `get_draws_freeze_reason`     | when frozen, when not frozen  |
//! | `is_credit_line_frozen`       | when frozen, when not frozen  |
//! | `get_credit_line_freeze_reason` | when frozen, when not frozen |
//! | `is_borrower_frozen`          | when frozen, when not frozen  |
//! | `get_borrower_frozen_until`   | when set, when not set        |
//!
//! Run with:
//! ```bash
//! cargo test -p creditra-freeze --test gas_snap
//! ```
//!
//! To accept updated baselines after an intentional change:
//! ```bash
//! INSTA_UPDATE=always cargo test -p creditra-freeze --test gas_snap
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

/// Reset the budget, run `f`, and return consumed CPU + memory.
fn measure(env: &Env, f: impl FnOnce()) -> (u64, u64) {
    budget(env).reset_unlimited();
    f();
    let cpu = budget(env).cpu_instruction_cost();
    let mem = budget(env).memory_bytes_cost();
    (cpu, mem)
}

/// Measure `f` and assert the snapshot matches the stored baseline.
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

/// Deploy a fresh contract, initialise `admin`, and open a credit line for
/// `borrower`. Uses `mock_all_auths_allowing_non_root_auth` so auth recording
/// is active while signature verification is skipped.
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
//  1. freeze_draws — state-changing, admin-only
// ═════════════════════════════════════════════════════════════════════════════

/// Baseline cost for `freeze_draws` with `LiquidityReserve` reason.
#[test]
fn gas_freeze_draws() {
    let (env, client, _admin, _borrower) = setup();
    snap("freeze_draws", &env, || {
        client.freeze_draws(&FreezeReason::LiquidityReserve);
    });
}

/// `freeze_draws` with `Compliance` reason should have the same cost as
/// the baseline — reason is encoded as a compact enum discriminant.
#[test]
fn gas_freeze_draws_compliance() {
    let (env, client, _admin, _borrower) = setup();
    snap("freeze_draws_compliance", &env, || {
        client.freeze_draws(&FreezeReason::Compliance);
    });
}

/// `freeze_draws` with `RiskInvestigation` reason.
#[test]
fn gas_freeze_draws_risk_investigation() {
    let (env, client, _admin, _borrower) = setup();
    snap("freeze_draws_risk_investigation", &env, || {
        client.freeze_draws(&FreezeReason::RiskInvestigation);
    });
}

/// `freeze_draws` when already frozen (idempotent re-freeze).
#[test]
fn gas_freeze_draws_idempotent() {
    let (env, client, _admin, _borrower) = setup();
    client.freeze_draws(&FreezeReason::LiquidityReserve);
    snap("freeze_draws_idempotent", &env, || {
        client.freeze_draws(&FreezeReason::Compliance);
    });
}

// ═════════════════════════════════════════════════════════════════════════════
//  2. unfreeze_draws — state-changing, admin-only
// ═════════════════════════════════════════════════════════════════════════════

/// Baseline cost for `unfreeze_draws` when draws are currently frozen.
#[test]
fn gas_unfreeze_draws() {
    let (env, client, _admin, _borrower) = setup();
    client.freeze_draws(&FreezeReason::LiquidityReserve);
    snap("unfreeze_draws", &env, || {
        client.unfreeze_draws();
    });
}

/// `unfreeze_draws` when draws are already unfrozen (no-op path).
#[test]
fn gas_unfreeze_draws_already_unfrozen() {
    let (env, client, _admin, _borrower) = setup();
    snap("unfreeze_draws_already_unfrozen", &env, || {
        client.unfreeze_draws();
    });
}

// ═════════════════════════════════════════════════════════════════════════════
//  3. freeze_credit_line — state-changing, admin-only
// ═════════════════════════════════════════════════════════════════════════════

/// Baseline cost for `freeze_credit_line` with `RiskInvestigation` reason.
#[test]
fn gas_freeze_credit_line() {
    let (env, client, _admin, borrower) = setup();
    snap("freeze_credit_line", &env, || {
        client.freeze_credit_line(&borrower, &FreezeReason::RiskInvestigation);
    });
}

/// `freeze_credit_line` with `Compliance` reason.
#[test]
fn gas_freeze_credit_line_compliance() {
    let (env, client, _admin, borrower) = setup();
    snap("freeze_credit_line_compliance", &env, || {
        client.freeze_credit_line(&borrower, &FreezeReason::Compliance);
    });
}

/// Re-freeze the same credit line (overwrites existing reason in storage).
#[test]
fn gas_freeze_credit_line_idempotent() {
    let (env, client, _admin, borrower) = setup();
    client.freeze_credit_line(&borrower, &FreezeReason::Compliance);
    snap("freeze_credit_line_idempotent", &env, || {
        client.freeze_credit_line(&borrower, &FreezeReason::RiskInvestigation);
    });
}

// ═════════════════════════════════════════════════════════════════════════════
//  4. unfreeze_credit_line — state-changing, admin-only
// ═════════════════════════════════════════════════════════════════════════════

/// Baseline cost for `unfreeze_credit_line` when credit line is frozen.
#[test]
fn gas_unfreeze_credit_line() {
    let (env, client, _admin, borrower) = setup();
    client.freeze_credit_line(&borrower, &FreezeReason::RiskInvestigation);
    snap("unfreeze_credit_line", &env, || {
        client.unfreeze_credit_line(&borrower);
    });
}

/// `unfreeze_credit_line` when no freeze exists (no-op path).
#[test]
fn gas_unfreeze_credit_line_noop() {
    let (env, client, _admin, borrower) = setup();
    snap("unfreeze_credit_line_noop", &env, || {
        client.unfreeze_credit_line(&borrower);
    });
}

// ═════════════════════════════════════════════════════════════════════════════
//  5. freeze_borrower_until — state-changing, admin-only
// ═════════════════════════════════════════════════════════════════════════════

/// Baseline cost for `freeze_borrower_until` with a 1-hour expiry.
#[test]
fn gas_freeze_borrower_until() {
    let (env, client, admin, borrower) = setup();
    let expiry = env.ledger().timestamp() + 3_600_u64;
    snap("freeze_borrower_until", &env, || {
        client.freeze_borrower_until(&admin, &borrower, &expiry);
    });
}

/// `freeze_borrower_until` with a longer expiry (24 hours) — same code path.
#[test]
fn gas_freeze_borrower_until_24h() {
    let (env, client, admin, borrower) = setup();
    let expiry = env.ledger().timestamp() + 86_400_u64;
    snap("freeze_borrower_until_24h", &env, || {
        client.freeze_borrower_until(&admin, &borrower, &expiry);
    });
}

// ═════════════════════════════════════════════════════════════════════════════
//  6. unfreeze_borrower — state-changing, admin-only
// ═════════════════════════════════════════════════════════════════════════════

/// Baseline cost for `unfreeze_borrower` when a freeze record exists.
#[test]
fn gas_unfreeze_borrower() {
    let (env, client, admin, borrower) = setup();
    let expiry = env.ledger().timestamp() + 3_600_u64;
    client.freeze_borrower_until(&admin, &borrower, &expiry);
    snap("unfreeze_borrower", &env, || {
        client.unfreeze_borrower(&admin, &borrower);
    });
}

/// `unfreeze_borrower` when no freeze exists (no-op path).
#[test]
fn gas_unfreeze_borrower_noop() {
    let (env, client, admin, borrower) = setup();
    snap("unfreeze_borrower_noop", &env, || {
        client.unfreeze_borrower(&admin, &borrower);
    });
}

// ═════════════════════════════════════════════════════════════════════════════
//  7. is_draws_frozen — read-only query
// ═════════════════════════════════════════════════════════════════════════════

/// Read cost when draws are not frozen (default state, key absent).
#[test]
fn gas_is_draws_frozen_false() {
    let (env, client, _admin, _borrower) = setup();
    snap("is_draws_frozen_false", &env, || {
        client.is_draws_frozen();
    });
}

/// Read cost when draws are frozen (key present).
#[test]
fn gas_is_draws_frozen_true() {
    let (env, client, _admin, _borrower) = setup();
    client.freeze_draws(&FreezeReason::LiquidityReserve);
    snap("is_draws_frozen_true", &env, || {
        client.is_draws_frozen();
    });
}

// ═════════════════════════════════════════════════════════════════════════════
//  8. get_draws_freeze_reason — read-only query
// ═════════════════════════════════════════════════════════════════════════════

/// Read cost when draws are not frozen (returns `None`).
#[test]
fn gas_get_draws_freeze_reason_none() {
    let (env, client, _admin, _borrower) = setup();
    snap("get_draws_freeze_reason_none", &env, || {
        client.get_draws_freeze_reason();
    });
}

/// Read cost when draws are frozen (returns `Some(FreezeReason)`).
#[test]
fn gas_get_draws_freeze_reason_some() {
    let (env, client, _admin, _borrower) = setup();
    client.freeze_draws(&FreezeReason::Compliance);
    snap("get_draws_freeze_reason_some", &env, || {
        client.get_draws_freeze_reason();
    });
}

// ═════════════════════════════════════════════════════════════════════════════
//  9. is_credit_line_frozen — read-only query
// ═════════════════════════════════════════════════════════════════════════════

/// Read cost when credit line is not frozen (key absent).
#[test]
fn gas_is_credit_line_frozen_false() {
    let (env, client, _admin, borrower) = setup();
    snap("is_credit_line_frozen_false", &env, || {
        client.is_credit_line_frozen(&borrower);
    });
}

/// Read cost when credit line is frozen (key present, TTL bumped on access).
#[test]
fn gas_is_credit_line_frozen_true() {
    let (env, client, _admin, borrower) = setup();
    client.freeze_credit_line(&borrower, &FreezeReason::Compliance);
    snap("is_credit_line_frozen_true", &env, || {
        client.is_credit_line_frozen(&borrower);
    });
}

// ═════════════════════════════════════════════════════════════════════════════
//  10. get_credit_line_freeze_reason — read-only query
// ═════════════════════════════════════════════════════════════════════════════

/// Read cost when credit line is not frozen (returns `None`).
#[test]
fn gas_get_credit_line_freeze_reason_none() {
    let (env, client, _admin, borrower) = setup();
    snap("get_credit_line_freeze_reason_none", &env, || {
        client.get_credit_line_freeze_reason(&borrower);
    });
}

/// Read cost when credit line is frozen (returns `Some(FreezeReason)`).
#[test]
fn gas_get_credit_line_freeze_reason_some() {
    let (env, client, _admin, borrower) = setup();
    client.freeze_credit_line(&borrower, &FreezeReason::RiskInvestigation);
    snap("get_credit_line_freeze_reason_some", &env, || {
        client.get_credit_line_freeze_reason(&borrower);
    });
}

// ═════════════════════════════════════════════════════════════════════════════
//  11. is_borrower_frozen — read-only query
// ═════════════════════════════════════════════════════════════════════════════

/// Read cost when borrower is not frozen (key absent).
#[test]
fn gas_is_borrower_frozen_false() {
    let (env, client, _admin, borrower) = setup();
    snap("is_borrower_frozen_false", &env, || {
        client.is_borrower_frozen(&borrower);
    });
}

/// Read cost when borrower has an active temporary freeze.
#[test]
fn gas_is_borrower_frozen_true() {
    let (env, client, admin, borrower) = setup();
    let expiry = env.ledger().timestamp() + 3_600_u64;
    client.freeze_borrower_until(&admin, &borrower, &expiry);
    snap("is_borrower_frozen_true", &env, || {
        client.is_borrower_frozen(&borrower);
    });
}

/// Read cost when borrower's freeze has expired (returns `false`, key remains).
#[test]
fn gas_is_borrower_frozen_expired() {
    let (env, client, admin, borrower) = setup();
    let expiry = env.ledger().timestamp() + 3_600_u64;
    client.freeze_borrower_until(&admin, &borrower, &expiry);
    // Advance time past expiry
    env.ledger().with_mut(|l| l.timestamp += 7_200_u64);
    snap("is_borrower_frozen_expired", &env, || {
        client.is_borrower_frozen(&borrower);
    });
}

// ═════════════════════════════════════════════════════════════════════════════
//  12. get_borrower_frozen_until — read-only query
// ═════════════════════════════════════════════════════════════════════════════

/// Read cost when no freeze record exists (returns `None`).
#[test]
fn gas_get_borrower_frozen_until_none() {
    let (env, client, _admin, borrower) = setup();
    snap("get_borrower_frozen_until_none", &env, || {
        client.get_borrower_frozen_until(&borrower);
    });
}

/// Read cost when a freeze record exists (returns `Some(expiry_ts)`).
#[test]
fn gas_get_borrower_frozen_until_some() {
    let (env, client, admin, borrower) = setup();
    let expiry = env.ledger().timestamp() + 3_600_u64;
    client.freeze_borrower_until(&admin, &borrower, &expiry);
    snap("get_borrower_frozen_until_some", &env, || {
        client.get_borrower_frozen_until(&borrower);
    });
}

// ═════════════════════════════════════════════════════════════════════════════
//  13. Aggregate summary snapshot of all freeze entrypoints
// ═════════════════════════════════════════════════════════════════════════════

/// Aggregate snapshot of all freeze entrypoints in a single test.
///
/// This is the canonical regression baseline for CI: any entrypoint that
/// changes its CPU or memory footprint will fail this snapshot and require
/// an explicit review and snapshot update.
#[test]
fn freeze_gas_summary() {
    let (env, client, admin, borrower) = setup();

    let mut samples = std::collections::BTreeMap::new();

    macro_rules! measure_one {
        ($name:expr, $body:block) => {{
            let (cpu, mem) = measure(&env, || $body);
            samples.insert($name, (cpu, mem));
        }};
    }

    // — state-changing entrypoints —
    measure_one!("freeze_draws", {
        client.freeze_draws(&FreezeReason::LiquidityReserve);
    });
    measure_one!("unfreeze_draws", {
        client.unfreeze_draws();
    });
    measure_one!("freeze_credit_line", {
        client.freeze_credit_line(&borrower, &FreezeReason::RiskInvestigation);
    });
    measure_one!("unfreeze_credit_line", {
        client.unfreeze_credit_line(&borrower);
    });
    let expiry = env.ledger().timestamp() + 3_600_u64;
    measure_one!("freeze_borrower_until", {
        client.freeze_borrower_until(&admin, &borrower, &expiry);
    });
    measure_one!("unfreeze_borrower", {
        client.unfreeze_borrower(&admin, &borrower);
    });

    // — read-only queries —
    measure_one!("is_draws_frozen", {
        client.is_draws_frozen();
    });
    measure_one!("get_draws_freeze_reason", {
        client.get_draws_freeze_reason();
    });
    measure_one!("is_credit_line_frozen", {
        client.is_credit_line_frozen(&borrower);
    });
    measure_one!("get_credit_line_freeze_reason", {
        client.get_credit_line_freeze_reason(&borrower);
    });
    measure_one!("is_borrower_frozen", {
        client.is_borrower_frozen(&borrower);
    });
    measure_one!("get_borrower_frozen_until", {
        client.get_borrower_frozen_until(&borrower);
    });

    insta::assert_debug_snapshot!("freeze_gas_summary", samples);
}
