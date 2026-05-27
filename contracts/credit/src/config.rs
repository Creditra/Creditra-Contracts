// SPDX-License-Identifier: MIT

//! Contract initialization and configuration helpers.
//!
//! # Initialization contract
//!
//! [`init`] is a **one-time** operation. It stores the admin address and sets
//! the default liquidity source. Calling it a second time reverts with
//! [`ContractError::AlreadyInitialized`] so the admin cannot be overwritten
//! after deployment.
//!
//! See `docs/deploy.md` for the required deployment sequence.

use crate::auth::require_admin_auth;
use crate::storage::{admin_key, DataKey};
use crate::types::ContractError;
use soroban_sdk::{Address, Env};

/// Initialize the contract with an admin address (one-time only).
///
/// # Behavior
/// - Stores `admin` in instance storage under the `"admin"` key.
/// - Sets `LiquiditySource` to the contract's own address as the default.
/// - Reverts with [`ContractError::AlreadyInitialized`] if called again,
///   preventing admin takeover via re-initialization.
///
/// # Parameters
/// - `env`:   Soroban execution environment.
/// - `admin`: Address that will hold admin authority over this contract.
///
/// # Errors
/// - [`ContractError::AlreadyInitialized`] — contract has already been initialized.
///
/// # Security
/// - Must be called by the deployer immediately after deployment.
/// - The admin address is immutable after initialization.
/// - No event is emitted on failure; state is not mutated if already initialized.
pub fn init(env: Env, admin: Address) {
    // Guard: prevent re-initialization and admin takeover.
    if env.storage().instance().has(&admin_key(&env)) {
        env.panic_with_error(ContractError::AlreadyInitialized);
    }
    env.storage().instance().set(&admin_key(&env), &admin);
    env.storage()
        .instance()
        .set(&DataKey::LiquiditySource, &env.current_contract_address());
}

/// @notice Sets the token contract used for reserve/liquidity checks and draw transfers.
/// @dev Admin-only.
#[allow(dead_code)]
pub fn set_liquidity_token(env: Env, token_address: Address) {
    require_admin_auth(&env);
    env.storage()
        .instance()
        .set(&DataKey::LiquidityToken, &token_address);
}

/// @notice Sets the address that provides liquidity for draw operations.
/// @dev Admin-only. If unset, init config uses the contract address.
#[allow(dead_code)]
pub fn set_liquidity_source(env: Env, reserve_address: Address) {
    require_admin_auth(&env);
    env.storage()
        .instance()
        .set(&DataKey::LiquiditySource, &reserve_address);
}
