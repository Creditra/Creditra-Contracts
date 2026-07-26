// SPDX-License-Identifier: MIT
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(coverage_nightly, coverage(off))]

//! Event types and publishers for the Credit contract.

use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol};

use crate::types::CreditStatus;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]

/// Dedicated lifecycle event emitted when a credit line is opened.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreditLineOpenedEvent {
    pub borrower: Address,
    pub credit_limit: i128,
    pub interest_rate_bps: u32,
    pub risk_score: u32,
    pub timestamp: u64,
}

/// Dedicated lifecycle event emitted when a credit line is suspended.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreditLineSuspendedEvent {
    pub borrower: Address,
    pub reason: Symbol,
    pub timestamp: u64,
}

/// Dedicated lifecycle event emitted when a credit line is closed.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreditLineClosedEvent {
    pub borrower: Address,
    pub closer: Address,
    pub remaining_utilized_amount: i128,
    pub timestamp: u64,
}

/// Dedicated lifecycle event emitted when a credit line is defaulted.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreditLineDefaultedEvent {
    pub borrower: Address,
    pub utilized_amount: i128,
    pub timestamp: u64,
}

/// Dedicated lifecycle event emitted when a credit line is reinstated.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreditLineReinstatedEvent {
    pub borrower: Address,
    pub target_status: CreditStatus,
    pub timestamp: u64,
}
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


pub fn publish_credit_line_opened_event(env: &Env, event: CreditLineOpenedEvent) {
    env.events()
        .publish((symbol_short!("credit"), symbol_short!("opened_v2")), event);
}

pub fn publish_credit_line_suspended_event(env: &Env, event: CreditLineSuspendedEvent) {
    env.events()
        .publish((symbol_short!("credit"), symbol_short!("susp_v2")), event);
}

pub fn publish_credit_line_closed_event(env: &Env, event: CreditLineClosedEvent) {
    env.events()
        .publish((symbol_short!("credit"), symbol_short!("closed_v2")), event);
}

pub fn publish_credit_line_defaulted_event(env: &Env, event: CreditLineDefaultedEvent) {
    env.events()
        .publish((symbol_short!("credit"), symbol_short!("def_v2")), event);
}

pub fn publish_credit_line_reinstated_event(env: &Env, event: CreditLineReinstatedEvent) {
    env.events()
        .publish((symbol_short!("credit"), symbol_short!("rein_v2")), event);
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




