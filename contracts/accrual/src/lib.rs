// SPDX-License-Identifier: MIT
#![cfg_attr(not(test), no_std)]

//! Creditra accrual v7 contract — re-exports the credit contract's accrual
//! surface for error-stability testing and compositional reuse.
//!
//! The accrual engine lives in [`creditra_credit::accrual`]; this crate is a
//! thin wrapper that anchors the [`creditra_credit::types::ContractError`]
//! discriminants relevant to the v7 accrual subsystem for CI stability
//! guards. See [`tests/err_stab.rs`] for the pinning assertions.

pub use creditra_credit::*;
