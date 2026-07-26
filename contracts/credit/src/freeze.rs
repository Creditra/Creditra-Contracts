// SPDX-License-Identifier: MIT

//! Credit-line and global draw-freeze controls with structured reason taxonomy.
//!
//! Provides admin-only emergency controls that block `draw_credit` while
//! preserving repayment access. Two complementary mechanisms live here:
//!
//! | Mechanism | Scope | Storage | Lifecycle impact |
//! | --------- | ----- | ------- | ---------------- |
//! | Global draw freeze | all borrowers | instance [`DataKey::DrawsFrozen`] | none |
//! | Credit-line freeze | one borrower | persistent [`DataKey::CreditLineFreeze`] | none |
//!
//! Both paths record a [`FreezeReason`] so indexers and governance tooling can
//! classify operational actions without relying on off-chain metadata.
//!
//! # Comparison with other draw blocks
//!
//! | Switch | Scope | Affects repayments | Intended use |
//! | ------ | ----- | ------------------ | ------------ |
//! | `DrawsFrozen` | only `draw_credit` | no | scheduled reserve operations |
//! | `CreditLineFreeze` | one borrower's draws | no | compliance / investigation holds |
//! | `Paused` | every mutating entrypoint except `repay_credit` | no | emergency stop |
//! | `CreditStatus::Suspended` | one line's draws + status | no | lifecycle suspension |
//!
//! # Threat model
//! An attacker with admin credentials could freeze draws to disrupt borrowers.
//! This is mitigated by the same admin-key security requirements that protect
//! all other admin operations. Freeze reasons are emitted on-chain for audit.

use crate::auth::require_admin_auth;
use crate::events::{publish_credit_line_freeze_event, publish_draws_frozen_event};
use crate::storage::{
    enforce_freeze_cooldown, get_credit_line, record_freeze_timestamp_if_cooldown, DataKey,
};
use crate::types::{ContractError, DrawsFreezeState, FreezeReason};
use soroban_sdk::{Address, Env};

/// Freeze all draws globally (admin only).
///
/// Sets [`DataKey::DrawsFrozen`] with `frozen = true` and records `reason`.
///
/// # Storage
/// - **Type**: Instance storage (shared TTL with all instance keys)
/// - **Key**: `DataKey::DrawsFrozen`
/// - **Value**: [`DrawsFreezeState`]
///
/// # Events
/// Emits [`DrawsFrozenEvent`] with `frozen = true` and the supplied `reason`.
pub fn freeze_draws(env: Env, reason: FreezeReason) {
    require_admin_auth(&env);
    enforce_freeze_cooldown(&env);
    env.storage().instance().set(
        &DataKey::DrawsFrozen,
        &DrawsFreezeState {
            frozen: true,
            reason,
        },
    );
    publish_draws_frozen_event(&env, true, reason);
    record_freeze_timestamp_if_cooldown(&env);
}

/// Unfreeze draws globally (admin only).
///
/// Sets `frozen = false` while preserving the last recorded reason for audit reads.
///
/// # Events
/// Emits [`DrawsFrozenEvent`] with `frozen = false` and the last stored reason
/// (defaults to [`FreezeReason::LiquidityReserve`] when never frozen before).
pub fn unfreeze_draws(env: Env) {
    require_admin_auth(&env);
    enforce_freeze_cooldown(&env);
    let reason = get_draws_freeze_state(&env)
        .map(|state| state.reason)
        .unwrap_or(FreezeReason::LiquidityReserve);
    env.storage().instance().set(
        &DataKey::DrawsFrozen,
        &DrawsFreezeState {
            frozen: false,
            reason,
        },
    );
    publish_draws_frozen_event(&env, false, reason);
    record_freeze_timestamp_if_cooldown(&env);
}

/// Returns `true` when draws are globally frozen.
///
/// Defaults to `false` (draws allowed) if the key has never been set.
pub fn is_draws_frozen(env: &Env) -> bool {
    get_draws_freeze_state(env).map_or(false, |state| state.frozen)
}

/// Returns the active global freeze reason, if draws are currently frozen.
pub fn get_draws_freeze_reason(env: &Env) -> Option<FreezeReason> {
    get_draws_freeze_state(env)
        .filter(|state| state.frozen)
        .map(|state| state.reason)
}

/// Freeze a single credit line's draws (admin only).
///
/// Records `reason` under [`DataKey::CreditLineFreeze`] without mutating
/// [`crate::types::CreditStatus`]. Repayments remain available.
///
/// # Errors
/// - [`ContractError::CreditLineNotFound`] when no credit line exists for `borrower`.
///
/// # Events
/// Emits [`CreditLineFreezeEvent`] on `("credit", "line_frz")` with `frozen = true`.
pub fn freeze_credit_line(env: Env, borrower: Address, reason: FreezeReason) {
    require_admin_auth(&env);
    enforce_freeze_cooldown(&env);
    if get_credit_line(&env, &borrower).is_none() {
        env.panic_with_error(ContractError::CreditLineNotFound);
    }
    env.storage()
        .persistent()
        .set(&DataKey::CreditLineFreeze(borrower.clone()), &reason);
    publish_credit_line_freeze_event(&env, &borrower, reason, true);
    record_freeze_timestamp_if_cooldown(&env);
}

/// Lift a per-credit-line draw freeze (admin only).
///
/// No-op when the borrower was not frozen. Repayments were never blocked.
///
/// # Events
/// Emits [`CreditLineFreezeEvent`] with `frozen = false` when a freeze record existed.
pub fn unfreeze_credit_line(env: Env, borrower: Address) {
    require_admin_auth(&env);
    enforce_freeze_cooldown(&env);
    let key = DataKey::CreditLineFreeze(borrower.clone());
    let Some(reason) = env
        .storage()
        .persistent()
        .get::<DataKey, FreezeReason>(&key)
    else {
        return;
    };
    env.storage().persistent().remove(&key);
    publish_credit_line_freeze_event(&env, &borrower, reason, false);
    record_freeze_timestamp_if_cooldown(&env);
}

/// Returns `true` when a credit line has an active admin freeze.
pub fn is_credit_line_frozen(env: &Env, borrower: &Address) -> bool {
    env.storage()
        .persistent()
        .has(&DataKey::CreditLineFreeze(borrower.clone()))
}

/// Returns the structured freeze reason for a credit line, if frozen.
pub fn get_credit_line_freeze_reason(env: &Env, borrower: &Address) -> Option<FreezeReason> {
    env.storage()
        .persistent()
        .get(&DataKey::CreditLineFreeze(borrower.clone()))
}

fn get_draws_freeze_state(env: &Env) -> Option<DrawsFreezeState> {
    env.storage()
        .instance()
        .get::<DataKey, DrawsFreezeState>(&DataKey::DrawsFrozen)
}
