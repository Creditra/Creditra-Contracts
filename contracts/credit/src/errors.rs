// SPDX-License-Identifier: MIT

//! Contract error definitions and taxonomy.
//!
//! Provides the primary [`ContractError`] enum and [`ContractErrorCategory`] classification
//! for error handling in the `creditra-credit` contract.
//!
//! # Error Code 39
//!
//! - [`ContractError::InsufficientCollateralBalance`] (`39`): Triggered when a borrower attempts to
//!   withdraw or release collateral exceeding their stored deposited collateral balance.

pub use crate::types::{ContractError, ContractErrorCategory};
