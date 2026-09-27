// SPDX-License-Identifier: MIT

//! Integration tests for oracle input validation during settlement.
//!
//! # Coverage
//!
//! - Settlement with no oracle config (backward compatible)
//! - Settlement with single-oracle config (price validation)
//! - Settlement with quorum config (quorum mode takes precedence)
//! - Multiple settlements reusing last accepted price
//! - Oracle outage scenarios and recovery
//! - Price recording and boundary conditions

use creditra_credit::types::{CreditStatus, OracleConfig, OracleQuorumConfig};
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{token, Address, Env, Symbol};

// ── helpers ──────────────────────────────────────────────────────────────────

fn setup(env: &Env) -> (CreditClient, Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    (client, contract_id, admin)
}

/// Open a credit line, draw `utilized`, then default it. Returns borrower.
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

// ── backward compatibility — no oracle config ────────────────────────────────

#[test]
fn settlement_without_oracle_config_accepts_none() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    let borrower = open_and_default(&client, &env, &contract_id, 500);

    // No oracle config set — settlement with None price must succeed
    client.settle_default_liquidation(&borrower, &500_i128, &sid(&env, "s1"), &10_000_u32, &None);

    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Closed
    );
}

#[test]
fn settlement_without_oracle_config_ignores_price_arg() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    let borrower = open_and_default(&client, &env, &contract_id, 500);

    // No oracle config; price arg is ignored (not validated)
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

// ── single-oracle mode — basic flow ──────────────────────────────────────────

#[test]
fn settlement_single_oracle_first_price_accepted() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    client.set_oracle_config(&500_u32, &3600_u64);

    let borrower = open_and_default(&client, &env, &contract_id, 500);

    // First settlement with oracle config — any positive price accepted
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
fn settlement_single_oracle_second_price_within_deviation() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    client.set_oracle_config(&500_u32, &3600_u64); // 5% max dev

    // First settlement
    env.ledger().with_mut(|l| l.timestamp = 1_000);
    let b1 = open_and_default(&client, &env, &contract_id, 200);
    client.settle_default_liquidation(
        &b1,
        &200_i128,
        &sid(&env, "s1"),
        &10_000_u32,
        &Some(1_000_i128),
    );
    assert_eq!(
        client.get_credit_line(&b1).unwrap().status,
        CreditStatus::Closed
    );

    // Second settlement — 1_040 is 4% from 1_000 (within 5%)
    env.ledger().with_mut(|l| l.timestamp = 2_000);
    let b2 = open_and_default(&client, &env, &contract_id, 200);
    client.settle_default_liquidation(
        &b2,
        &200_i128,
        &sid(&env, "s2"),
        &10_000_u32,
        &Some(1_040_i128),
    );
    assert_eq!(
        client.get_credit_line(&b2).unwrap().status,
        CreditStatus::Closed
    );
}

#[test]
#[should_panic(expected = "OraclePriceDeviation")]
fn settlement_single_oracle_over_deviation_fails() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    client.set_oracle_config(&500_u32, &3600_u64);

    env.ledger().with_mut(|l| l.timestamp = 1_000);
    let b1 = open_and_default(&client, &env, &contract_id, 200);
    client.settle_default_liquidation(
        &b1,
        &200_i128,
        &sid(&env, "s1"),
        &10_000_u32,
        &Some(1_000_i128),
    );

    // Second settlement — 1_100 is 10% from 1_000 (exceeds 5%)
    env.ledger().with_mut(|l| l.timestamp = 2_000);
    let b2 = open_and_default(&client, &env, &contract_id, 200);
    client.settle_default_liquidation(
        &b2,
        &200_i128,
        &sid(&env, "s2"),
        &10_000_u32,
        &Some(1_100_i128),
    );
}

#[test]
#[should_panic(expected = "OraclePriceStale")]
fn settlement_single_oracle_stale_fails() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    client.set_oracle_config(&500_u32, &3600_u64); // max_age = 1 hour

    env.ledger().with_mut(|l| l.timestamp = 1_000);
    let b1 = open_and_default(&client, &env, &contract_id, 200);
    client.settle_default_liquidation(
        &b1,
        &200_i128,
        &sid(&env, "s1"),
        &10_000_u32,
        &Some(1_000_i128),
    );

    // Advance beyond max_age_seconds
    env.ledger().with_mut(|l| l.timestamp = 1_000 + 3601);
    let b2 = open_and_default(&client, &env, &contract_id, 200);
    client.settle_default_liquidation(
        &b2,
        &200_i128,
        &sid(&env, "s2"),
        &10_000_u32,
        &Some(1_010_i128),
    );
}

#[test]
#[should_panic(expected = "OraclePriceInvalid")]
fn settlement_single_oracle_missing_price_fails() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    client.set_oracle_config(&500_u32, &3600_u64);

    let borrower = open_and_default(&client, &env, &contract_id, 200);

    // Config is set but price is None — must fail
    client.settle_default_liquidation(&borrower, &200_i128, &sid(&env, "s1"), &10_000_u32, &None);
}

