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
//! | Event struct                    | Topic                      | Emitted when …                                    |
//! |---------------------------------|----------------------------|---------------------------------------------------|
//! | [`CreditLineQueriedEvent`]      | `("query", "cl_read")`     | `get_credit_line` is called for a borrower        |
//! | [`HealthFactorQueriedEvent`]    | `("query", "hf_read")`     | `get_health_factor` is called for a borrower      |
//! | [`DelinquencyCheckedEvent`]     | `("query", "dlq_chk")`     | `is_delinquent` is called for a borrower          |
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
//!
//! - [`creditra_credit::query`] — the read-only query implementation.
//! - [`contracts/accrual/src/events.rs`] — accrual event pattern reference.

use soroban_sdk::{contracttype, symbol_short, Address, Env};

// ── Event structs ─────────────────────────────────────────────────────────────

/// Emitted when `get_credit_line` is called for a borrower.
///
/// # Fields
///
/// | Field | Type | Description |
/// |-------|------|-------------|
/// | `borrower` | `Address` | The address whose credit line was queried |
/// | `found` | `bool` | `true` when a credit-line record exists; `false` when `None` |
/// | `timestamp` | `u64` | Ledger timestamp at which the query was executed |
///
/// # Topic
///
/// `("query", "cl_read")` — both segments encoded with `symbol_short!`.
///
/// # ABI stability
///
/// Field positions are stable. New fields may only be appended. The topic
/// string `"cl_read"` is pinned; do not reuse it for a different payload.
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
///
/// | Field | Type | Description |
/// |-------|------|-------------|
/// | `borrower` | `Address` | The address whose health factor was queried |
/// | `health_bps` | `u32` | Computed health factor in basis points; `u32::MAX` = zero utilization |
/// | `timestamp` | `u64` | Ledger timestamp at which the query was executed |
///
/// # `health_bps` interpretation
///
/// - `u32::MAX` — the borrower has no outstanding debt (infinitely healthy).
/// - `< 10_000` — under-collateralized; position is eligible for liquidation.
/// - `10_000` — collateral exactly meets the minimum ratio.
/// - `> 10_000` — over-collateralized.
///
/// # Topic
///
/// `("query", "hf_read")` — both segments encoded with `symbol_short!`.
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
///
/// | Field | Type | Description |
/// |-------|------|-------------|
/// | `borrower` | `Address` | The address whose delinquency status was checked |
/// | `is_delinquent` | `bool` | `true` when the borrower has missed an installment past the grace window |
/// | `timestamp` | `u64` | Ledger timestamp at which the check was executed |
///
/// # `is_delinquent` semantics
///
/// `false` is also returned when the short-circuit conditions are not met:
/// no credit line, closed line, zero utilization, or no repayment schedule.
/// Publishers should record the exact conditions in the surrounding call
/// context rather than relying solely on this flag for audit trails.
///
/// # Topic
///
/// `("query", "dlq_chk")` — both segments encoded with `symbol_short!`.
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
///
/// | Field | Type | Description |
/// |-------|------|-------------|
/// | `total_utilized` | `i128` | Global sum of all credit-line `utilized_amount` values at query time |
/// | `active_line_count` | `u32` | Number of Active credit lines at query time |
/// | `timestamp` | `u64` | Ledger timestamp at which the summary was read |
///
/// # Notes
///
/// - `total_utilized` reflects the lazy-accrual state: it does not include
///   pending interest since each borrower's last checkpoint.
/// - `active_line_count` counts only lines with status `Active`; Suspended,
///   Defaulted, and Closed lines are excluded.
///
/// # Topic
///
/// `("query", "proto_rd")` — both segments encoded with `symbol_short!`.
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

