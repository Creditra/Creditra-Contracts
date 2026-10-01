// SPDX-License-Identifier: MIT

//! Risk admin cooldown management and instance-storage TTL helpers.
//!
//! # What
//!
//! Provides two distinct responsibilities:
//!
//! 1. **Time-based circuit-breaker protection** for admin actions that modify
//!    risk parameters. When a non-zero cooldown is configured, successive calls
//!    to risk-mutating entrypoints are rejected until the configured interval
//!    has elapsed since the last action.
//!
//! 2. **Instance-storage TTL hygiene**. Every storage read and write in this
//!    module calls [`bump_instance_ttl`] so that the contract's instance
//!    storage is never silently archived by the Stellar network — even if only
//!    read-only entrypoints are exercised. This mirrors the identical pattern
//!    in `contracts/credit/src/storage.rs` (`bump_instance_ttl` /
//!    `INSTANCE_BUMP_AMOUNT` / `INSTANCE_BUMP_THRESHOLD`).
//!
//! # TTL policy
//!
//! | Constant                    | Value        | Approximate duration |
//! |-----------------------------|--------------|----------------------|
//! | [`INSTANCE_BUMP_AMOUNT`]    | 3 110 400    | ~6 months            |
//! | [`INSTANCE_BUMP_THRESHOLD`] | 1 555 200    | ~3 months            |
//!
//! The *extend-to* target is always `INSTANCE_BUMP_AMOUNT`. The bump is only
//! written to the ledger when the remaining TTL has dropped below
//! `INSTANCE_BUMP_THRESHOLD`, giving a 2:1 ratio that keeps the average write
//! cost at at most one TTL update every three months for a continuously active
//! contract.
//!
//! # Cooldown storage keys
//!
//! - `Symbol("rad_cool")` — cooldown duration in seconds (`u64`, default `0`,
//!   meaning disabled).
//! - `Symbol("rad_last")` — ledger timestamp of the last risk admin action
//!   (`u64`, default `0`, meaning no prior action).
//!
//! # Guard
//!
//! [`assert_risk_admin_cooldown_elapsed`] is the guard injected into every
//! state-changing risk entrypoint. It reads the configured cooldown, and if
//! non-zero, compares `env.ledger().timestamp()` against the stored
//! last-action timestamp plus the cooldown. When `now < last_ts + cooldown`,
//! the call reverts with [`crate::ContractError::RiskAdminCooldownActive`].
//!
//! [`set_last_risk_admin_action_ts`] is called at the end of every successful
//! risk-mutation entrypoint so that the guard can enforce the minimum interval
//! on the next call.
//!
//! A cooldown of `0` (default) disables enforcement entirely, preserving
//! backward compatibility with contracts that have no cooldown configured.
//!
//! # Why
//!
//! Limits the blast radius of compromised admin keys. An attacker who obtains
//! an admin key can execute at most one risk-mutation per cooldown window,
//! giving time for other admins or monitoring systems to detect and respond.

#![warn(missing_docs)]

use soroban_sdk::Env;

use crate::ContractError;

// ── TTL constants ─────────────────────────────────────────────────────────────

/// Target TTL (in ledgers) that instance storage is extended *to* whenever a
/// bump is needed.
///
/// Derivation (assuming ~5 s / ledger):
/// ```text
/// 6 months = 15 552 000 s  = 3 110 400 ledgers
/// ```
///
/// Matches `INSTANCE_BUMP_AMOUNT` in `contracts/credit/src/storage.rs`.
pub const INSTANCE_BUMP_AMOUNT: u32 = 3_110_400; // ~6 months

/// Remaining-TTL threshold (in ledgers) below which instance storage is
/// extended to [`INSTANCE_BUMP_AMOUNT`].
///
/// Derivation (assuming ~5 s / ledger):
/// ```text
/// 3 months = 7 776 000 s  = 1 555 200 ledgers
/// ```
///
/// The 2:1 ratio between extend-to and threshold keeps the average number of
/// TTL writes at most one per three months for a continuously active contract.
///
/// Matches `INSTANCE_BUMP_THRESHOLD` in `contracts/credit/src/storage.rs`.
pub const INSTANCE_BUMP_THRESHOLD: u32 = 1_555_200; // ~3 months

// ── Instance-storage TTL helper ───────────────────────────────────────────────

/// Extend instance storage TTL to [`INSTANCE_BUMP_AMOUNT`] when the remaining
/// TTL has fallen below [`INSTANCE_BUMP_THRESHOLD`].
///
/// This is a no-op (no ledger write) when the remaining TTL is still above the
/// threshold, so calling it on every read path is cheap for active contracts.
///
/// # Side effects
/// - May write a TTL extension to ledger state.
///
/// # Example
/// ```ignore
/// pub fn my_view(env: Env) -> u64 {
///     bump_instance_ttl(&env);           // keep contract live
///     env.storage().instance().get(&key).unwrap_or(0)
/// }
/// ```
pub fn bump_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);
}

// ── Storage key helpers ───────────────────────────────────────────────────────

