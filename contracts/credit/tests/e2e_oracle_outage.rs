// SPDX-License-Identifier: MIT

//! End-to-end simulation of oracle price-feed outage and recovery.
//!
//! Simulates a production scenario where the oracle price feed becomes
//! unavailable (stale) or unreliable (excessive deviation), blocking default
//! liquidation settlement, and then recovers via admin configuration update.
//!
//! Scenarios:
//! - Healthy oracle: settlement succeeds normally.
//! - Stale price outage: advanced ledger timestamp triggers `OraclePriceStale`.
//! - Deviation outage: sudden price swing triggers `OraclePriceDeviation`.
//! - Recovery after stale outage: admin extends `max_age_seconds`, settlement
//!   proceeds with the same price.
//! - Recovery after deviation outage: admin widens `max_deviation_bps`,
//!   settlement proceeds with the previously rejected price.
//! - Consecutive settlement attempts blocked during continuous outage.
//! - Active credit lines unaffected by oracle outage.
//! - Downward price deviation also blocked.

use creditra_credit::types::CreditStatus;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{token, Address, Env, Symbol};

const CLOSE_FACTOR_FULL: u32 = 10_000;
const CREDIT_LIMIT: i128 = 10_000;
const ONE_HOUR: u64 = 3600;

// ── helpers ───────────────────────────────────────────────────────────────────

struct EnvDeployment<'a> {
    client: CreditClient<'a>,
    contract_id: Address,
    token_addr: Address,
}

fn setup(env: &Env) -> EnvDeployment<'_> {
    env.mock_all_auths();
    let admin = Address::generate(env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token_addr = token_id.address();
    client.set_liquidity_token(&token_addr);
    // Mint liquidity to the contract.
    token::StellarAssetClient::new(env, &token_addr).mint(&contract_id, &1_000_000_i128);

    EnvDeployment {
        client,
        contract_id,
        token_addr,
    }
}

/// Open a credit line, deposit sufficient collateral, draw `draw_amount`,
/// then default the line. Returns the borrower address.
fn open_draw_default(d: &EnvDeployment<'_>, env: &Env, draw_amount: i128) -> Address {
    let borrower = Address::generate(env);

    // Mint enough tokens for collateral (150% min ratio) plus some buffer.
    let required_collateral = draw_amount * 150 / 100;
    token::StellarAssetClient::new(env, &d.token_addr)
        .mint(&borrower, &(required_collateral + 100_000));

    d.client
        .open_credit_line(&borrower, &CREDIT_LIMIT, &0_u32, &60_u32);

    if draw_amount > 0 {
        d.client.deposit_collateral(&borrower, &required_collateral);
        d.client.draw_credit(&borrower, &draw_amount);
    }

    d.client.default_credit_line(&borrower);
    borrower
}

fn sid(env: &Env, s: &str) -> Symbol {
    Symbol::new(env, s)
}

// ── 1. Healthy oracle — baseline ─────────────────────────────────────────────

#[test]
fn e2e_oracle_healthy_settlement_succeeds() {
    let env = Env::default();
    let d = setup(&env);
    d.client.set_oracle_config(&500_u32, &ONE_HOUR);

    let b1 = open_draw_default(&d, &env, 500);
    d.client.settle_default_liquidation(
        &b1,
        &500_i128,
        &sid(&env, "s1"),
        &CLOSE_FACTOR_FULL,
        &Some(1_000_i128),
    );

    let line = d.client.get_credit_line(&b1).unwrap();
    assert_eq!(line.status, CreditStatus::Closed);
    assert_eq!(line.utilized_amount, 0);

    let cfg = d.client.get_oracle_config().unwrap();
    assert_eq!(cfg.max_deviation_bps, 500);
    assert_eq!(cfg.max_age_seconds, ONE_HOUR);
}

// ── 2. Oracle outage — stale price blocks settlement ─────────────────────────

#[test]
#[should_panic(expected = "HostError: Error(Contract, #37)")]
fn e2e_oracle_outage_stale_blocks_settlement() {
    let env = Env::default();
    let d = setup(&env);
    d.client.set_oracle_config(&500_u32, &ONE_HOUR);

    // Seed last price at t = 1000.
    env.ledger().with_mut(|l| l.timestamp = 1_000);
    let b1 = open_draw_default(&d, &env, 200);
    d.client.settle_default_liquidation(
        &b1,
        &200_i128,
        &sid(&env, "seed"),
        &CLOSE_FACTOR_FULL,
        &Some(1_000_i128),
    );

    // Advance time beyond max_age — price is now stale.
    env.ledger().with_mut(|l| l.timestamp = 1_000 + ONE_HOUR + 1);

    let b2 = open_draw_default(&d, &env, 200);
    d.client.settle_default_liquidation(
        &b2,
        &200_i128,
        &sid(&env, "stale"),
        &CLOSE_FACTOR_FULL,
        &Some(1_010_i128),
    );
}

