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
//! - Settlement at exactly `max_age_seconds` is accepted; one second later it
//!   reverts `OraclePriceStale` (#37).
//! - A missing quorum submission reverts `OracleQuorumNotMet` (#50), and a
//!   caller-supplied `oracle_price` cannot stand in for it.
//! - A caller-supplied `oracle_price` cannot refresh a stale quorum price.
//! - A rejected settlement mutates no state and does not consume its settlement id.
//! - `orc_qcfg` and `orc_qprc` events are emitted correctly.

use creditra_credit::types::{ContractError, CreditStatus, OracleQuorumConfig};
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
    // `config::init` intends a 0 bps floor in tests ("0 in tests" in the comment
    // there) but guards the 15000 bps production floor with `#[cfg(not(test))]`,
    // which is not set for the library when an integration test links it. The
    // floor therefore applies here and any uncollateralized `draw_credit` below
    // would abort with `CollateralRatioBelowMinimum` (#35). These tests exercise
    // oracle aging, not collateral policy, so the test-environment floor is set
    // explicitly instead of depending on that `cfg` semantics.
    client.set_min_collateral_ratio_bps(&0_u32);
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
        // `draw_credit` enforces the 150% minimum collateral ratio, so the
        // borrower must post collateral before drawing.
        client.deposit_collateral(&borrower, &(utilized * 150 / 100));
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
    client.settle_default_liquidation(&borrower, &500_i128, &sid(&env, "q1"), &10_000_u32, &None);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Closed
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #50)")] // OracleQuorumNotMet
fn settlement_fails_when_no_quorum_price_submitted() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    // Quorum config set but submit_oracle_prices never called
    client.set_oracle_quorum_config(&2_u32, &500_u32, &3_600_u64);

    let borrower = open_and_default(&client, &env, &contract_id, 500);
    client.settle_default_liquidation(&borrower, &500_i128, &sid(&env, "q1"), &10_000_u32, &None);
}

#[test]
#[should_panic(expected = "Error(Contract, #37)")] // OraclePriceStale
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
    client.settle_default_liquidation(&borrower, &500_i128, &sid(&env, "q1"), &10_000_u32, &None);
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
    client.settle_default_liquidation(&borrower, &500_i128, &sid(&env, "q1"), &10_000_u32, &None);
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
    client.settle_default_liquidation(&borrower, &500_i128, &sid(&env, "q1"), &10_000_u32, &None);
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
    client.settle_default_liquidation(&b1, &300_i128, &sid(&env, "s1"), &10_000_u32, &None);
    assert_eq!(
        client.get_credit_line(&b1).unwrap().status,
        CreditStatus::Closed
    );

    // Second settlement reuses the stored quorum price without re-submitting
    let b2 = open_and_default(&client, &env, &contract_id, 400);
    client.settle_default_liquidation(&b2, &400_i128, &sid(&env, "s2"), &10_000_u32, &None);
    assert_eq!(
        client.get_credit_line(&b2).unwrap().status,
        CreditStatus::Closed
    );
}

// ── age boundary and typed error identity ────────────────────────────────────
//
// The rejection tests above use a bare `#[should_panic]`, which also passes when
// settlement fails for an unrelated reason. `try_settle_default_liquidation`
// returns the typed error, so the tests below pin the exact error — the
// `OraclePriceStale` (#37) and `OracleQuorumNotMet` (#50) required by the
// acceptance criteria — check both sides of the `now - submitted_at <=
// max_age_seconds` boundary one second apart, and assert that a rejected
// settlement mutates no state.

/// `ContractError::OraclePriceStale` — stale price, see `docs/errors.md`.
const ORACLE_PRICE_STALE_CODE: u32 = 37;
/// `ContractError::OracleQuorumNotMet` — no qualifying quorum price.
const ORACLE_QUORUM_NOT_MET_CODE: u32 = 50;

#[test]
fn settlement_exactly_at_max_age_is_accepted_without_error() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    client.set_oracle_quorum_config(&2_u32, &500_u32, &3_600_u64);

    env.ledger().with_mut(|l| l.timestamp = 1_000);
    client.submit_oracle_prices(&vec![&env, 1_000i128, 1_020i128]);

    // age == max_age_seconds: the check is `>` (not `>=`), so this must not revert.
    env.ledger().with_mut(|l| l.timestamp = 1_000 + 3_600);
    let borrower = open_and_default(&client, &env, &contract_id, 500);

    let result = client.try_settle_default_liquidation(
        &borrower,
        &500_i128,
        &sid(&env, "boundary_ok"),
        &10_000_u32,
        &None,
    );
    assert!(
        result.is_ok(),
        "settlement at exactly max_age_seconds must be accepted"
    );
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Closed
    );
}

