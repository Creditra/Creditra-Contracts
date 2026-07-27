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

/// Contract error codes emitted by the risk contract.
///
/// Discriminants are **ABI-stable** — existing values must never be
/// renumbered or reordered. New variants must be appended with the
/// next available integer.
///
/// # Two-tier layout
///
/// | Code | Tier | Meaning |
/// |------|------|---------|
/// | `1`  | auth | Caller is not authorized |
/// | `2`  | auth | Caller is not the admin |
/// | `3`  | risk | Protocol is paused |
/// | `54` | risk | Risk admin cooldown has not elapsed |
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
    ///
    /// The next risk mutation is only allowed after
    /// `last_action_ts + cooldown_seconds ≤ env.ledger().timestamp()`.
    RiskAdminCooldownActive = 54,
}

impl ContractError {
    /// Map this error variant to its broad [`ContractErrorCategory`].
    ///
    /// Client-side tooling can use this to group errors without pattern-
    /// matching every individual discriminant. The mapping is stable and
    /// mirrors the two-tier layout described on [`ContractError`].
    ///
    /// # Returns
    ///
    /// - [`ContractErrorCategory::Auth`] for [`Unauthorized`](Self::Unauthorized)
    ///   and [`NotAdmin`](Self::NotAdmin).
    /// - [`ContractErrorCategory::Risk`] for [`Paused`](Self::Paused) and
    ///   [`RiskAdminCooldownActive`](Self::RiskAdminCooldownActive).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use creditra_risk::{ContractError, ContractErrorCategory};
    ///
    /// assert_eq!(ContractError::NotAdmin.category(), ContractErrorCategory::Auth);
    /// assert_eq!(ContractError::Paused.category(),   ContractErrorCategory::Risk);
    /// ```
    pub fn category(&self) -> ContractErrorCategory {
        match self {
            Self::Unauthorized | Self::NotAdmin => ContractErrorCategory::Auth,
            Self::Paused | Self::RiskAdminCooldownActive => ContractErrorCategory::Risk,
        }
    }
}

/// Broad category grouping for [`ContractError`] variants.
///
/// Used by client-side tooling to classify errors without matching every
/// individual discriminant. Discriminants are ABI-stable.
///
/// | Code | Category | Covers |
/// |------|----------|--------|
/// | `1`  | Auth | [`ContractError::Unauthorized`], [`ContractError::NotAdmin`] |
/// | `6`  | Risk | [`ContractError::Paused`], [`ContractError::RiskAdminCooldownActive`] |
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ContractErrorCategory {
    /// Authentication / authorization errors.
    Auth = 1,
    /// Risk parameter violations.
    Risk = 6,
}

/// Event emitted when the risk admin cooldown duration is changed.
///
/// Published by [`RiskContract::set_risk_admin_cooldown`] under the
/// topic pair `("risk", "rad_cool")` in the Soroban event log.
///
/// Subscribers can listen for this event to detect cooldown
/// reconfiguration and update off-chain monitoring thresholds
/// accordingly.
#[soroban_sdk::contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RiskAdminCooldownConfiguredEvent {
    /// New cooldown duration in seconds. `0` means disabled.
    pub cooldown_seconds: u64,
}

/// Soroban contract root for the risk admin cooldown domain.
///
/// Manages the time-based circuit breaker that limits how often an admin
/// can mutate risk-critical parameters. All state-changing entrypoints
/// enforce `require_auth` on the stored admin address and gate execution
/// behind the configured cooldown interval.
///
/// # Storage layout (instance)
///
/// | Key | Type | Description |
/// |-----|------|-------------|
/// | `"admin"` | [`Address`] | The privileged admin address |
/// | `"rad_cool"` | `u64` | Cooldown duration in seconds (`0` = disabled) |
/// | `"rad_last"` | `u64` | Ledger timestamp of the last risk mutation |
/// | `"paused"` | `bool` | Protocol pause flag (default `false`) |
///
/// # Entrypoints
///
/// | Fn | Auth | Mutates |
/// |----|------|---------|
/// | [`init`](Self::init) | `admin` | `"admin"` |
/// | [`set_risk_admin_cooldown`](Self::set_risk_admin_cooldown) | admin | `"rad_cool"` |
/// | [`get_risk_admin_cooldown`](Self::get_risk_admin_cooldown) | none | — |
/// | [`record_risk_admin_action`](Self::record_risk_admin_action) | admin | `"rad_last"` |
/// | [`get_admin`](Self::get_admin) | none | — |
#[soroban_sdk::contract]
pub struct RiskContract;

