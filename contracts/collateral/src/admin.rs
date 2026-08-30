// SPDX-License-Identifier: MIT

//! Admin collateral configuration with a cool-off between critical actions (v7).
//!
//! Critical admin entrypoints (`set_admin_collateral_cooldown_seconds`,
//! `set_min_collateral_ratio_bps`, `collateral_risk_weight`,
//! `collateral_token_allowlist`) share a single cooldown clock stored in
//! instance storage. The interval is configured via
//! [``set_admin_collateral_cooldown_seconds``]; when unset or zero, the guard is
//! disabled (same semantics as borrower draw cooldown).

use crate::auth::require_admin_auth;
use crate::storage::{self, assert_not_paused};
use crate::types::ContractError;
use soroban_sdk::{Address, Env, Vec};

/// Enforce the configured cool-off since the last critical collateral admin action.
fn enforce_admin_collateral_cooldown(env: &Env) {
    let Some(cooldown_secs) = storage::get_admin_collateral_cooldown_seconds(env) else {
        return;
    };
    if cooldown_secs == 0 {
        return;
    }
    if let Some(last_ts) = storage::get_last_admin_collateral_critical_action_ts(env) {
        let now = env.ledger().timestamp();
        if now < last_ts.saturating_add(cooldown_secs) {
            env.panic_with_error(ContractError::AdminCollateralCooldownActive);
        }
    }
}

/// Record the ledger timestamp of a successful critical collateral admin action.
fn touch_admin_collateral_critical_action_ts(env: &Env) {
    let now = env.ledger().timestamp();
    storage::set_last_admin_collateral_critical_action_ts(env, now);
}

/// Set the minimum interval between critical collateral admin actions (admin only).
///
/// This action is itself a critical admin action and is subject to the
/// currently configured cool-off. Pass `0`to disable the cool-off guard.
pub fn set_admin_collateral_cooldown_seconds(env: &Env, seconds: u64) {
    assert_not_paused(env);
    require_admin_auth(env);
    enforce_admin_collateral_cooldown(env);
    storage::set_admin_collateral_cooldown_seconds(env, seconds);
    touch_admin_collateral_critical_action_ts(env);
}

/// Return the configured admin collateral cool-off interval, if set.
pub fn get_admin_collateral_cooldown_seconds(env: &Env) -> Option<u64> {
    storage::get_admin_collateral_cooldown_seconds(env)
}

/// Return the ledger timestamp of the last critical collateral admin action, if any.
pub fn get_last_admin_collateral_critical_action_ts(env: &Env) -> Option<u64> {
    storage::get_last_admin_collateral_critical_action_ts(env)
}

/// Set the protocol-wide minimum collateral ratio in basis points (admin only).
pub fn set_min_collateral_ratio_bps(env: &Env, ratio_bps: u32) {
    assert_not_paused(env);
    require_admin_auth(env);
    enforce_admin_collateral_cooldown(env);
    storage::set_min_collateral_ratio_bps(env, ratio_bps);
    touch_admin_collateral_critical_action_ts(env);
}

/// Set the risk weight for a collateral asset in basis points (admin only).
pub fn set_collateral_risk_weight(env: &Env, asset: &Address, weight_bps: u32) {
    assert_not_paused(env);
    require_admin_auth(env);
    if weight_bps > 10_000 {
        env.panic_with_error(ContractError::InvalidRiskWeight);
    }
    enforce_admin_collateral_cooldown(env);
    storage::set_collateral_risk_weight_bps(env, asset, weight_bps);
    touch_admin_collateral_critical_action_ts(env);
}

/// Replace the collateral token allowlist (admin only).
pub fn set_collateral_token_allowlist(env: &Env, tokens: &Vec<Address>) {
    assert_not_paused(env);
    require_admin_auth(env);
    enforce_admin_collateral_cooldown(env);

    // Validate factory-created contract addresses before registration.
    // Ensure every address is a valid contract using the Soroban host environment.
    for token in tokens.iter() {
        // Invoking a standard method lets the host trap if the address is not a contract.
        let _ = soroban_sdk::token::Client::new(env, &token).decimals();
    }

    storage::set_collateral_token_allowlist(env, tokens);
    touch_admin_collateral_critical_action_ts(env);
}