#[test]
fn settlement_one_second_past_max_age_reverts_with_error_37() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    client.set_oracle_quorum_config(&2_u32, &500_u32, &3_600_u64);

    env.ledger().with_mut(|l| l.timestamp = 1_000);
    client.submit_oracle_prices(&vec![&env, 1_000i128, 1_020i128]);

    // age == max_age_seconds + 1: one second past the accepted boundary.
    env.ledger().with_mut(|l| l.timestamp = 1_000 + 3_600 + 1);
    let borrower = open_and_default(&client, &env, &contract_id, 500);

    let result = client.try_settle_default_liquidation(
        &borrower,
        &500_i128,
        &sid(&env, "boundary_stale"),
        &10_000_u32,
        &None,
    );
    assert!(
        result.is_err(),
        "one second past max_age_seconds must revert"
    );

    assert_eq!(
        ContractError::OraclePriceStale as u32,
        ORACLE_PRICE_STALE_CODE,
        "OraclePriceStale must keep discriminant 37"
    );
    let err = result.err().unwrap();
    assert_eq!(
        err.unwrap(),
        ContractError::OraclePriceStale.into(),
        "stale quorum price must revert with #37 OraclePriceStale"
    );

    // Rejection happens before any state mutation.
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Defaulted);
    assert_eq!(line.utilized_amount, 500_i128);
}

#[test]
fn settlement_with_quorum_config_but_no_submission_reverts_with_error_50() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    client.set_oracle_quorum_config(&2_u32, &500_u32, &3_600_u64);

    let borrower = open_and_default(&client, &env, &contract_id, 500);
    let result = client.try_settle_default_liquidation(
        &borrower,
        &500_i128,
        &sid(&env, "no_price"),
        &10_000_u32,
        &None,
    );
    assert!(
        result.is_err(),
        "quorum config without a submission must revert"
    );

    assert_eq!(
        ContractError::OracleQuorumNotMet as u32,
        ORACLE_QUORUM_NOT_MET_CODE,
        "OracleQuorumNotMet must keep discriminant 50"
    );
    let err = result.err().unwrap();
    assert_eq!(
        err.unwrap(),
        ContractError::OracleQuorumNotMet.into(),
        "missing quorum price must revert with #50 OracleQuorumNotMet"
    );

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Defaulted);
    assert_eq!(line.utilized_amount, 500_i128);
}

#[test]
fn caller_oracle_price_cannot_replace_a_missing_quorum_price() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    client.set_oracle_quorum_config(&2_u32, &500_u32, &3_600_u64);

    // A plausible caller-supplied price must not stand in for a quorum submission.
    let borrower = open_and_default(&client, &env, &contract_id, 500);
    let result = client.try_settle_default_liquidation(
        &borrower,
        &500_i128,
        &sid(&env, "caller_only"),
        &10_000_u32,
        &Some(1_000_i128),
    );
    assert!(
        result.is_err(),
        "a caller-supplied oracle_price must not satisfy the quorum requirement"
    );
    let err = result.err().unwrap();
    assert_eq!(err.unwrap(), ContractError::OracleQuorumNotMet.into());
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Defaulted
    );
}

#[test]
fn caller_oracle_price_cannot_rescue_a_stale_quorum_price() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    client.set_oracle_quorum_config(&2_u32, &500_u32, &3_600_u64);

    env.ledger().with_mut(|l| l.timestamp = 1_000);
    client.submit_oracle_prices(&vec![&env, 1_000i128, 1_020i128]);

    env.ledger().with_mut(|l| l.timestamp = 1_000 + 3_601);
    let borrower = open_and_default(&client, &env, &contract_id, 500);

    // A fresh-looking caller price must not refresh the stored quorum price.
    let result = client.try_settle_default_liquidation(
        &borrower,
        &500_i128,
        &sid(&env, "stale_caller"),
        &10_000_u32,
        &Some(1_020_i128),
    );
    assert!(
        result.is_err(),
        "a caller-supplied oracle_price must not refresh a stale quorum price"
    );
    let err = result.err().unwrap();
    assert_eq!(err.unwrap(), ContractError::OraclePriceStale.into());
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Defaulted
    );
}

