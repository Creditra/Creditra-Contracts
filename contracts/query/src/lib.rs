// SPDX-License-Identifier: MIT
#![cfg_attr(not(test), no_std)]

//! Creditra query v7 — gas snapshot and property tests for query entrypoints.
//!
//! This crate anchors focused integration tests that exercise the read-only
//! query surface of the Creditra credit contract:
//!
//! - `contracts/query/tests/gas_snap.rs`  — CPU/memory regression baselines
//! - `contracts/query/tests/proptest.rs`  — state-invariant property tests
//!
//! Implementation lives in [`creditra_credit`].

pub use creditra_credit::*;
