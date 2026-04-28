// SPDX-License-Identifier: MIT
//! Config module: contract initialization.

use crate::storage::admin_key;
use crate::types::ContractError;
use soroban_sdk::{Address, Env};

pub fn init(env: Env, admin: Address) {
    let key = admin_key(&env);
    if env.storage().instance().has(&key) {
        env.panic_with_error(ContractError::AlreadyInitialized);
    }
    env.storage().instance().set(&key, &admin);
}