/// Storage key for the risk admin cooldown duration in seconds.
///
/// Symbol: `"rad_cool"` (≤ 9 ASCII chars, Soroban `symbol_short!` budget).
fn rad_cool_key() -> soroban_sdk::Symbol {
    soroban_sdk::symbol_short!("rad_cool")
}

/// Storage key for the timestamp of the last risk admin action.
///
/// Symbol: `"rad_last"` (≤ 9 ASCII chars, Soroban `symbol_short!` budget).
fn rad_last_key() -> soroban_sdk::Symbol {
    soroban_sdk::symbol_short!("rad_last")
}

// ── Cooldown getters / setters ────────────────────────────────────────────────

/// Get the configured risk admin cooldown duration in seconds.
///
/// Bumps instance-storage TTL as a side-effect so that read-only call paths
/// keep the contract live. Returns `0` when the cooldown is disabled (default).
///
/// # Arguments
/// * `env` - The Soroban environment.
///
/// # Returns
/// The cooldown duration in seconds, or `0` if disabled.
///
/// # TTL side-effect
/// Extends instance storage TTL to [`INSTANCE_BUMP_AMOUNT`] when remaining TTL
/// is below [`INSTANCE_BUMP_THRESHOLD`].
pub fn get_risk_admin_cooldown_seconds(env: &Env) -> u64 {
    bump_instance_ttl(env);
    env.storage()
        .instance()
        .get(&rad_cool_key())
        .unwrap_or(0)
}

/// Set the risk admin cooldown duration in seconds.
///
/// Admin-only. Callers must provide their own auth via `require_admin_auth`
/// **before** calling this function.
///
/// Bumps instance-storage TTL as a side-effect so that write paths also keep
/// the contract live.
///
/// # Arguments
/// * `env`     - The Soroban environment.
/// * `seconds` - Cooldown duration in seconds. Pass `0` to disable.
///
/// # TTL side-effect
/// Extends instance storage TTL to [`INSTANCE_BUMP_AMOUNT`] when remaining TTL
/// is below [`INSTANCE_BUMP_THRESHOLD`].
pub fn set_risk_admin_cooldown_seconds(env: &Env, seconds: u64) {
    bump_instance_ttl(env);
    env.storage()
        .instance()
        .set(&rad_cool_key(), &seconds);
}

/// Get the timestamp of the last risk admin action.
///
/// Bumps instance-storage TTL as a side-effect. Returns `0` when no risk
/// admin action has been recorded yet (treated as "no prior action").
///
/// # Arguments
/// * `env` - The Soroban environment.
///
/// # Returns
/// The ledger timestamp of the last action, or `0` if none recorded.
///
/// # TTL side-effect
/// Extends instance storage TTL to [`INSTANCE_BUMP_AMOUNT`] when remaining TTL
/// is below [`INSTANCE_BUMP_THRESHOLD`].
pub fn get_last_risk_admin_action_ts(env: &Env) -> u64 {
    bump_instance_ttl(env);
    env.storage()
        .instance()
        .get(&rad_last_key())
        .unwrap_or(0)
}

/// Set the timestamp of the last risk admin action.
///
/// Admin callers must invoke this at the end of every successful risk-mutation
/// entrypoint so that the cooldown guard can enforce the minimum interval on
/// the next call.
///
/// Bumps instance-storage TTL as a side-effect so that write paths also keep
/// the contract live.
///
/// # Arguments
/// * `env` - The Soroban environment.
/// * `ts`  - The ledger timestamp at which the action occurred.
///
/// # TTL side-effect
/// Extends instance storage TTL to [`INSTANCE_BUMP_AMOUNT`] when remaining TTL
/// is below [`INSTANCE_BUMP_THRESHOLD`].
pub fn set_last_risk_admin_action_ts(env: &Env, ts: u64) {
    bump_instance_ttl(env);
    env.storage()
        .instance()
        .set(&rad_last_key(), &ts);
}

// ── Cooldown guard ────────────────────────────────────────────────────────────

/// Assert that the risk admin cooldown has elapsed since the last action.
///
/// This is the primary guard injected into every state-changing risk
/// entrypoint. When the cooldown is `0`, the function returns immediately
/// (no enforcement). Otherwise, it reads the stored last-action timestamp,
/// computes `last_ts + cooldown` using saturating arithmetic, and reverts
/// with [`ContractError::RiskAdminCooldownActive`] if the current ledger
/// timestamp has not yet reached that threshold.
///
/// Bumps instance-storage TTL as a side-effect of the internal reads so that
/// even guard-only call paths keep the contract live.
///
/// # Errors
/// Panics with [`ContractError::RiskAdminCooldownActive`] when the cooldown
/// interval has not yet elapsed.
///
/// # Storage reads
/// - `rad_cool` — cooldown duration in seconds.
/// - `rad_last` — timestamp of the last risk admin action.
///
/// # TTL side-effect
/// Extends instance storage TTL to [`INSTANCE_BUMP_AMOUNT`] (via the internal
/// reads of `rad_cool` and `rad_last`) when remaining TTL is below
/// [`INSTANCE_BUMP_THRESHOLD`].
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
