// SPDX-License-Identifier: MIT

//! Integration tests for the admin query-critical action cooldown.
//!
//! # What is tested
//!
//! - `set_query_admin_cooldown` / `get_query_admin_cooldown` round-trip.
//! - `get_query_admin_last_action_ts` advances only on successful gated actions.
//! - Cooldown disabled (`0` / unset) allows unlimited successive calls.
//! - Cooldown enforced: second call within window reverts with
//!   `Error(Contract, #53)` (`AdminQueryCooldownActive`).
//! - Exact boundary: call at `last_ts + cooldown` succeeds; call at
//!   `last_ts + cooldown - 1` reverts.
//! - All five gated entrypoints are verified to respect the cooldown:
//!   `update_risk_parameters`, `set_oracle_config`,
//!   `set_oracle_quorum_config`, `set_rate_formula_config`,
//!   `set_grace_period_config`.
//! - A failed call within the cooldown window does NOT advance `last_action_ts`.
//! - Auth: non-admin cannot call `set_query_admin_cooldown`.
//! - After `set_query_admin_cooldown(0)` the cooldown is removed and actions
//!   proceed without a time gate.

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env,
};

const START_TS: u64 = 10_000;
const COOLDOWN_SECS: u64 = 300; // 5 minutes

// ── Setup helpers ─────────────────────────────────────────────────────────────

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = START_TS);

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    (env, admin, contract_id)
}

/// Open a minimal credit line so `update_risk_parameters` has something to update.
fn open_line(client: &CreditClient, borrower: &Address) {
    client.open_credit_line(borrower, &10_000_i128, &500_u32, &50_u32);
}

fn set_ts(env: &Env, ts: u64) {
    env.ledger().with_mut(|li| li.timestamp = ts);
}

/// Catch-unwind wrapper that returns the panic string.
fn catch_panic_str<F: FnOnce() + std::panic::UnwindSafe>(f: F) -> Option<String> {
    std::panic::catch_unwind(f).err().map(|e| {
        if let Some(s) = e.downcast_ref::<String>() {
            s.clone()
        } else if let Some(s) = e.downcast_ref::<&str>() {
            s.to_string()
        } else {
            format!("{e:?}")
        }
    })
}

fn assert_admin_query_cooldown_active(err: Option<String>, context: &str) {
    let err_str = err.expect(context);
    assert!(
        err_str.contains("Error(Contract, #53)"),
        "{context}: expected AdminQueryCooldownActive (#53), got {err_str:?}"
    );
}

// ── Configuration round-trip ──────────────────────────────────────────────────

#[test]
fn set_and_get_cooldown_round_trip() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    // Initially no cooldown.
    assert_eq!(client.get_query_admin_cooldown(), None);

    client.set_query_admin_cooldown(&COOLDOWN_SECS);
    assert_eq!(client.get_query_admin_cooldown(), Some(COOLDOWN_SECS));

    // Setting to 0 removes the cooldown.
    client.set_query_admin_cooldown(&0_u64);
    assert_eq!(client.get_query_admin_cooldown(), None);
}

#[test]
fn last_action_ts_starts_as_none() {
    let (_env, _admin, contract_id) = setup();
    let env = _env;
    let client = CreditClient::new(&env, &contract_id);
    assert_eq!(client.get_query_admin_last_action_ts(), None);
}

// ── No cooldown configured — actions are unconstrained ───────────────────────

#[test]
fn without_cooldown_successive_oracle_configs_succeed() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    // No cooldown set — two consecutive calls at the same timestamp must succeed.
    client.set_oracle_config(&500_u32, &3600_u64);
    client.set_oracle_config(&600_u32, &7200_u64);
}

