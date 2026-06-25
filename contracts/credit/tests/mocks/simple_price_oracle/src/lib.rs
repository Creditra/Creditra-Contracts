// SPDX-License-Identifier: MIT

#![cfg_attr(not(test), no_std)]

//! # simple-price-oracle
//!
//! Minimal Soroban price-feed mock for cross-contract credit integration tests.
//!
//! Off-chain liquidation orchestration is expected to read a price from an oracle
//! contract and pass it into [`creditra_credit::Credit::settle_default_liquidation`].
//! This crate provides a deployable stand-in so tests can exercise that path
//! without a production oracle deployment.
//!
//! ## Interface
//!
//! | Function     | Access | Description                          |
//! |--------------|--------|--------------------------------------|
//! | `init`       | once   | Store the admin address              |
//! | `get_price`  | public | Return the current stored price      |
//! | `set_price`  | admin  | Update the stored price (`i128`)     |
//!
//! ## Usage in integration tests
//!
//! ```ignore
//! let oracle_id = env.register(SimplePriceOracle, ());
//! let oracle = SimplePriceOracleClient::new(&env, &oracle_id);
//! oracle.init(&admin);
//! oracle.set_price(&1_000_i128);
//!
//! let price = oracle.get_price();
//! credit.settle_default_liquidation(&borrower, &amount, &settlement_id, &Some(price));
//! ```

use soroban_sdk::{contract, contractimpl, symbol_short, Address, Env};

const ADMIN_KEY: soroban_sdk::Symbol = symbol_short!("admin");
const PRICE_KEY: soroban_sdk::Symbol = symbol_short!("price");

/// Deployable mock oracle used by credit default-liquidation integration tests.
#[contract]
pub struct SimplePriceOracle;

#[contractimpl]
impl SimplePriceOracle {
    /// One-time setup: records the admin that may call [`set_price`].
    pub fn init(env: Env, admin: Address) {
        if env.storage().instance().has(&ADMIN_KEY) {
            panic!("already initialized");
        }
        env.storage().instance().set(&ADMIN_KEY, &admin);
    }

    /// Returns the last price written by the admin via [`set_price`].
    ///
    /// Returns `0` when no price has been set yet.
    pub fn get_price(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&PRICE_KEY)
            .unwrap_or(0_i128)
    }

    /// Admin-only price update used by tests to simulate oracle feed changes.
    pub fn set_price(env: Env, price: i128) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&ADMIN_KEY)
            .expect("oracle not initialized");
        admin.require_auth();
        env.storage().instance().set(&PRICE_KEY, &price);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    fn deploy(env: &Env) -> (Address, SimplePriceOracleClient<'_>) {
        let admin = Address::generate(env);
        let contract_id = env.register(SimplePriceOracle, ());
        let client = SimplePriceOracleClient::new(env, &contract_id);
        client.init(&admin);
        (admin, client)
    }

    #[test]
    fn get_price_defaults_to_zero_before_set() {
        let env = Env::default();
        env.mock_all_auths();
        let (_admin, client) = deploy(&env);
        assert_eq!(client.get_price(), 0);
    }

    #[test]
    fn set_price_updates_get_price() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, client) = deploy(&env);
        client.set_price(&1_234_i128);
        assert_eq!(client.get_price(), 1_234_i128);
        client.set_price(&9_999_i128);
        assert_eq!(client.get_price(), 9_999_i128);
        let _ = admin;
    }

    #[test]
    #[should_panic(expected = "not initialized")]
    fn set_price_before_init_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(SimplePriceOracle, ());
        let client = SimplePriceOracleClient::new(&env, &contract_id);
        client.set_price(&100_i128);
    }

    #[test]
    #[should_panic(expected = "already initialized")]
    fn init_twice_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let contract_id = env.register(SimplePriceOracle, ());
        let client = SimplePriceOracleClient::new(&env, &contract_id);
        client.init(&admin);
        client.init(&admin);
    }
}
