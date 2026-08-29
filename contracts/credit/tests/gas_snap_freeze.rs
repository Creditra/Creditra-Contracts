// SPDX-License-Identifier: MIT

//! Per-entrypoint CPU/memory gas snapshots for the freeze subsystem (issue #872, v7).
//!
//! Establishes a regression baseline for every freeze-related entrypoint on
//! the `creditra-credit` contract. Any change that shifts CPU or memory
//! consumption will cause a snapshot mismatch in CI and require an explicit
//! review + baseline update.
//!
//! # Entrypoints covered
//!
//! ## State-changing (admin-only)
//! | Entrypoint              | Scenarios                                      |
//! |-------------------------|------------------------------------------------|
//! | `freeze_draws`          | baseline (`LiquidityReserve`), per reason, re-freeze |
//! | `unfreeze_draws`        | when frozen, when already unfrozen             |
//! | `freeze_credit_line`    | baseline, re-freeze                            |
//! | `unfreeze_credit_line`  | when frozen, no-op                             |
//!
//! ## Read-only (no auth required)
//! | Entrypoint                      | Scenarios             |
//! |---------------------------------|-----------------------|
//! | `is_draws_frozen`               | true, false           |
//! | `get_draws_freeze_reason`       | `Some`, `None`        |
//! | `is_credit_line_frozen`         | true, false           |
//! | `get_credit_line_freeze_reason` | `Some`, `None`        |
//!
//! Run with:
//! ```bash
//! cargo test -p creditra-credit --test gas_snap_freeze
//! ```
//!
//! To accept updated baselines after an intentional change:
//! ```bash
//! INSTA_UPDATE=always cargo test -p creditra-credit --test gas_snap_freeze
//! ```
//!
//! # See also
//! - `contracts/credit/src/freeze.rs` — the freeze implementation.
//! - `contracts/credit/tests/freeze_auth_snap.rs` — authorization shape tests.
//! - `contracts/credit/tests/risk_gas_snap.rs` — analogous file for the risk module.

use creditra_credit::{Credit, CreditClient, FreezeReason};
use soroban_sdk::{
    testutils::{budget::Budget, Address as _},
    Address, Env,
};

// ── Snapshot type ────────────────────────────────────────────────────────────

/// Single entrypoint gas measurement, serialised by `insta` for regression tracking.
#[derive(Debug)]
struct FreezeGasSample {
    /// Human-readable entrypoint label (doubles as the `insta` snapshot name).
    entrypoint: &'static str,
    /// Soroban CPU instruction count for the call.
    cpu_instructions: u64,
    /// Soroban memory bytes consumed for the call.
    memory_bytes: u64,
}

fn budget(env: &Env) -> Budget {
    env.cost_estimate().budget()
}

/// Reset the budget, execute `f`, then return `(cpu, mem)`.
fn measure(env: &Env, f: impl FnOnce()) -> (u64, u64) {
    budget(env).reset_unlimited();
    f();
    let cpu = budget(env).cpu_instruction_cost();
    let mem = budget(env).memory_bytes_cost();
    (cpu, mem)
}

/// Measure `f` and assert the snapshot matches the stored `insta` baseline.
fn snap(entrypoint: &'static str, env: &Env, f: impl FnOnce()) {
    let (cpu, mem) = measure(env, f);
    let sample = FreezeGasSample {
        entrypoint,
        cpu_instructions: cpu,
        memory_bytes: mem,
    };
    insta::assert_debug_snapshot!(entrypoint, sample);
}

// ── Test harness ─────────────────────────────────────────────────────────────

/// Deploy a fresh contract, initialise `admin`, and open a credit line for
/// `borrower`. `mock_all_auths_allowing_non_root_auth` allows auth recording
/// without real signature verification.
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

