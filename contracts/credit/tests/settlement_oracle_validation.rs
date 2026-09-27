// SPDX-License-Identifier: MIT

//! Integration tests for wiring the weighted-median oracle registry into
//! `settle_default_liquidation` via the `oracle_validation` module.
//!
//! # Coverage
//! - A settlement driven by three oracle reports resolves and succeeds.
//! - Registry mode rejects settlement with `OracleQuorumNotMet` when the
//!   configured quorum is not reached.
//! - The caller-supplied `oracle_price` is ignored while registry mode is on.
//! - Stale reports do not count towards quorum.
//! - The registry median takes precedence over the quorum-of-K price feed.
//! - The registry median takes precedence over the single-oracle circuit
//!   breaker.
//! - Without a registry configuration the single-oracle circuit breaker and
//!   the unconfigured path behave exactly as before.
//! - The weighted median respects per-oracle weights.

use creditra_credit::types::CreditStatus;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Ledger};
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

/// Open a credit line for `utilized` units, draw, then default it.
/// Returns the borrower.
fn open_and_default(
    client: &CreditClient,
    env: &Env,
    contract_id: &Address,
    utilized: i128,
) -> Address {
    let borrower = Address::generate(env);
    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token_addr = token_id.address();
    client.set_liquidity_token(&token_addr);
    token::StellarAssetClient::new(env, &token_addr).mint(contract_id, &1_000_000_i128);
    token::StellarAssetClient::new(env, &token_addr).mint(&borrower, &1_000_000_i128);
    token::Client::new(env, &token_addr).approve(
        &borrower,
        contract_id,
        &1_000_000_i128,
        &1_000_000_u32,
    );
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

/// Register three oracles (weights 10/20/30) and configure a 60-weight quorum
/// with a one-hour freshness window. Returns the three oracle addresses so a
/// test can drive their reports.
fn configure_three_oracle_registry(
    env: &Env,
    client: &CreditClient,
) -> (Address, Address, Address) {
    let o1 = Address::generate(env);
    let o2 = Address::generate(env);
    let o3 = Address::generate(env);
    client.add_oracle(&o1, &10_u32);
    client.add_oracle(&o2, &20_u32);
    client.add_oracle(&o3, &30_u32);
    client.set_quorum_threshold(&60_u32);
    client.set_reporting_window(&3_600_u64);
    (o1, o2, o3)
}

// ── registry mode: happy path ─────────────────────────────────────────────────

#[test]
fn settlement_uses_registry_median_with_three_oracles() {
    let env = Env::default();
    let (client, contract_id, _admin) = setup(&env);
    let (o1, o2, o3) = configure_three_oracle_registry(&env, &client);

    // Sorted: (1000, w10), (1100, w20), (1200, w30). Total weight 60,
    // target = ceil(60 / 2) = 30; cumulative weight reaches 30 at 1100.
    client.report_value(&o1, &1_000_u128);
    client.report_value(&o2, &1_100_u128);
    client.report_value(&o3, &1_200_u128);

    let borrower = open_and_default(&client, &env, &contract_id, 500);
    // No caller-supplied price: the registry median is authoritative.
    client.settle_default_liquidation(&borrower, &500_i128, &sid(&env, "s1"), &10_000_u32, &None);

    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Closed
    );
}

#[test]
fn weighted_median_respects_oracle_weights() {
    let env = Env::default();
    let (client, _contract_id, _admin) = setup(&env);
    let (o1, o2, o3) = configure_three_oracle_registry(&env, &client);

    // Sorted: (100, w30), (500, w10), (900, w20). Target = 30; the heaviest
    // *lowest* report (weight 30) is reached first, so the median is 100.
    client.report_value(&o1, &500_u128);
    client.report_value(&o2, &900_u128);
    client.report_value(&o3, &100_u128);

    assert_eq!(client.get_median_value(), 100);
}

// ── registry mode: caller-supplied price is ignored ───────────────────────────

#[test]
fn settlement_ignores_caller_supplied_price_in_registry_mode() {
    let env = Env::default();
    let (client, contract_id, _admin) = setup(&env);
    let (o1, o2, o3) = configure_three_oracle_registry(&env, &client);

    client.report_value(&o1, &1_000_u128);
    client.report_value(&o2, &1_100_u128);
    client.report_value(&o3, &1_200_u128);

    let borrower = open_and_default(&client, &env, &contract_id, 500);
    // A wildly off caller price must not be consulted (no deviation check, no
    // rejection): the registry median wins.
    client.settle_default_liquidation(
        &borrower,
        &500_i128,
        &sid(&env, "s1"),
        &10_000_u32,
        &Some(999_999_i128),
    );

    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Closed
    );
}

// ── registry mode: quorum failures ────────────────────────────────────────────

#[test]
#[should_panic(expected = "Error(Contract, #51)")]
fn settlement_rejects_when_registry_quorum_not_met() {
    let env = Env::default();
    let (client, contract_id, _admin) = setup(&env);
    let (o1, _o2, _o3) = configure_three_oracle_registry(&env, &client);

    // Only one of the three oracles reports: 10 of the required 60 weight.
    client.report_value(&o1, &1_000_u128);

    let borrower = open_and_default(&client, &env, &contract_id, 500);
    client.settle_default_liquidation(&borrower, &500_i128, &sid(&env, "s1"), &10_000_u32, &None);
}