#[test]
fn valid_quorum_price_settles_despite_a_nonsense_caller_price() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    client.set_oracle_quorum_config(&2_u32, &500_u32, &3_600_u64);

    env.ledger().with_mut(|l| l.timestamp = 1_000);
    client.submit_oracle_prices(&vec![&env, 1_000i128, 1_020i128]);

    env.ledger().with_mut(|l| l.timestamp = 1_100);
    let borrower = open_and_default(&client, &env, &contract_id, 500);

    // Quorum mode ignores the argument entirely: a price far outside the quorum
    // window must neither reject the settlement nor replace the stored price.
    client.settle_default_liquidation(
        &borrower,
        &500_i128,
        &sid(&env, "ignored_arg"),
        &10_000_u32,
        &Some(999_999_i128),
    );
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Closed
    );
}

#[test]
fn rejected_settlement_does_not_consume_its_settlement_id() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    client.set_oracle_quorum_config(&2_u32, &500_u32, &3_600_u64);

    env.ledger().with_mut(|l| l.timestamp = 1_000);
    client.submit_oracle_prices(&vec![&env, 1_000i128, 1_020i128]);

    env.ledger().with_mut(|l| l.timestamp = 1_000 + 3_601);
    let borrower = open_and_default(&client, &env, &contract_id, 500);

    let id = sid(&env, "reuse_id");
    let rejected =
        client.try_settle_default_liquidation(&borrower, &500_i128, &id, &10_000_u32, &None);
    assert!(rejected.is_err(), "stale quorum price must revert");
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Defaulted
    );

    // Refresh the quorum price and reuse the same settlement id: the rejected
    // attempt must not have recorded it as already settled.
    client.submit_oracle_prices(&vec![&env, 1_020i128, 1_040i128]);
    client.settle_default_liquidation(&borrower, &500_i128, &id, &10_000_u32, &None);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Closed
    );
}

// ── quorum refresh across the age boundary (#1362) ───────────────────────────

#[test]
fn resubmitting_quorum_price_resets_the_age_across_settlements() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    client.set_oracle_quorum_config(&2_u32, &500_u32, &3_600_u64);

    // Submission at t=1_000.
    env.ledger().with_mut(|l| l.timestamp = 1_000);
    client.submit_oracle_prices(&vec![&env, 1_000i128, 1_020i128]);

    // First settlement lands exactly on the boundary and is accepted.
    env.ledger().with_mut(|l| l.timestamp = 1_000 + 3_600);
    let b1 = open_and_default(&client, &env, &contract_id, 300);
    client.settle_default_liquidation(&b1, &300_i128, &sid(&env, "r1"), &10_000_u32, &None);
    assert_eq!(
        client.get_credit_line(&b1).unwrap().status,
        CreditStatus::Closed
    );

    // One second later the submission from t=1_000 would be stale, but a fresh
    // submission restarts the age clock for the next settlement.
    env.ledger().with_mut(|l| l.timestamp = 1_000 + 3_601);
    client.submit_oracle_prices(&vec![&env, 1_010i128, 1_030i128]);

    let b2 = open_and_default(&client, &env, &contract_id, 300);
    client.settle_default_liquidation(&b2, &300_i128, &sid(&env, "r2"), &10_000_u32, &None);
    assert_eq!(
        client.get_credit_line(&b2).unwrap().status,
        CreditStatus::Closed
    );

    // The refreshed submission is itself subject to the same boundary.
    env.ledger().with_mut(|l| l.timestamp = 8_201); // 1_000 + 3_601 + 3_600
    let b3 = open_and_default(&client, &env, &contract_id, 300);
    client.settle_default_liquidation(&b3, &300_i128, &sid(&env, "r3"), &10_000_u32, &None);
    assert_eq!(
        client.get_credit_line(&b3).unwrap().status,
        CreditStatus::Closed
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #37)")] // OraclePriceStale
fn second_settlement_across_the_boundary_requires_a_refresh() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    client.set_oracle_quorum_config(&2_u32, &500_u32, &3_600_u64);

    env.ledger().with_mut(|l| l.timestamp = 1_000);
    client.submit_oracle_prices(&vec![&env, 1_000i128, 1_020i128]);

    // First borrower settles exactly at the boundary — accepted.
    env.ledger().with_mut(|l| l.timestamp = 1_000 + 3_600);
    let b1 = open_and_default(&client, &env, &contract_id, 300);
    client.settle_default_liquidation(&b1, &300_i128, &sid(&env, "b1"), &10_000_u32, &None);
    assert_eq!(
        client.get_credit_line(&b1).unwrap().status,
        CreditStatus::Closed
    );

    // The age limit is absolute, not per borrower: the same submission is one
    // second too old for the next settlement unless the quorum is refreshed.
    env.ledger().with_mut(|l| l.timestamp = 1_000 + 3_601);
    let b2 = open_and_default(&client, &env, &contract_id, 300);
    client.settle_default_liquidation(&b2, &300_i128, &sid(&env, "b2"), &10_000_u32, &None);
}
