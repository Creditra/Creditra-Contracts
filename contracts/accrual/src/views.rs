// SPDX-License-Identifier: MIT

//! Accrual (v7) read-only capabilities view.
//!
//! # What
//!
//! Exposes [`accrual_capabilities`], a pure read-only query that returns an
//! [`AccrualCapabilities`] bitmap for a given borrower. Clients and
//! off-chain tooling can call this view to understand which accrual-related
//! operations are currently available without simulating a full transaction.
//!
//! # Design
//!
//! The implementation delegates entirely to
//! [`creditra_credit::views::accrual_capabilities`] via the re-export in
//! [`crate`]. The accrual crate is a thin wrapper over the credit contract;
//! all state lives there. This module exists as the stable public API anchor
//! for the v7 accrual surface.
//!
//! # See also
//! - [`creditra_credit::types::AccrualCapabilities`] — the returned type.
//! - [`creditra_credit::views::accrual_capabilities`] — the underlying implementation.
//! - [`tests/capabilities.rs`] — focused unit tests for this view.

use creditra_credit::types::AccrualCapabilities;
use soroban_sdk::{Address, Env};

/// Return the accrual-subsystem capabilities bitmap for `borrower`.
///
/// # What
///
/// Evaluates the same pre-flight conditions as `accrue_batch` and
/// `apply_accrual` **without** executing any accrual math or writing any
/// storage. Suitable for off-chain tooling, keeper bots, and on-chain
/// integrations that need to query availability before constructing a
/// transaction.
///
/// # Parameters
/// - `env`:      Soroban execution environment.
/// - `borrower`: The borrower address to query.
///
/// # Returns
///
/// An [`AccrualCapabilities`] struct with four read-only flags:
///
/// | Field                  | `true` when …                                                               |
/// |------------------------|-----------------------------------------------------------------------------|
/// | `can_accrue`           | Line exists, `Active`, `utilized_amount > 0`, protocol not paused           |
/// | `batch_open`           | Protocol circuit breaker is not engaged                                     |
/// | `penalty_rate_active`  | `penalty_surcharge_bps > 0` and borrower is currently delinquent            |
/// | `grace_waiver_active`  | Line is `Suspended`, grace config set, and `now ≤ suspension_ts + grace`    |
///
/// # Security
///
/// - **No authentication required** — this is a pure read-only view.
/// - **No state mutations** — ledger storage is only read, never written.
/// - **No token CPIs** — this function makes no cross-contract calls.
pub fn accrual_capabilities(env: Env, borrower: Address) -> AccrualCapabilities {
    creditra_credit::views::accrual_capabilities(env, borrower)
}
