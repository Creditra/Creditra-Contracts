// SPDX-License-Identifier: MIT
#![cfg_attr(not(test), no_std)]

//! # creditra-query (v7)
//!
//! Thin wrapper over the credit contract's read-only query surface.
//!
//! ## What
//!
//! This crate bundles three cohesive concerns for the Creditra v7 query
//! subsystem:
//!
//! - [`events`] — opt-in structured lifecycle events for query entrypoints,
//!   emitted so off-chain indexers can observe when read-only queries execute.
//! - [`views::capabilities`] — read-only [`creditra_credit::types::QueryCapabilities`]
//!   bitmap compiled into `creditra-credit` via `#[path]`; consumed by the
//!   `query_capabilities` contract entrypoint.
//! - Re-exported `creditra_credit::*` — the full public surface of the credit
//!   contract, available to tests and downstream integrators via a single
//!   `use creditra_query::*` import.
//!
//! ## Error stability
//!
//! Anchors the [`creditra_credit::types::ContractError`] discriminants
//! relevant to the v7 query subsystem for CI stability guards.
//! See `tests/err_stab.rs` for the pinning assertions.
//!
//! ## Query entrypoints covered
//!
//! The following credit-contract entrypoints are part of the query surface
//! this crate documents and tests:
//!
//! | Entrypoint | Returns | Notes |
//! |---|---|---|
//! | `get_credit_line` | `Option<CreditLineData>` | Pure read, no auth |
//! | `get_credit_line_summary` | `Option<CreditLineData>` | Backward-compat alias |
//! | `get_protocol_summary` | `ProtocolSummary` | Aggregate counters only |
//! | `get_repayment_schedule` | `Option<RepaymentSchedule>` | Bumps TTL on read |
//! | `get_health_factor` | `u32` bps | `u32::MAX` = zero utilization |
//! | `is_delinquent` | `bool` | Checks schedule + grace window |
//! | `get_credit_lines_paginated` | `CreditLinesPage` | Cursor-based, max 100 |
//! | `borrow_capabilities` | `BorrowCapabilities` | Pre-flight bitmap |
//! | `query_capabilities` | `QueryCapabilities` | Query availability bitmap |
//!
//! ## Module structure
//!
//! ```text
//! creditra_query
//! ├── events          (this crate's own module)
//! └── creditra_credit::* (re-exported wholesale)
//! ```
//!
//! ## Design notes
//!
//! `views.rs` is **not** declared as a submodule here. It is compiled into
//! `creditra-credit` via a `#[path = "../../query/src/views.rs"]` include so
//! that `crate::` paths inside `views.rs` resolve to the credit crate's
//! namespace. This is the same pattern used by
//! `contracts/lifecycle/src/views.rs`.
//!
//! ## See also
//!
//! - [`docs/PROTOCOL_SPEC.md`](../../../docs/PROTOCOL_SPEC.md) — per-entrypoint
//!   signatures, storage keys, and error returns.
//! - [`docs/EVENTS_CATALOG.md`](../../../docs/EVENTS_CATALOG.md) — full event
//!   topic registry.

/// Structured lifecycle events for the v7 query subsystem.
///
/// Each publisher helper in this module emits a single Soroban event under
/// the `("query", _)` topic namespace.  Call sites that want their read-only
/// queries to be observable on-chain invoke the corresponding `publish_*`
/// function alongside the query.
///
/// See the module-level rustdoc in [`events`] for the full event table,
/// topic strings, and ABI-stability guarantees.
pub mod events;

// `views` is compiled into `creditra-credit` via `#[path]` (see
// `contracts/credit/src/lib.rs`). It is intentionally not declared as a
// submodule here so `crate::` inside `views.rs` resolves to the credit crate.

/// Re-export the entire `creditra-credit` public API.
///
/// This allows downstream crates, integration tests, and SDK clients to
/// depend only on `creditra-query` while still accessing the full
/// `creditra_credit` type and contract surface:
///
/// ```rust,ignore
/// use creditra_query::{Credit, CreditClient, types::ContractError};
/// ```
///
/// # Re-exported items (non-exhaustive)
///
/// - `Credit` — the `#[contract]` struct (for `env.register`).
/// - `CreditClient` — the auto-generated test client.
/// - `types::*` — `CreditLineData`, `ContractError`, `QueryCapabilities`, …
/// - `events::*` — all 25+ event payload structs.
/// - `compute_rate_from_score` — the risk-pricing formula.
///
/// # Stability
///
/// The re-export is `pub use creditra_credit::*`, so any item added or
/// removed from `creditra-credit`'s public API is automatically reflected
/// here without a change to this crate.
pub use creditra_credit::*;