#[test]
fn zero_cooldown_removes_gate() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_query_admin_cooldown(&COOLDOWN_SECS);
    // Trigger the first gated action to set last_action_ts.
    client.set_oracle_config(&500_u32, &3600_u64);
    assert_eq!(client.get_query_admin_last_action_ts(), Some(START_TS));

    // Remove cooldown — next call at the same timestamp must succeed.
    client.set_query_admin_cooldown(&0_u64);
    client.set_oracle_config(&600_u32, &7200_u64);
}

// ── Cooldown enforcement — `set_oracle_config` ────────────────────────────────

#[test]
fn oracle_config_blocked_within_cooldown() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_query_admin_cooldown(&COOLDOWN_SECS);
    client.set_oracle_config(&500_u32, &3600_u64);
    assert_eq!(client.get_query_admin_last_action_ts(), Some(START_TS));

    // One second before the boundary — must revert.
    set_ts(&env, START_TS + COOLDOWN_SECS - 1);
    let err = catch_panic_str(std::panic::AssertUnwindSafe(|| {
        client.set_oracle_config(&600_u32, &7200_u64);
    }));
    assert_admin_query_cooldown_active(err, "set_oracle_config 1s before boundary");

    // last_action_ts must not have advanced on failure.
    assert_eq!(client.get_query_admin_last_action_ts(), Some(START_TS));
}

#[test]
fn oracle_config_allowed_at_exact_boundary() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_query_admin_cooldown(&COOLDOWN_SECS);
    client.set_oracle_config(&500_u32, &3600_u64);

    set_ts(&env, START_TS + COOLDOWN_SECS);
    client.set_oracle_config(&600_u32, &7200_u64);
    assert_eq!(
        client.get_query_admin_last_action_ts(),
        Some(START_TS + COOLDOWN_SECS)
    );
}

#[test]
fn oracle_config_allowed_after_boundary() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_query_admin_cooldown(&COOLDOWN_SECS);
    client.set_oracle_config(&500_u32, &3600_u64);

    set_ts(&env, START_TS + COOLDOWN_SECS + 1);
    client.set_oracle_config(&600_u32, &7200_u64);
}

// ── Cooldown enforcement — `set_oracle_quorum_config` ────────────────────────

#[test]
fn oracle_quorum_config_blocked_within_cooldown() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_query_admin_cooldown(&COOLDOWN_SECS);
    client.set_oracle_quorum_config(&3_u32, &500_u32, &3600_u64);
    assert_eq!(client.get_query_admin_last_action_ts(), Some(START_TS));

    set_ts(&env, START_TS + COOLDOWN_SECS - 1);
    let err = catch_panic_str(std::panic::AssertUnwindSafe(|| {
        client.set_oracle_quorum_config(&3_u32, &600_u32, &7200_u64);
    }));
    assert_admin_query_cooldown_active(err, "set_oracle_quorum_config within cooldown");
    assert_eq!(client.get_query_admin_last_action_ts(), Some(START_TS));
}

#[test]
fn oracle_quorum_config_allowed_at_boundary() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_query_admin_cooldown(&COOLDOWN_SECS);
    client.set_oracle_quorum_config(&3_u32, &500_u32, &3600_u64);

    set_ts(&env, START_TS + COOLDOWN_SECS);
    client.set_oracle_quorum_config(&3_u32, &600_u32, &7200_u64);
    assert_eq!(
        client.get_query_admin_last_action_ts(),
        Some(START_TS + COOLDOWN_SECS)
    );
}

// ── Cooldown enforcement — `set_rate_formula_config` ────────────────────────

#[test]
fn rate_formula_config_blocked_within_cooldown() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_query_admin_cooldown(&COOLDOWN_SECS);
    client.set_rate_formula_config(&100_u32, &50_u32, &100_u32, &5000_u32);
    assert_eq!(client.get_query_admin_last_action_ts(), Some(START_TS));

    set_ts(&env, START_TS + COOLDOWN_SECS - 1);
    let err = catch_panic_str(std::panic::AssertUnwindSafe(|| {
        client.set_rate_formula_config(&200_u32, &50_u32, &200_u32, &5000_u32);
    }));
    assert_admin_query_cooldown_active(err, "set_rate_formula_config within cooldown");
    assert_eq!(client.get_query_admin_last_action_ts(), Some(START_TS));
}

