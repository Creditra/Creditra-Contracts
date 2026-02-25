//! Event types and topic constants for the Credit contract.
//! Stable event schemas for indexing and analytics.

use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol};

use crate::types::CreditStatus;

/// Event emitted when a credit line lifecycle event occurs (opened, suspend, closed, default).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreditLineEvent {
    pub event_type: Symbol,
    pub borrower: Address,
    pub status: CreditStatus,
    pub credit_limit: i128,
    pub interest_rate_bps: u32,
    pub risk_score: u32,
}

/// Event emitted when a borrower repays credit.
/// Used for indexing and analytics (borrower, amount, new utilized amount, timestamp).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepaymentEvent {
    pub borrower: Address,
    pub amount: i128,
    pub new_utilized_amount: i128,
    pub timestamp: u64,
}

/// Event emitted when admin updates risk parameters for a credit line.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RiskParametersUpdatedEvent {
    pub borrower: Address,
    pub credit_limit: i128,
    pub interest_rate_bps: u32,
    pub risk_score: u32,
}

/// Publish a credit line lifecycle event.
pub fn publish_credit_line_event(env: &Env, topic: (Symbol, Symbol), event: CreditLineEvent) {
    env.events().publish(topic, event);
}

/// Publish a repayment event.
pub fn publish_repayment_event(env: &Env, event: RepaymentEvent) {
    env.events()
        .publish((symbol_short!("credit"), symbol_short!("repay")), event);
}

/// Publish a risk parameters updated event.
pub fn publish_risk_parameters_updated(env: &Env, event: RiskParametersUpdatedEvent) {
    env.events()
        .publish((symbol_short!("credit"), symbol_short!("risk_upd")), event);
}
