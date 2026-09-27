// SPDX-License-Identifier: MIT

//! Structured lifecycle events for the query (v7) subsystem.
//!
//! # What
//!
//! Defines the event types and publisher helpers that off-chain indexers can
//! observe when the credit contract's read-only query entrypoints are invoked.
//! Because query calls are pure reads, these events are **opt-in**: call sites
//! that wish to make a query observable on-chain invoke the corresponding
//! `publish_*` helper alongside the query function.
//!
//! # Events
//!
//! | Event struct                  | Topic                        | Emitted when …                                    |
//! |-------------------------------|------------------------------|---------------------------------------------------|
//! | [`CreditLineQueriedEvent`]    | `("query", "cl_read")`       | `get_credit_line` is called for a borrower        |
//! | [`HealthFactorQueriedEvent`]  | `("query", "hf_read")`       | `get_health_factor` is called for a borrower      |
//! | [`DelinquencyCheckedEvent`]   | `("query", "dlq_chk")`       | `is_delinquent` is called for a borrower          |
//! | [`ProtocolSummaryQueriedEvent`] | `("query", "proto_rd")`    | `get_protocol_summary` is called                  |
//!
//! # Topics
//!
//! All events are published under the `("query", _)` namespace using
//! `symbol_short!` (≤ 9 characters) for cheap on-chain encoding. The second
//! topic identifies the specific query operation.
//!
//! # ABI Stability
//!
//! Event topics and payload field layouts are part of the contract's public ABI.
//! Breaking changes require a new topic with a version suffix
//! (e.g., `("query", "cl_read2")`). Existing topics must not be repurposed.
//!
//! # See also
//! - [`creditra_credit::query`] — the read-only query implementation.
//! - [`contracts/accrual/src/events.rs`] — accrual event pattern reference.

use soroban_sdk::{contracttype, symbol_short, Address, Env};

// ── Event structs ─────────────────────────────────────────────────────────────

/// Emitted when `get_credit_line` is called for a borrower.
///
/// # Fields
/// - `borrower`: The address whose credit line was queried.
/// - `found`: `true` when a credit line record exists; `false` when `None`.
/// - `timestamp`: Ledger timestamp at which the query was executed.
///
/// # Topic
/// `("query", "cl_read")`
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreditLineQueriedEvent {
    /// The borrower address that was queried.
    pub borrower: Address,
    /// Whether a credit line record was found (`true`) or not (`false`).
    pub found: bool,
    /// Ledger timestamp at the time of the query.
    pub timestamp: u64,
}

/// Emitted when `get_health_factor` is called for a borrower.
///
/// # Fields
/// - `borrower`: The address whose health factor was queried.
/// - `health_bps`: The computed health factor in basis points.
///   `u32::MAX` indicates zero utilization (infinitely healthy).
/// - `timestamp`: Ledger timestamp at which the query was executed.
///
/// # Topic
/// `("query", "hf_read")`
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HealthFactorQueriedEvent {
    /// The borrower address whose health factor was computed.
    pub borrower: Address,
    /// Health factor in basis points (10_000 = exactly at minimum ratio).
    /// `u32::MAX` means zero utilization.
    pub health_bps: u32,
    /// Ledger timestamp at time of query.
    pub timestamp: u64,
}

/// Emitted when `is_delinquent` is called for a borrower.
///
/// # Fields
/// - `borrower`: The address whose delinquency status was checked.
/// - `is_delinquent`: `true` when the borrower has missed an installment
///   past the grace window; `false` otherwise.
/// - `timestamp`: Ledger timestamp at which the check was executed.
///
/// # Topic
/// `("query", "dlq_chk")`
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DelinquencyCheckedEvent {
    /// The borrower address whose delinquency was checked.
    pub borrower: Address,
    /// Whether the borrower is currently delinquent.
    pub is_delinquent: bool,
    /// Ledger timestamp at time of check.
    pub timestamp: u64,
}

/// Emitted when `get_protocol_summary` is called.
///
/// # Fields
/// - `total_utilized`: Global sum of all credit line `utilized_amount` values.
/// - `active_line_count`: Number of currently Active credit lines.
/// - `timestamp`: Ledger timestamp at which the summary was read.
///
/// # Topic
/// `("query", "proto_rd")`
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtocolSummaryQueriedEvent {
    /// Global utilized principal at the time of the query.
    pub total_utilized: i128,
    /// Number of Active credit lines at the time of the query.
    pub active_line_count: u32,
    /// Ledger timestamp at time of query.
    pub timestamp: u64,
}

// ── Publisher helpers ─────────────────────────────────────────────────────────

/// Publish a credit-line-queried event.
///
/// # Topic
/// `("query", "cl_read")` — emitted once per `get_credit_line` call that
/// opts in to on-chain observability.
pub fn publish_credit_line_queried(env: &Env, event: CreditLineQueriedEvent) {
    env.events().publish(
        (symbol_short!("query"), symbol_short!("cl_read")),
        event,
    );
}

/// Publish a health-factor-queried event.
///
/// # Topic
/// `("query", "hf_read")` — emitted once per `get_health_factor` call that
/// opts in to on-chain observability.
pub fn publish_health_factor_queried(env: &Env, event: HealthFactorQueriedEvent) {
    env.events().publish(
        (symbol_short!("query"), symbol_short!("hf_read")),
        event,
    );
}

/// Publish a delinquency-checked event.
///
/// # Topic
/// `("query", "dlq_chk")` — emitted once per `is_delinquent` call that
/// opts in to on-chain observability.
pub fn publish_delinquency_checked(env: &Env, event: DelinquencyCheckedEvent) {
    env.events().publish(
        (symbol_short!("query"), symbol_short!("dlq_chk")),
        event,
    );
}

/// Publish a protocol-summary-queried event.
///
/// # Topic
/// `("query", "proto_rd")` — emitted once per `get_protocol_summary` call
/// that opts in to on-chain observability.
pub fn publish_protocol_summary_queried(env: &Env, event: ProtocolSummaryQueriedEvent) {
    env.events().publish(
        (symbol_short!("query"), symbol_short!("proto_rd")),
        event,
    );
}
