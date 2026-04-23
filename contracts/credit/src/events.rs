// SPDX-License-Identifier: MIT

//! Event types and topic constants for the Credit contract.
//! Stable event schemas for indexing and analytics.

use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol};

use crate::types::CreditStatus;

/// Event emitted when a credit line lifecycle event occurs (opened, suspend, closed, default).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreditLineEvent {
    /// Type of lifecycle event (e.g., "opened", "suspend", "closed", "default").
    pub event_type: Symbol,
    /// Address of the borrower.
    pub borrower: Address,
    /// New status of the credit line.
    pub status: CreditStatus,
    /// Credit limit of the line.
    pub credit_limit: i128,
    /// Interest rate in basis points.
    pub interest_rate_bps: u32,
    /// Risk score of the borrower.
    pub risk_score: u32,
}

/// Versioned lifecycle event for analytics/indexers.
///
/// Semver policy: this is additive and emitted alongside `CreditLineEvent` so
/// existing indexers remain compatible while new consumers migrate to v2.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreditLineEventV2 {
    pub event_type: Symbol,
    pub borrower: Address,
    pub status: CreditStatus,
    pub credit_limit: i128,
    pub interest_rate_bps: u32,
    pub risk_score: u32,
    pub timestamp: u64,
    pub actor: Address,
    pub amount: i128,
}

/// Event emitted when a borrower repays credit.
///
/// Allocation policy: accrued interest is repaid first, then principal.
/// Integrators can reconcile balances using `new_utilized_amount` and
/// `new_accrued_interest`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepaymentEvent {
    /// Address of the borrower.
    pub borrower: Address,
    /// Effective amount repaid (capped at total owed).
    pub amount: i128,
    /// Portion of the repayment applied to accrued interest.
    pub interest_repaid: i128,
    /// Portion of the repayment applied to principal.
    pub principal_repaid: i128,
    /// Total outstanding debt after repayment.
    pub new_utilized_amount: i128,
    /// Remaining accrued interest after repayment.
    pub new_accrued_interest: i128,
    /// Ledger timestamp of the repayment.
    pub timestamp: u64,
}

/// Versioned repayment event with explicit payer identifier and allocation breakdown.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepaymentEventV2 {
    pub borrower: Address,
    pub payer: Address,
    pub amount: i128,
    pub interest_repaid: i128,
    pub principal_repaid: i128,
    pub new_utilized_amount: i128,
    pub new_accrued_interest: i128,
    pub timestamp: u64,
}

/// Event emitted when admin updates risk parameters for a credit line.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RiskParametersUpdatedEvent {
    /// Address of the borrower.
    pub borrower: Address,
    /// New credit limit.
    pub credit_limit: i128,
    /// New interest rate in basis points.
    pub interest_rate_bps: u32,
    /// New risk score.
    pub risk_score: u32,
}

/// Versioned risk update event with timestamp and actor identifier.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RiskParametersUpdatedEventV2 {
    pub borrower: Address,
    pub credit_limit: i128,
    pub interest_rate_bps: u32,
    pub risk_score: u32,
    pub timestamp: u64,
    pub actor: Address,
}

/// Event emitted when a borrower draws credit.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DrawnEvent {
    /// Address of the borrower.
    pub borrower: Address,
    /// Amount drawn.
    pub amount: i128,
    /// New outstanding principal.
    pub new_utilized_amount: i128,
    /// Ledger timestamp of the draw operation.
    pub timestamp: u64,
}

/// Event emitted when interest is accrued and capitalized.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterestAccruedEvent {
    pub borrower: Address,
    pub accrued_amount: i128,
    pub total_accrued_interest: i128,
    pub new_utilized_amount: i128,
    pub timestamp: u64,
}

/// Versioned draw event with explicit recipient/source identifiers.
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

/// Event emitted when admin rotation is proposed.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminRotationProposedEvent {
    pub current_admin: Address,
    pub proposed_admin: Address,
    pub accept_after: u64,
}

/// Event emitted when admin rotation is accepted.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminRotationAcceptedEvent {
    pub previous_admin: Address,
    pub new_admin: Address,
}

/// Publish a credit line lifecycle event.
pub fn publish_credit_line_event(env: &Env, topic: (Symbol, Symbol), event: CreditLineEvent) {
    env.events().publish(topic, event);
}

/// Publish a v2 credit line lifecycle event.
#[allow(dead_code)]
pub fn publish_credit_line_event_v2(env: &Env, topic: (Symbol, Symbol), event: CreditLineEventV2) {
    env.events().publish(topic, event);
}

/// Publish a repayment event.
pub fn publish_repayment_event(env: &Env, event: RepaymentEvent) {
    env.events()
        .publish((symbol_short!("credit"), symbol_short!("repay")), event);
}

/// Publish a v2 repayment event.
#[allow(dead_code)]
pub fn publish_repayment_event_v2(env: &Env, event: RepaymentEventV2) {
    env.events().publish(
        (symbol_short!("credit"), Symbol::new(env, "repay_v2")),
        event,
    );
}

/// Publish a drawn event.
pub fn publish_drawn_event(env: &Env, event: DrawnEvent) {
    env.events()
        .publish((symbol_short!("credit"), symbol_short!("drawn")), event);
}

/// Publish a v2 drawn event.
#[allow(dead_code)]
pub fn publish_drawn_event_v2(env: &Env, event: DrawnEventV2) {
    env.events()
        .publish((symbol_short!("credit"), symbol_short!("drawn_v2")), event);
}

/// Publish a risk parameters updated event.
pub fn publish_risk_parameters_updated(env: &Env, event: RiskParametersUpdatedEvent) {
    env.events()
        .publish((symbol_short!("credit"), symbol_short!("risk_upd")), event);
}

/// Publish an interest accrued event.
#[allow(dead_code)]
pub fn publish_interest_accrued_event(env: &Env, event: InterestAccruedEvent) {
    env.events()
        .publish((symbol_short!("credit"), symbol_short!("accrue")), event);
}

/// Event emitted when a borrower's block status changes.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BorrowerBlockedEvent {
    /// Address of the borrower.
    pub borrower: Address,
    /// New blocked status.
    pub blocked: bool,
}

/// Publish a borrower blocked/unblocked event.
pub fn publish_borrower_blocked_event(env: &Env, event: BorrowerBlockedEvent) {
    let topic = if event.blocked {
        symbol_short!("blocked")
    } else {
        Symbol::new(env, "unblocked")
    };
    env.events()
        .publish((symbol_short!("credit"), topic), event);
}
