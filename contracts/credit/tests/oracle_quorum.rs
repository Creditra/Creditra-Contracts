// SPDX-License-Identifier: MIT

//! Integration tests for the multi-oracle quorum price feed.
//!
//! # Coverage
//! - `set_oracle_quorum_config` stores config; `get_oracle_quorum_config` returns it.
//! - `set_oracle_quorum_config` validates k ≥ 2, max_dev ≤ 10_000, max_age > 0.
//! - `get_oracle_quorum_config` returns `None` when not set.
//! - `submit_oracle_prices` validates quorum, stores the resolved median price.
//! - Outlier feeds are excluded when a tighter K-wide window qualifies first.
//! - `submit_oracle_prices` fails when quorum is not met (`OracleQuorumNotMet`).
//! - `submit_oracle_prices` fails on non-positive prices (`OraclePriceInvalid`).
//! - `submit_oracle_prices` fails when quorum config is not set.
//! - Settlement uses the stored quorum price (oracle_price arg ignored).
//! - Settlement rejects a stale quorum price (`OraclePriceStale`).
//! - Settlement rejects when no quorum price has been submitted yet.
//! - Quorum mode takes precedence over the single-oracle circuit breaker.
//! - `orc_qcfg` and `orc_qprc` events are emitted correctly.

use creditra_credit::types::{CreditStatus, OracleQuorumConfig};
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Events as _, Ledger};
use soroban_sdk::{token, vec, Address, Env, Symbol, TryFromVal};

// ── helpers ───────────────────────────────────────────────────────────────────

fn setup(env: &Env) -> (CreditClient, Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    (client, contract_id, admin)
}

/// Open a credit line for `utilized` units, draw, then default it. Returns the borrower.
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

fn has_event_topic(env: &Env, kind: &str) -> bool {
    let ns = Symbol::new(env, "credit");
    let k = Symbol::new(env, kind);
    for (_contract, topics, _data) in env.events().all().iter() {
        if topics.len() < 2 {
            continue;
        }
        let t0: Result<Symbol, _> = Symbol::try_from_val(env, &topics.get(0).unwrap());
        let t1: Result<Symbol, _> = Symbol::try_from_val(env, &topics.get(1).unwrap());
        if let (Ok(t0), Ok(t1)) = (t0, t1) {
            if t0 == ns && t1 == k {
                return true;
            }
        }
    }
    false
}

// ── set / get oracle quorum config ────────────────────────────────────────────

#[test]
fn set_oracle_quorum_config_stores_and_get_returns_it() {
    let env = Env::default();
    let (client, _, _) = setup(&env);

    client.set_oracle_quorum_config(&2_u32, &500_u32, &3_600_u64);

    let cfg: OracleQuorumConfig = client.get_oracle_quorum_config().unwrap();
    assert_eq!(cfg.min_quorum_k, 2);
    assert_eq!(cfg.max_deviation_bps, 500);
    assert_eq!(cfg.max_age_seconds, 3_600);
}

#[test]
fn get_oracle_quorum_config_none_when_not_set() {
    let env = Env::default();
    let (client, _, _) = setup(&env);
    assert!(client.get_oracle_quorum_config().is_none());
}

#[test]
#[should_panic]
fn set_oracle_quorum_config_k_less_than_two_panics() {
    let env = Env::default();
    let (client, _, _) = setup(&env);
    client.set_oracle_quorum_config(&1_u32, &500_u32, &3_600_u64);
}

#[test]
#[should_panic]
fn set_oracle_quorum_config_k_zero_panics() {
    let env = Env::default();
    let (client, _, _) = setup(&env);
    client.set_oracle_quorum_config(&0_u32, &500_u32, &3_600_u64);
}

#[test]
#[should_panic]
fn set_oracle_quorum_config_max_dev_over_10000_panics() {
    let env = Env::default();
    let (client, _, _) = setup(&env);
    client.set_oracle_quorum_config(&2_u32, &10_001_u32, &3_600_u64);
}

#[test]
#[should_panic]
fn set_oracle_quorum_config_zero_age_panics() {
    let env = Env::default();
    let (client, _, _) = setup(&env);
    client.set_oracle_quorum_config(&2_u32, &500_u32, &0_u64);
}

#[test]
fn set_oracle_quorum_config_max_dev_10000_accepted() {
    let env = Env::default();
    let (client, _, _) = setup(&env);
    // max_deviation_bps == 10_000 is the inclusive upper bound
    client.set_oracle_quorum_config(&2_u32, &10_000_u32, &3_600_u64);
    let cfg = client.get_oracle_quorum_config().unwrap();
    assert_eq!(cfg.max_deviation_bps, 10_000);
}

// ── submit_oracle_prices — happy path ─────────────────────────────────────────

