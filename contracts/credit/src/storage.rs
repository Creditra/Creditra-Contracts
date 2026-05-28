// SPDX-License-Identifier: MIT

use crate::types::{ContractError, CreditLineData};
use soroban_sdk::{contracttype, Address, Env, Symbol};

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
    /// Storage schema version for migration and compatibility checks.
    SchemaVersion,
    /// Monotonic count of unique borrowers that have had a credit line recorded.
    CreditLineCount,
    /// Borrower → stable numeric id used for deterministic enumeration.
    CreditLineIdByBorrower(Address),
    /// Stable numeric id → borrower address.
    CreditLineBorrowerById(u32),
    /// Global sum of every credit line's utilized_amount.
    TotalUtilized,
    MaxDrawAmount,
    MaxRepayAmount,
    /// Minimum interval in seconds required between successive draws for any borrower.
    DrawMinIntervalSeconds,
    /// Per-borrower last successful draw timestamp.
    LastDrawTs(Address),
    /// Per-borrower block flag; when `true`, draw_credit is rejected.
    BlockedBorrower(Address),
    /// Per-borrower max utilization ratio cap in basis points (e.g. 8000 = 80%).
    /// When set, draw_credit enforces: utilized_amount <= credit_limit * cap_bps / 10_000.
    UtilizationCapBps(Address),
    /// Protocol-wide maximum total utilization across all credit lines.
    /// When set, draw_credit reverts if total_utilized + amount > max_total_exposure.
    MaxTotalExposure,
}

/// Maximum number of credit lines returned per page.
/// Limits gas consumption and response size for enumeration queries.
pub const MAX_ENUMERATION_LIMIT: u32 = 100;

// ── Persistent storage TTL policy ────────────────────────────────────────────
//
// Soroban persistent entries can be archived if their TTL is not periodically
// extended. The credit contract stores live per-borrower state in persistent
// storage, so we proactively bump TTL on every read/write path.
//
// `extend_ttl(key, threshold, extend_to)` only writes when the remaining TTL is
// below `threshold`, so we can safely call these helpers frequently.
//
// Numbers below assume ~5 seconds/ledger close.
pub const LEDGER_BUMP_AMOUNT: u32 = 3_110_400; // ~6 months
pub const LEDGER_BUMP_THRESHOLD: u32 = 1_555_200; // ~3 months

/// Instance storage TTL policy (covers global config like admin/liquidity token).
pub const INSTANCE_BUMP_AMOUNT: u32 = LEDGER_BUMP_AMOUNT;
pub const INSTANCE_BUMP_THRESHOLD: u32 = LEDGER_BUMP_THRESHOLD;

pub fn bump_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);
}

fn bump_persistent_ttl<K>(env: &Env, key: &K)
where
    K: soroban_sdk::IntoVal<Env, soroban_sdk::Val>,
{
    bump_instance_ttl(env);
    env.storage()
        .persistent()
        .extend_ttl(key, LEDGER_BUMP_THRESHOLD, LEDGER_BUMP_AMOUNT);
}

/// Bump TTL for the borrower's `CreditLineData` entry (keyed by borrower address).
pub fn bump_credit_line_ttl(env: &Env, borrower: &Address) {
    bump_persistent_ttl(env, borrower);
}

/// Return the credit line for `borrower` and bump TTL if present.
pub fn get_credit_line(env: &Env, borrower: &Address) -> Option<CreditLineData> {
    if env.storage().persistent().has(borrower) {
        bump_credit_line_ttl(env, borrower);
        env.storage().persistent().get(borrower)
    } else {
        None
    }
}

/// Return the configured schema version, if any.
pub fn get_schema_version(env: &Env) -> Option<u32> {
    env.storage().instance().get(&DataKey::SchemaVersion)
}

/// Persist the schema version.
#[allow(dead_code)]
pub fn set_schema_version(env: &Env, version: u32) {
    env.storage()
        .instance()
        .set(&DataKey::SchemaVersion, &version);
}

/// Return the global total utilized accumulator.
pub fn get_total_utilized(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::TotalUtilized)
        .unwrap_or(0)
}

/// Return the number of indexed credit lines.
pub fn get_credit_line_count(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::CreditLineCount)
        .unwrap_or(0)
}

/// Return the configured global exposure cap, if set.
pub fn get_max_total_exposure(env: &Env) -> Option<i128> {
    env.storage().instance().get(&DataKey::MaxTotalExposure)
}

/// Set the global exposure cap. Passing `0` removes the cap.
pub fn set_max_total_exposure(env: &Env, cap: i128) {
    if cap == 0 {
        env.storage().instance().remove(&DataKey::MaxTotalExposure);
    } else {
        env.storage()
            .instance()
            .set(&DataKey::MaxTotalExposure, &cap);
    }
}

/// Return the stable id for a borrower, if present.
pub fn get_credit_line_id(env: &Env, borrower: &Address) -> Option<u32> {
    env.storage()
        .persistent()
        .get(&DataKey::CreditLineIdByBorrower(borrower.clone()))
}

/// Return the borrower for a stable id, if present.
pub fn get_borrower_by_credit_line_id(env: &Env, id: u32) -> Option<Address> {
    env.storage()
        .persistent()
        .get(&DataKey::CreditLineBorrowerById(id))
}

/// Ensure a borrower has a stable enumeration id and return it.
pub fn ensure_credit_line_id(env: &Env, borrower: &Address) -> u32 {
    if let Some(existing_id) = get_credit_line_id(env, borrower) {
        return existing_id;
    }

    let next_id = get_credit_line_count(env);
    env.storage()
        .persistent()
        .set(&DataKey::CreditLineIdByBorrower(borrower.clone()), &next_id);
    env.storage()
        .persistent()
        .set(&DataKey::CreditLineBorrowerById(next_id), borrower);
    env.storage()
        .instance()
        .set(&DataKey::CreditLineCount, &next_id.saturating_add(1));
    next_id
}

