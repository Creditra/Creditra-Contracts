// SPDX-License-Identifier: MIT
use crate::storage::admin_key;
use soroban_sdk::{Address, Env};

pub fn init(env: Env, admin: Address) {
    if env.storage().instance().has(&admin_key(&env)) {
        panic!("already initialized");
    }
    env.storage().instance().set(&admin_key(&env), &admin);
}
