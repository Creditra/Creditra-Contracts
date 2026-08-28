// SPDX-License-Identifier: MIT

//! `creditra-collateral` — stable ContractError catalog for collateral operations.
//!
//! # Purpose
//!
//! This crate publishes a stable, scoped [`CollateralError`] catalog for the
//! Creditra collateral domain. The catalog is the **source of truth** for the
//! integer error codes emitted by current and future contract paths that
//! handle collateral deposits, withdrawals, ratio checks, and the
//! admin-managed collateral allowlist.
//!
//! # Stability
//!
//! The discriminants exported here are **permanent on deployment** for the
//! Creditra collateral contract wasm. Once this catalog is published, the
//! following invariants apply (mirroring the conventions enforced by
//! `contracts/credit/tests/error_discriminants.rs`):
//!
//! - Existing variants must **never** be reordered or renumbered.
//! - New variants must always be **appended** with the next available integer.
//! - Adding/removing a variant requires updating the integration test
//!   (`tests/catalog.rs`) and `docs/errors/collateral.md` in the same change.
//!
//! # Two-tier discriminant policy
//!
//! The catalog contains two tiers of variants:
//!
//! 1. **Mirror tier** (codes `5`, `12`, `22`, `35`, `39`):
//!    Variants that match the canonical `ContractError` codes published by
//!    `contracts/credit/src/types.rs`. The collateral domain reuses these
//!    errors verbatim because they convey the same semantic meaning
//!    (e.g. *"withdrawal amount exceeds deposited balance"*). SDK consumers
//!    can map these codes against the canonical table at
//!    [`docs/ERROR_CODES.md`](../../../docs/ERROR_CODES.md).
//!
//! 2. **Collateral-specific tier** (codes `100+`):
//!    New variants exclusive to the collateral contract. These occupy the
//!    `100+` namespace deliberately — the credit contract uses `1..49` and
//!    the gap ensures no visual collision if a future PR appends to either
//!    catalog.
//!
//! # Why a separate crate?
//!
//! - **ABI isolation**: when this catalog is wired into a deployed
//!   collateral contract, its discriminants form their own ABI namespace;
//!   SDK consumers decode the discriminant against the
//!   *contract they invoked*, not against the global table.
//! - **Review hygiene**: changes to this catalog cannot accidentally
//!   destabilise `contracts/credit/tests/error_discriminants.rs` — the
//!   canonical credit test is untouched.
//! - **Forward compatibility**: when the collateral contract logic lands,
//!   it can adopt this enum verbatim without re-deriving any discriminant.
//!
//! # Security
//!
//! - No `unwrap()` calls are present in the catalog data path (the enum is
//!   pure value-type data).
//! - The `#[contracterror]` derive enforces `#[repr(u32)]`, which is the
//!   Soroban host boundary for contract-emitted errors.
//!
//! [`CollateralError`]: errors::CollateralError

pub mod errors;
pub mod events;
pub mod views;
pub use views::*;

pub use errors::CollateralError;

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env};

#[contracttype]
pub enum DataKey {
    Balance(Address),
}

/// Soroban contract root for the collateral domain.
#[contract]
pub struct Collateral;

#[contractimpl]
impl Collateral {
    /// Deposits collateral for a given user.
    ///
    /// # Arguments
    /// * `env` - The execution environment.
    /// * `user` - The address of the user depositing the collateral.
    /// * `amount` - The amount of collateral to deposit.
    ///
    /// # Returns
    /// * `Result<(), CollateralError>` - Success or an appropriate error code from the catalog.
    pub fn deposit(env: Env, user: Address, amount: i128) -> Result<(), CollateralError> {
        user.require_auth();

        if amount <= 0 {
            return Err(CollateralError::InvalidAmount);
        }

        let key = DataKey::Balance(user.clone());
        let current_balance: i128 = env.storage().persistent().get(&key).unwrap_or(0);

        let new_balance = current_balance
            .checked_add(amount)
            .ok_or(CollateralError::Overflow)?;

        env.storage().persistent().set(&key, &new_balance);

        events::publish_collateral_deposited(&env, &user, amount, new_balance);

        Ok(())
    }

    /// Withdraws previously deposited collateral for a given user.
    ///
    /// # Arguments
    /// * `env` - The execution environment.
    /// * `user` - The address of the user withdrawing the collateral.
    /// * `amount` - The amount of collateral to withdraw.
    ///
    /// # Returns
    /// * `Result<(), CollateralError>` - Success or an appropriate error code from the catalog.
    pub fn withdraw(env: Env, user: Address, amount: i128) -> Result<(), CollateralError> {
        user.require_auth();

        if amount <= 0 {
            return Err(CollateralError::InvalidAmount);
        }

        let key = DataKey::Balance(user.clone());
        let current_balance: i128 = env.storage().persistent().get(&key).unwrap_or(0);

        let new_balance = current_balance
            .checked_sub(amount)
            .ok_or(CollateralError::InsufficientCollateralBalance)?;

        env.storage().persistent().set(&key, &new_balance);

        events::publish_collateral_withdrawn(&env, &user, amount, new_balance);

        Ok(())
    }
}
