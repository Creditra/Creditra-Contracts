// SPDX-License-Identifier: MIT

//! Cross-contract default-liquidation tests that deploy [`SimplePriceOracle`].

mod with_mock;

use creditra_credit::{Credit, CreditClient};
use simple_price_oracle::{SimplePriceOracle, SimplePriceOracleClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

/// Deploy a [`SimplePriceOracle`] and seed `price` via the admin account.
pub(crate) fn deploy_mock_oracle(
    env: &Env,
    admin: &Address,
    price: i128,
) -> SimplePriceOracleClient<'_> {
    let oracle_id = env.register(SimplePriceOracle, ());
    let client = SimplePriceOracleClient::new(env, &oracle_id);
    client.init(admin);
    if price != 0 {
        client.set_price(&price);
    }
    client
}

/// Deploy credit with admin initialized; returns `(client, contract_id, admin)`.
pub(crate) fn setup_credit(env: &Env) -> (CreditClient<'_>, Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    (client, contract_id, admin)
}
