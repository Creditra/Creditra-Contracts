// SPDX-License-Identifier: MIT
#![cfg_attr(not(test), no_std)]

//! Creditra borrow v7 — error-stability test crate (#847).
//!
//! Borrow / draw / repay entrypoints live in [`creditra_credit::borrow`] and
//! the matching surface on [`creditra_credit::Credit`]. This package anchors
//! focused CI guards that freeze client-facing `ContractError` discriminants
//! for the v7 borrow subsystem. See [`tests/err_stab.rs`].
//!
//! # State Versioning
//!
//! The constant [`STATE_VERSION`] is the canonical on-chain state version
//! marker for the borrow subsystem.

pub use creditra_credit::*;

/// Explicit version marker for persisted borrow state.
pub const STATE_VERSION: u32 = 1;
