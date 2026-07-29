// SPDX-License-Identifier: MIT
#![cfg_attr(not(test), no_std)]

//! Creditra query (v7) crate.
//!
//! ## What
//!
//! Thin wrapper over the credit contract's read-only query surface.
//! Provides:
//!
//! - [`events`] — structured lifecycle events for query entrypoints, allowing
//!   off-chain indexers to observe when read-only queries are executed.
//! - [`views`] — read-only [`views::capabilities`] bitmap (compiled into
//!   `creditra-credit` via `#[path]`; see that module's rustdoc).
//!
//! ## Error stability
//!
//! Anchors the [`creditra_credit::types::ContractError`] discriminants
//! relevant to the v7 query subsystem for CI stability guards.
//! See [`tests/err_stab.rs`] for the pinning assertions.
//!
//! ## Auth snapshot
//!
//! Every query entrypoint is a pure read with no `require_auth` call. See
//! [`tests/auth_snap.rs`] for the per-entrypoint pinning assertions that
//! guard against a future entrypoint silently gaining an auth requirement.
//!
//! ## Query entrypoints covered
//!
//! - `get_credit_line` / `get_credit_line_summary`
//! - `get_protocol_summary`
//! - `get_repayment_schedule`
//! - `get_health_factor`
//! - `is_delinquent`
//! - `get_credit_lines_paginated`
//! - `borrow_capabilities`
//! - `query_capabilities` / `capabilities` (query capabilities view)

pub mod events;
// `views` is compiled into `creditra-credit` via `#[path]` (see
// `contracts/credit/src/lib.rs`). It is intentionally not declared as a
// submodule here so `crate::` inside `views.rs` resolves to the credit crate.
pub use creditra_credit::*;