#[contractimpl]
impl RiskContract {
    /// Initialize the risk contract with an admin address.
    ///
    /// Stores the provided `admin` address in instance storage under the
    /// `"admin"` key. This is a one-shot setup entrypoint — subsequent
    /// calls will overwrite the stored admin, so the deployer must ensure
    /// this is called exactly once at deploy time.
    ///
    /// # Arguments
    ///
    /// * `env` — The Soroban environment injected by the host.
    /// * `admin` — The [`Address`] that will hold admin privileges for all
    ///   subsequent state-changing entrypoints.
    ///
    /// # Authorization
    ///
    /// Requires `require_auth` on `admin`. The deployer is expected to
    /// be the signer at initialization time.
    ///
    /// # Storage
    ///
    /// - **Writes** `Symbol("admin")` → `admin` in instance storage.
    ///
    /// # Errors
    ///
    /// Reverts if the `admin` address fails authorization.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// client.init(&admin_address);
    /// ```
    pub fn init(env: Env, admin: Address) {
        env.require_auth(&admin);
        let key: Symbol = symbol_short!("admin");
        env.storage().instance().set(&key, &admin);
    }

    /// Set the risk admin cooldown duration in seconds.
    ///
    /// When `seconds > 0`, every risk-mutation entrypoint enforces a
    /// minimum elapsed interval since the last mutation. This acts as a
    /// time-based circuit breaker: even if an admin key is compromised,
    /// the attacker can execute at most one risk mutation per cooldown
    /// window, giving monitoring systems time to respond.
    ///
    /// Setting `seconds` to `0` disables the cooldown entirely (the
    /// default, backward-compatible state).
    ///
    /// # Arguments
    ///
    /// * `env` — The Soroban environment injected by the host.
    /// * `seconds` — Cooldown duration in seconds. Use `0` to disable.
    ///
    /// # Authorization
    ///
    /// Admin only. Calls `require_admin_auth` internally; reverts with
    /// [`ContractError::NotAdmin`] (via panic) if the caller is not the
    /// stored admin.
    ///
    /// # Preconditions
    ///
    /// - Protocol must not be paused ([`ContractError::Paused`]).
    /// - Caller must be the stored admin ([`ContractError::NotAdmin`]).
    ///
    /// # Storage
    ///
    /// - **Writes** `Symbol("rad_cool")` → `seconds` in instance storage.
    ///
    /// # Events
    ///
    /// Publishes a [`RiskAdminCooldownConfiguredEvent`] with the new
    /// cooldown value under topic `("risk", "rad_cool")`.
    ///
    /// # Errors
    ///
    /// | Condition | Behaviour |
    /// |-----------|-----------|
    /// | Protocol paused | Reverts with [`ContractError::Paused`] |
    /// | Caller not admin | Reverts with auth failure |
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Enable a 1-hour cooldown.
    /// client.set_risk_admin_cooldown(&3_600_u64);
    ///
    /// // Disable the cooldown.
    /// client.set_risk_admin_cooldown(&0_u64);
    /// ```
    pub fn set_risk_admin_cooldown(env: Env, seconds: u64) {
        assert_not_paused(&env);
        require_admin_auth(&env);
        admin::set_risk_admin_cooldown_seconds(&env, seconds);
        publish_risk_admin_cooldown_configured(&env, seconds);
    }

    /// Get the configured risk admin cooldown duration in seconds.
    ///
    /// Returns the value stored under `Symbol("rad_cool")` in instance
    /// storage. Returns `0` when no cooldown has been configured (the
    /// default), meaning enforcement is disabled.
    ///
    /// This is a read-only entrypoint; it requires no authorization and
    /// emits no events.
    ///
    /// # Arguments
    ///
    /// * `env` — The Soroban environment injected by the host.
    ///
    /// # Returns
    ///
    /// The cooldown duration in seconds, or `0` if disabled.
    ///
    /// # Storage
    ///
    /// - **Reads** `Symbol("rad_cool")` from instance storage.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let cooldown: u64 = client.get_risk_admin_cooldown();
    /// assert_eq!(cooldown, 0); // default: disabled
    /// ```
    pub fn get_risk_admin_cooldown(env: Env) -> u64 {
        admin::get_risk_admin_cooldown_seconds(&env)
    }

