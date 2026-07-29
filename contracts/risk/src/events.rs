// SPDX-License-Identifier: MIT
//! Event types and publishers for the Risk contract.
//!
//! # What
//!
//! Every event the risk contract emits is defined here as a
//! `#[contracttype]` payload struct paired with a `publish_*` helper.

use soroban_sdk::{contracttype, symbol_short, Address, Env};

/// Payload emitted when the risk admin cooldown is configured.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RiskAdminCooldownConfiguredEvent {
    /// New cooldown duration in seconds. `0` means disabled.
    pub cooldown_seconds: u64,
}

/// Payload emitted when a risk admin action is recorded.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RiskAdminActionRecordedEvent {
    /// Timestamp of the recorded action.
    pub timestamp: u64,
}

/// Payload emitted when the risk contract is initialized.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RiskInitializedEvent {
    /// Admin address.
    pub admin: Address,
}

/// Payload emitted when the risk contract is paused or unpaused.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RiskPausedEvent {
    /// True if paused, false if unpaused.
    pub paused: bool,
}

/// Publish a risk admin cooldown configured event.
pub fn publish_risk_admin_cooldown_configured(env: &Env, cooldown_seconds: u64) {
    env.events().publish(
        (symbol_short!("risk"), symbol_short!("rad_cool")),
        RiskAdminCooldownConfiguredEvent { cooldown_seconds },
    );
}

/// Publish a risk admin action recorded event.
pub fn publish_risk_admin_action_recorded(env: &Env, timestamp: u64) {
    env.events().publish(
        (symbol_short!("risk"), symbol_short!("rad_act")),
        RiskAdminActionRecordedEvent { timestamp },
    );
}

/// Publish a risk initialized event.
pub fn publish_risk_initialized(env: &Env, admin: &Address) {
    env.events().publish(
        (symbol_short!("risk"), symbol_short!("init")),
        RiskInitializedEvent { admin: admin.clone() },
    );
}

/// Publish a risk paused event.
pub fn publish_risk_paused(env: &Env, paused: bool) {
    let topic = if paused {
        symbol_short!("paused")
    } else {
        symbol_short!("unpaused")
    };
    env.events().publish(
        (symbol_short!("risk"), topic),
        RiskPausedEvent { paused },
    );
}