/// Baseline cost for `freeze_draws` (first call, `LiquidityReserve` reason).
///
/// Covers: admin auth check, freeze-cooldown check, instance storage write,
/// event emission, and cooldown timestamp recording.
#[test]
fn gas_freeze_draws_baseline() {
    let (env, client, _admin, _borrower) = setup();
    snap("freeze_draws_baseline", &env, || {
        client.freeze_draws(&FreezeReason::LiquidityReserve);
    });
}

/// `freeze_draws` with `Compliance` reason. Enum discriminant differs but the
/// storage path is identical — cost must match the baseline.
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

/// `freeze_draws` with `OperationalMaintenance` reason.
#[test]
fn gas_freeze_draws_operational_maintenance() {
    let (env, client, _admin, _borrower) = setup();
    snap("freeze_draws_operational_maintenance", &env, || {
        client.freeze_draws(&FreezeReason::OperationalMaintenance);
    });
}

/// `freeze_draws` with `BorrowerRequest` reason.
#[test]
fn gas_freeze_draws_borrower_request() {
    let (env, client, _admin, _borrower) = setup();
    snap("freeze_draws_borrower_request", &env, || {
        client.freeze_draws(&FreezeReason::BorrowerRequest);
    });
}

/// Re-freeze when draws are already frozen (idempotent path).
/// Instance storage is overwritten with the same key — same hot-path cost.
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
///
/// Covers: admin auth check, freeze-cooldown check, instance storage read
/// (to preserve reason), instance storage write, event emission, and cooldown
/// timestamp recording.
#[test]
fn gas_unfreeze_draws_when_frozen() {
    let (env, client, _admin, _borrower) = setup();
    client.freeze_draws(&FreezeReason::LiquidityReserve);
    snap("unfreeze_draws_when_frozen", &env, || {
        client.unfreeze_draws();
    });
}

/// `unfreeze_draws` when draws were never frozen (key absent, no-op path).
/// The storage read returns `None`, but a write still occurs.
#[test]
fn gas_unfreeze_draws_when_not_frozen() {
    let (env, client, _admin, _borrower) = setup();
    snap("unfreeze_draws_when_not_frozen", &env, || {
        client.unfreeze_draws();
    });
}

// ═════════════════════════════════════════════════════════════════════════════
//  3. freeze_credit_line — state-changing, admin-only
// ═════════════════════════════════════════════════════════════════════════════

/// Baseline cost for `freeze_credit_line` (first freeze, `Compliance` reason).
///
/// Covers: admin auth check, borrow-cooldown check, `get_credit_line` lookup,
/// persistent storage write, TTL bump, event emission, cooldown recording.
#[test]
fn gas_freeze_credit_line_baseline() {
    let (env, client, _admin, borrower) = setup();
    snap("freeze_credit_line_baseline", &env, || {
        client.freeze_credit_line(&borrower, &FreezeReason::Compliance);
    });
}

/// `freeze_credit_line` with `RiskInvestigation` reason.
#[test]
fn gas_freeze_credit_line_risk_investigation() {
    let (env, client, _admin, borrower) = setup();
    snap("freeze_credit_line_risk_investigation", &env, || {
        client.freeze_credit_line(&borrower, &FreezeReason::RiskInvestigation);
    });
}

/// Re-freeze the same credit line (overwrites the existing persistent key).
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
///
/// Covers: admin auth check, borrow-cooldown check, persistent storage get,
/// persistent storage remove, event emission, cooldown recording.
#[test]
fn gas_unfreeze_credit_line_when_frozen() {
    let (env, client, _admin, borrower) = setup();
    client.freeze_credit_line(&borrower, &FreezeReason::RiskInvestigation);
    snap("unfreeze_credit_line_when_frozen", &env, || {
        client.unfreeze_credit_line(&borrower);
    });
}

/// `unfreeze_credit_line` when no freeze record exists (early-return no-op).
#[test]
fn gas_unfreeze_credit_line_noop() {
    let (env, client, _admin, borrower) = setup();
    snap("unfreeze_credit_line_noop", &env, || {
        client.unfreeze_credit_line(&borrower);
    });
}

