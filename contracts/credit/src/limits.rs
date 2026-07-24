// SPDX-License-Identifier: MIT

//! Per-borrower exposure cap for the Credit contract.
//!
//! # What
//!
//! Provides an absolute `i128` cap on total borrowed exposure per borrower,
//! enforced during [`draw_credit`]. Unlike the utilization cap (which is a
//! percentage of `credit_limit`), this is a flat amount ceiling independent of
//! the credit limit.
//!
//! # How
//!
//! The cap is stored under [`DataKey::MaxBorrowerExposure(Address)`] in
//! persistent storage and administered via `set_borrower_exposure_cap`.
//! The check function [`check_borrower_exposure_cap`] is called during
//! `draw_credit` after the utilization cap check and before the global
//! exposure cap check.
//!
//! # Why
//!
//! A per-borrower absolute exposure cap protects the protocol from
//! concentration risk — no single borrower can draw more than the cap
//! even if their credit limit is higher. This is complementary to the
//! global [`MaxTotalExposure`] cap.

use crate::storage::get_max_borrower_exposure;
use crate::types::ContractError;
use soroban_sdk::{Address, Env};

/// Check that a borrower's updated utilization does not exceed their
/// configured per-borrower exposure cap.
///
/// Returns `Ok(())` when:
/// - No cap is configured (`None`),
/// - `updated_utilized <= cap`.
///
/// Returns `Err(ContractError::BorrowerExposureCapExceeded)` when
/// `updated_utilized > cap`.
pub fn check_borrower_exposure_cap(
    env: &Env,
    borrower: &Address,
    updated_utilized: i128,
) -> Result<(), ContractError> {
    if let Some(cap) = get_max_borrower_exposure(env, borrower) {
        if updated_utilized > cap {
            return Err(ContractError::BorrowerExposureCapExceeded);
        }
    }
    Ok(())
}
