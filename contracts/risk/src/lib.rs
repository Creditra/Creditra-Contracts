// SPDX-License-Identifier: MIT
#![cfg_attr(not(test), no_std)]
#![allow(clippy::unused_unit)]

//! # Creditra Risk Admin Cooldown Contract
//!
//! Standalone Soroban contract that manages the cooldown period between admin
//! actions on risk-critical parameters, and keeps instance storage alive via
//! automatic TTL bumps on every hot read path.
//!
//! ## What
//!
//! Enforces a time-based circuit breaker on admin-mutated risk settings. When
//! a non-zero cooldown is configured, every risk mutation is gated so that at
//! most one can occur per cooldown window.
//!
//! ## Storage TTL hygiene
//!
//! Every entrypoint — including read-only views — calls
//! [`admin::bump_instance_ttl`] to extend the contract's instance storage TTL.
//! This prevents archival pressure during periods where only queries (no
//! state-changing mutations) are exercised, which is the normal steady state
//! for an active campaign.
//!
//! The policy mirrors `contracts/credit/src/storage.rs`:
//!
//! | Constant                             | Value     | Duration  |
//! |--------------------------------------|-----------|-----------|
//! | [`INSTANCE_BUMP_AMOUNT`]             | 3 110 400 | ~6 months |
//! | [`INSTANCE_BUMP_THRESHOLD`]          | 1 555 200 | ~3 months |
//!
//! The bump is a no-op (no ledger write) when the remaining TTL is still above
//! the threshold, so the overhead on an active contract is negligible.
//!
//! ## How
//!
//! The contract stores two keys in instance storage:
//!
//! - `Symbol("rad_cool")` — cooldown duration in seconds
//!   (`u64`, default `0` = disabled).
//! - `Symbol("rad_last")` — ledger timestamp of the last risk admin action
//!   (`u64`, default `0` = no prior action).
//!
//! The guard function [`admin::assert_risk_admin_cooldown_elapsed`] is called
//! at the top of every state-changing entrypoint. It reads both values and
//! reverts with [`ContractError::RiskAdminCooldownActive`] when the cooldown
//! interval has not yet elapsed. The public error-code values are ABI-stable
//! and pinned in [`tests/err_stab.rs`](../tests/err_stab.rs).
//!
//! ## Why
//!
//! Limits the blast radius of compromised admin keys. An attacker who obtains
//! an admin key can execute at most one risk-mutation per cooldown window,
//! giving time for other admins or monitoring systems to detect and respond.
//!
//! ## Security
//!
//! - `require_auth` is enforced on every state-changing entrypoint.
//! - All arithmetic uses saturating operations to prevent overflow.
//! - No `unwrap()` calls in production paths; defaults use `unwrap_or` or
//!   pattern matching.
//! - The `RiskAdminCooldownActive` error variant is ABI-stable (discriminant
//!   `54`).

use soroban_sdk::{contract, contractimpl, symbol_short, Address, Env, Symbol};

pub mod admin;
mod events;

pub use admin::{INSTANCE_BUMP_AMOUNT, INSTANCE_BUMP_THRESHOLD};
pub use events::{
    RiskAdminActionRecordedEvent, RiskAdminCooldownConfiguredEvent, RiskInitializedEvent,
    RiskPausedEvent,
};

// ── Error types ───────────────────────────────────────────────────────────────

/// Contract-level errors for the risk admin cooldown contract.
///
/// Discriminants are ABI-stable. Do not reorder or renumber variants.
///
/// | Variant                    | Discriminant | Category |
/// |----------------------------|--------------|----------|
/// | `Unauthorized`             | 1            | Auth     |
/// | `NotAdmin`                 | 2            | Auth     |
/// | `Paused`                   | 3            | Risk     |
/// | `RiskAdminCooldownActive`  | 54           | Risk     |
#[soroban_sdk::contracterror]
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

/// Stable category grouping for [`ContractError`] variants.
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ContractErrorCategory {
    /// Authentication / authorization errors.
    Auth = 1,
    /// Risk parameter violations.
    Risk = 6,
}

// ── Contract ──────────────────────────────────────────────────────────────────

#[contract]
pub struct RiskContract;

#[contractimpl]
impl RiskContract {
    /// Initialize the risk contract with an admin address.
    ///
    /// Can only be called once. Requires the caller to be the deployer/admin
    /// of the contract.
    ///
    /// Bumps instance storage TTL so the contract remains live immediately
    /// after deployment.
    ///
    /// # Arguments
    /// * `env`   - The Soroban environment.
    /// * `admin` - The address that holds admin privileges.
    ///
    /// # TTL side-effect
    /// Extends instance storage TTL to [`INSTANCE_BUMP_AMOUNT`] when remaining
    /// TTL is below [`INSTANCE_BUMP_THRESHOLD`].
    pub fn init(env: Env, admin: Address) {
        admin.require_auth();
        admin::bump_instance_ttl(&env);
        let key: Symbol = symbol_short!("admin");
        env.storage().instance().set(&key, &admin);
        events::publish_risk_initialized(&env, &admin);
    }

