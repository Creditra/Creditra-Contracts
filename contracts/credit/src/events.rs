// SPDX-License-Identifier: MIT
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(coverage_nightly, coverage(off))]

//! Event types and publishers for the Credit contract.

use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol};

use crate::types::CreditStatus;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreditLineEvent {
    pub borrower: Address,
    pub status: CreditStatus,
    pub credit_limit: i128,
    pub interest_rate_bps: u32,
    pub risk_score: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepaymentEvent {
    pub borrower: Address,
    pub amount: i128,
    pub new_utilized_amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DrawnEvent {
    pub borrower: Address,
    pub amount: i128,
    pub new_utilized_amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterestAccruedEvent {
    pub borrower: Address,
    pub accrued_amount: i128,
    pub new_utilized_amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DefaultLiquidationSettledEvent {
    pub borrower: Address,
    pub settlement_id: Symbol,
    pub recovered_amount: i128,
    pub remaining_utilized_amount: i128,
    pub status: CreditStatus,
    /// Oracle price applied to bound the recovery, when an oracle was configured.
    ///
    /// `Some(price)` ⇒ an oracle config existed at settlement time and we
    /// validated and used its price. `None` ⇒ no oracle was configured
    /// (legacy path, see [`crate::lifecycle::settle_default_liquidation`]).
    ///
    /// Field is appended at the end of the struct to preserve backward
    /// compatibility with indexers that only decode the first five fields.
    pub oracle_price: Option<i128>,
    /// Timestamp from the latest oracle price point, when an oracle was used.
    pub oracle_price_ts: Option<u64>,
    /// Address of the oracle contract that returned the applied price.
    pub oracle_address: Option<Address>,
    /// Computed upper bound on `recovered_amount` (`floor(price * utilized / 1e9)`).
    pub max_recovery_value: Option<i128>,
}

/// Event emitted when the default-oracle configuration is set or cleared.
///
/// Topic: `("credit", "oracle_cfg")`. Payload carries the oracle address and
/// max age on `set_default_oracle`; `oracle_address` is `None` when the admin
/// removes the config. Indexers can rely on the topic alone for routing.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DefaultOracleConfigEvent {
    /// `Some(addr)` on `set_default_oracle`, `None` when removed.
    pub oracle_address: Option<Address>,
    /// Max price age in seconds (unset when `oracle_address` is `None`).
    pub max_price_age_seconds: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminRotationProposedEvent {
    pub proposed_admin: Address,
    pub accept_after: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminRotationAcceptedEvent {
    pub new_admin: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RiskParametersUpdatedEvent {
    pub borrower: Address,
    pub credit_limit: i128,
    pub interest_rate_bps: u32,
    pub risk_score: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DrawReversedEvent {
    pub borrower: Address,
    pub amount: i128,
    pub original_ts: u64,
    pub reason_code: u32,
    pub new_utilized_amount: i128,
    pub timestamp: u64,
    pub admin: Address,
    pub accounting_only: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DrawsFrozenEvent {
    pub frozen: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BorrowerBlockedEvent {
    pub borrower: Address,
    pub blocked: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DrawnEventV2 {
    pub borrower: Address,
    pub recipient: Address,
    pub reserve_source: Address,
    pub amount: i128,
    pub new_utilized_amount: i128,
    pub timestamp: u64,
}

pub fn publish_credit_line_event(env: &Env, topic: (Symbol, Symbol), event: CreditLineEvent) {
    env.events().publish(topic, event);
}

pub fn publish_repayment_event(env: &Env, event: RepaymentEvent) {
    env.events()
        .publish((symbol_short!("credit"), symbol_short!("repay")), event);
}

pub fn publish_drawn_event(env: &Env, event: DrawnEvent) {
    env.events()
        .publish((symbol_short!("credit"), symbol_short!("drawn")), event);
}

/// Publish a draw reversal event.
pub fn publish_draw_reversed_event(env: &Env, event: DrawReversedEvent) {
    env.events()
        .publish((symbol_short!("credit"), symbol_short!("draw_rev")), event);
}

/// Publish a v2 drawn event.
#[allow(dead_code)]
pub fn publish_drawn_event_v2(env: &Env, event: DrawnEventV2) {
    env.events()
        .publish((symbol_short!("credit"), symbol_short!("drawn_v2")), event);
}

pub fn publish_admin_rotation_proposed(env: &Env, proposed_admin: &Address, accept_after: u64) {
    env.events().publish(
        (symbol_short!("credit"), Symbol::new(env, "admin_prop")),
        AdminRotationProposedEvent {
            proposed_admin: proposed_admin.clone(),
            accept_after,
        },
    );
}

pub fn publish_admin_rotation_accepted(env: &Env, new_admin: &Address) {
    env.events().publish(
        (symbol_short!("credit"), Symbol::new(env, "admin_acc")),
        AdminRotationAcceptedEvent {
            new_admin: new_admin.clone(),
        },
    );
}

pub fn publish_risk_parameters_updated(
    env: &Env,
    borrower: &Address,
    credit_limit: i128,
    interest_rate_bps: u32,
    risk_score: u32,
) {
    env.events().publish(
        (symbol_short!("credit"), symbol_short!("risk_upd")),
        RiskParametersUpdatedEvent {
            borrower: borrower.clone(),
            credit_limit,
            interest_rate_bps,
            risk_score,
        },
    );
}

pub fn publish_interest_accrued_event(env: &Env, event: InterestAccruedEvent) {
    env.events()
        .publish((symbol_short!("credit"), symbol_short!("accrue")), event);
}

pub fn publish_draws_frozen_event(env: &Env, frozen: bool) {
    env.events().publish(
        (symbol_short!("credit"), Symbol::new(env, "drw_freeze")),
        DrawsFrozenEvent { frozen },
    );
}

pub fn publish_rate_formula_config_event(env: &Env, enabled: bool) {
    env.events().publish(
        (symbol_short!("credit"), Symbol::new(env, "rate_form")),
        enabled,
    );
}

pub fn publish_default_liquidation_requested_event(
    env: &Env,
    borrower: &Address,
    utilized_amount: i128,
) {
    env.events().publish(
        (symbol_short!("credit"), Symbol::new(env, "liq_req")),
        (borrower.clone(), utilized_amount),
    );
}

pub fn publish_default_liquidation_settled_event(
    env: &Env,
    event: DefaultLiquidationSettledEvent,
) {
    env.events().publish(
        (symbol_short!("credit"), Symbol::new(env, "liq_setl")),
        event,
    );
}

/// Publish a `("credit", "oracle_cfg")` event on default-oracle config changes.
pub fn publish_default_oracle_config_event(
    env: &Env,
    event: DefaultOracleConfigEvent,
) {
    env.events().publish(
        (symbol_short!("credit"), Symbol::new(env, "oracle_cfg")),
        event,
    );
}

pub fn publish_paused_event(env: &Env, paused: bool) {
    let topic = if paused {
        Symbol::new(env, "paused")
    } else {
        Symbol::new(env, "unpaused")
    };
    env.events().publish((symbol_short!("credit"), topic), paused);
}

/// Publish a borrower blocked/unblocked event.
#[allow(dead_code)]
pub fn publish_borrower_blocked_event(env: &Env, event: BorrowerBlockedEvent) {
    env.events()
        .publish((symbol_short!("credit"), symbol_short!("blk_chg")), event);
}


