// SPDX-License-Identifier: MIT
#![cfg_attr(not(test), no_std)]

//! Creditra collateral v7 — admin cool-off between critical collateral actions.
//!
//! Implementation lives in [`creditra_credit`] via
//! `contracts/collateral/src/admin.rs` (compiled into the credit crate).
//! This crate anchors focused integration tests for the v7 admin cooldown guard.

pub mod views;
pub use views::*;

pub use creditra_credit::*;