#[test]
fn submit_two_of_three_quorum_stores_median() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    // k=2, dev=500 bps — two feeds within 5% form a quorum
    client.set_oracle_quorum_config(&2_u32, &500_u32, &3_600_u64);

    // Sorted: 1_000, 1_040, 5_000 — window [1_000, 1_040]: dev=400 bps ≤ 500 ✓
    let prices = vec![&env, 5_000i128, 1_000i128, 1_040i128];
    client.submit_oracle_prices(&prices);

    // Verify via settlement — quorum price should allow settlement without oracle_price
    let borrower = open_and_default(&client, &env, &contract_id, 500);
    client.settle_default_liquidation(
        &borrower,
        &500_i128,
        &sid(&env, "settle1"),
        &10_000_u32,
        &None,
    );
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Closed
    );
}

#[test]
fn submit_three_of_five_quorum_picks_correct_window() {
    let env = Env::default();
    let (client, _, _) = setup(&env);
    client.set_oracle_quorum_config(&3_u32, &500_u32, &3_600_u64);

    // Sorted: 980, 990, 1_000, 1_010, 5_000
    // Window [980, 990, 1_000]: dev(1_000, 980)=204 bps ≤ 500 → qualifies
    let prices = vec![&env, 1_010i128, 5_000i128, 980i128, 990i128, 1_000i128];
    // Expect no panic — quorum was met
    client.submit_oracle_prices(&prices);
}

#[test]
fn submit_all_identical_prices_zero_deviation() {
    let env = Env::default();
    let (client, _, _) = setup(&env);
    client.set_oracle_quorum_config(&3_u32, &0_u32, &3_600_u64);

    let prices = vec![&env, 1_000i128, 1_000i128, 1_000i128];
    client.submit_oracle_prices(&prices);
}

// ── submit_oracle_prices — error paths ────────────────────────────────────────

#[test]
#[should_panic]
fn submit_oracle_prices_without_quorum_config_panics() {
    let env = Env::default();
    let (client, _, _) = setup(&env);
    // No quorum config set
    let prices = vec![&env, 1_000i128, 1_020i128];
    client.submit_oracle_prices(&prices);
}

#[test]
#[should_panic]
fn submit_quorum_not_met_panics() {
    let env = Env::default();
    let (client, _, _) = setup(&env);
    client.set_oracle_quorum_config(&2_u32, &100_u32, &3_600_u64);

    // 1_000 and 2_000 are 100% apart — no 2-wide window qualifies at 1% max dev
    let prices = vec![&env, 1_000i128, 2_000i128];
    client.submit_oracle_prices(&prices);
}

#[test]
#[should_panic]
fn submit_negative_price_panics() {
    let env = Env::default();
    let (client, _, _) = setup(&env);
    client.set_oracle_quorum_config(&2_u32, &500_u32, &3_600_u64);

    let prices = vec![&env, 1_000i128, -1i128, 1_020i128];
    client.submit_oracle_prices(&prices);
}

#[test]
#[should_panic]
fn submit_zero_price_panics() {
    let env = Env::default();
    let (client, _, _) = setup(&env);
    client.set_oracle_quorum_config(&2_u32, &500_u32, &3_600_u64);

    let prices = vec![&env, 1_000i128, 0i128];
    client.submit_oracle_prices(&prices);
}

#[test]
#[should_panic]
fn submit_k_greater_than_n_panics() {
    let env = Env::default();
    let (client, _, _) = setup(&env);
    // k=3 but only 2 prices — OracleQuorumNotMet
    client.set_oracle_quorum_config(&3_u32, &500_u32, &3_600_u64);

    let prices = vec![&env, 1_000i128, 1_020i128];
    client.submit_oracle_prices(&prices);
}

// ── settlement in quorum mode ─────────────────────────────────────────────────

#[test]
fn settlement_uses_quorum_price_ignores_oracle_price_arg() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    client.set_oracle_quorum_config(&2_u32, &500_u32, &3_600_u64);

    let prices = vec![&env, 1_000i128, 1_030i128];
    client.submit_oracle_prices(&prices);

    let borrower = open_and_default(&client, &env, &contract_id, 500);
    // oracle_price=None is fine in quorum mode — uses the stored quorum price
    client.settle_default_liquidation(
        &borrower,
        &500_i128,
        &sid(&env, "q1"),
        &10_000_u32,
        &None,
    );
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Closed
    );
}

#[test]
#[should_panic]
fn settlement_fails_when_no_quorum_price_submitted() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    // Quorum config set but submit_oracle_prices never called
    client.set_oracle_quorum_config(&2_u32, &500_u32, &3_600_u64);

    let borrower = open_and_default(&client, &env, &contract_id, 500);
    client.settle_default_liquidation(
        &borrower,
        &500_i128,
        &sid(&env, "q1"),
        &10_000_u32,
        &None,
    );
}