/// Publish a [`CreditLineQueriedEvent`] to the Soroban event ledger.
///
/// Opt-in observability for `get_credit_line` call sites. Invoke this helper
/// alongside `get_credit_line` when you want the read to be observable by
/// off-chain indexers.
///
/// # Parameters
///
/// - `env`: Soroban execution environment.  Must be the active contract
///   environment; the event is attributed to the currently executing contract.
/// - `event`: Fully constructed [`CreditLineQueriedEvent`] payload.  The
///   caller is responsible for populating `borrower`, `found`, and `timestamp`
///   before passing the struct here.
///
/// # Returns
///
/// `()` — this function has no return value.  On success, exactly one event
/// is appended to the transaction's event log.
///
/// # Errors
///
/// This function does not panic or return an error under normal operation.
/// Soroban's event API itself is infallible in the WASM context.
///
/// # Security
///
/// - Events are append-only and cannot be removed or altered after emission.
/// - The emitted topic and payload are visible to all indexers watching the
///   `("query", "cl_read")` filter — do not include sensitive fields.
/// - There is no auth gate: any code executing inside the contract can call
///   this helper.  It must only be called from within a trusted call path.
///
/// # ABI stability
///
/// The topic string `("query", "cl_read")` is pinned.  The payload layout
/// ([`CreditLineQueriedEvent`] field order) is stable; new fields may only
/// be appended.
///
/// # Example
///
/// ```rust,ignore
/// use creditra_query::events::{publish_credit_line_queried, CreditLineQueriedEvent};
///
/// let event = CreditLineQueriedEvent {
///     borrower: borrower.clone(),
///     found: line.is_some(),
///     timestamp: env.ledger().timestamp(),
/// };
/// publish_credit_line_queried(&env, event);
/// ```
///
/// # See also
///
/// - [`CreditLineQueriedEvent`] — the payload type.
/// - [`publish_health_factor_queried`] — companion helper for health-factor reads.
pub fn publish_credit_line_queried(env: &Env, event: CreditLineQueriedEvent) {
    env.events().publish(
        (symbol_short!("query"), symbol_short!("cl_read")),
        event,
    );
}

/// Publish a [`HealthFactorQueriedEvent`] to the Soroban event ledger.
///
/// Opt-in observability for `get_health_factor` call sites. Invoke this
/// helper alongside `get_health_factor` when you want the read to be
/// observable by off-chain indexers and liquidation bots.
///
/// # Parameters
///
/// - `env`: Soroban execution environment.  Must be the active contract
///   environment; the event is attributed to the currently executing contract.
/// - `event`: Fully constructed [`HealthFactorQueriedEvent`] payload.
///   The caller is responsible for populating `borrower`, `health_bps`
///   (from the return value of `get_health_factor`), and `timestamp`.
///
/// # Returns
///
/// `()` — this function has no return value.  On success, exactly one event
/// is appended to the transaction's event log.
///
/// # Errors
///
/// This function does not panic or return an error under normal operation.
///
/// # Security
///
/// - The emitted topic `("query", "hf_read")` and payload are visible to all
///   indexers.  `health_bps = u32::MAX` is the public sentinel for zero
///   utilization and is safe to emit.
/// - There is no auth gate; callers must only invoke this from trusted paths.
///
/// # ABI stability
///
/// The topic string `("query", "hf_read")` is pinned.  Field order in
/// [`HealthFactorQueriedEvent`] is stable.
///
/// # Example
///
/// ```rust,ignore
/// use creditra_query::events::{publish_health_factor_queried, HealthFactorQueriedEvent};
///
/// let hf = client.get_health_factor(&borrower);
/// let event = HealthFactorQueriedEvent {
///     borrower: borrower.clone(),
///     health_bps: hf,
///     timestamp: env.ledger().timestamp(),
/// };
/// publish_health_factor_queried(&env, event);
/// ```
///
/// # See also
///
/// - [`HealthFactorQueriedEvent`] — the payload type and `health_bps` interpretation.
/// - [`publish_delinquency_checked`] — companion helper for delinquency reads.
pub fn publish_health_factor_queried(env: &Env, event: HealthFactorQueriedEvent) {
    env.events().publish(
        (symbol_short!("query"), symbol_short!("hf_read")),
        event,
    );
}

