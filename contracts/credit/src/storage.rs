// SPDX-License-Identifier: MIT

use crate::types::ContractError;
use soroban_sdk::{contracttype, Env, Symbol};

/// Storage keys used in instance and persistent storage.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    /// Address of the liquidity token (SAC or compatible token contract).
    LiquidityToken,
    /// Address of the liquidity source / reserve that funds draws.
    LiquiditySource,
    /// Global emergency switch: when `true`, all `draw_credit` calls revert.
    /// Does not affect repayments. Distinct from per-line `Suspended` status.
    DrawsFrozen,
    MaxDrawAmount,
    /// Minimum interval in seconds required between successive draws for any borrower.
    DrawMinIntervalSeconds,
    /// Per-borrower last successful draw timestamp.
    LastDrawTs(Address),
    /// Per-borrower block flag; when `true`, draw_credit is rejected.
    BlockedBorrower(Address),
    /// Per-borrower max utilization ratio cap in basis points (e.g. 8000 = 80%).
    /// When set, draw_credit enforces: utilized_amount <= credit_limit * cap_bps / 10_000.
    UtilizationCapBps(Address),
}

/// Maximum number of credit lines returned per page.
/// Limits gas consumption and response size for enumeration queries.
pub const MAX_ENUMERATION_LIMIT: u32 = 100;

pub fn admin_key(env: &Env) -> Symbol {
    Symbol::new(env, "admin")
}

pub fn proposed_admin_key(env: &Env) -> Symbol {
    Symbol::new(env, "proposed_admin")
}

pub fn proposed_at_key(env: &Env) -> Symbol {
    Symbol::new(env, "proposed_at")
}

pub fn reentrancy_key(env: &Env) -> Symbol {
    Symbol::new(env, "reentrancy")
}

pub fn rate_cfg_key(env: &Env) -> Symbol {
    Symbol::new(env, "rate_cfg")
}

/// Instance storage key for the risk-score-based rate formula configuration.
pub fn rate_formula_key(env: &Env) -> Symbol {
    Symbol::new(env, "rate_form")
}

/// Instance storage key for the protocol pause flag.
pub fn paused_key(env: &Env) -> Symbol {
    Symbol::new(env, "paused")
}

/// Instance storage key for the grace period configuration.
pub fn grace_period_key(env: &Env) -> Symbol {
    Symbol::new(env, "grace_cfg")
}

/// Assert reentrancy guard is not set; set it for the duration of the call.
///
/// Panics with [`ContractError::Reentrancy`] if the guard is already active,
/// indicating a reentrant call. Caller **must** call [`clear_reentrancy_guard`]
/// on every success and failure path to release the guard.
///
/// # Storage
/// - **Type**: Instance storage (shared TTL with all instance keys)
/// - **Key**: `Symbol("reentrancy")`
/// - **TTL Note**: Guard is functionally temporary (set on entry, cleared on all exits)
///   but stored in instance storage for simplicity. Instance TTL must be maintained
///   separately via `extend_ttl()` calls in frequently-invoked functions.
pub fn set_reentrancy_guard(env: &Env) {
    let key = reentrancy_key(env);
    let current: bool = env.storage().instance().get(&key).unwrap_or(false);
    if current {
        env.panic_with_error(ContractError::Reentrancy);
    }
    env.storage().instance().set(&key, &true);
}

/// Clear the reentrancy guard set by [`set_reentrancy_guard`].
///
/// Must be called on every exit path (success and failure) of any function
/// that called [`set_reentrancy_guard`].
///
/// # Storage
/// - **Type**: Instance storage
/// - **Key**: `Symbol("reentrancy")`
/// - **Value**: `false` (effectively removes the guard)
pub fn clear_reentrancy_guard(env: &Env) {
    env.storage().instance().set(&reentrancy_key(env), &false);
}

/// Check whether a borrower is blocked from drawing credit.
///
/// # Storage
/// - **Type**: Persistent storage (independent TTL per borrower)
/// - **Key**: `DataKey::BlockedBorrower(borrower)`
/// - **TTL Note**: Each borrower's block status has its own TTL, independent
///   of their credit line data. TTL should be extended on access.
pub fn is_borrower_blocked(env: &Env, borrower: &Address) -> bool {
    env.storage()
        .persistent()
        .get(&DataKey::BlockedBorrower(borrower.clone()))
        .unwrap_or(false)
}

/// Set or clear the blocked status for a borrower.
///
/// # Storage
/// - **Type**: Persistent storage (independent TTL per borrower)
/// - **Key**: `DataKey::BlockedBorrower(borrower)`
/// - **TTL Note**: Writes extend the TTL for this specific borrower's block flag.
#[allow(dead_code)]
pub fn set_borrower_blocked(env: &Env, borrower: &Address, blocked: bool) {
    env.storage()
        .persistent()
        .set(&DataKey::BlockedBorrower(borrower.clone()), &blocked);
}

/// Get the configured minimum draw interval in seconds.
pub fn get_draw_min_interval(env: &Env) -> Option<u64> {
    env.storage()
        .instance()
        .get(&DataKey::DrawMinIntervalSeconds)
}

/// Set or clear the configured minimum draw interval in seconds.
pub fn set_draw_min_interval(env: &Env, interval_seconds: u64) {
    if interval_seconds == 0 {
        env.storage().instance().remove(&DataKey::DrawMinIntervalSeconds);
    } else {
        env.storage()
            .instance()
            .set(&DataKey::DrawMinIntervalSeconds, &interval_seconds);
    }
}

/// Get the last successful draw timestamp for a borrower.
pub fn get_last_draw_ts(env: &Env, borrower: &Address) -> Option<u64> {
    env.storage()
        .persistent()
        .get(&DataKey::LastDrawTs(borrower.clone()))
}

/// Record the last successful draw timestamp for a borrower.
pub fn set_last_draw_ts(env: &Env, borrower: &Address, ts: u64) {
    env.storage()
        .persistent()
        .set(&DataKey::LastDrawTs(borrower.clone()), &ts);
}

/// Check whether the protocol is paused.
///
/// # Storage
/// - **Type**: Instance storage (shared TTL with all instance keys)
/// - **Key**: `Symbol("paused")`
/// - **TTL Note**: Shares instance TTL — extend alongside other instance keys.
pub fn is_paused(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&paused_key(env))
        .unwrap_or(false)
}

/// Set the protocol pause state (admin only, enforced by caller).
///
/// # Storage
/// - **Type**: Instance storage (shared TTL with all instance keys)
/// - **Key**: `Symbol("paused")`
/// - **TTL Note**: Shares instance TTL — extend alongside other instance keys.
pub fn set_paused(env: &Env, paused: bool) {
    env.storage().instance().set(&paused_key(env), &paused);
}

/// Assert the protocol is not paused. Reverts with ContractError::Paused if paused.
/// This is the circuit breaker guard injected into all mutating entrypoints except repay_credit.
pub fn assert_not_paused(env: &Env) {
    if is_paused(env) {
        env.panic_with_error(crate::types::ContractError::Paused);
    }
}

/// Instance storage key for the grace period policy.
pub fn grace_period_key(env: &Env) -> Symbol {
    Symbol::new(env, "grace_cfg")
}
