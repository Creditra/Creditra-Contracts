// SPDX-License-Identifier: MIT
#![no_std]

//! Creditra freeze contract: account and global emergency freeze management.
//!
//! # Safety & Authentication Audit
//! All state-changing entrypoints require explicit signature/authentication verification
//! by calling `require_auth()` on the acting address (`admin` or `freezer`).

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, Vec,
};

/// Storage keys for freeze contract storage.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    /// Instance key for contract admin address.
    Admin,
    /// Persistent key for frozen address status.
    Frozen(Address),
    /// Persistent key for freezer role permission.
    Freezer(Address),
    /// Instance key for global protocol emergency freeze status.
    GlobalFreeze,
}

/// Errors returned by the freeze contract.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(u32)]
pub enum ContractError {
    /// Contract has already been initialized.
    AlreadyInitialized = 1,
    /// Contract has not been initialized yet.
    NotInitialized = 2,
    /// Caller is unauthorized to perform the requested operation.
    Unauthorized = 3,
    /// The target address is already frozen.
    AlreadyFrozen = 4,
    /// The target address is not currently frozen.
    NotFrozen = 5,
    /// Invalid address provided.
    InvalidAddress = 6,
    /// New admin is the same as current admin.
    SameAdmin = 7,
}

/// Event emitted when an address is frozen.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FreezeEvent {
    /// Address that executed the freeze.
    pub acting_address: Address,
    /// Target address that was frozen.
    pub target: Address,
    /// Ledger timestamp when frozen.
    pub timestamp: u64,
}

/// Event emitted when an address is unfrozen.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnfreezeEvent {
    /// Address that executed the unfreeze.
    pub acting_address: Address,
    /// Target address that was unfrozen.
    pub target: Address,
    /// Ledger timestamp when unfrozen.
    pub timestamp: u64,
}

/// Event emitted when admin authority is updated.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminUpdatedEvent {
    /// Previous admin address.
    pub old_admin: Address,
    /// New admin address.
    pub new_admin: Address,
    /// Ledger timestamp when updated.
    pub timestamp: u64,
}

/// Event emitted when freezer role status is updated.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FreezerUpdatedEvent {
    /// Admin address that granted/revoked permission.
    pub admin: Address,
    /// Freezer address being updated.
    pub freezer: Address,
    /// Whether freezer permission is enabled.
    pub enabled: bool,
    /// Ledger timestamp when updated.
    pub timestamp: u64,
}

/// Event emitted when global freeze status is toggled.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GlobalFreezeEvent {
    /// Admin address that toggled global freeze.
    pub admin: Address,
    /// New global freeze status.
    pub frozen: bool,
    /// Ledger timestamp when updated.
    pub timestamp: u64,
}

/// Soroban smart contract managing protocol freeze state.
#[contract]
pub struct FreezeContract;