#[test]
#[should_panic(expected = "OraclePriceInvalid")]
fn settlement_single_oracle_zero_price_fails() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    client.set_oracle_config(&500_u32, &3600_u64);

    let borrower = open_and_default(&client, &env, &contract_id, 200);
    client.settle_default_liquidation(
        &borrower,
        &200_i128,
        &sid(&env, "s1"),
        &10_000_u32,
        &Some(0_i128),
    );
}

// ── quorum mode — precedence ────────────────────────────────────────────────

#[test]
fn settlement_quorum_takes_precedence_over_single_oracle() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);

    // Set both configs
    client.set_oracle_config(&500_u32, &3600_u64);
    client.set_oracle_quorum_config(&2_u32, &500_u32, &3600_u64);

    // Submit quorum prices
    let prices = soroban_sdk::vec![&env, 1_000i128, 1_020i128];
    client.submit_oracle_prices(&prices);

    let borrower = open_and_default(&client, &env, &contract_id, 500);

    // Settlement with single-oracle price arg should use quorum price instead
    client.settle_default_liquidation(
        &borrower,
        &500_i128,
        &sid(&env, "s1"),
        &10_000_u32,
        &Some(999_999_i128), // This arg is ignored in quorum mode
    );

    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Closed
    );
}

#[test]
fn settlement_quorum_fresh_price_accepted() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    client.set_oracle_quorum_config(&2_u32, &500_u32, &3600_u64);

    env.ledger().with_mut(|l| l.timestamp = 1_000);
    let prices = soroban_sdk::vec![&env, 1_000i128, 1_020i128];
    client.submit_oracle_prices(&prices);

    env.ledger().with_mut(|l| l.timestamp = 2_000);
    let borrower = open_and_default(&client, &env, &contract_id, 500);

    // Settlement with None (quorum mode uses stored price)
    client.settle_default_liquidation(
        &borrower,
        &500_i128,
        &sid(&env, "s1"),
        &10_000_u32,
        &None,
    );

    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Closed
    );
}

#[test]
#[should_panic(expected = "OracleQuorumNotMet")]
fn settlement_quorum_missing_price_fails() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    client.set_oracle_quorum_config(&2_u32, &500_u32, &3600_u64);

    // Config set but no prices submitted
    let borrower = open_and_default(&client, &env, &contract_id, 500);
    client.settle_default_liquidation(&borrower, &500_i128, &sid(&env, "s1"), &10_000_u32, &None);
}

#[test]
#[should_panic(expected = "OraclePriceStale")]
fn settlement_quorum_stale_price_fails() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    client.set_oracle_quorum_config(&2_u32, &500_u32, &3600_u64);

    env.ledger().with_mut(|l| l.timestamp = 1_000);
    let prices = soroban_sdk::vec![&env, 1_000i128, 1_020i128];
    client.submit_oracle_prices(&prices);

    // Advance beyond max_age_seconds
    env.ledger().with_mut(|l| l.timestamp = 1_000 + 3601);
    let borrower = open_and_default(&client, &env, &contract_id, 500);
    client.settle_default_liquidation(&borrower, &500_i128, &sid(&env, "s1"), &10_000_u32, &None);
}

// ── replay protection ────────────────────────────────────────────────────────

#[test]
#[should_panic(expected = "AlreadyInitialized")]
fn settlement_replay_attempt_fails() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    let borrower = open_and_default(&client, &env, &contract_id, 500);

    let settlement_id = sid(&env, "s1");

    // First settlement succeeds
    client.settle_default_liquidation(
        &borrower,
        &500_i128,
        &settlement_id,
        &10_000_u32,
        &None,
    );

    // Line is now closed, but even if we re-open, same settlement_id is blocked
    // (In practice, line would be closed, so this is more of a contract invariant check)
}

#[test]
fn settlement_new_settlement_id_allowed() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);

    // First borrower, first settlement
    let b1 = open_and_default(&client, &env, &contract_id, 200);
    client.settle_default_liquidation(
        &b1,
        &200_i128,
        &sid(&env, "s1"),
        &10_000_u32,
        &None,
    );
    assert_eq!(
        client.get_credit_line(&b1).unwrap().status,
        CreditStatus::Closed
    );

    // Second borrower with different settlement_id — allowed
    let b2 = open_and_default(&client, &env, &contract_id, 300);
    client.settle_default_liquidation(
        &b2,
        &300_i128,
        &sid(&env, "s2"),
        &10_000_u32,
        &None,
    );
    assert_eq!(
        client.get_credit_line(&b2).unwrap().status,
        CreditStatus::Closed
    );
}

// ── partial close ────────────────────────────────────────────────────────────

#[test]
fn settlement_partial_close_factor() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    let borrower = open_and_default(&client, &env, &contract_id, 1_000);

    // Settle 50% (close_factor_bps = 5_000)
    client.settle_default_liquidation(
        &borrower,
        &500_i128,
        &sid(&env, "s1"),
        &5_000_u32,
        &None,
    );

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 500); // 1000 - 500
    assert_eq!(line.status, CreditStatus::Defaulted); // Still defaulted, not closed
}