// ── 3. Oracle outage — excessive deviation blocks settlement ─────────────────

#[test]
#[should_panic(expected = "HostError: Error(Contract, #38)")]
fn e2e_oracle_outage_deviation_blocks_settlement() {
    let env = Env::default();
    let d = setup(&env);
    d.client.set_oracle_config(&500_u32, &ONE_HOUR); // 5% max deviation

    let b1 = open_draw_default(&d, &env, 200);
    d.client.settle_default_liquidation(
        &b1,
        &200_i128,
        &sid(&env, "seed"),
        &CLOSE_FACTOR_FULL,
        &Some(1_000_i128),
    );

    // Price 1_100 is 10% deviation — exceeds 5% threshold.
    let b2 = open_draw_default(&d, &env, 200);
    d.client.settle_default_liquidation(
        &b2,
        &200_i128,
        &sid(&env, "dev"),
        &CLOSE_FACTOR_FULL,
        &Some(1_100_i128),
    );
}

// ── 4. Recovery after stale outage — admin extends max_age ───────────────────

#[test]
fn e2e_oracle_stale_recovery_via_config_update() {
    let env = Env::default();
    let d = setup(&env);
    d.client.set_oracle_config(&500_u32, &ONE_HOUR);

    // Seed price at t = 1000.
    env.ledger().with_mut(|l| l.timestamp = 1_000);
    let b1 = open_draw_default(&d, &env, 200);
    d.client.settle_default_liquidation(
        &b1,
        &200_i128,
        &sid(&env, "seed"),
        &CLOSE_FACTOR_FULL,
        &Some(1_000_i128),
    );

    // Advance beyond max_age.
    env.ledger().with_mut(|l| l.timestamp = 1_000 + ONE_HOUR + 1);

    let b2 = open_draw_default(&d, &env, 300);

    // Admin extends max_age to cover the current time.
    let extended_age = 2 * ONE_HOUR;
    d.client.set_oracle_config(&500_u32, &extended_age);

    // Settlement now succeeds — same price, not stale under new config.
    d.client.settle_default_liquidation(
        &b2,
        &300_i128,
        &sid(&env, "recov"),
        &CLOSE_FACTOR_FULL,
        &Some(1_010_i128),
    );

    let line = d.client.get_credit_line(&b2).unwrap();
    assert_eq!(line.status, CreditStatus::Closed);
    assert_eq!(line.utilized_amount, 0);

    let cfg = d.client.get_oracle_config().unwrap();
    assert_eq!(cfg.max_age_seconds, extended_age);
}

// ── 5. Recovery after deviation outage — admin widens deviation ──────────────

#[test]
fn e2e_oracle_deviation_recovery_via_config_update() {
    let env = Env::default();
    let d = setup(&env);
    d.client.set_oracle_config(&200_u32, &ONE_HOUR); // 2% max deviation

    let b1 = open_draw_default(&d, &env, 200);
    d.client.settle_default_liquidation(
        &b1,
        &200_i128,
        &sid(&env, "seed"),
        &CLOSE_FACTOR_FULL,
        &Some(1_000_i128),
    );

    let b2 = open_draw_default(&d, &env, 400);

    // Admin widens deviation bound from 2% to 10%.
    d.client.set_oracle_config(&1_000_u32, &ONE_HOUR);

    // Price 1_080 is 8% deviation — rejected under 2%, accepted under 10%.
    d.client.settle_default_liquidation(
        &b2,
        &400_i128,
        &sid(&env, "recov"),
        &CLOSE_FACTOR_FULL,
        &Some(1_080_i128),
    );

    let line = d.client.get_credit_line(&b2).unwrap();
    assert_eq!(line.status, CreditStatus::Closed);
    assert_eq!(line.utilized_amount, 0);

    let cfg = d.client.get_oracle_config().unwrap();
    assert_eq!(cfg.max_deviation_bps, 1_000);
}

// ── 6. Multiple settlement attempts blocked during continuous outage ─────────

