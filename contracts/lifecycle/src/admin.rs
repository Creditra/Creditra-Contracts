// SPDX-License-Identifier: MIT

//! Admin lifecycle configuration with a cool-off between critical actions (v7).
//!
//! Critical admin entrypoints (`set_credit_limit_bounds`,
//! `set_per_borrower_liquidation_grace`, `set_repayment_schedule`,
//! `set_late_fee_flat`, `set_late_fee_config`) share a single
//! cooldown clock stored in instance storage. The interval is configured via
//! [`set_admin_lifecycle_cooldown_seconds`]; when unset or zero, the guard is
//! disabled.

use crate::auth::require_admin_auth;
use crate::storage::{self, assert_not_paused};
use crate::types::{ContractError, LateFeeConfig};
use soroban_sdk::{Address, Env};

/// Enforce the configured cool-off since the last critical lifecycle admin action.
pub fn enforce_admin_lifecycle_cooldown(env: &Env) {
    let Some(cooldown_secs) = storage::get_admin_lifecycle_cooldown_seconds(env) else {
        return;
    };
    if cooldown_secs == 0 {
        return;
    }
    if let Some(last_ts) = storage::get_last_admin_lifecycle_critical_action_ts(env) {
        let now = env.ledger().timestamp();
        if now < last_ts.saturating_add(cooldown_secs) {
            env.panic_with_error(ContractError::AdminLifecycleCooldownActive);
        }
    }
}

/// Record the ledger timestamp of a successful critical lifecycle admin action.
pub fn touch_admin_lifecycle_critical_action_ts(env: &Env) {
    let now = env.ledger().timestamp();
    storage::set_last_admin_lifecycle_critical_action_ts(env, now);
}

/// Set the minimum interval between critical lifecycle admin actions (admin only).
///
/// Pass `0` to disable the cool-off guard.
pub fn set_admin_lifecycle_cooldown_seconds(env: &Env, seconds: u64) {
    assert_not_paused(env);
    require_admin_auth(env);
    storage::set_admin_lifecycle_cooldown_seconds(env, seconds);
}

/// Return the configured admin lifecycle cool-off interval, if set.
pub fn get_admin_lifecycle_cooldown_seconds(env: &Env) -> Option<u64> {
    storage::get_admin_lifecycle_cooldown_seconds(env)
}

/// Return the ledger timestamp of the last critical lifecycle admin action, if any.
pub fn get_last_admin_lifecycle_critical_action_ts(env: &Env) -> Option<u64> {
    storage::get_last_admin_lifecycle_critical_action_ts(env)
}

/// Set the credit limit bounds (admin only).
pub fn set_credit_limit_bounds(env: &Env, min: i128, max: i128) {
    assert_not_paused(env);
    require_admin_auth(env);
    enforce_admin_lifecycle_cooldown(env);
    crate::lifecycle::set_credit_limit_bounds(env.clone(), min, max);
    touch_admin_lifecycle_critical_action_ts(env);
}

/// Set or update the per-borrower liquidation grace period in seconds (admin only).
pub fn set_per_borrower_liquidation_grace(
    env: &Env,
    borrower: Address,
    grace_period_seconds: u64,
) {
    assert_not_paused(env);
    require_admin_auth(env);
    enforce_admin_lifecycle_cooldown(env);
    crate::lifecycle::set_per_borrower_liquidation_grace(env, borrower, grace_period_seconds);
    touch_admin_lifecycle_critical_action_ts(env);
}

/// Set the repayment schedule for a borrower (admin only).
pub fn set_repayment_schedule(
    env: &Env,
    borrower: Address,
    amount_per_period: i128,
    period_seconds: u64,
    first_due_ts: u64,
) {
    assert_not_paused(env);
    require_admin_auth(env);
    enforce_admin_lifecycle_cooldown(env);
    crate::lifecycle::set_repayment_schedule(
        env,
        borrower,
        amount_per_period,
        period_seconds,
        first_due_ts,
    );
    touch_admin_lifecycle_critical_action_ts(env);
}

/// Set the flat late fee per missed installment (admin only).
pub fn set_late_fee_flat(env: &Env, fee: i128) {
    assert_not_paused(env);
    require_admin_auth(env);
    enforce_admin_lifecycle_cooldown(env);
    crate::lifecycle::set_late_fee_flat(env.clone(), fee);
    touch_admin_lifecycle_critical_action_ts(env);
}

/// Set the structured late-fee configuration (admin only).
pub fn set_late_fee_config(env: &Env, config: Option<LateFeeConfig>) {
    assert_not_paused(env);
    require_admin_auth(env);
    enforce_admin_lifecycle_cooldown(env);
    if let Some(cfg) = &config {
        match cfg {
            LateFeeConfig::Flat(crate::penalties::FlatFeeConfig { amount }) => {
                if *amount < 0 {
                    env.panic_with_error(ContractError::InvalidAmount);
                }
            }
            LateFeeConfig::AprBased(crate::penalties::AprFeeConfig { surcharge_bps }) => {
                if *surcharge_bps > crate::risk::MAX_INTEREST_RATE_BPS {
                    env.panic_with_error(ContractError::RateTooHigh);
                }
            }
        }
    }
    crate::storage::set_late_fee_config(env, config);
    touch_admin_lifecycle_critical_action_ts(env);
}
