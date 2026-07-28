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
/// # Authorization
/// Requires administrative privileges. The configured admin must authorize this
/// call via `require_auth()`; unauthorized callers are rejected before any
/// storage mutation occurs.
///
/// # Storage
/// - **Type**: Instance storage (shared TTL with all instance keys)
/// - **Key**: `DataKey::DrawsFrozen`
/// - **Value**: [`DrawsFreezeState`]
///
/// # Events
/// Emits [`DrawsFrozenEvent`] with `frozen = true`.
///
/// # Errors
/// - Panics with auth error if the caller is not the configured admin.
pub fn freeze_draws(env: Env) {
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
/// Sets [`DataKey::DrawsFrozen`] to `false`. Idempotent: calling when already
/// unfrozen is a no-op (no event emitted for the redundant call).
///
/// # Authorization
/// Requires administrative privileges. The configured admin must authorize this
/// call via `require_auth()`; unauthorized callers are rejected before any
/// storage mutation occurs.
///
/// # Storage
/// - **Type**: Instance storage (shared TTL with all instance keys)
/// - **Key**: `DataKey::DrawsFrozen`
/// - **TTL Note**: Shares instance TTL — extend alongside other instance keys.
///
/// # Events
/// Emits [`DrawsFrozenEvent`] with `frozen = false`.
///
/// # Errors
/// - Panics with auth error if the caller is not the configured admin.
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
///
/// # Authorization
/// No authentication required — this is a pure read with no side effects.
///
/// # Storage
/// - **Type**: Instance storage (shared TTL with all instance keys)
/// - **Key**: `DataKey::DrawsFrozen`
///
/// # Returns
/// - `true` if draws are frozen
/// - `false` if draws are not frozen or the key has never been set
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
    let key = DataKey::CreditLineFreeze(borrower.clone());
    env.storage().persistent().set(&key, &reason);
    crate::storage::bump_credit_line_freeze_ttl(&env, &borrower);
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
    let key = DataKey::CreditLineFreeze(borrower.clone());
    if env.storage().persistent().has(&key) {
        crate::storage::bump_credit_line_freeze_ttl(env, borrower);
        true
    } else {
        false
    }
}

/// Returns the structured freeze reason for a credit line, if frozen.
pub fn get_credit_line_freeze_reason(env: &Env, borrower: &Address) -> Option<FreezeReason> {
    let key = DataKey::CreditLineFreeze(borrower.clone());
    if env.storage().persistent().has(&key) {
        crate::storage::bump_credit_line_freeze_ttl(env, borrower);
        env.storage().persistent().get(&key)
    } else {
        None
    }
}

fn get_draws_freeze_state(env: &Env) -> Option<DrawsFreezeState> {
    env.storage()
        .instance()
        .get::<DataKey, DrawsFreezeState>(&DataKey::DrawsFrozen)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{LEDGER_BUMP_AMOUNT, LEDGER_BUMP_THRESHOLD};
    use crate::{Credit, CreditClient};
    use soroban_sdk::testutils::storage::Persistent as _;
    use soroban_sdk::testutils::{Address as _, Ledger};

    fn setup(env: &Env) -> (Address, CreditClient<'_>, Address) {
        env.mock_all_auths();
        let admin = Address::generate(env);
        let borrower = Address::generate(env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(env, &contract_id);
        client.init(&admin);
        client.open_credit_line(&borrower, &1_000, &300, &70);
        (contract_id, client, borrower)
    }

    fn ttl_for_key(env: &Env, contract_id: &Address, key: &DataKey) -> u32 {
        env.as_contract(contract_id, || env.storage().persistent().get_ttl(key))
    }

    fn advance_to_ttl_threshold(env: &Env, ttl: u32) {
        env.ledger().with_mut(|ledger| {
            ledger.sequence_number = ledger
                .sequence_number
                .saturating_add(ttl.saturating_sub(LEDGER_BUMP_THRESHOLD - 1));
        });
    }

    #[test]
    fn credit_line_freeze_read_refreshes_persistent_ttl() {
        let env = Env::default();
        let (contract_id, client, borrower) = setup(&env);
        let key = DataKey::CreditLineFreeze(borrower.clone());

        client.freeze_credit_line(&borrower, &FreezeReason::Compliance);
        let initial_ttl = ttl_for_key(&env, &contract_id, &key);
        assert!(initial_ttl >= LEDGER_BUMP_AMOUNT);

        advance_to_ttl_threshold(&env, initial_ttl);
        assert!(client.is_credit_line_frozen(&borrower));
        assert!(ttl_for_key(&env, &contract_id, &key) >= LEDGER_BUMP_AMOUNT);
    }
}