/// Adjust the global utilized accumulator by the change in a single credit line.
pub fn adjust_total_utilized(env: &Env, previous_utilized: i128, new_utilized: i128) {
    let delta = new_utilized
        .checked_sub(previous_utilized)
        .unwrap_or_else(|| env.panic_with_error(ContractError::Overflow));
    if delta == 0 {
        return;
    }

    let updated_total = get_total_utilized(env)
        .checked_add(delta)
        .unwrap_or_else(|| env.panic_with_error(ContractError::Overflow));
    env.storage()
        .instance()
        .set(&DataKey::TotalUtilized, &updated_total);
}

/// Persist a credit line and atomically apply its contribution delta to the
/// global total utilized accumulator.
pub fn persist_credit_line(
    env: &Env,
    borrower: &Address,
    line: &CreditLineData,
    previous_utilized: i128,
) {
    ensure_credit_line_id(env, borrower);
    env.storage().persistent().set(borrower, line);
    bump_credit_line_ttl(env, borrower);
    adjust_total_utilized(env, previous_utilized, line.utilized_amount);
}

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

// ── BlockedBorrower Storage Policy ───────────────────────────────────────────
//
// Key: DataKey::BlockedBorrower(Address)
// Type: Persistent (survives archival window; bump on every read/write)
// Value: bool — true = blocked; absent key == not blocked (never store false)
//
// TTL: Bumped to BLOCKED_BORROWER_TTL on every read and write.
// Absence of a key is equivalent to "not blocked"; a restored-but-missing
// key must NOT be treated as blocked.
// ─────────────────────────────────────────────────────────────────────────────
const BLOCKED_BORROWER_TTL: u32 = 3_110_400; // ~6 months at 5 s/ledger
const BLOCKED_BORROWER_BUMP: u32 = 1_555_200; // bump threshold ~3 months

/// Store `borrower` as blocked. Bumps TTL.
pub fn set_borrower_blocked(env: &Env, borrower: &Address) {
    let key = DataKey::BlockedBorrower(borrower.clone());
    env.storage().persistent().set(&key, &true);
    env.storage()
        .persistent()
        .extend_ttl(&key, BLOCKED_BORROWER_BUMP, BLOCKED_BORROWER_TTL);
}

/// Remove the blocked entry for `borrower`. No-op if not blocked (idempotent).
pub fn set_borrower_unblocked(env: &Env, borrower: &Address) {
    let key = DataKey::BlockedBorrower(borrower.clone());
    if env.storage().persistent().has(&key) {
        env.storage().persistent().remove(&key);
    }
}

/// Return true if `borrower` is currently blocked. Bumps TTL on hit.
pub fn is_borrower_blocked(env: &Env, borrower: &Address) -> bool {
    let key = DataKey::BlockedBorrower(borrower.clone());
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, BLOCKED_BORROWER_BUMP, BLOCKED_BORROWER_TTL);
        env.storage().persistent().get(&key).unwrap_or(false)
    } else {
        false
    }
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
        env.storage()
            .instance()
            .remove(&DataKey::DrawMinIntervalSeconds);
    } else {
        env.storage()
            .instance()
            .set(&DataKey::DrawMinIntervalSeconds, &interval_seconds);
    }
}

/// Get the last successful draw timestamp for a borrower.
#[allow(dead_code)]
pub fn get_last_draw_ts(env: &Env, borrower: &Address) -> Option<u64> {
    let key = DataKey::LastDrawTs(borrower.clone());
    if env.storage().persistent().has(&key) {
        bump_persistent_ttl(env, &key);
        env.storage().persistent().get(&key)
    } else {
        None
    }
}

/// Record the last successful draw timestamp for a borrower.
#[allow(dead_code)]
pub fn set_last_draw_ts(env: &Env, borrower: &Address, ts: u64) {
    let key = DataKey::LastDrawTs(borrower.clone());
    env.storage().persistent().set(&key, &ts);
    bump_persistent_ttl(env, &key);
}

/// Get the per-borrower utilization cap in bps (persistent) and bump TTL on hit.
pub fn get_utilization_cap_bps(env: &Env, borrower: &Address) -> Option<u32> {
    let key = DataKey::UtilizationCapBps(borrower.clone());
    if env.storage().persistent().has(&key) {
        bump_persistent_ttl(env, &key);
        env.storage().persistent().get(&key)
    } else {
        None
    }
}

/// Set or clear the per-borrower utilization cap in bps (persistent). Bumps TTL on set.
pub fn set_utilization_cap_bps(env: &Env, borrower: &Address, cap_bps: Option<u32>) {
    let key = DataKey::UtilizationCapBps(borrower.clone());
    match cap_bps {
        Some(v) => {
            env.storage().persistent().set(&key, &v);
            bump_persistent_ttl(env, &key);
        }
        None => {
            if env.storage().persistent().has(&key) {
                env.storage().persistent().remove(&key);
            }
        }
    }
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

/// Assert that a timestamp update is monotonic.
///
/// Reverts if `new_ts <= stored_ts` and `stored_ts != 0`.
/// A `stored_ts` of 0 is treated as "never written" and always passes.
pub fn assert_ts_monotonic(env: &Env, stored_ts: u64, new_ts: u64) {
    if stored_ts != 0 && new_ts <= stored_ts {
        env.panic_with_error(crate::types::ContractError::TimestampRegression);
    }
}
