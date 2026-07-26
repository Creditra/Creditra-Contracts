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
//!
//! ## Error stability
//!
//! Anchors the [`creditra_credit::types::ContractError`] discriminants
//! relevant to the v7 query subsystem for CI stability guards.
//! See [`tests/err_stab.rs`] for the pinning assertions.
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
//! - `capabilities` (accrual capabilities view)

pub mod events;
pub use creditra_credit::*;