#[test]
#[should_panic(expected = "Error(Contract, #51)")]
fn settlement_rejects_stale_registry_reports() {
    let env = Env::default();
    let (client, contract_id, _admin) = setup(&env);

    let o1 = Address::generate(&env);
    client.add_oracle(&o1, &10_u32);
    client.set_quorum_threshold(&10_u32);
    client.set_reporting_window(&60_u64); // one-minute freshness window

    env.ledger().with_mut(|l| l.timestamp = 1_000);
    client.report_value(&o1, &1_000_u128);

    // The only report ages out of the freshness window.
    env.ledger().with_mut(|l| l.timestamp = 1_061);

    let borrower = open_and_default(&client, &env, &contract_id, 500);
    client.settle_default_liquidation(&borrower, &500_i128, &sid(&env, "s1"), &10_000_u32, &None);
}

#[test]
#[should_panic(expected = "Error(Contract, #51)")]
fn settlement_rejects_when_registry_has_no_reports() {
    let env = Env::default();
    let (client, contract_id, _admin) = setup(&env);

    // Quorum configured but the oracle list is empty.
    client.set_quorum_threshold(&10_u32);
    client.set_reporting_window(&3_600_u64);

    let borrower = open_and_default(&client, &env, &contract_id, 500);
    client.settle_default_liquidation(&borrower, &500_i128, &sid(&env, "s1"), &10_000_u32, &None);
}

// ── precedence ────────────────────────────────────────────────────────────────

#[test]
#[should_panic(expected = "Error(Contract, #51)")]
fn registry_mode_takes_precedence_over_quorum_price_feed() {
    let env = Env::default();
    let (client, contract_id, _admin) = setup(&env);

    // Quorum-of-K price feed is configured and a valid price is stored …
    client.set_oracle_quorum_config(&2_u32, &500_u32, &3_600_u64);
    let prices = soroban_sdk::vec![&env, 1_000_i128, 1_020_i128];
    client.submit_oracle_prices(&prices);

    // … but the registry is now active and cannot reach quorum, so settlement
    // must fail closed instead of falling back to the stored quorum price.
    client.set_quorum_threshold(&10_u32);
    client.set_reporting_window(&3_600_u64);

    let borrower = open_and_default(&client, &env, &contract_id, 500);
    client.settle_default_liquidation(&borrower, &500_i128, &sid(&env, "s1"), &10_000_u32, &None);
}

#[test]
#[should_panic(expected = "Error(Contract, #51)")]
fn registry_mode_takes_precedence_over_single_oracle_config() {
    let env = Env::default();
    let (client, contract_id, _admin) = setup(&env);

    // Single-oracle circuit breaker is configured and a valid price is supplied …
    client.set_oracle_config(&500_u32, &3_600_u64);

    // … but the registry is active and cannot reach quorum: reject rather than
    // validate the caller-supplied price.
    client.set_quorum_threshold(&10_u32);
    client.set_reporting_window(&3_600_u64);

    let borrower = open_and_default(&client, &env, &contract_id, 500);
    client.settle_default_liquidation(
        &borrower,
        &500_i128,
        &sid(&env, "s1"),
        &10_000_u32,
        &Some(1_000_i128),
    );
}

// ── legacy behaviour without registry configuration ───────────────────────────

#[test]
fn single_oracle_mode_unchanged_without_registry() {
    let env = Env::default();
    let (client, contract_id, _admin) = setup(&env);

    client.set_oracle_config(&500_u32, &3_600_u64);

    let borrower = open_and_default(&client, &env, &contract_id, 500);
    client.settle_default_liquidation(
        &borrower,
        &500_i128,
        &sid(&env, "s1"),
        &10_000_u32,
        &Some(1_000_i128),
    );

    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Closed
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #36)")]
fn single_oracle_mode_still_rejects_missing_price_without_registry() {
    let env = Env::default();
    let (client, contract_id, _admin) = setup(&env);

    client.set_oracle_config(&500_u32, &3_600_u64);

    let borrower = open_and_default(&client, &env, &contract_id, 500);
    // `OraclePriceInvalid` (#36) is unchanged when only `OracleConfig` is set.
    client.settle_default_liquidation(&borrower, &500_i128, &sid(&env, "s1"), &10_000_u32, &None);
}

#[test]
fn no_oracle_config_settlement_unchanged() {
    let env = Env::default();
    let (client, contract_id, _admin) = setup(&env);

    let borrower = open_and_default(&client, &env, &contract_id, 500);
    client.settle_default_liquidation(&borrower, &500_i128, &sid(&env, "s1"), &10_000_u32, &None);

    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Closed
    );
}

#[test]
fn no_oracle_config_accepts_any_price_unchanged() {
    let env = Env::default();
    let (client, contract_id, _admin) = setup(&env);

    let borrower = open_and_default(&client, &env, &contract_id, 500);
    // With no configuration, the caller price is ignored entirely (legacy).
    client.settle_default_liquidation(
        &borrower,
        &500_i128,
        &sid(&env, "s1"),
        &10_000_u32,
        &Some(42_i128),
    );

    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Closed
    );
}