/// Publish a [`DelinquencyCheckedEvent`] to the Soroban event ledger.
///
/// Opt-in observability for `is_delinquent` call sites. Invoke this helper
/// alongside `is_delinquent` when you want delinquency checks to be
/// observable by off-chain risk monitors and collections bots.
///
/// # Parameters
///
/// - `env`: Soroban execution environment.  Must be the active contract
///   environment; the event is attributed to the currently executing contract.
/// - `event`: Fully constructed [`DelinquencyCheckedEvent`] payload.
///   The caller is responsible for populating `borrower`, `is_delinquent`
///   (from the return value of `is_delinquent`), and `timestamp`.
///
/// # Returns
///
/// `()` — this function has no return value.  On success, exactly one event
/// is appended to the transaction's event log.
///
/// # Errors
///
/// This function does not panic or return an error under normal operation.
///
/// # Security
///
/// - Emitting `is_delinquent = false` is safe and does not reveal sensitive
///   state; it simply records that the check was performed.
/// - The topic `("query", "dlq_chk")` is pinned; do not reuse for other
///   delinquency-adjacent events.
/// - There is no auth gate; callers must only invoke this from trusted paths.
///
/// # ABI stability
///
/// The topic string `("query", "dlq_chk")` is pinned.  Field order in
/// [`DelinquencyCheckedEvent`] is stable.
///
/// # Example
///
/// ```rust,ignore
/// use creditra_query::events::{publish_delinquency_checked, DelinquencyCheckedEvent};
///
/// let delinquent = client.is_delinquent(&borrower);
/// let event = DelinquencyCheckedEvent {
///     borrower: borrower.clone(),
///     is_delinquent: delinquent,
///     timestamp: env.ledger().timestamp(),
/// };
/// publish_delinquency_checked(&env, event);
/// ```
///
/// # See also
///
/// - [`DelinquencyCheckedEvent`] — the payload type and `is_delinquent` semantics.
/// - [`publish_credit_line_queried`] — companion helper for credit-line reads.
pub fn publish_delinquency_checked(env: &Env, event: DelinquencyCheckedEvent) {
    env.events().publish(
        (symbol_short!("query"), symbol_short!("dlq_chk")),
        event,
    );
}

/// Publish a [`ProtocolSummaryQueriedEvent`] to the Soroban event ledger.
///
/// Opt-in observability for `get_protocol_summary` call sites. Invoke this
/// helper alongside `get_protocol_summary` when you want the aggregate read
/// to be observable by off-chain dashboards and protocol monitors.
///
/// # Parameters
///
/// - `env`: Soroban execution environment.  Must be the active contract
///   environment; the event is attributed to the currently executing contract.
/// - `event`: Fully constructed [`ProtocolSummaryQueriedEvent`] payload.
///   The caller is responsible for populating `total_utilized`,
///   `active_line_count` (from the `ProtocolSummary` returned by
///   `get_protocol_summary`), and `timestamp`.
///
/// # Returns
///
/// `()` — this function has no return value.  On success, exactly one event
/// is appended to the transaction's event log.
///
/// # Errors
///
/// This function does not panic or return an error under normal operation.
///
/// # Security
///
/// - Protocol summary aggregates are not sensitive (no per-borrower data is
///   included); emitting them is safe.
/// - The topic `("query", "proto_rd")` is pinned; do not reuse for other
///   protocol-level reads.
/// - There is no auth gate; callers must only invoke this from trusted paths.
///
/// # ABI stability
///
/// The topic string `("query", "proto_rd")` is pinned.  Field order in
/// [`ProtocolSummaryQueriedEvent`] is stable; new fields may only be appended.
///
/// # Example
///
/// ```rust,ignore
/// use creditra_query::events::{
///     publish_protocol_summary_queried, ProtocolSummaryQueriedEvent,
/// };
///
/// let summary = client.get_protocol_summary();
/// let event = ProtocolSummaryQueriedEvent {
///     total_utilized: summary.total_utilized,
///     active_line_count: summary.count,
///     timestamp: env.ledger().timestamp(),
/// };
/// publish_protocol_summary_queried(&env, event);
/// ```
///
/// # See also
///
/// - [`ProtocolSummaryQueriedEvent`] — the payload type and field semantics.
/// - [`publish_credit_line_queried`] — companion helper for per-borrower reads.
pub fn publish_protocol_summary_queried(env: &Env, event: ProtocolSummaryQueriedEvent) {
    env.events().publish(
        (symbol_short!("query"), symbol_short!("proto_rd")),
        event,
    );
}
