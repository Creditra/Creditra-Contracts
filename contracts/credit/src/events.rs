// SPDX-License-Identifier: MIT

//! # Event Types, Topic Constants, and Publishers for the Credit Contract
//!
//! ## Stable Event Schema
//!
//! All events emitted by this contract follow a frozen topic structure that
//! indexers and analytics pipelines can rely on without version negotiation.
//!
//! ### Topic layout
//!
//! Every event topic is a two-element tuple:
//!
//! ```text
//! (namespace: Symbol, qualifier: Symbol | Address)
//! ```
//!
//! The first element is always one of the four namespace constants defined
//! below.  The second element is either a sub-action symbol (lifecycle events)
//! or the borrower's `Address` (financial events), giving indexers a direct
//! filter key without decoding the data payload.
//!
//! ### Trust boundary
//!
//! Indexers **must** verify the emitting contract ID before trusting event
//! content.  Any contract can publish events with the same topic structure;
//! the contract address is the authoritative identity, not the topic symbols.
//!
//! ### Address authorization
//!
//! `Address` objects that appear in topics are always the borrower address
//! that was authenticated by `require_auth()` earlier in the same call frame.
//! They are never caller-supplied without prior auth.

use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol};

use crate::types::CreditStatus;

// ─── Namespace constants ──────────────────────────────────────────────────────

/// Namespace for credit-line lifecycle events (open, suspend, close, default, reinstate).
///
/// Topic element 0 for all lifecycle events.
pub const TOPIC_CREDIT: &str = "credit";

/// Namespace for draw (borrow) events.
///
/// Topic element 0 for draw events.
pub const TOPIC_DRAWN: &str = "drawn";

/// Namespace for repayment events.
///
/// Topic element 0 for repayment events.
pub const TOPIC_REPAY: &str = "repay";

/// Namespace for lifecycle sub-action events (suspend, close, default, reinstate).
///
/// Topic element 0 for lifecycle action events.
#[allow(dead_code)]
pub const TOPIC_LIFECYCLE: &str = "lifecycle";

// ─── Lifecycle action sub-symbols ────────────────────────────────────────────

/// Sub-action symbol: credit line opened.
pub const ACTION_OPENED: &str = "opened";

/// Sub-action symbol: credit line suspended.
pub const ACTION_SUSPEND: &str = "suspend";

/// Sub-action symbol: credit line closed.
pub const ACTION_CLOSED: &str = "closed";

/// Sub-action symbol: credit line defaulted.
pub const ACTION_DEFAULT: &str = "default";

/// Sub-action symbol: credit line reinstated.
pub const ACTION_REINSTATE: &str = "reinstate";

/// Sub-action symbol: risk parameters updated.
pub const ACTION_RISK_UPD: &str = "risk_upd";

// ─── Event payload types ──────────────────────────────────────────────────────

/// Payload for credit-line lifecycle events (open, suspend, close, default, reinstate).
///
/// # Topic
/// `(Symbol::new(env, "credit"), action: Symbol)`
///
/// where `action` is one of: `"opened"`, `"suspend"`, `"closed"`, `"default"`, `"reinstate"`.
///
/// # Data
/// `CreditLineEvent { event_type, borrower, status, credit_limit, interest_rate_bps, risk_score }`
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreditLineEvent {
    /// Sub-action symbol (e.g. `"opened"`, `"closed"`).
    pub event_type: Symbol,
    /// Address of the borrower.
    pub borrower: Address,
    /// New status of the credit line after this event.
    pub status: CreditStatus,
    /// Credit limit at the time of the event.
    pub credit_limit: i128,
    /// Interest rate in basis points at the time of the event.
    pub interest_rate_bps: u32,
    /// Risk score at the time of the event.
    pub risk_score: u32,
}

/// Versioned lifecycle event for analytics/indexers.
///
/// Emitted alongside `CreditLineEvent` so existing indexers remain compatible
/// while new consumers migrate to v2.
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

/// Payload for repayment events.
///
/// # Topic
/// `(Symbol::new(env, "repay"), borrower: Address)`
///
/// # Data
/// `RepaymentEvent { borrower, amount, new_utilized_amount, timestamp }`
///
/// `amount` is the **effective** repayment (capped at `utilized_amount`).
/// `new_utilized_amount` is the outstanding balance after this repayment.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepaymentEvent {
    /// Address of the borrower.
    pub borrower: Address,
    /// Effective amount repaid (≤ original `utilized_amount`).
    pub amount: i128,
    /// Outstanding principal after this repayment.
    pub new_utilized_amount: i128,
    /// Ledger timestamp of the repayment.
    pub timestamp: u64,
}

