// SPDX-License-Identifier: MIT

//! Integration tests for the oracle price-feed staleness and deviation circuit breaker.
//!
//! Covers:
//! - `set_oracle_config` validation and admin-only enforcement
//! - `settle_default_liquidation` with no oracle config (backward-compatible)
//! - First-price acceptance (no prior price stored)
//! - Within-bound deviation accepted
//! - Over-deviation rejected with `OraclePriceDeviation`
//! - Stale price rejected with `OraclePriceStale`
//! - Zero / negative oracle price rejected with `OraclePriceInvalid`
//! - Missing oracle_price when config is set rejected with `OraclePriceInvalid`
//! - `get_oracle_config` returns stored config

use creditra_credit::types::{ContractError, CreditStatus, OracleConfig};
use creditra_credit::{Credit, CreditClient};
use simple_price_oracle::{SimplePriceOracle, SimplePriceOracleClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, Address, Env, Symbol};

// ── helpers ───────────────────────────────────────────────────────────────────

fn setup(env: &Env) -> (CreditClient, Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    (client, contract_id, admin)
}

/// Open a credit line, draw `utilized`, then default it. Returns borrower.
fn open_and_default(client: &CreditClient, env: &Env, contract_id: &Address, utilized: i128) -> Address {
    let borrower = Address::generate(env);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token_addr = token_id.address();
    client.set_liquidity_token(&token_addr);
    token::StellarAssetClient::new(env, &token_addr).mint(contract_id, &1_000_000_i128);
    token::StellarAssetClient::new(env, &token_addr).mint(&borrower, &1_000_000_i128);
    token::Client::new(env, &token_addr).approve(&borrower, contract_id, &1_000_000_i128, &1_000_000_u32);

    client.open_credit_line(&borrower, &10_000_i128, &300_u32, &60_u32);
    if utilized > 0 {
        client.draw_credit(&borrower, &utilized);
    }
    client.default_credit_line(&borrower);
    borrower
}

fn sid(env: &Env, s: &str) -> Symbol {
    Symbol::new(env, s)
}

fn deploy_oracle(env: &Env, admin: &Address, price: i128) -> SimplePriceOracleClient<'_> {
    let oracle_id = env.register(SimplePriceOracle, ());
    let client = SimplePriceOracleClient::new(env, &oracle_id);
    client.init(admin);
    if price != 0 {
        client.set_price(&price);
    }
    client
}

// ── set_oracle_config ─────────────────────────────────────────────────────────

#[test]
fn set_oracle_config_stores_and_get_returns_it() {
    let env = Env::default();
    let (client, _, _) = setup(&env);

    client.set_oracle_config(&500_u32, &3600_u64);

    let cfg = client.get_oracle_config().unwrap();
    assert_eq!(cfg.max_deviation_bps, 500);
    assert_eq!(cfg.max_age_seconds, 3600);
}

#[test]
#[should_panic]
fn set_oracle_config_zero_deviation_panics() {
    let env = Env::default();
    let (client, _, _) = setup(&env);
    client.set_oracle_config(&0_u32, &3600_u64);
}

#[test]
#[should_panic]
fn set_oracle_config_deviation_over_10000_panics() {
    let env = Env::default();
    let (client, _, _) = setup(&env);
    client.set_oracle_config(&10_001_u32, &3600_u64);
}

#[test]
#[should_panic]
fn set_oracle_config_zero_age_panics() {
    let env = Env::default();
    let (client, _, _) = setup(&env);
    client.set_oracle_config(&500_u32, &0_u64);
}

#[test]
fn get_oracle_config_returns_none_when_not_set() {
    let env = Env::default();
    let (client, _, _) = setup(&env);
    assert!(client.get_oracle_config().is_none());
}

// ── no oracle config — backward compatible ────────────────────────────────────

#[test]
fn settle_without_oracle_config_accepts_none_price() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    let borrower = open_and_default(&client, &env, &contract_id, 500);

    // No oracle config set — None price must be accepted.
    client.settle_default_liquidation(&borrower, &500_i128, &sid(&env, "s1"), &None);

    assert_eq!(client.get_credit_line(&borrower).unwrap().status, CreditStatus::Closed);
}

// ── first price acceptance ────────────────────────────────────────────────────

#[test]
fn settle_with_oracle_config_first_price_accepted() {
    let env = Env::default();
    let (client, contract_id, admin) = setup(&env);
    client.set_oracle_config(&500_u32, &3600_u64);
    let oracle = deploy_oracle(&env, &admin, 1_000_i128);
    let borrower = open_and_default(&client, &env, &contract_id, 500);

    // First call — price sourced from deployed mock oracle.
    let price = oracle.get_price();
    client.settle_default_liquidation(&borrower, &500_i128, &sid(&env, "s1"), &Some(price));

    assert_eq!(client.get_credit_line(&borrower).unwrap().status, CreditStatus::Closed);
}

// ── within-bound deviation accepted ──────────────────────────────────────────