    /// Set the risk admin cooldown duration in seconds (admin only).
    ///
    /// When `seconds > 0`, every risk-mutation entrypoint enforces a minimum
    /// elapsed interval since the last mutation. This provides a time-based
    /// circuit breaker that limits the blast radius of compromised admin keys.
    ///
    /// A value of `0` disables the cooldown (default, backward compatible).
    ///
    /// Bumps instance storage TTL as a side-effect.
    ///
    /// # Arguments
    /// * `env`     - The Soroban environment.
    /// * `seconds` - Cooldown duration in seconds. Pass `0` to disable.
    ///
    /// # Errors
    /// - Panics with [`ContractError::Paused`] if the protocol is paused.
    /// - Panics with [`ContractError::NotAdmin`] if the caller is not the
    ///   contract admin.
    ///
    /// # TTL side-effect
    /// Extends instance storage TTL to [`INSTANCE_BUMP_AMOUNT`] when remaining
    /// TTL is below [`INSTANCE_BUMP_THRESHOLD`].
    pub fn set_risk_admin_cooldown(env: Env, seconds: u64) {
        assert_not_paused(&env);
        require_admin_auth(&env);
        admin::set_risk_admin_cooldown_seconds(&env, seconds);
        events::publish_risk_admin_cooldown_configured(&env, seconds);
    }

    /// Set the paused state of the contract (admin only).
    ///
    /// When paused, all state-changing entrypoints except `set_paused` itself
    /// are blocked. Bumps instance storage TTL as a side-effect.
    ///
    /// # Arguments
    /// * `env`    - The Soroban environment.
    /// * `paused` - `true` to pause; `false` to unpause.
    ///
    /// # Errors
    /// - Panics with [`ContractError::NotAdmin`] if the caller is not the
    ///   contract admin.
    ///
    /// # TTL side-effect
    /// Extends instance storage TTL to [`INSTANCE_BUMP_AMOUNT`] when remaining
    /// TTL is below [`INSTANCE_BUMP_THRESHOLD`].
    pub fn set_paused(env: Env, paused: bool) {
        require_admin_auth(&env);
        admin::bump_instance_ttl(&env);
        let key: Symbol = symbol_short!("paused");
        env.storage().instance().set(&key, &paused);
        events::publish_risk_paused(&env, paused);
    }

    /// Get the configured risk admin cooldown duration in seconds.
    ///
    /// Returns `0` when the cooldown is disabled (default). Bumps instance
    /// storage TTL so that read-only call paths also keep the contract live.
    ///
    /// # Arguments
    /// * `env` - The Soroban environment.
    ///
    /// # Returns
    /// The cooldown duration in seconds, or `0` if disabled.
    ///
    /// # TTL side-effect
    /// Extends instance storage TTL to [`INSTANCE_BUMP_AMOUNT`] when remaining
    /// TTL is below [`INSTANCE_BUMP_THRESHOLD`].
    pub fn get_risk_admin_cooldown(env: Env) -> u64 {
        admin::get_risk_admin_cooldown_seconds(&env)
    }

    /// Record a risk admin action timestamp for cooldown enforcement.
    ///
    /// Updates the stored `last_action_ts` to the current ledger timestamp.
    /// This entrypoint is the canonical mechanism for recording a risk admin
    /// action in the standalone risk contract. In the credit contract,
    /// `set_last_risk_admin_action_ts` is called at the end of
    /// `update_risk_parameters`.
    ///
    /// Bumps instance storage TTL as a side-effect.
    ///
    /// # Arguments
    /// * `env` - The Soroban environment.
    ///
    /// # Errors
    /// - Panics with [`ContractError::Paused`] if the protocol is paused.
    /// - Panics with [`ContractError::NotAdmin`] if the caller is not the
    ///   contract admin.
    /// - Panics with [`ContractError::RiskAdminCooldownActive`] if the
    ///   cooldown interval has not yet elapsed since the last action.
    ///
    /// # TTL side-effect
    /// Extends instance storage TTL to [`INSTANCE_BUMP_AMOUNT`] when remaining
    /// TTL is below [`INSTANCE_BUMP_THRESHOLD`].
    pub fn record_risk_admin_action(env: Env) {
        assert_not_paused(&env);
        require_admin_auth(&env);
        admin::assert_risk_admin_cooldown_elapsed(&env);
        let ts = env.ledger().timestamp();
        admin::set_last_risk_admin_action_ts(&env, ts);
        events::publish_risk_admin_action_recorded(&env, ts);
    }

    /// Get the admin address.
    ///
    /// Bumps instance storage TTL so that read-only call paths keep the
    /// contract live.
    ///
    /// # Arguments
    /// * `env` - The Soroban environment.
    ///
    /// # Returns
    /// The admin address.
    ///
    /// # Panics
    /// - If the admin address has not been initialized.
    ///
    /// # TTL side-effect
    /// Extends instance storage TTL to [`INSTANCE_BUMP_AMOUNT`] when remaining
    /// TTL is below [`INSTANCE_BUMP_THRESHOLD`].
    pub fn get_admin(env: Env) -> Address {
        admin::bump_instance_ttl(&env);
        let key: Symbol = symbol_short!("admin");
        env.storage()
            .instance()
            .get(&key)
            .expect("admin not initialized")
    }
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Require that the caller is the stored admin address.
///
/// Reads the `"admin"` key from instance storage (bumping TTL as a
/// side-effect) and calls `require_auth()` on it. Panics with
/// [`ContractError::NotAdmin`] if no admin has been initialized.
fn require_admin_auth(env: &Env) {
    let key: Symbol = symbol_short!("admin");
    let admin: Address = env
        .storage()
        .instance()
        .get(&key)
        .unwrap_or_else(|| env.panic_with_error(ContractError::NotAdmin));
    admin.require_auth();
}

/// Panic with [`ContractError::Paused`] when the protocol is paused.
///
/// Reads the `"paused"` boolean from instance storage (bumping TTL as a
/// side-effect). When the key is absent the contract is considered unpaused.
fn assert_not_paused(env: &Env) {
    let key: Symbol = symbol_short!("paused");
    let paused: bool = env.storage().instance().get(&key).unwrap_or(false);
    if paused {
        env.panic_with_error(ContractError::Paused);
    }
}
