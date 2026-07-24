// SPDX-License-Identifier: MIT

//! Structured events for the accrual (v7) contract.
//!
//! # What
//!
//! Defines the structured event types and publisher helpers emitted by the
//! accrual subsystem. These events provide off-chain indexers with type-safe,
//! versioned payloads for tracking interest accrual operations.
//!
//! # Events
//!
//! - [`AccrualBatchCompletedEvent`] — emitted after `accrue_batch` completes,
//!   reporting the number of borrowers processed and the total interest accrued.
//! - [`InterestAccruedEvent`] — emitted per-borrower when interest is capitalized
//!   into `utilized_amount` via `apply_accrual`.
//!
//! # Topics
//!
//! All events are published under the `("accrual", _)` namespace using
//! `symbol_short!` (≤ 9 characters) for cheap on-chain encoding.
//!
//! # ABI Stability
//!
//! Event topics and payload field layouts are part of the contract's public ABI.
//! Breaking changes require a new event topic with a version suffix
//! (e.g., `("accrual","batch_v2")`).
//!
//! # See also
//! - [`creditra_credit::events`] — the credit contract's event definitions.
//! - [`docs/EVENTS_CATALOG.md`](../../../docs/EVENTS_CATALOG.md) — canonical event catalog.

use soroban_sdk::{contracttype, symbol_short, Address, Env};

/// Emitted after a batch accrual operation completes.
///
/// # Fields
/// - `borrowers_processed`: Number of borrower addresses submitted in the batch.
/// - `lines_accrued`: Number of credit lines that had interest capitalized
///   (subset of `borrowers_processed`; excludes missing/inactive lines).
/// - `total_interest_accrued`: Sum of all interest amounts capitalized across
///   all lines in this batch.
/// - `timestamp`: Ledger timestamp at which the batch was executed.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccrualBatchCompletedEvent {
    /// Number of borrower addresses submitted in the batch.
    pub borrowers_processed: u32,
    /// Number of credit lines with interest capitalized.
    pub lines_accrued: u32,
    /// Total interest capitalized across all lines in the batch.
    pub total_interest_accrued: i128,
    /// Ledger timestamp when the batch was executed.
    pub timestamp: u64,
}

/// Emitted per-borrower when interest is capitalized into `utilized_amount`.
///
/// # Fields
/// - `borrower`: The borrower whose credit line had interest accrued.
/// - `accrued_amount`: Amount of interest capitalized in this accrual step.
/// - `new_utilized_amount`: `utilized_amount` after capitalizing the accrued interest.
/// - `new_accrued_interest`: `accrued_interest` after this step.
/// - `elapsed_seconds`: Time delta since the last accrual timestamp.
/// - `timestamp`: Ledger timestamp at which accrual was executed.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterestAccruedEvent {
    /// Borrower whose credit line was accrued.
    pub borrower: Address,
    /// Interest amount capitalized in this step.
    pub accrued_amount: i128,
    /// utilized_amount after capitalizing interest.
    pub new_utilized_amount: i128,
    /// accrued_interest after this step.
    pub new_accrued_interest: i128,
    /// Seconds elapsed since last accrual (drives interest computation).
    pub elapsed_seconds: u64,
    /// Ledger timestamp at time of accrual.
    pub timestamp: u64,
}

/// Publish a batch accrual completed event.
///
/// # Topic
/// `("accrual", "batch")` — emitted once per `accrue_batch` call.
pub fn publish_accrual_batch_completed(env: &Env, event: AccrualBatchCompletedEvent) {
    env.events()
        .publish((symbol_short!("accrual"), symbol_short!("batch")), event);
}

/// Publish a per-borrower interest-accrued event.
///
/// # Topic
/// `("accrual", "accrue")` — emitted for each borrower whose line accrued interest.
pub fn publish_interest_accrued(env: &Env, event: InterestAccruedEvent) {
    env.events()
        .publish((symbol_short!("accrual"), symbol_short!("accrue")), event);
}