#[test]
fn e2e_oracle_outage_blocks_consecutive_settlements() {
    let env = Env::default();
    let d = setup(&env);
    d.client.set_oracle_config(&300_u32, &ONE_HOUR); // 3% max deviation

    // Seed price.
    let b1 = open_draw_default(&d, &env, 200);
    d.client.settle_default_liquidation(
        &b1,
        &200_i128,
        &sid(&env, "seed"),
        &CLOSE_FACTOR_FULL,
        &Some(1_000_i128),
    );

    let b2 = open_draw_default(&d, &env, 200);

    // First outage attempt — over-deviation.
    let r1 = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        d.client.settle_default_liquidation(
            &b2,
            &200_i128,
            &sid(&env, "attempt1"),
            &CLOSE_FACTOR_FULL,
            &Some(1_200_i128),
        );
    }));
    assert!(r1.is_err(), "first over-deviation settlement should panic");

    // Credit line still defaulted, state unchanged.
    let line1 = d.client.get_credit_line(&b2).unwrap();
    assert_eq!(line1.status, CreditStatus::Defaulted);
    assert_eq!(line1.utilized_amount, 200);

    // Second outage attempt — also over-deviation.
    let r2 = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        d.client.settle_default_liquidation(
            &b2,
            &200_i128,
            &sid(&env, "attempt2"),
            &CLOSE_FACTOR_FULL,
            &Some(900_i128),
        );
    }));
    assert!(r2.is_err(), "second over-deviation settlement should panic");

    let line2 = d.client.get_credit_line(&b2).unwrap();
    assert_eq!(line2.status, CreditStatus::Defaulted);
    assert_eq!(line2.utilized_amount, 200);

    // Admin widens deviation — recovery.
    d.client.set_oracle_config(&2_000_u32, &ONE_HOUR);
    d.client.settle_default_liquidation(
        &b2,
        &200_i128,
        &sid(&env, "final"),
        &CLOSE_FACTOR_FULL,
        &Some(1_200_i128),
    );

    let line3 = d.client.get_credit_line(&b2).unwrap();
    assert_eq!(line3.status, CreditStatus::Closed);
    assert_eq!(line3.utilized_amount, 0);
}

// ── 7. Oracle outage does not affect active credit lines ─────────────────────

#[test]
fn e2e_oracle_outage_does_not_affect_active_credit_lines() {
    let env = Env::default();
    let d = setup(&env);
    d.client.set_oracle_config(&500_u32, &ONE_HOUR);

    env.ledger().with_mut(|l| l.timestamp = 1_000);

    // Active borrower — not defaulted, not affected by oracle.
    // Deposit collateral sufficient for max possible draw (500 util).
    let active = Address::generate(&env);
    let max_expected_utilized = 500_i128;
    let required_collateral = max_expected_utilized * 150 / 100;
    let mint_amount = required_collateral + 100_000;
    token::StellarAssetClient::new(&env, &d.token_addr).mint(&active, &mint_amount);
    token::Client::new(&env, &d.token_addr).approve(
        &active,
        &d.contract_id,
        &mint_amount,
        &1_000_000_u32,
    );
    d.client
        .open_credit_line(&active, &CREDIT_LIMIT, &0_u32, &60_u32);
    d.client.deposit_collateral(&active, &required_collateral);
    d.client.draw_credit(&active, &300_i128);

    let line_before = d.client.get_credit_line(&active).unwrap();
    assert_eq!(line_before.status, CreditStatus::Active);
    assert_eq!(line_before.utilized_amount, 300);

    // Seed oracle price.
    let b1 = open_draw_default(&d, &env, 200);
    d.client.settle_default_liquidation(
        &b1,
        &200_i128,
        &sid(&env, "seed"),
        &CLOSE_FACTOR_FULL,
        &Some(1_000_i128),
    );

    // Advance time beyond max_age — oracle is now stale.
    env.ledger().with_mut(|l| l.timestamp = 1_000 + ONE_HOUR + 1);

    // Active borrower can still draw and repay.
    d.client.draw_credit(&active, &100_i128);
    let line_mid = d.client.get_credit_line(&active).unwrap();
    assert_eq!(line_mid.utilized_amount, 400);

    d.client.repay_credit(&active, &50_i128);
    let line_end = d.client.get_credit_line(&active).unwrap();
    assert_eq!(line_end.utilized_amount, 350);

    // Only default-liquidation settlement is blocked during outage.
    let b2 = open_draw_default(&d, &env, 200);
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        d.client.settle_default_liquidation(
            &b2,
            &200_i128,
            &sid(&env, "blocked"),
            &CLOSE_FACTOR_FULL,
            &Some(1_010_i128),
        );
    }));
    assert!(r.is_err(), "stale oracle should block settlement");
}

// ── 8. Oracle outage — downward deviation also blocked ───────────────────────

#[test]
#[should_panic(expected = "HostError: Error(Contract, #38)")]
fn e2e_oracle_outage_downward_deviation_blocked() {
    let env = Env::default();
    let d = setup(&env);
    d.client.set_oracle_config(&500_u32, &ONE_HOUR);

    // Seed price at 1_000.
    let b1 = open_draw_default(&d, &env, 200);
    d.client.settle_default_liquidation(
        &b1,
        &200_i128,
        &sid(&env, "seed"),
        &CLOSE_FACTOR_FULL,
        &Some(1_000_i128),
    );

    // Sharp downward price — 20% drop (800 from 1000), exceeds 5%.
    let b2 = open_draw_default(&d, &env, 200);
    d.client.settle_default_liquidation(
        &b2,
        &200_i128,
        &sid(&env, "down"),
        &CLOSE_FACTOR_FULL,
        &Some(800_i128),
    );
}
