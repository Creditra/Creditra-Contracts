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
/// Read-only, no-auth view. See [`QueryCapabilities`] for field semantics.
///
/// [`QueryCapabilities`]: crate::types::QueryCapabilities
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
            let delinquency_applicable =
                open && has_utilization && has_repayment_schedule;
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
