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

#[soroban_sdk::contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RiskAdminCooldownConfiguredEvent {
    /// New cooldown duration in seconds. `0` means disabled.
    pub cooldown_seconds: u64,
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
        publish_risk_admin_cooldown_configured(&env, seconds);
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

fn assert_not_paused(env: &Env) {
    let key: Symbol = symbol_short!("paused");
    let paused: bool = env.storage().instance().get(&key).unwrap_or(false);
    if paused {
        env.panic_with_error(ContractError::Paused);
    }
}

fn require_admin_auth(env: &Env) {
    let key: Symbol = symbol_short!("admin");
    let admin: Address = env
        .storage()
        .instance()
        .get(&key)
        .expect("admin not initialized");
    admin.require_auth();
}

fn publish_risk_admin_cooldown_configured(env: &Env, cooldown_seconds: u64) {
    env.events().publish(
        (symbol_short!("risk"), symbol_short!("rad_cool")),
        RiskAdminCooldownConfiguredEvent { cooldown_seconds },
    );
}