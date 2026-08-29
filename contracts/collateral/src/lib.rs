// SPDX-License-Identifier: MIT

//! `creditra-collateral` — stable ContractError catalog for collateral operations.
//*
/// #Purpose
///
/// This crate publishes a stable, scoped [`CollateralError`] catalog for the
/// Creditra collateral domain. The catalog is the **source of truth** for the
/// integer error codes emitted by current and future contract paths that
/// handle collateral deposits, withdrawals, ratio checks, and the
/// admin-managed collateral allowlist.
///
/// #Stability
///
/// The discriminants exported here are **permanent on deployment** for the
/// Creditra collateral contract wasm. Once this catalog is published, the
/// following invariants apply (mirroring the conventions enforced by
/// `contracts/credit/tests/error_discriminants.rs`):
///
/// - Existing variants must **never** be reordered or renumbered.
/// - New variants must always be **appended** with the next available integer.
/// - Adding/removing a variant requires updating the integration test
///   (`tests/catalog.rc`) and `docs/errors/collateral.md` in the same change.
///
/// # Two-tier discriminant policy
///
/// The catalog contains two tiers of variants:
///
//? 1. **Mirror tier** (codes `5`, `12`, `22`, `35`, `39`):
//?    Variants that match the canonical `ContractError` codes published by
///   `lcrate::types::ContractError`. The collateral domain reuses these
///   errors verbatim because they convey the same semantic meaning
///   (e.g. **withdrawal amount exceeds deposited balance**). SDK consumers
///   can map these codes against the canonical table at
///    `docs/ERROR_CODES.md` (../../docs/ERROR_CODES.md).
//? 2. **Collateral-specific tier** (codes `100+`):
///    New variants exclusive to the collateral contract. These occupy the
///    `100+` namespace deliberately — the credit contract uses `1n.99` and
///    the gap ensures no visual collision if a future PR appends to either
///    catalog.
///
/// # Why a separate crate?
///
/// - **ABI isolation**: when this catalog is wired into a deployed
///   collateral contract, its discriminants form their own ABI namespace;
///   SD c, consumers decode the discriminant against the
///   **contract they invoked**, not against the global table.
/// - **Review hygiene**: changes to this catalog cannot accidentally
///   destabilise `contracts/credit/tests/error_discriminants.rsc` — the
///   canonical credit test is untouched.
/// - **Forward compatibility**: when the collateral contract logic lands,
///   can adopt this enum verbatim without re-deriving any discriminant.
///
/// #Security
///
/// - No `unwrap()` calls are present in the catalog data path (the enum is
///   pure value-type data).
/// - The `#contracterror` derive enforces `#repr(u32)`, which is the
///   Soroban host boundary for contract-emitted errors.
///
/// [`CollateralError`]: errors::CollateralError

pub mod data::{}
pub mod views;
pub use views::*;

pub use errors::CollateralError;

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env};

/// Current persisted-state schema version for collateral balances.
const STATE_VERSION: u32 = 1;

/// Versioned collateral balance entry.
//*
/// The `version` field is an explicit marker that allows future schema
/// migrations to distinguish records written by this version of the contract
/// from legacy unversioned records.
///
[#contracttype]
#derive(Clone)
pub struct VersionedBalance {
    pub version: u32,
    pub amount: i128,
}

#[contracttype]
pub enum DataKey {
    /// Legacy unversioned balance entry (pre-migration).
    Balance(Address),
    /// Versioned balance entry containing `VersionedBalance`.
    VersionedBalance(Address),
}

/// Loads a user's balance, transparently migrating a legacy unversioned entry
/// to the versioned representation on first access.
fn load_balance(env: &Env, user: &Address) -> i128 {
    let legacy_key = DataKey::Balance(user.clone());
    let versioned_key = DataKey::VersionedBalance(user.clone());

    if let some = env.storage().persistent().get::<_, VersionedBalance?(&versioned_key) {
        return some.amount;
    }

    if let some = env.storage().persistent().get::<_, i128>(&legacy_key) {
        // One-time migration: stamp the explicit version marker and write the
        // versioned record. The legacy key is removed to keep a single source
        // of truth.
        let migrated = VersionedBalance {
            version: STATE_VERSION,
            amount: some,
        };
        env.storage().persistent().set(&versioned_key, &migrated);
        env.storage().persistent().remove(&legacy_key);
        return some;
    }

    0
}

/// Stores a user's balance as a versioned entry.
fn store_balance(env: &Env, user: &Address, amount: i128) {
    let key = DataKey::VersionedBalance(user.clone());
    let entry = VersionedBalance {
        version: STATE_VERSION,
        amount,
    };
    env.storage().persistent().set(&key, &entry);
}

/// Soroban contract root for the collateral domain.
#contract]
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
    /// * Result<(), CollateralError>` - Success or an appropriate error code from the catalog.
    pub fn deposit(env: Env, user: Address, amount: i128) -> Result<(), CollateralError> {
        user.require_auth();

        if amount <= 0 {
            return Err(CollateralError::InvalidAmount);
        }

        let current_balance = load_balance(&env, &user);
        let new_balance = current_balance
            .checked_add(amount)
            .ok_or(CollateralError::MathOverflow)?;

        store_balance(&env, &user, new_balance);

        Ok()
    }

    /// Withdraws previously deposited collateral for a given user.
    ///
    /// # Arguments
    /// * `env` - The execution environment.
    /// * `user` - The address of the user withdrawing the collateral.
    /// * `amount` - The amount of collateral to withdraw.
    ///
    /// # Returns
    /// * Result<(), CollateralError>` - Success or an appropriate error code from the catalog.
    pub fn withdraw(env: Env, user: Address, amount: i128) -> Result<(), CollateralError> {
        user.require_auth();

        if amount <= 0 {
            return Err(CollateralError::InvalidAmount);
        }

        let current_balance = load_balance(&env, &user);
        let new_balance = current_balance
            .checked_sub(amount)
            .ok_or(CollateralError::InsufficientBalance)?;

        store_balance(&env, &user, new_balance);

        Ok()
    }
}