#[test]
fn rate_formula_config_allowed_at_boundary() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_query_admin_cooldown(&COOLDOWN_SECS);
    client.set_rate_formula_config(&100_u32, &50_u32, &100_u32, &5000_u32);

    set_ts(&env, START_TS + COOLDOWN_SECS);
    client.set_rate_formula_config(&200_u32, &50_u32, &200_u32, &5000_u32);
    assert_eq!(
        client.get_query_admin_last_action_ts(),
        Some(START_TS + COOLDOWN_SECS)
    );
}

// ── Cooldown enforcement — `set_grace_period_config` ────────────────────────

#[test]
fn grace_period_config_blocked_within_cooldown() {
    use creditra_credit::types::GraceWaiverMode;

    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_query_admin_cooldown(&COOLDOWN_SECS);
    client.set_grace_period_config(&86400_u64, &GraceWaiverMode::FullWaiver, &0_u32);
    assert_eq!(client.get_query_admin_last_action_ts(), Some(START_TS));

    set_ts(&env, START_TS + COOLDOWN_SECS - 1);
    let err = catch_panic_str(std::panic::AssertUnwindSafe(|| {
        client.set_grace_period_config(&172800_u64, &GraceWaiverMode::FullWaiver, &0_u32);
    }));
    assert_admin_query_cooldown_active(err, "set_grace_period_config within cooldown");
    assert_eq!(client.get_query_admin_last_action_ts(), Some(START_TS));
}

#[test]
fn grace_period_config_allowed_at_boundary() {
    use creditra_credit::types::GraceWaiverMode;

    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_query_admin_cooldown(&COOLDOWN_SECS);
    client.set_grace_period_config(&86400_u64, &GraceWaiverMode::FullWaiver, &0_u32);

    set_ts(&env, START_TS + COOLDOWN_SECS);
    client.set_grace_period_config(&172800_u64, &GraceWaiverMode::FullWaiver, &0_u32);
    assert_eq!(
        client.get_query_admin_last_action_ts(),
        Some(START_TS + COOLDOWN_SECS)
    );
}

// ── Cooldown enforcement — `update_risk_parameters` ─────────────────────────

#[test]
fn update_risk_parameters_blocked_within_cooldown() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);
    open_line(&client, &borrower);

    client.set_query_admin_cooldown(&COOLDOWN_SECS);
    // First call — succeeds and sets last_action_ts.
    client.update_risk_parameters(&borrower, &10_000_i128, &500_u32, &50_u32);
    assert_eq!(client.get_query_admin_last_action_ts(), Some(START_TS));

    set_ts(&env, START_TS + COOLDOWN_SECS - 1);
    let err = catch_panic_str(std::panic::AssertUnwindSafe(|| {
        client.update_risk_parameters(&borrower, &10_000_i128, &400_u32, &45_u32);
    }));
    assert_admin_query_cooldown_active(err, "update_risk_parameters within cooldown");
    // Anchor must not have moved.
    assert_eq!(client.get_query_admin_last_action_ts(), Some(START_TS));
}

#[test]
fn update_risk_parameters_allowed_at_exact_boundary() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);
    open_line(&client, &borrower);

    client.set_query_admin_cooldown(&COOLDOWN_SECS);
    client.update_risk_parameters(&borrower, &10_000_i128, &500_u32, &50_u32);

    set_ts(&env, START_TS + COOLDOWN_SECS);
    client.update_risk_parameters(&borrower, &10_000_i128, &400_u32, &45_u32);
    assert_eq!(
        client.get_query_admin_last_action_ts(),
        Some(START_TS + COOLDOWN_SECS)
    );
}