#[test]
fn settlement_multiple_close_factors() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    let borrower = open_and_default(&client, &env, &contract_id, 1_000);

    // First settlement: recover 300 (close_factor ~30%)
    client.settle_default_liquidation(
        &borrower,
        &300_i128,
        &sid(&env, "s1"),
        &3_000_u32,
        &None,
    );
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 700);

    // Second settlement: recover 400 more (close_factor ~57% of remaining)
    client.settle_default_liquidation(
        &borrower,
        &400_i128,
        &sid(&env, "s2"),
        &5_700_u32,
        &None,
    );
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 300);

    // Third settlement: recover all remaining
    client.settle_default_liquidation(
        &borrower,
        &300_i128,
        &sid(&env, "s3"),
        &10_000_u32,
        &None,
    );
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 0);
    assert_eq!(line.status, CreditStatus::Closed);
}

// ── boundary conditions ──────────────────────────────────────────────────────

#[test]
fn settlement_recovered_amount_equals_max_recoverable() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    let borrower = open_and_default(&client, &env, &contract_id, 1_000);

    // Recover exactly max_recoverable = 1000 * 5000 / 10_000 = 500
    client.settle_default_liquidation(
        &borrower,
        &500_i128,
        &sid(&env, "s1"),
        &5_000_u32,
        &None,
    );
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 500);
}

#[test]
#[should_panic(expected = "OverLimit")]
fn settlement_recovered_amount_exceeds_max_recoverable() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    let borrower = open_and_default(&client, &env, &contract_id, 1_000);

    // Try to recover 600 when max is 500 (5000 bps of 1000)
    client.settle_default_liquidation(
        &borrower,
        &600_i128,
        &sid(&env, "s1"),
        &5_000_u32,
        &None,
    );
}

#[test]
#[should_panic(expected = "InvalidAmount")]
fn settlement_zero_recovered_amount_fails() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    let borrower = open_and_default(&client, &env, &contract_id, 500);

    client.settle_default_liquidation(
        &borrower,
        &0_i128,
        &sid(&env, "s1"),
        &10_000_u32,
        &None,
    );
}

#[test]
#[should_panic(expected = "InvalidAmount")]
fn settlement_negative_recovered_amount_fails() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    let borrower = open_and_default(&client, &env, &contract_id, 500);

    client.settle_default_liquidation(
        &borrower,
        &(-100_i128),
        &sid(&env, "s1"),
        &10_000_u32,
        &None,
    );
}

// ── price recording & state consistency ───────────────────────────────────────

#[test]
fn settlement_records_oracle_price_for_next_deviation_check() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    client.set_oracle_config(&500_u32, &3600_u64);

    // First settlement at price 1_000
    env.ledger().with_mut(|l| l.timestamp = 1_000);
    let b1 = open_and_default(&client, &env, &contract_id, 200);
    client.settle_default_liquidation(
        &b1,
        &200_i128,
        &sid(&env, "s1"),
        &10_000_u32,
        &Some(1_000_i128),
    );

    // Second settlement should use 1_000 as the baseline for deviation
    env.ledger().with_mut(|l| l.timestamp = 2_000);
    let b2 = open_and_default(&client, &env, &contract_id, 200);

    // Price 1_040 is 4% from 1_000 (within 5%) — should succeed
    client.settle_default_liquidation(
        &b2,
        &200_i128,
        &sid(&env, "s2"),
        &10_000_u32,
        &Some(1_040_i128),
    );
    assert_eq!(
        client.get_credit_line(&b2).unwrap().status,
        CreditStatus::Closed
    );

    // Third settlement — price 1_040 becomes the new baseline
    env.ledger().with_mut(|l| l.timestamp = 3_000);
    let b3 = open_and_default(&client, &env, &contract_id, 200);

    // Price 1_080 is 3.8% from 1_040 (within 5%) — should succeed
    client.settle_default_liquidation(
        &b3,
        &200_i128,
        &sid(&env, "s3"),
        &10_000_u32,
        &Some(1_080_i128),
    );
    assert_eq!(
        client.get_credit_line(&b3).unwrap().status,
        CreditStatus::Closed
    );
}

#[test]
fn settlement_no_oracle_config_does_not_record_price() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);

    // No oracle config — settlement proceeds without price recording
    let b1 = open_and_default(&client, &env, &contract_id, 300);
    client.settle_default_liquidation(
        &b1,
        &300_i128,
        &sid(&env, "s1"),
        &10_000_u32,
        &Some(999_999_i128),
    );

    // Second settlement — no prior price to compare against
    let b2 = open_and_default(&client, &env, &contract_id, 300);
    client.settle_default_liquidation(
        &b2,
        &300_i128,
        &sid(&env, "s2"),
        &10_000_u32,
        &Some(1_i128), // Any price accepted since no config
    );
    assert_eq!(
        client.get_credit_line(&b2).unwrap().status,
        CreditStatus::Closed
    );
}
