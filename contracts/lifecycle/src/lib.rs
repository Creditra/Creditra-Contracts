// SPDX-License-Identifier: MIT
#![cfg_attr(not(test), no_std)]

//! Creditra lifecycle v7 contract — re-exports the credit contract's lifecycle
//! surface for error-stability testing and compositional reuse.
//!
//! The lifecycle engine lives in [`creditra_credit::lifecycle`]; this crate is a
//! thin wrapper that anchors the [`creditra_credit::types::ContractError`]
//! discriminants relevant to the v7 lifecycle subsystem for CI stability
//! guards. See [`tests/err_stab.rs`] for the pinning assertions.

//! Lifecycle hot reads refresh persistent credit-line TTL without changing
//! contract entrypoints, parameters, return values, events, or error codes.

pub use creditra_credit::*;