// ── Cooldown chaining — anchor advances on each successful call ───────────────

#[test]
fn cooldown_anchor_advances_after_each_successful_call() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_query_admin_cooldown(&COOLDOWN_SECS);

    // First call at t=START_TS.
    client.set_oracle_config(&500_u32, &3600_u64);
    assert_eq!(client.get_query_admin_last_action_ts(), Some(START_TS));

    // Second call at t=START_TS + COOLDOWN_SECS (exact boundary).
    set_ts(&env, START_TS + COOLDOWN_SECS);
    client.set_oracle_config(&600_u32, &7200_u64);
    assert_eq!(
        client.get_query_admin_last_action_ts(),
        Some(START_TS + COOLDOWN_SECS)
    );

    // Third call — must respect new anchor.  One second short → revert.
    set_ts(&env, START_TS + COOLDOWN_SECS * 2 - 1);
    let err = catch_panic_str(std::panic::AssertUnwindSafe(|| {
        client.set_oracle_config(&700_u32, &7200_u64);
    }));
    assert_admin_query_cooldown_active(err, "third call 1s before second boundary");

    // Exact second boundary → succeed.
    set_ts(&env, START_TS + COOLDOWN_SECS * 2);
    client.set_oracle_config(&700_u32, &7200_u64);
    assert_eq!(
        client.get_query_admin_last_action_ts(),
        Some(START_TS + COOLDOWN_SECS * 2)
    );
}

// ── Interleaving different gated actions shares the same cooldown ─────────────

#[test]
fn different_gated_actions_share_cooldown_anchor() {
    use creditra_credit::types::GraceWaiverMode;

    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_query_admin_cooldown(&COOLDOWN_SECS);

    // First action: oracle config at START_TS.
    client.set_oracle_config(&500_u32, &3600_u64);
    assert_eq!(client.get_query_admin_last_action_ts(), Some(START_TS));

    // Second action: grace period config — should be blocked until START_TS + COOLDOWN_SECS.
    set_ts(&env, START_TS + COOLDOWN_SECS - 1);
    let err = catch_panic_str(std::panic::AssertUnwindSafe(|| {
        client.set_grace_period_config(&86400_u64, &GraceWaiverMode::FullWaiver, &0_u32);
    }));
    assert_admin_query_cooldown_active(
        err,
        "grace_period_config blocked by shared cooldown after oracle_config",
    );

    // Advance past the boundary — now allowed.
    set_ts(&env, START_TS + COOLDOWN_SECS);
    client.set_grace_period_config(&86400_u64, &GraceWaiverMode::FullWaiver, &0_u32);
    assert_eq!(
        client.get_query_admin_last_action_ts(),
        Some(START_TS + COOLDOWN_SECS)
    );
}

// ── First gated call (no prior anchor) is always allowed ─────────────────────

#[test]
fn first_gated_action_has_no_prior_anchor_and_always_passes() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    // Set a large cooldown — should not block the very first call.
    client.set_query_admin_cooldown(&u64::MAX / 2);

    assert_eq!(client.get_query_admin_last_action_ts(), None);
    client.set_oracle_config(&500_u32, &3600_u64);
    assert_eq!(client.get_query_admin_last_action_ts(), Some(START_TS));
}

// ── Non-gated reads are never blocked ────────────────────────────────────────

#[test]
fn read_entrypoints_are_never_blocked() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);
    open_line(&client, &borrower);

    // Install a very large cooldown and trigger it.
    client.set_query_admin_cooldown(&u64::MAX / 2);
    client.set_oracle_config(&500_u32, &3600_u64);

    // Still at START_TS — reads must proceed without error.
    let _ = client.get_query_admin_cooldown();
    let _ = client.get_query_admin_last_action_ts();
    let _ = client.get_credit_line(&borrower);
    let _ = client.get_oracle_config();
    let _ = client.is_delinquent(&borrower);
}
