// SPDX-License-Identifier: MIT

//! Read-only query capabilities view (v7).
//!
//! Mirrors the short-circuit gates used by the credit contract's read-only
//! query entrypoints so off-chain clients and keepers can inspect which
//! borrower-scoped query results are currently meaningful — without issuing
//! multiple separate reads or simulating reverting calls.
//!
//! # What
//!
//! [`capabilities`] returns a [`crate::types::QueryCapabilities`] bitmap
//! covering the borrower-facing query surface:
//! `get_credit_line`, `get_repayment_schedule`, `get_health_factor`, and
//! `is_delinquent`.
//!
//! # How
//!
//! Each field is derived purely from storage (credit line, repayment
//! schedule, utilization, delinquency math) — a pure read with no token
//! CPIs, no auth checks, and no mutation.
//!
//! # Why
//!
//! Keepers and dashboards often need to know whether a borrower has a line,
//! a schedule, meaningful health-factor debt, or an active delinquency
//! before constructing follow-up transactions. Bundling those flags into one
//! view avoids N round-trips.
//!
//! # Compilation
//!
//! This module is compiled into `creditra-credit` via a `#[path]` include
//! (same pattern as [`contracts/lifecycle/src/views.rs`]). The public
//! entrypoint is `Credit::query_capabilities`.
//!
//! See [`docs/PROTOCOL_SPEC.md`](../../../docs/PROTOCOL_SPEC.md) for the
//! query surface this bitmap summarizes.

use crate::query::{get_credit_line, get_repayment_schedule, is_delinquent};
use crate::types::{CreditStatus, QueryCapabilities};
use soroban_sdk::{Address, Env};

/// Return the query-subsystem capabilities bitmap for `borrower`.
///
/// Each field in the returned [`QueryCapabilities`] struct answers one
/// "is this query currently meaningful?" question, allowing an off-chain
/// keeper or dashboard to issue a single contract call instead of
/// `N` separate reads followed by error handling.
///
/// # Parameters
///
/// - `env`: The Soroban execution environment. Provides ledger context,
///   storage access, and the timestamp used for delinquency evaluation.
/// - `borrower`: The on-chain address whose query capabilities are being
///   assessed. Any address may be passed; a missing credit line is handled
///   gracefully (all flags are `false`).
///
/// # Returns
///
/// A [`QueryCapabilities`] bitmap with the following fields:
///
/// | Field | `true` when |
/// |-------|-------------|
/// | `has_credit_line` | A credit-line record exists in persistent storage |
/// | `has_repayment_schedule` | A repayment schedule is configured for the borrower |
/// | `health_factor_applicable` | `utilized_amount > 0`; otherwise `get_health_factor` returns `u32::MAX` |
/// | `delinquency_applicable` | Line is open **and** `utilized_amount > 0` **and** schedule exists |
/// | `is_delinquent` | Delinquency check passes (only evaluated when `delinquency_applicable`) |
///
/// # Security
///
/// - **No authentication required.** This is a pure read-only view.
/// - No storage is mutated. Soroban TTL bumps may occur on persistent-entry
///   reads inside `get_credit_line` and `get_repayment_schedule`, but these
///   do not change any logical contract state.
/// - All inputs are validated implicitly: an unknown `borrower` address
///   returns an all-`false` bitmap without panicking.
///
/// # Errors
///
/// This function does not panic. All code paths return a valid
/// [`QueryCapabilities`] struct. Callers must not assume any field is `true`
/// without inspecting the returned value.
///
/// # Performance
///
/// Executes at most three storage reads:
/// 1. `get_credit_line` — persistent read, bumps TTL if below threshold.
/// 2. `get_repayment_schedule` — persistent read, bumps TTL if below threshold.
/// 3. `is_delinquent` (conditional) — reads grace-period config from instance
///    storage; only executed when `delinquency_applicable` is `true`.
///
/// # Example
///
/// ```rust,ignore
/// let caps = client.query_capabilities(&borrower);
/// if caps.delinquency_applicable && caps.is_delinquent {
///     // Borrower has missed an installment — trigger default flow.
/// }
/// if caps.health_factor_applicable {
///     let hf = client.get_health_factor(&borrower);
///     if hf < 10_000 {
///         // Position is under-collateralized — eligible for liquidation.
///     }
/// }
/// ```
///
/// # See also
///
/// - [`crate::query::get_health_factor`] — the health-factor formula and
///   edge-case documentation.
/// - [`crate::query::is_delinquent`] — the delinquency check this function
///   conditionally delegates to.
/// - [`QueryCapabilities`](crate::types::QueryCapabilities) — field-level
///   rustdoc for the returned type.
pub fn capabilities(env: Env, borrower: Address) -> QueryCapabilities {
    let credit_line = get_credit_line(env.clone(), borrower.clone());
    let schedule = get_repayment_schedule(env.clone(), borrower.clone());

    let has_credit_line = credit_line.is_some();
    let has_repayment_schedule = schedule.is_some();

    let (health_factor_applicable, delinquency_applicable) = match &credit_line {
        None => (false, false),
        Some(line) => {
            let has_utilization = line.utilized_amount > 0;
            let open = line.status != CreditStatus::Closed;
            let health_factor_applicable = has_utilization;
            // Mirrors `query::is_delinquent` short-circuits: needs an open
            // line with utilization and a configured repayment schedule.
            let delinquency_applicable = open && has_utilization && has_repayment_schedule;
            (health_factor_applicable, delinquency_applicable)
        }
    };

    let is_delinquent = if delinquency_applicable {
        is_delinquent(env, borrower)
    } else {
        false
    };

    QueryCapabilities {
        has_credit_line,
        has_repayment_schedule,
        health_factor_applicable,
        delinquency_applicable,
        is_delinquent,
    }
}
