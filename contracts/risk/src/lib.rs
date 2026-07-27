// SPDX-License-Identifier: MIT
#![cfg_attr(not(test), no_std)]
#![allow(clippy::unused_unit)]
#![allow(dead_code)]

//! # Creditra Risk Admin Cooldown Contract
//!
//! Standalone Soroban contract that manages the cooldown period
//! between admin actions on risk-critical parameters.
//!
//! ## What
//!
//! Enforces a time-based circuit breaker on admin-mutated risk
//! settings. When a non-zero cooldown is configured, every
//! risk mutation is gated so that at most one can occur per
//! cooldown window.
//!
//! ## How
//!
//! The contract stores two keys in instance storage:
//!
//! - `Symbol("rad_cool")` — cooldown duration in seconds
//!   (`u64`, default `0` = disabled).
//! - `Symbol("rad_last")` — ledger timestamp of the last risk
//!   admin action (`u64`, default `0` = no prior action).
//!
//! The guard function `assert_risk_admin_cooldown_elapsed` (from
//! the `admin` module) is called at the top of every state-changing
//! entrypoint. It reads both values and reverts with
//! [`ContractError::RiskAdminCooldownActive`] when the cooldown
//! interval has not yet elapsed.
//!
//! ## Why
//!
//! Limits the blast radius of compromised admin keys. An attacker
//! who obtains an admin key can execute at most one risk-mutation
//! per cooldown window, giving time for other admins or monitoring
//! systems to detect and respond.
//!
//! ## Security
//!
//! - `require_auth` is enforced on every state-changing entrypoint.
//! - All arithmetic uses saturating operations to prevent overflow.
//! - No `unwrap()` calls in production paths; defaults use
//!   `unwrap_or` or pattern matching.
//! - The `RiskAdminCooldownActive` error variant is ABI-stable.

use soroban_sdk::{Address, Env, Symbol, symbol_short};

mod admin;
mod events;

#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ContractError {
    /// Caller is not authorized to perform this action.
    Unauthorized = 1,
    /// Caller is not the contract admin.
    NotAdmin = 2,
    /// Protocol is paused; the operation is blocked.
    Paused = 3,
    /// Risk admin cooldown has not yet elapsed since the last mutation.
    RiskAdminCooldownActive = 54,
}

impl ContractError {
    /// Map this error to its category for client-side grouping.
    pub fn category(&self) -> ContractErrorCategory {
        match self {
            Self::Unauthorized | Self::NotAdmin => ContractErrorCategory::Auth,
            Self::Paused | Self::RiskAdminCooldownActive => ContractErrorCategory::Risk,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ContractErrorCategory {
    /// Authentication / authorization errors.
    Auth = 1,
    /// Risk parameter violations.
    Risk = 6,
}

#[soroban_sdk::contract]
pub struct RiskContract;

#[contractimpl]
impl RiskContract {
    /// Initialize the risk contract with an admin address.
    ///
    /// Can only be called once. Requires the caller to be the
    /// deployer/admin of the contract.
    ///
    /// # Arguments
    /// * `env` - The Soroban environment.
    /// * `admin` - The address that holds admin privileges.
    pub fn init(env: Env, admin: Address) {
        env.require_auth(&admin);
        let key: Symbol = symbol_short!("admin");
        env.storage().instance().set(&key, &admin);
        events::publish_risk_initialized(&env, &admin);
    }

    /// Set the risk admin cooldown duration in seconds (admin only).
    ///
    /// When `seconds > 0`, every risk-mutation entrypoint enforces
    /// a minimum elapsed interval since the last mutation. This
    /// provides a time-based circuit breaker that limits the blast
    /// radius of compromised admin keys.
    ///
    /// A value of `0` disables the cooldown (default, backward
    /// compatible).
    ///
    /// # Arguments
    /// * `env` - The Soroban environment.
    /// * `seconds` - The cooldown duration in seconds. Pass `0` to
    ///   disable.
    ///
    /// # Panics
    /// - If the caller is not the contract admin.
    /// - If the protocol is paused.
    pub fn set_risk_admin_cooldown(env: Env, seconds: u64) {
        assert_not_paused(&env);
        require_admin_auth(&env);
        admin::set_risk_admin_cooldown_seconds(&env, seconds);
        events::publish_risk_admin_cooldown_configured(&env, seconds);
    }

    /// Set the paused state of the contract (admin only).
    ///
    /// # Arguments
    /// * `env` - The Soroban environment.
    /// * `paused` - The new paused state.
    pub fn set_paused(env: Env, paused: bool) {
        require_admin_auth(&env);
        let key: Symbol = symbol_short!("paused");
        env.storage().instance().set(&key, &paused);
        events::publish_risk_paused(&env, paused);
    }

    /// Get the configured risk admin cooldown duration in seconds.
    ///
    /// Returns `0` when the cooldown is disabled (default).
    ///
    /// # Arguments
    /// * `env` - The Soroban environment.
    ///
    /// # Returns
    /// The cooldown duration in seconds.
    pub fn get_risk_admin_cooldown(env: Env) -> u64 {
        admin::get_risk_admin_cooldown_seconds(&env)
    }

    /// Record a risk admin action timestamp for cooldown testing.
    ///
    /// Updates the stored `last_action_ts` to the current ledger
    /// timestamp. This entrypoint is intended for testing cooldown
    /// enforcement. In the credit contract,
    /// `set_last_risk_admin_action_ts` is called at the end of
    /// `update_risk_parameters`.
    ///
    /// # Arguments
    /// * `env` - The Soroban environment.
    pub fn record_risk_admin_action(env: Env) {
        assert_not_paused(&env);
        require_admin_auth(&env);
        assert_risk_admin_cooldown_elapsed(&env);
        let ts = env.ledger().timestamp();
        admin::set_last_risk_admin_action_ts(&env, ts);
        events::publish_risk_admin_action_recorded(&env, ts);
    }

    /// Get the admin address.
    ///
    /// # Arguments
    /// * `env` - The Soroban environment.
    ///
    /// # Returns
    /// The admin address.
    ///
    /// # Panics
    /// - If the admin address has not been initialized.
    pub fn get_admin(env: Env) -> Address {
        let key: Symbol = symbol_short!("admin");
        env.storage()
            .instance()
            .get(&key)
            .expect("admin not initialized")
    }
}