// ═════════════════════════════════════════════════════════════════════════════
//  5. is_draws_frozen — read-only query
// ═════════════════════════════════════════════════════════════════════════════

/// Cost when draws are not frozen (instance storage key absent → default `false`).
#[test]
fn gas_is_draws_frozen_false() {
    let (env, client, _admin, _borrower) = setup();
    snap("is_draws_frozen_false", &env, || {
        client.is_draws_frozen();
    });
}

/// Cost when draws are frozen (instance storage key present).
#[test]
fn gas_is_draws_frozen_true() {
    let (env, client, _admin, _borrower) = setup();
    client.freeze_draws(&FreezeReason::LiquidityReserve);
    snap("is_draws_frozen_true", &env, || {
        client.is_draws_frozen();
    });
}

// ═════════════════════════════════════════════════════════════════════════════
//  6. get_draws_freeze_reason — read-only query
// ═════════════════════════════════════════════════════════════════════════════

/// Cost when draws are not frozen (returns `None`).
#[test]
fn gas_get_draws_freeze_reason_none() {
    let (env, client, _admin, _borrower) = setup();
    snap("get_draws_freeze_reason_none", &env, || {
        client.get_draws_freeze_reason();
    });
}

/// Cost when draws are frozen (returns `Some(FreezeReason)`).
#[test]
fn gas_get_draws_freeze_reason_some() {
    let (env, client, _admin, _borrower) = setup();
    client.freeze_draws(&FreezeReason::Compliance);
    snap("get_draws_freeze_reason_some", &env, || {
        client.get_draws_freeze_reason();
    });
}

// ═════════════════════════════════════════════════════════════════════════════
//  7. is_credit_line_frozen — read-only query
// ═════════════════════════════════════════════════════════════════════════════

/// Cost when credit line is not frozen (persistent key absent).
#[test]
fn gas_is_credit_line_frozen_false() {
    let (env, client, _admin, borrower) = setup();
    snap("is_credit_line_frozen_false", &env, || {
        client.is_credit_line_frozen(&borrower);
    });
}

/// Cost when credit line is frozen (persistent key present, TTL bumped on read).
#[test]
fn gas_is_credit_line_frozen_true() {
    let (env, client, _admin, borrower) = setup();
    client.freeze_credit_line(&borrower, &FreezeReason::Compliance);
    snap("is_credit_line_frozen_true", &env, || {
        client.is_credit_line_frozen(&borrower);
    });
}

// ═════════════════════════════════════════════════════════════════════════════
//  8. get_credit_line_freeze_reason — read-only query
// ═════════════════════════════════════════════════════════════════════════════

/// Cost when credit line is not frozen (returns `None`).
#[test]
fn gas_get_credit_line_freeze_reason_none() {
    let (env, client, _admin, borrower) = setup();
    snap("get_credit_line_freeze_reason_none", &env, || {
        client.get_credit_line_freeze_reason(&borrower);
    });
}

/// Cost when credit line is frozen (returns `Some(FreezeReason)`, TTL bumped).
#[test]
fn gas_get_credit_line_freeze_reason_some() {
    let (env, client, _admin, borrower) = setup();
    client.freeze_credit_line(&borrower, &FreezeReason::RiskInvestigation);
    snap("get_credit_line_freeze_reason_some", &env, || {
        client.get_credit_line_freeze_reason(&borrower);
    });
}

// ═════════════════════════════════════════════════════════════════════════════
//  9. Aggregate summary snapshot
// ═════════════════════════════════════════════════════════════════════════════

/// Aggregate snapshot of all freeze entrypoints in a single test.
///
/// This is the canonical CI regression baseline. Any entrypoint whose CPU or
/// memory footprint changes will break this snapshot and require a deliberate
/// review and re-acceptance of the new baseline.
#[test]
fn freeze_gas_summary() {
    let (env, client, _admin, borrower) = setup();

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

    insta::assert_debug_snapshot!("freeze_gas_summary", samples);
}
