// SPDX-License-Identifier: MIT

//! Risk admin cooldown management for risk-critical actions.
//!
//! # What
//!
//! Provides time-based circuit-breaker protection for admin actions
//! that modify risk parameters. When a non-zero cooldown is configured,
//! successive calls to risk-mutating entrypoints are rejected until the
//! configured interval has elapsed since the last action.
//!
//! # How
//!
//! The cooldown is stored in instance storage under two keys:
//! - `Symbol("rad_cool")` — cooldown duration in seconds (default `0`, disabled)
//! - `Symbol("rad_last")` — ledger timestamp of the last risk admin action (default `0`)
//!
//! `assert_risk_admin_cooldown_elapsed` is the guard injected into every
//! state-changing risk entrypoint. It reads the configured cooldown, and
//! if non-zero, compares `env.ledger().timestamp()` against the stored
//! last-action timestamp plus the cooldown. When `now < last_ts + cooldown`,
//! the call reverts with [`crate::ContractError::RiskAdminCooldownActive`].
//!
//! `set_last_risk_admin_action_ts` is called at the end of every
//! successful risk-mutation entrypoint to refresh the timestamp.
//!
//! A cooldown of `0` (default) disables enforcement entirely, preserving
//! backward compatibility with contracts that have no cooldown configured.
//!
//! # Why
//!
//! Limits the blast radius of compromised admin keys. An attacker who
//! obtains an admin key can execute at most one risk-mutation per cooldown
//! window, giving time for other admins or monitoring systems to detect
//! and respond.

#![warn(missing_docs)]

use soroban_sdk::Env;

use crate::ContractError;

/// Storage key for the risk admin cooldown duration in seconds.
fn rad_cool_key(env: &Env) -> soroban_sdk::Symbol {
    soroban_sdk::symbol_short!("rad_cool")
}

/// Storage key for the timestamp of the last risk admin action.
fn rad_last_key(env: &Env) -> soroban_sdk::Symbol {
    soroban_sdk::symbol_short!("rad_last")
}

/// Get the configured risk admin cooldown duration in seconds.
///
/// Returns `0` when the cooldown is disabled (default).
///
/// # Arguments
/// * `env` - The Soroban environment.
///
/// # Returns
/// The cooldown duration in seconds, or `0` if disabled.
pub fn get_risk_admin_cooldown_seconds(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&rad_cool_key(env))
        .unwrap_or(0)
}

/// Set the risk admin cooldown duration in seconds.
///
/// Admin-only. Callers must provide their own auth via
/// `require_admin_auth`.
///
/// # Arguments
/// * `env` - The Soroban environment.
/// * `seconds` - The cooldown duration in seconds. Pass `0` to disable.
pub fn set_risk_admin_cooldown_seconds(env: &Env, seconds: u64) {
    env.storage()
        .instance()
        .set(&rad_cool_key(env), &seconds);
}

/// Get the timestamp of the last risk admin action.
///
/// Returns `0` when no risk admin action has been recorded yet.
///
/// # Arguments
/// * `env` - The Soroban environment.
///
/// # Returns
/// The ledger timestamp of the last action, or `0` if none recorded.
pub fn get_last_risk_admin_action_ts(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&rad_last_key(env))
        .unwrap_or(0)
}

/// Set the timestamp of the last risk admin action.
///
/// Admin callers must invoke this at the end of every successful
/// risk-mutation entrypoint so that the cooldown guard can enforce
/// the minimum interval on the next call.
///
/// # Arguments
/// * `env` - The Soroban environment.
/// * `ts` - The ledger timestamp at which the action occurred.
pub fn set_last_risk_admin_action_ts(env: &Env, ts: u64) {
    env.storage()
        .instance()
        .set(&rad_last_key(env), &ts);
}

/// Assert that the risk admin cooldown has elapsed since the last action.
///
/// This is the primary guard injected into every state-changing risk
/// entrypoint. When the cooldown is `0`, the function returns immediately
/// (no enforcement). Otherwise, it reads the stored last-action timestamp,
/// computes `last_ts + cooldown` using saturating arithmetic, and reverts
/// with [`ContractError::RiskAdminCooldownActive`] if the current ledger
/// timestamp has not yet reached that threshold.
///
/// # Panics
/// - With [`ContractError::RiskAdminCooldownActive`] when the cooldown
///   interval has not yet elapsed.
///
/// # Storage
/// - **Keys**: `rad_cool`, `rad_last` (instance storage)
/// - **TTL Note**: Both keys are in instance storage, which shares the
///   contract's TTL with other instance keys. No additional TTL bumps are
///   required here.
pub fn assert_risk_admin_cooldown_elapsed(env: &Env) {
    let cooldown = get_risk_admin_cooldown_seconds(env);
    if cooldown == 0 {
        return;
    }
    let last_ts = get_last_risk_admin_action_ts(env);
    if last_ts == 0 {
        return;
    }
    let now = env.ledger().timestamp();
    if now < last_ts.saturating_add(cooldown) {
        env.panic_with_error(ContractError::RiskAdminCooldownActive);
    }
}