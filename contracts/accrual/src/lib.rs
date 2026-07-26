// SPDX-License-Identifier: MIT
#![cfg_attr(not(test), no_std)]

//! Creditra accrual v7 contract — re-exports the credit contract's accrual
//! surface for error-stability testing, event indexer support, and compositional reuse.
//!
//! # Overview
//!
//! The accrual engine lives in [`creditra_credit::accrual`]. This crate serves as:
//! 1. A thin wrapper anchoring [`creditra_credit::types::ContractError`] discriminants
//!    relevant to the v7 interest-accrual subsystem for CI stability guards (see [`tests/err_stab.rs`]).
//! 2. The canonical definition crate for versioned, structured accrual events
//!    ([`events::AccrualBatchCompletedEvent`] and [`events::InterestAccruedEvent`]).
//!
//! # Public Surface
//!
//! - [`events`] — Structured event definitions and event publishing helpers.
//! - Re-exported public functions and types from [`creditra_credit`].

pub mod events;

pub use creditra_credit::*;

