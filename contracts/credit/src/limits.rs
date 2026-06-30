// SPDX-License-Identifier: MIT

//! Borrower-level exposure limits.
//!
//! These helpers centralize the admin-configurable cap that bounds a
//! borrower's outstanding debt across the protocol. The cap is enforced on
//! draw execution and is intentionally distinct from the global protocol-wide
//! exposure cap and the per-line credit limit.

use crate::auth::require_admin_auth;
use crate::storage;
use crate::types::ContractError;
use soroban_sdk::{Address, Env};

/// Configure the maximum outstanding exposure for a borrower.
///
/// Passing `0` removes the cap. Negative values are rejected with
/// [`ContractError::InvalidAmount`].
pub fn set_borrower_exposure_cap(env: &Env, borrower: &Address, amount: i128) {
    require_admin_auth(env);

    if amount < 0 {
        env.panic_with_error(ContractError::InvalidAmount);
    }

    if amount == 0 {
        storage::set_borrower_exposure_cap(env, borrower, None);
    } else {
        storage::set_borrower_exposure_cap(env, borrower, Some(amount));
    }
}

/// Return the configured borrower exposure cap, if any.
pub fn get_borrower_exposure_cap(env: &Env, borrower: &Address) -> Option<i128> {
    storage::get_borrower_exposure_cap(env, borrower)
}

/// Enforce the configured per-borrower exposure cap on an updated balance.
///
/// Returns `true` when the draw remains within the configured maximum
/// outstanding exposure and `false` otherwise.
pub fn enforce_borrower_exposure_cap(
    env: &Env,
    borrower: &Address,
    updated_utilized: i128,
) -> bool {
    if let Some(max_exposure) = get_borrower_exposure_cap(env, borrower) {
        updated_utilized <= max_exposure
    } else {
        true
    }
}