/// Versioned repayment event with explicit payer identifier.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepaymentEventV2 {
    pub borrower: Address,
    pub payer: Address,
    pub amount: i128,
    pub new_utilized_amount: i128,
    pub timestamp: u64,
}

/// Payload for risk-parameter update events.
///
/// # Topic
/// `(Symbol::new(env, "credit"), Symbol::new(env, "risk_upd"))`
///
/// # Data
/// `RiskParametersUpdatedEvent { borrower, credit_limit, interest_rate_bps, risk_score }`
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

/// Payload for draw (borrow) events.
///
/// # Topic
/// `(Symbol::new(env, "drawn"), borrower: Address)`
///
/// # Data
/// `DrawnEvent { borrower, amount, new_utilized_amount, timestamp }`
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DrawnEvent {
    /// Address of the borrower.
    pub borrower: Address,
    /// Amount drawn in this operation.
    pub amount: i128,
    /// New outstanding principal after this draw.
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

// ─── Publisher helpers ────────────────────────────────────────────────────────

/// Publish a credit-line lifecycle event.
///
/// # Topic
/// `(Symbol::new(env, "credit"), action: Symbol)`
///
/// # Data
/// [`CreditLineEvent`]
pub fn publish_credit_line_event(env: &Env, action: Symbol, event: CreditLineEvent) {
    env.events()
        .publish((Symbol::new(env, TOPIC_CREDIT), action), event);
}

/// Publish a v2 credit line lifecycle event.
#[allow(dead_code)]
pub fn publish_credit_line_event_v2(env: &Env, action: Symbol, event: CreditLineEventV2) {
    env.events()
        .publish((Symbol::new(env, TOPIC_CREDIT), action), event);
}

/// Publish a repayment event.
///
/// # Topic
/// `(Symbol::new(env, "repay"), borrower: Address)`
///
/// # Data
/// [`RepaymentEvent`]
pub fn publish_repayment_event(env: &Env, event: RepaymentEvent) {
    env.events().publish(
        (
            Symbol::new(env, TOPIC_REPAY),
            event.borrower.clone(),
        ),
        event,
    );
}

/// Publish a v2 repayment event.
#[allow(dead_code)]
pub fn publish_repayment_event_v2(env: &Env, event: RepaymentEventV2) {
    env.events().publish(
        (
            Symbol::new(env, TOPIC_REPAY),
            event.borrower.clone(),
        ),
        event,
    );
}

/// Publish a draw event.
///
/// # Topic
/// `(Symbol::new(env, "drawn"), borrower: Address)`
///
/// # Data
/// [`DrawnEvent`]
pub fn publish_drawn_event(env: &Env, event: DrawnEvent) {
    env.events().publish(
        (
            Symbol::new(env, TOPIC_DRAWN),
            event.borrower.clone(),
        ),
        event,
    );
}

/// Publish a v2 drawn event.
#[allow(dead_code)]
pub fn publish_drawn_event_v2(env: &Env, event: DrawnEventV2) {
    env.events().publish(
        (
            Symbol::new(env, TOPIC_DRAWN),
            event.borrower.clone(),
        ),
        event,
    );
}

/// Publish a risk parameters updated event.
///
/// # Topic
/// `(Symbol::new(env, "credit"), Symbol::new(env, "risk_upd"))`
///
/// # Data
/// [`RiskParametersUpdatedEvent`]
pub fn publish_risk_parameters_updated(env: &Env, event: RiskParametersUpdatedEvent) {
    env.events().publish(
        (
            Symbol::new(env, TOPIC_CREDIT),
            Symbol::new(env, ACTION_RISK_UPD),
        ),
        event,
    );
}

/// Publish an interest accrued event.
#[allow(dead_code)]
pub fn publish_interest_accrued_event(env: &Env, event: InterestAccruedEvent) {
    env.events().publish(
        (
            Symbol::new(env, TOPIC_CREDIT),
            symbol_short!("accrue"),
        ),
        event,
    );
}