#[contractimpl]
impl FreezeContract {
    /// Initialize the contract with an admin address.
    ///
    /// # Parameters
    /// - `env`: Soroban environment handle.
    /// - `admin`: Address holding primary administrative authority.
    ///
    /// # Errors
    /// - [`ContractError::AlreadyInitialized`] if called more than once.
    ///
    /// # Security
    /// State-changing entrypoint. Verifies auth on `admin`.
    pub fn init(env: Env, admin: Address) {
        admin.require_auth();

        if env.storage().instance().has(&DataKey::Admin) {
            env.panic_with_error(ContractError::AlreadyInitialized);
        }

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::GlobalFreeze, &false);
    }

    /// Freeze a target address.
    ///
    /// # Parameters
    /// - `env`: Soroban environment handle.
    /// - `admin`: Acting admin or freezer address.
    /// - `target`: Target address to freeze.
    ///
    /// # Errors
    /// - [`ContractError::NotInitialized`] if contract is uninitialized.
    /// - [`ContractError::Unauthorized`] if `admin` is neither contract admin nor authorized freezer.
    /// - [`ContractError::AlreadyFrozen`] if target is already frozen.
    ///
    /// # Security
    /// State-changing entrypoint. Explicitly requires authentication from `admin`.
    pub fn freeze(env: Env, admin: Address, target: Address) {
        admin.require_auth();
        Self::verify_admin_or_freezer(&env, &admin);

        let key = DataKey::Frozen(target.clone());
        let is_already_frozen = env
            .storage()
            .persistent()
            .get::<_, bool>(&key)
            .unwrap_or(false);

        if is_already_frozen {
            env.panic_with_error(ContractError::AlreadyFrozen);
        }

        env.storage().persistent().set(&key, &true);

        env.events().publish(
            (symbol_short!("freeze"), symbol_short!("account")),
            FreezeEvent {
                acting_address: admin,
                target,
                timestamp: env.ledger().timestamp(),
            },
        );
    }

    /// Unfreeze a target address.
    ///
    /// # Parameters
    /// - `env`: Soroban environment handle.
    /// - `admin`: Acting admin or freezer address.
    /// - `target`: Target address to unfreeze.
    ///
    /// # Errors
    /// - [`ContractError::NotInitialized`] if contract is uninitialized.
    /// - [`ContractError::Unauthorized`] if `admin` is neither contract admin nor authorized freezer.
    /// - [`ContractError::NotFrozen`] if target is not currently frozen.
    ///
    /// # Security
    /// State-changing entrypoint. Explicitly requires authentication from `admin`.
    pub fn unfreeze(env: Env, admin: Address, target: Address) {
        admin.require_auth();
        Self::verify_admin_or_freezer(&env, &admin);

        let key = DataKey::Frozen(target.clone());
        let is_currently_frozen = env
            .storage()
            .persistent()
            .get::<_, bool>(&key)
            .unwrap_or(false);

        if !is_currently_frozen {
            env.panic_with_error(ContractError::NotFrozen);
        }

        env.storage().persistent().set(&key, &false);

        env.events().publish(
            (symbol_short!("unfreeze"), symbol_short!("account")),
            UnfreezeEvent {
                acting_address: admin,
                target,
                timestamp: env.ledger().timestamp(),
            },
        );
    }

    /// Transfer administrative authority to a new address.
    ///
    /// # Parameters
    /// - `env`: Soroban environment handle.
    /// - `admin`: Current admin address.
    /// - `new_admin`: Proposed new admin address.
    ///
    /// # Errors
    /// - [`ContractError::NotInitialized`] if contract is uninitialized.
    /// - [`ContractError::Unauthorized`] if `admin` does not match stored admin.
    /// - [`ContractError::SameAdmin`] if `new_admin` is equal to current `admin`.
    ///
    /// # Security
    /// State-changing entrypoint. Explicitly requires authentication from `admin`.
    pub fn set_admin(env: Env, admin: Address, new_admin: Address) {
        admin.require_auth();
        let current_admin = Self::require_admin(&env);

        if admin != current_admin {
            env.panic_with_error(ContractError::Unauthorized);
        }

        if admin == new_admin {
            env.panic_with_error(ContractError::SameAdmin);
        }

        env.storage().instance().set(&DataKey::Admin, &new_admin);

        env.events().publish(
            (symbol_short!("admin"), symbol_short!("update")),
            AdminUpdatedEvent {
                old_admin: admin,
                new_admin,
                timestamp: env.ledger().timestamp(),
            },
        );
    }

    /// Grant or revoke freezer permissions for an address.
    ///
    /// # Parameters
    /// - `env`: Soroban environment handle.
    /// - `admin`: Acting contract admin address.
    /// - `freezer`: Address to grant/revoke freezer permission.
    /// - `enabled`: `true` to enable freezer role, `false` to disable.
    ///
    /// # Errors
    /// - [`ContractError::NotInitialized`] if contract is uninitialized.
    /// - [`ContractError::Unauthorized`] if `admin` is not the contract admin.
    ///
    /// # Security
    /// State-changing entrypoint. Explicitly requires authentication from `admin`.
    pub fn set_freezer(env: Env, admin: Address, freezer: Address, enabled: bool) {
        admin.require_auth();
        let current_admin = Self::require_admin(&env);

        if admin != current_admin {
            env.panic_with_error(ContractError::Unauthorized);
        }

        let key = DataKey::Freezer(freezer.clone());
        env.storage().persistent().set(&key, &enabled);

        env.events().publish(
            (symbol_short!("freezer"), symbol_short!("update")),
            FreezerUpdatedEvent {
                admin,
                freezer,
                enabled,
                timestamp: env.ledger().timestamp(),
            },
        );
    }

    /// Freeze a target address using designated freezer role.
    ///
    /// # Parameters
    /// - `env`: Soroban environment handle.
    /// - `freezer`: Acting freezer address.
    /// - `target`: Target address to freeze.
    ///
    /// # Errors
    /// - [`ContractError::NotInitialized`] if contract is uninitialized.
    /// - [`ContractError::Unauthorized`] if `freezer` is not authorized.
    /// - [`ContractError::AlreadyFrozen`] if target is already frozen.
    ///
    /// # Security
    /// State-changing entrypoint. Explicitly requires authentication from `freezer`.
    pub fn freeze_with_freezer(env: Env, freezer: Address, target: Address) {
        freezer.require_auth();
        Self::verify_freezer(&env, &freezer);

        let key = DataKey::Frozen(target.clone());
        let is_already_frozen = env
            .storage()
            .persistent()
            .get::<_, bool>(&key)
            .unwrap_or(false);

        if is_already_frozen {
            env.panic_with_error(ContractError::AlreadyFrozen);
        }

        env.storage().persistent().set(&key, &true);

        env.events().publish(
            (symbol_short!("freeze"), symbol_short!("account")),
            FreezeEvent {
                acting_address: freezer,
                target,
                timestamp: env.ledger().timestamp(),
            },
        );
    }

    /// Unfreeze a target address using designated freezer role.
    ///
    /// # Parameters
    /// - `env`: Soroban environment handle.
    /// - `freezer`: Acting freezer address.
    /// - `target`: Target address to unfreeze.
    ///
    /// # Errors
    /// - [`ContractError::NotInitialized`] if contract is uninitialized.
    /// - [`ContractError::Unauthorized`] if `freezer` is not authorized.
    /// - [`ContractError::NotFrozen`] if target is not currently frozen.
    ///
    /// # Security
    /// State-changing entrypoint. Explicitly requires authentication from `freezer`.
    pub fn unfreeze_with_freezer(env: Env, freezer: Address, target: Address) {
        freezer.require_auth();
        Self::verify_freezer(&env, &freezer);

        let key = DataKey::Frozen(target.clone());
        let is_currently_frozen = env
            .storage()
            .persistent()
            .get::<_, bool>(&key)
            .unwrap_or(false);

        if !is_currently_frozen {
            env.panic_with_error(ContractError::NotFrozen);
        }

        env.storage().persistent().set(&key, &false);

        env.events().publish(
            (symbol_short!("unfreeze"), symbol_short!("account")),
            UnfreezeEvent {
                acting_address: freezer,
                target,
                timestamp: env.ledger().timestamp(),
            },
        );
    }

    /// Freeze multiple target addresses in a batch.
    ///
    /// # Parameters
    /// - `env`: Soroban environment handle.
    /// - `admin`: Acting admin or freezer address.
    /// - `targets`: Vector of addresses to freeze.
    ///
    /// # Errors
    /// - [`ContractError::NotInitialized`] if contract is uninitialized.
    /// - [`ContractError::Unauthorized`] if `admin` is neither contract admin nor authorized freezer.
    ///
    /// # Security
    /// State-changing entrypoint. Explicitly requires authentication from `admin`.
    pub fn batch_freeze(env: Env, admin: Address, targets: Vec<Address>) {
        admin.require_auth();
        Self::verify_admin_or_freezer(&env, &admin);

        let timestamp = env.ledger().timestamp();
        for target in targets.iter() {
            let key = DataKey::Frozen(target.clone());
            let is_already_frozen = env
                .storage()
                .persistent()
                .get::<_, bool>(&key)
                .unwrap_or(false);

            if !is_already_frozen {
                env.storage().persistent().set(&key, &true);
                env.events().publish(
                    (symbol_short!("freeze"), symbol_short!("account")),
                    FreezeEvent {
                        acting_address: admin.clone(),
                        target,
                        timestamp,
                    },
                );
            }
        }
    }

    /// Unfreeze multiple target addresses in a batch.
    ///
    /// # Parameters
    /// - `env`: Soroban environment handle.
    /// - `admin`: Acting admin or freezer address.
    /// - `targets`: Vector of addresses to unfreeze.
    ///
    /// # Errors
    /// - [`ContractError::NotInitialized`] if contract is uninitialized.
    /// - [`ContractError::Unauthorized`] if `admin` is neither contract admin nor authorized freezer.
    ///
    /// # Security
    /// State-changing entrypoint. Explicitly requires authentication from `admin`.
    pub fn batch_unfreeze(env: Env, admin: Address, targets: Vec<Address>) {
        admin.require_auth();
        Self::verify_admin_or_freezer(&env, &admin);

        let timestamp = env.ledger().timestamp();
        for target in targets.iter() {
            let key = DataKey::Frozen(target.clone());
            let is_currently_frozen = env
                .storage()
                .persistent()
                .get::<_, bool>(&key)
                .unwrap_or(false);

            if is_currently_frozen {
                env.storage().persistent().set(&key, &false);
                env.events().publish(
                    (symbol_short!("unfreeze"), symbol_short!("account")),
                    UnfreezeEvent {
                        acting_address: admin.clone(),
                        target,
                        timestamp,
                    },
                );
            }
        }
    }

    /// Toggle global protocol emergency freeze status.
    ///
    /// # Parameters
    /// - `env`: Soroban environment handle.
    /// - `admin`: Acting admin address.
    /// - `frozen`: `true` to freeze globally, `false` to unfreeze globally.
    ///
    /// # Errors
    /// - [`ContractError::NotInitialized`] if contract is uninitialized.
    /// - [`ContractError::Unauthorized`] if `admin` is not the contract admin.
    ///
    /// # Security
    /// State-changing entrypoint. Explicitly requires authentication from `admin`.
    pub fn toggle_global_freeze(env: Env, admin: Address, frozen: bool) {
        admin.require_auth();
        let current_admin = Self::require_admin(&env);

        if admin != current_admin {
            env.panic_with_error(ContractError::Unauthorized);
        }

        env.storage()
            .instance()
            .set(&DataKey::GlobalFreeze, &frozen);

        env.events().publish(
            (symbol_short!("freeze"), symbol_short!("global")),
            GlobalFreezeEvent {
                admin,
                frozen,
                timestamp: env.ledger().timestamp(),
            },
        );
    }

    /// Check if a target address or the entire protocol is frozen.
    ///
    /// # Parameters
    /// - `env`: Soroban environment handle.
    /// - `target`: Address to query.
    ///
    /// # Returns
    /// `true` if global freeze is active or `target` address is frozen; `false` otherwise.
    ///
    /// # View Function
    /// Non-state-changing query.
    pub fn is_frozen(env: Env, target: Address) -> bool {
        let is_global_frozen = env
            .storage()
            .instance()
            .get::<_, bool>(&DataKey::GlobalFreeze)
            .unwrap_or(false);

        if is_global_frozen {
            return true;
        }

        env.storage()
            .persistent()
            .get::<_, bool>(&DataKey::Frozen(target))
            .unwrap_or(false)
    }

    /// Check if protocol global freeze is active.
    ///
    /// # Parameters
    /// - `env`: Soroban environment handle.
    ///
    /// # Returns
    /// `true` if global emergency freeze is active; `false` otherwise.
    ///
    /// # View Function
    /// Non-state-changing query.
    pub fn is_globally_frozen(env: Env) -> bool {
        env.storage()
            .instance()
            .get::<_, bool>(&DataKey::GlobalFreeze)
            .unwrap_or(false)
    }

    /// Retrieve current contract admin address.
    ///
    /// # Parameters
    /// - `env`: Soroban environment handle.
    ///
    /// # Returns
    /// `Some(Address)` if set; `None` if uninitialized.
    ///
    /// # View Function
    /// Non-state-changing query.
    pub fn get_admin(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::Admin)
    }

    /// Check if an address has authorized freezer role.
    ///
    /// # Parameters
    /// - `env`: Soroban environment handle.
    /// - `freezer`: Address to check.
    ///
    /// # Returns
    /// `true` if freezer role is enabled; `false` otherwise.
    ///
    /// # View Function
    /// Non-state-changing query.
    pub fn is_freezer(env: Env, freezer: Address) -> bool {
        env.storage()
            .persistent()
            .get::<_, bool>(&DataKey::Freezer(freezer))
            .unwrap_or(false)
    }

    // Helper functions

    fn require_admin(env: &Env) -> Address {
        env.storage()
            .instance()
            .get::<_, Address>(&DataKey::Admin)
            .unwrap_or_else(|| env.panic_with_error(ContractError::NotInitialized))
    }

    fn verify_admin_or_freezer(env: &Env, acting: &Address) {
        let admin = Self::require_admin(env);
        if acting == &admin {
            return;
        }

        let is_freezer = env
            .storage()
            .persistent()
            .get::<_, bool>(&DataKey::Freezer(acting.clone()))
            .unwrap_or(false);

        if !is_freezer {
            env.panic_with_error(ContractError::Unauthorized);
        }
    }

    fn verify_freezer(env: &Env, acting: &Address) {

        let is_freezer = env
            .storage()
            .persistent()
            .get::<_, bool>(&DataKey::Freezer(acting.clone()))
            .unwrap_or(false);

        if !is_freezer {
            env.panic_with_error(ContractError::Unauthorized);
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::testutils::Events as _;
    use soroban_sdk::{vec, IntoVal};

    fn setup_test_env() -> (Env, FreezeContractClient<'static>, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let contract_id = env.register(FreezeContract, ());
        let client = FreezeContractClient::new(&env, &contract_id);
        client.init(&admin);
        (env, client, admin)
    }

    #[test]
    fn test_init_success() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let contract_id = env.register(FreezeContract, ());
        let client = FreezeContractClient::new(&env, &contract_id);
        client.init(&admin);

        assert_eq!(client.get_admin(), Some(admin));
        assert_eq!(client.is_globally_frozen(), false);
    }

    #[test]
    #[should_panic(expected = "HostError")]
    fn test_init_already_initialized_reverts() {
        let (_env, client, admin) = setup_test_env();
        client.init(&admin);
    }

    #[test]
    fn test_freeze_and_unfreeze_success() {
        let (_env, client, admin) = setup_test_env();
        let target = Address::generate(&_env);

        assert_eq!(client.is_frozen(&target), false);

        client.freeze(&admin, &target);
        assert_eq!(client.is_frozen(&target), true);

        client.unfreeze(&admin, &target);
        assert_eq!(client.is_frozen(&target), false);
    }

    #[test]
    #[should_panic(expected = "HostError")]
    fn test_freeze_already_frozen_reverts() {
        let (_env, client, admin) = setup_test_env();
        let target = Address::generate(&_env);

        client.freeze(&admin, &target);
        client.freeze(&admin, &target);
    }

    #[test]
    #[should_panic(expected = "HostError")]
    fn test_unfreeze_not_frozen_reverts() {
        let (_env, client, admin) = setup_test_env();
        let target = Address::generate(&_env);

        client.unfreeze(&admin, &target);
    }

    #[test]
    fn test_set_admin_success() {
        let (_env, client, admin) = setup_test_env();
        let new_admin = Address::generate(&_env);

        client.set_admin(&admin, &new_admin);
        assert_eq!(client.get_admin(), Some(new_admin.clone()));

        // Check new admin can execute operations
        let target = Address::generate(&_env);
        client.freeze(&new_admin, &target);
        assert_eq!(client.is_frozen(&target), true);
    }

    #[test]
    #[should_panic(expected = "HostError")]
    fn test_set_admin_same_admin_reverts() {
        let (_env, client, admin) = setup_test_env();
        client.set_admin(&admin, &admin);
    }

    #[test]
    #[should_panic(expected = "HostError")]
    fn test_set_admin_unauthorized_reverts() {
        let (_env, client, _admin) = setup_test_env();
        let unauthorized = Address::generate(&_env);
        let new_admin = Address::generate(&_env);
        client.set_admin(&unauthorized, &new_admin);
    }

    #[test]
    fn test_freezer_role_management() {
        let (_env, client, admin) = setup_test_env();
        let freezer = Address::generate(&_env);
        let target = Address::generate(&_env);

        assert_eq!(client.is_freezer(&freezer), false);

        client.set_freezer(&admin, &freezer, &true);
        assert_eq!(client.is_freezer(&freezer), true);

        // Freezer freezes target
        client.freeze_with_freezer(&freezer, &target);
        assert_eq!(client.is_frozen(&target), true);

        // Freezer unfreezes target
        client.unfreeze_with_freezer(&freezer, &target);
        assert_eq!(client.is_frozen(&target), false);

        // Revoke freezer role
        client.set_freezer(&admin, &freezer, &false);
        assert_eq!(client.is_freezer(&freezer), false);
    }

    #[test]
    #[should_panic(expected = "HostError")]
    fn test_freeze_with_unauthorized_freezer_reverts() {
        let (_env, client, _admin) = setup_test_env();
        let unauthorized = Address::generate(&_env);
        let target = Address::generate(&_env);

        client.freeze_with_freezer(&unauthorized, &target);
    }

    #[test]
    #[should_panic(expected = "HostError")]
    fn test_unfreeze_with_unauthorized_freezer_reverts() {
        let (_env, client, admin) = setup_test_env();
        let freezer = Address::generate(&_env);
        let unauthorized = Address::generate(&_env);
        let target = Address::generate(&_env);

        client.set_freezer(&admin, &freezer, &true);
        client.freeze_with_freezer(&freezer, &target);

        client.unfreeze_with_freezer(&unauthorized, &target);
    }

    #[test]
    fn test_batch_freeze_and_unfreeze() {
        let (env, client, admin) = setup_test_env();
        let t1 = Address::generate(&env);
        let t2 = Address::generate(&env);
        let t3 = Address::generate(&env);

        let targets = vec![&env, t1.clone(), t2.clone(), t3.clone()];

        client.batch_freeze(&admin, &targets);
        assert_eq!(client.is_frozen(&t1), true);
        assert_eq!(client.is_frozen(&t2), true);
        assert_eq!(client.is_frozen(&t3), true);

        client.batch_unfreeze(&admin, &targets);
        assert_eq!(client.is_frozen(&t1), false);
        assert_eq!(client.is_frozen(&t2), false);
        assert_eq!(client.is_frozen(&t3), false);
    }

    #[test]
    fn test_toggle_global_freeze() {
        let (_env, client, admin) = setup_test_env();
        let user = Address::generate(&_env);

        assert_eq!(client.is_globally_frozen(), false);
        assert_eq!(client.is_frozen(&user), false);

        client.toggle_global_freeze(&admin, &true);
        assert_eq!(client.is_globally_frozen(), true);
        assert_eq!(client.is_frozen(&user), true);

        client.toggle_global_freeze(&admin, &false);
        assert_eq!(client.is_globally_frozen(), false);
        assert_eq!(client.is_frozen(&user), false);
    }

    #[test]
    #[should_panic(expected = "HostError")]
    fn test_freeze_unauthorized_user_reverts() {
        let (_env, client, _admin) = setup_test_env();
        let unauthorized = Address::generate(&_env);
        let target = Address::generate(&_env);

        client.freeze(&unauthorized, &target);
    }

    #[test]
    #[should_panic(expected = "HostError")]
    fn test_unfreeze_unauthorized_user_reverts() {
        let (_env, client, admin) = setup_test_env();
        let unauthorized = Address::generate(&_env);
        let target = Address::generate(&_env);

        client.freeze(&admin, &target);
        client.unfreeze(&unauthorized, &target);
    }

    #[test]
    #[should_panic(expected = "HostError")]
    fn test_toggle_global_freeze_unauthorized_reverts() {
        let (_env, client, _admin) = setup_test_env();
        let unauthorized = Address::generate(&_env);

        client.toggle_global_freeze(&unauthorized, &true);
    }

    #[test]
    #[should_panic(expected = "HostError")]
    fn test_uninitialized_contract_reverts() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(FreezeContract, ());
        let client = FreezeContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let target = Address::generate(&env);

        client.freeze(&admin, &target);
    }

    #[test]
    fn test_auth_verifications() {
        let env = Env::default();
        // Do not call env.mock_all_auths(); auth verification should check require_auth
        let admin = Address::generate(&env);
        let contract_id = env.register(FreezeContract, ());
        let client = FreezeContractClient::new(&env, &contract_id);
        
        // env.mock_all_auths() was not called, so calls will fail auth check
        let res = std::panic::catch_unwind(|| {
            client.init(&admin);
        });
        assert!(res.is_err());
    }
}
#![cfg_attr(not(test), no_std)]

//! Creditra freeze contract (v7).
//!
//! Freeze controls live in [`creditra_credit::freeze`] and the matching
//! entrypoints on [`creditra_credit::Credit`]. This package anchors focused
//! per-entrypoint authorization boundary tests and exposes a read-only
//! [`views::freeze_capabilities`] bitmap so clients can detect supported
//! freeze features at runtime.
//!
//! # Public surface
//!
//! | Module  | What                                                          |
//! |---------|---------------------------------------------------------------|
//! | `errors`| Stable ABI-pinned [`FreezeError`] catalog (mirror + specific) |
//! | `views` | Read-only [`freeze_capabilities`] bitmap (v7)                  |

pub use creditra_credit::*;

pub mod errors;
pub use errors::*;

pub mod views;
pub use views::*;
