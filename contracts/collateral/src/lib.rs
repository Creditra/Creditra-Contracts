// SPDX-License-Identifier: MIT
#![cfg_attr(not(test), no_std)]

//! Creditra collateral v7 — admin cool-off between critical collateral actions.
//!
//! Implementation lives in [`creditra_credit`] via
//! `contracts/collateral/src/admin.rs` (compiled into the credit crate).
//! This crate anchors focused integration tests for the v7 admin cooldown guard.
//!
//! # Modules
//!
//! - [`views`] — read-only `collateral_capabilities` bitmap view (#861).

pub use creditra_credit::*;

/// Read-only capabilities view for collateral operations.
///
/// Re-exports [`views::collateral_capabilities`] at the crate root so callers
/// can use `creditra_collateral::collateral_capabilities` without knowing the
/// internal module structure.
pub mod views;
pub use views::collateral_capabilities;