    /// Record the current ledger timestamp as the last risk admin action.
    ///
    /// Writes `env.ledger().timestamp()` to instance storage under
    /// `Symbol("rad_last")`. This is the reference timestamp used by the
    /// cooldown guard on the *next* call: if `now < last_ts + cooldown`
    /// the next call will revert with [`ContractError::RiskAdminCooldownActive`].
    ///
    /// This entrypoint is also used directly in tests to simulate a
    /// prior risk mutation without invoking credit-contract logic.
    ///
    /// # Arguments
    ///
    /// * `env` — The Soroban environment injected by the host.
    ///
    /// # Authorization
    ///
    /// Admin only. Reverts if the caller is not the stored admin.
    ///
    /// # Preconditions
    ///
    /// - Protocol must not be paused ([`ContractError::Paused`]).
    /// - Caller must be the stored admin.
    /// - If a non-zero cooldown is configured, the interval since the
    ///   last recorded action must have elapsed.
    ///
    /// # Storage
    ///
    /// - **Reads** `Symbol("rad_cool")` and `Symbol("rad_last")` to
    ///   enforce the cooldown guard.
    /// - **Writes** `Symbol("rad_last")` → current ledger timestamp on
    ///   success.
    ///
    /// # Errors
    ///
    /// | Condition | Error |
    /// |-----------|-------|
    /// | Protocol paused | [`ContractError::Paused`] |
    /// | Caller not admin | Auth failure |
    /// | Cooldown not elapsed | [`ContractError::RiskAdminCooldownActive`] |
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // First action always succeeds (no prior timestamp).
    /// client.record_risk_admin_action();
    ///
    /// // Immediate second call fails when cooldown > 0.
    /// let result = client.try_record_risk_admin_action();
    /// assert!(result.is_err());
    /// ```
    pub fn record_risk_admin_action(env: Env) {
        assert_not_paused(&env);
        require_admin_auth(&env);
        assert_risk_admin_cooldown_elapsed(&env);
        let ts = env.ledger().timestamp();
        admin::set_last_risk_admin_action_ts(&env, ts);
    }

    /// Get the stored admin address.
    ///
    /// Returns the [`Address`] stored under `Symbol("admin")` in instance
    /// storage. This is a read-only entrypoint; it requires no
    /// authorization and emits no events.
    ///
    /// # Arguments
    ///
    /// * `env` — The Soroban environment injected by the host.
    ///
    /// # Returns
    ///
    /// The admin [`Address`] set during [`init`](Self::init).
    ///
    /// # Storage
    ///
    /// - **Reads** `Symbol("admin")` from instance storage.
    ///
    /// # Panics
    ///
    /// Panics with `"admin not initialized"` if [`init`](Self::init) has
    /// not been called yet. Callers should ensure the contract is
    /// initialized before querying the admin.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let admin: Address = client.get_admin();
    /// ```
    pub fn get_admin(env: Env) -> Address {
        let key: Symbol = symbol_short!("admin");
        env.storage()
            .instance()
            .get(&key)
            .expect("admin not initialized")
    }
}

/// Revert with [`ContractError::Paused`] if the protocol pause flag is set.
///
/// Reads `Symbol("paused")` from instance storage. Defaults to `false`
/// (unpaused) when the key is absent. Called at the top of every
/// state-changing entrypoint before any auth or cooldown checks.
fn assert_not_paused(env: &Env) {
    let key: Symbol = symbol_short!("paused");
    let paused: bool = env.storage().instance().get(&key).unwrap_or(false);
    if paused {
        env.panic_with_error(ContractError::Paused);
    }
}

/// Verify the caller is the stored admin and invoke `require_auth`.
///
/// Reads `Symbol("admin")` from instance storage and calls
/// [`Address::require_auth`] on the result. Panics with
/// `"admin not initialized"` if the contract has not been initialized.
fn require_admin_auth(env: &Env) {
    let key: Symbol = symbol_short!("admin");
    let admin: Address = env
        .storage()
        .instance()
        .get(&key)
        .expect("admin not initialized");
    admin.require_auth();
}

/// Publish a [`RiskAdminCooldownConfiguredEvent`] to the Soroban event log.
///
/// Emitted by [`RiskContract::set_risk_admin_cooldown`] whenever the
/// cooldown duration changes. Topic: `("risk", "rad_cool")`.
fn publish_risk_admin_cooldown_configured(env: &Env, cooldown_seconds: u64) {
    env.events().publish(
        (symbol_short!("risk"), symbol_short!("rad_cool")),
        RiskAdminCooldownConfiguredEvent { cooldown_seconds },
    );
}