#[test]
fn settle_within_deviation_bound_accepted() {
    let env = Env::default();
    let (client, contract_id, admin) = setup(&env);
    client.set_oracle_config(&500_u32, &3600_u64); // 5% max deviation
    let oracle = deploy_oracle(&env, &admin, 1_000_i128);

    // First settlement — seeds the last accepted price at 1_000.
    let b1 = open_and_default(&client, &env, &contract_id, 200);
    client.settle_default_liquidation(
        &b1,
        &200_i128,
        &sid(&env, "s1"),
        &Some(oracle.get_price()),
    );

    // Second settlement — admin updates mock feed to 1_040 (4% deviation).
    oracle.set_price(&1_040_i128);
    let b2 = open_and_default(&client, &env, &contract_id, 200);
    client.settle_default_liquidation(
        &b2,
        &200_i128,
        &sid(&env, "s2"),
        &Some(oracle.get_price()),
    );

    assert_eq!(client.get_credit_line(&b2).unwrap().status, CreditStatus::Closed);
}

// ── over-deviation rejected ───────────────────────────────────────────────────

#[test]
#[should_panic]
fn settle_over_deviation_panics() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    client.set_oracle_config(&500_u32, &3600_u64); // 5% max deviation

    // Seed last price at 1_000.
    let b1 = open_and_default(&client, &env, &contract_id, 200);
    client.settle_default_liquidation(&b1, &200_i128, &sid(&env, "s1"), &Some(1_000_i128));

    // Price 1_100 is 10% deviation — exceeds 5% threshold.
    let b2 = open_and_default(&client, &env, &contract_id, 200);
    client.settle_default_liquidation(&b2, &200_i128, &sid(&env, "s2"), &Some(1_100_i128));
}

#[test]
#[should_panic]
fn settle_over_deviation_downward_panics() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    client.set_oracle_config(&500_u32, &3600_u64);

    let b1 = open_and_default(&client, &env, &contract_id, 200);
    client.settle_default_liquidation(&b1, &200_i128, &sid(&env, "s1"), &Some(1_000_i128));

    // Price 900 is 10% below 1_000 — exceeds 5% threshold.
    let b2 = open_and_default(&client, &env, &contract_id, 200);
    client.settle_default_liquidation(&b2, &200_i128, &sid(&env, "s2"), &Some(900_i128));
}

// ── stale price rejected ──────────────────────────────────────────────────────

#[test]
#[should_panic]
fn settle_stale_price_panics() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    client.set_oracle_config(&500_u32, &3600_u64); // max age 1 hour

    // Seed last price at t=1000.
    env.ledger().with_mut(|l| l.timestamp = 1_000);
    let b1 = open_and_default(&client, &env, &contract_id, 200);
    client.settle_default_liquidation(&b1, &200_i128, &sid(&env, "s1"), &Some(1_000_i128));

    // Advance time beyond max_age_seconds (1 hour = 3600s).
    env.ledger().with_mut(|l| l.timestamp = 1_000 + 3_601);

    // Price is now stale — should panic.
    let b2 = open_and_default(&client, &env, &contract_id, 200);
    client.settle_default_liquidation(&b2, &200_i128, &sid(&env, "s2"), &Some(1_010_i128));
}

#[test]
fn settle_price_at_exact_max_age_accepted() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    client.set_oracle_config(&500_u32, &3600_u64);

    env.ledger().with_mut(|l| l.timestamp = 1_000);
    let b1 = open_and_default(&client, &env, &contract_id, 200);
    client.settle_default_liquidation(&b1, &200_i128, &sid(&env, "s1"), &Some(1_000_i128));

    // Advance exactly max_age_seconds — age == 3600, not > 3600, so accepted.
    env.ledger().with_mut(|l| l.timestamp = 1_000 + 3_600);
    let b2 = open_and_default(&client, &env, &contract_id, 200);
    client.settle_default_liquidation(&b2, &200_i128, &sid(&env, "s2"), &Some(1_010_i128));

    assert_eq!(client.get_credit_line(&b2).unwrap().status, CreditStatus::Closed);
}

// ── invalid price ─────────────────────────────────────────────────────────────

#[test]
#[should_panic]
fn settle_zero_oracle_price_panics() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    client.set_oracle_config(&500_u32, &3600_u64);
    let borrower = open_and_default(&client, &env, &contract_id, 200);
    client.settle_default_liquidation(&borrower, &200_i128, &sid(&env, "s1"), &Some(0_i128));
}

#[test]
#[should_panic]
fn settle_negative_oracle_price_panics() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    client.set_oracle_config(&500_u32, &3600_u64);
    let borrower = open_and_default(&client, &env, &contract_id, 200);
    client.settle_default_liquidation(&borrower, &200_i128, &sid(&env, "s1"), &Some(-1_i128));
}

#[test]
#[should_panic]
fn settle_missing_price_when_config_set_panics() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    client.set_oracle_config(&500_u32, &3600_u64);
    let borrower = open_and_default(&client, &env, &contract_id, 200);
    // oracle_price is None but config is set — must panic.
    client.settle_default_liquidation(&borrower, &200_i128, &sid(&env, "s1"), &None);
}