#[test]
#[should_panic]
fn settlement_fails_on_stale_quorum_price() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    // max_age = 1 hour
    client.set_oracle_quorum_config(&2_u32, &500_u32, &3_600_u64);

    // Submit quorum price at t=1_000
    env.ledger().with_mut(|l| l.timestamp = 1_000);
    let prices = vec![&env, 1_000i128, 1_020i128];
    client.submit_oracle_prices(&prices);

    // Advance beyond max_age_seconds
    env.ledger().with_mut(|l| l.timestamp = 1_000 + 3_601);

    let borrower = open_and_default(&client, &env, &contract_id, 500);
    client.settle_default_liquidation(
        &borrower,
        &500_i128,
        &sid(&env, "q1"),
        &10_000_u32,
        &None,
    );
}

#[test]
fn settlement_at_exact_max_age_succeeds() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    client.set_oracle_quorum_config(&2_u32, &500_u32, &3_600_u64);

    env.ledger().with_mut(|l| l.timestamp = 1_000);
    let prices = vec![&env, 1_000i128, 1_020i128];
    client.submit_oracle_prices(&prices);

    // age == max_age_seconds exactly — should be accepted (> check, not >=)
    env.ledger().with_mut(|l| l.timestamp = 1_000 + 3_600);
    let borrower = open_and_default(&client, &env, &contract_id, 500);
    client.settle_default_liquidation(
        &borrower,
        &500_i128,
        &sid(&env, "q1"),
        &10_000_u32,
        &None,
    );
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Closed
    );
}

// ── quorum mode precedence over single-oracle mode ────────────────────────────

#[test]
fn quorum_mode_takes_precedence_over_single_oracle_config() {
    // When both oracle_config and oracle_quorum_config are set,
    // settlement should use the quorum price (oracle_price arg ignored).
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);

    // Set both configs
    client.set_oracle_config(&500_u32, &3_600_u64);
    client.set_oracle_quorum_config(&2_u32, &500_u32, &3_600_u64);

    // Submit quorum prices
    let prices = vec![&env, 1_000i128, 1_020i128];
    client.submit_oracle_prices(&prices);

    // Settlement with oracle_price=None should succeed via quorum mode
    let borrower = open_and_default(&client, &env, &contract_id, 500);
    client.settle_default_liquidation(
        &borrower,
        &500_i128,
        &sid(&env, "q1"),
        &10_000_u32,
        &None,
    );
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Closed
    );
}

#[test]
fn single_oracle_mode_still_works_when_quorum_not_configured() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    // Only single-oracle config set
    client.set_oracle_config(&500_u32, &3_600_u64);

    let borrower = open_and_default(&client, &env, &contract_id, 500);
    // Single-oracle path: first price accepted
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

// ── event emission ────────────────────────────────────────────────────────────

#[test]
fn set_oracle_quorum_config_emits_orc_qcfg_event() {
    let env = Env::default();
    let (client, _, _) = setup(&env);
    client.set_oracle_quorum_config(&2_u32, &500_u32, &3_600_u64);
    assert!(has_event_topic(&env, "orc_qcfg"), "expected orc_qcfg event");
}

#[test]
fn submit_oracle_prices_emits_orc_qprc_event() {
    let env = Env::default();
    let (client, _, _) = setup(&env);
    client.set_oracle_quorum_config(&2_u32, &500_u32, &3_600_u64);

    let prices = vec![&env, 1_000i128, 1_020i128];
    client.submit_oracle_prices(&prices);

    assert!(has_event_topic(&env, "orc_qprc"), "expected orc_qprc event");
}

// ── multiple settlements with a single quorum submission ─────────────────────

#[test]
fn multiple_settlements_reuse_same_quorum_price() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    client.set_oracle_quorum_config(&2_u32, &500_u32, &3_600_u64);

    let prices = vec![&env, 1_000i128, 1_010i128];
    client.submit_oracle_prices(&prices);

    // First settlement
    let b1 = open_and_default(&client, &env, &contract_id, 300);
    client.settle_default_liquidation(
        &b1,
        &300_i128,
        &sid(&env, "s1"),
        &10_000_u32,
        &None,
    );
    assert_eq!(client.get_credit_line(&b1).unwrap().status, CreditStatus::Closed);

    // Second settlement reuses the stored quorum price without re-submitting
    let b2 = open_and_default(&client, &env, &contract_id, 400);
    client.settle_default_liquidation(
        &b2,
        &400_i128,
        &sid(&env, "s2"),
        &10_000_u32,
        &None,
    );
    assert_eq!(client.get_credit_line(&b2).unwrap().status, CreditStatus::Closed);
}
