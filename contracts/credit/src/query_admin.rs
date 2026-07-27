// SPDX-License-Identifier: MIT

//! Admin cooldown guard for query-critical state-changing entrypoints.
//!
//! # What
//!
//! Certain admin actions directly influence the values returned by read-only
//! query entrypoints such as [`crate::query::get_health_factor`],
//! [`crate::query::is_delinquent`], and the interest rate embedded in
//! [`crate::query::get_credit_line`]:
//!
//! - [`crate::risk::update_risk_parameters`] — changes per-borrower rate & limit
//! - `set_oracle_config` / `set_oracle_quorum_config` — changes oracle validation
//! - `set_rate_formula_config` — changes the global rate-formula coefficients
//! - `set_grace_period_config` — changes the delinquency grace window
//!
//! Rapid automated cycling of these parameters can be used to manipulate query
//! outcomes in ways that harm borrowers or protocol keepers. The cooldown
//! enforces a minimum time gap between consecutive critical actions, analogous
//! to the per-borrower draw cooldown (`DrawMinIntervalSeconds` /
//! [`crate::types::ContractError::DrawCooldownActive`]).
//!
//! # How
//!
//! Two instance-storage slots (see [`crate::storage`]):
//!
//! - `AdminQueryCooldownSeconds` — admin-configurable interval; absent = disabled.
//! - `AdminQueryLastActionTs`    — timestamp of the most recent gated action.
//!
//! The check-and-advance function [`assert_and_advance_query_admin_cooldown`]
//! is called at the **start** of each gated entrypoint (after admin auth, before
//! the mutation). On success it **immediately** records the current timestamp so
//! the next call measures from this point.
//!
//! # Why shared cooldown
//!
//! A single shared cooldown covers all gated actions rather than per-action
//! counters. This prevents an attacker from interleaving different actions to
//! circumvent a per-action cooldown while still letting an admin batch distinct
//! config changes in a single ledger if the cooldown is 0 (disabled).
//!
//! # Storage
//!
//! Both keys live in **instance** storage so they are hot-loaded on every
//! invocation and do not incur an extra Soroban persistent-read cost.

use crate::auth::require_admin_auth;
use crate::storage::{
    assert_not_paused, get_admin_query_cooldown_seconds, get_admin_query_last_action_ts,
    set_admin_query_cooldown_seconds, set_admin_query_last_action_ts,
};
use crate::types::ContractError;
use soroban_sdk::Env;

/// Assert that the admin-query cooldown has elapsed and, if so, advance the
/// last-action timestamp to `now`.
///
/// Must be called after admin auth and before any state mutation in a
/// query-critical entrypoint.
///
/// # Errors
/// Panics with [`ContractError::AdminQueryCooldownActive`] when the configured
/// `AdminQueryCooldownSeconds` interval has not yet elapsed since the last
/// gated action.  No-op when the cooldown is unset (disabled).
pub fn assert_and_advance_query_admin_cooldown(env: &Env) {
    let now = env.ledger().timestamp();

    if let Some(cooldown) = get_admin_query_cooldown_seconds(env) {
        if let Some(last_ts) = get_admin_query_last_action_ts(env) {
            // Revert when now < last_ts + cooldown.
            // saturating_add ensures no wrap-around on extreme timestamps.
            if now < last_ts.saturating_add(cooldown) {
                env.panic_with_error(ContractError::AdminQueryCooldownActive);
            }
        }
        // Record the successful attempt before returning so that the next call
        // measures the gap from this ledger, not from the previous action.
        set_admin_query_last_action_ts(env, now);
    }
    // When cooldown is unset (None), no tracking or gating is applied.
}

/// Set the minimum interval between consecutive admin query critical actions.
///
/// Pass `0` to disable the cooldown entirely.  When disabled, the last-action
/// timestamp is preserved but is not consulted until a non-zero cooldown is
/// configured again.
///
/// # Authorization
/// Requires admin privileges (asserted via [`require_admin_auth`]).
///
/// # Errors
/// - Panics with [`ContractError::Paused`] if the protocol is paused.
/// - Panics with auth error if the caller is not the configured admin.
pub fn set_query_admin_cooldown(env: Env, seconds: u64) {
    assert_not_paused(&env);
    require_admin_auth(&env);
    set_admin_query_cooldown_seconds(&env, seconds);
}

/// Return the configured admin-query cooldown interval in seconds.
///
/// Returns `None` when no cooldown is configured (disabled).
/// Returns `Some(0)` is never stored — passing `0` to
/// [`set_query_admin_cooldown`] removes the key and returns `None` here.
///
/// # Authorization
/// None — pure read.
pub fn get_query_admin_cooldown(env: Env) -> Option<u64> {
    get_admin_query_cooldown_seconds(&env)
}

/// Return the ledger timestamp of the most recent admin query critical action.
///
/// Returns `None` before any gated action has been performed.
///
/// # Authorization
/// None — pure read.
pub fn get_query_admin_last_action_ts(env: Env) -> Option<u64> {
    get_admin_query_last_action_ts(&env)
}
