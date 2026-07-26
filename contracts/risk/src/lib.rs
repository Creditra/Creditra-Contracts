// SPDX-License-Identifier: MIT
#![cfg_attr(not(test), no_std)]

//! Creditra risk v7 — read-only capabilities bitmap for the risk subsystem.
//!
//! The view implementation lives in `contracts/risk/src/views.rs`, compiled
//! into [`creditra_credit`] via `#[path]`. This crate anchors focused
//! integration tests for the v7 risk capabilities guard.

pub use creditra_credit::*;
