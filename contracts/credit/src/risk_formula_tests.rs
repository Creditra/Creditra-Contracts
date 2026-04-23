// SPDX-License-Identifier: MIT

//! Tests for the risk-score-based dynamic interest rate formula (issue #265).

use crate::types::{CreditStatus, RateFormulaConfig};
use crate::risk::{compute_rate_from_score, MAX_INTEREST_RATE_BPS};
use crate::{Credit, CreditClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};


fn make_cfg(base: u32, slope: u32, min: u32, max: u32) -> RateFormulaConfig {
    RateFormulaConfig {
        base_rate_bps: base,
        slope_bps_per_score: slope,
        min_rate_bps: min,
        max_rate_bps: max,
    }
}

// ── Pure formula unit tests ───────────────────────────────────────────────

#[test]
fn compute_rate_score_zero_returns_base() {
    let cfg = make_cfg(200, 50, 100, 5000);
    assert_eq!(compute_rate_from_score(&cfg, 0), 200);
}

#[test]
fn compute_rate_score_max_returns_clamped() {
    let cfg = make_cfg(200, 50, 100, 5000);
    // raw = 200 + 100*50 = 5200, clamped to 5000
    assert_eq!(compute_rate_from_score(&cfg, 100), 5000);
}

#[test]
fn compute_rate_score_mid() {
    let cfg = make_cfg(200, 50, 100, 5000);
    // raw = 200 + 50*50 = 2700
    assert_eq!(compute_rate_from_score(&cfg, 50), 2700);
}

#[test]
fn compute_rate_floors_to_min() {
    let cfg = make_cfg(100, 10, 500, 5000);
    // raw = 100 + 0*10 = 100, floored to 500
    assert_eq!(compute_rate_from_score(&cfg, 0), 500);
}

#[test]
fn compute_rate_clamps_to_max() {
    let cfg = make_cfg(5000, 100, 100, 6000);
    // raw = 5000 + 100*100 = 15000, clamped to 6000
    assert_eq!(compute_rate_from_score(&cfg, 100), 6000);
}

#[test]
fn compute_rate_respects_global_cap() {
    let cfg = make_cfg(5000, 100, 100, 10_000);
    // raw = 5000 + 100*100 = 15000, clamped to 10000
    assert_eq!(compute_rate_from_score(&cfg, 100), MAX_INTEREST_RATE_BPS);
}

#[test]
fn compute_rate_overflow_saturates() {
    let cfg = make_cfg(u32::MAX, u32::MAX, 0, 10_000);
    assert_eq!(compute_rate_from_score(&cfg, 100), 10_000);
}

#[test]
fn compute_rate_min_equals_max() {
    let cfg = make_cfg(0, 0, 500, 500);
    assert_eq!(compute_rate_from_score(&cfg, 0), 500);
    assert_eq!(compute_rate_from_score(&cfg, 50), 500);
    assert_eq!(compute_rate_from_score(&cfg, 100), 500);
}

#[test]
fn compute_rate_zero_slope() {
    let cfg = make_cfg(300, 0, 100, 5000);
    assert_eq!(compute_rate_from_score(&cfg, 0), 300);
    assert_eq!(compute_rate_from_score(&cfg, 100), 300);
}

// ── Integration tests via contract client ────────────────────────────────

#[test]
fn formula_disabled_by_default() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    assert!(client.get_rate_formula_config().is_none());
}

#[test]
fn set_and_get_rate_formula_config() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    client.set_rate_formula_config(&200_u32, &50_u32, &100_u32, &5000_u32);

    let cfg = client.get_rate_formula_config().unwrap();
    assert_eq!(cfg.base_rate_bps, 200);
    assert_eq!(cfg.slope_bps_per_score, 50);
    assert_eq!(cfg.min_rate_bps, 100);
    assert_eq!(cfg.max_rate_bps, 5000);
}

#[test]
fn clear_rate_formula_config_removes_it() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    client.set_rate_formula_config(&200_u32, &50_u32, &100_u32, &5000_u32);
    assert!(client.get_rate_formula_config().is_some());

    client.clear_rate_formula_config();
    assert!(client.get_rate_formula_config().is_none());
}

#[test]
fn update_risk_uses_formula_when_enabled() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    client.open_credit_line(&borrower, &10_000_i128, &300_u32, &50_u32);

    // Enable formula: base=200, slope=50, min=100, max=5000
    client.set_rate_formula_config(&200_u32, &50_u32, &100_u32, &5000_u32);

    // Update with risk_score=60. Formula: 200 + 60*50 = 3200
    // The passed interest_rate_bps (9999) should be IGNORED.
    client.update_risk_parameters(&borrower, &10_000_i128, &9999_u32, &60_u32);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.interest_rate_bps, 3200);
    assert_eq!(line.risk_score, 60);
}

#[test]
fn update_risk_uses_manual_rate_when_disabled() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    client.open_credit_line(&borrower, &10_000_i128, &300_u32, &50_u32);

    client.update_risk_parameters(&borrower, &10_000_i128, &750_u32, &60_u32);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.interest_rate_bps, 750);
}

#[test]
fn formula_clamps_to_min_for_low_score() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    client.open_credit_line(&borrower, &10_000_i128, &500_u32, &0_u32);

    // base=100, slope=10, min=500 → raw at score 0 = 100, floored to 500
    client.set_rate_formula_config(&100_u32, &10_u32, &500_u32, &5000_u32);
    client.update_risk_parameters(&borrower, &10_000_i128, &0_u32, &0_u32);

    assert_eq!(client.get_credit_line(&borrower).unwrap().interest_rate_bps, 500);
}

#[test]
fn formula_clamps_to_max_for_high_score() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    client.open_credit_line(&borrower, &10_000_i128, &300_u32, &50_u32);

    // base=200, slope=100, max=5000 → raw at score 100 = 10200, clamped to 5000
    client.set_rate_formula_config(&200_u32, &100_u32, &100_u32, &5000_u32);
    client.update_risk_parameters(&borrower, &10_000_i128, &0_u32, &100_u32);

    assert_eq!(client.get_credit_line(&borrower).unwrap().interest_rate_bps, 5000);
}

#[test]
fn clearing_formula_restores_manual_mode() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    client.open_credit_line(&borrower, &10_000_i128, &300_u32, &50_u32);

    // Formula mode
    client.set_rate_formula_config(&200_u32, &50_u32, &100_u32, &5000_u32);
    client.update_risk_parameters(&borrower, &10_000_i128, &9999_u32, &60_u32);
    assert_eq!(client.get_credit_line(&borrower).unwrap().interest_rate_bps, 3200);

    // Back to manual
    client.clear_rate_formula_config();
    client.update_risk_parameters(&borrower, &10_000_i128, &800_u32, &60_u32);
    assert_eq!(client.get_credit_line(&borrower).unwrap().interest_rate_bps, 800);
}

// ── Config validation tests ──────────────────────────────────────────────

#[test]
#[should_panic(expected = "min_rate_bps must be <= max_rate_bps")]
fn set_config_min_greater_than_max_reverts() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    client.set_rate_formula_config(&200_u32, &50_u32, &5000_u32, &100_u32);
}

#[test]
#[should_panic(expected = "max_rate_bps exceeds MAX_INTEREST_RATE_BPS")]
fn set_config_max_exceeds_cap_reverts() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    client.set_rate_formula_config(&200_u32, &50_u32, &100_u32, &10_001_u32);
}

#[test]
#[should_panic(expected = "base_rate_bps exceeds MAX_INTEREST_RATE_BPS")]
fn set_config_base_exceeds_cap_reverts() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    client.set_rate_formula_config(&10_001_u32, &50_u32, &100_u32, &5000_u32);
}

// ── Auth tests ───────────────────────────────────────────────────────────

#[test]
#[should_panic]
fn set_config_requires_admin() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    client.set_rate_formula_config(&200_u32, &50_u32, &100_u32, &5000_u32);
}

#[test]
#[should_panic]
fn clear_config_requires_admin() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    client.clear_rate_formula_config();
}

// ── Edge: boundary scores ────────────────────────────────────────────────

#[test]
fn formula_with_all_boundary_scores() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    client.open_credit_line(&borrower, &10_000_i128, &300_u32, &0_u32);

    // base=300, slope=70, min=200, max=8000
    client.set_rate_formula_config(&300_u32, &70_u32, &200_u32, &8000_u32);

    // Score 0: raw=300 → 300
    client.update_risk_parameters(&borrower, &10_000_i128, &0_u32, &0_u32);
    assert_eq!(client.get_credit_line(&borrower).unwrap().interest_rate_bps, 300);

    // Score 50: raw=300+3500=3800
    client.update_risk_parameters(&borrower, &10_000_i128, &0_u32, &50_u32);
    assert_eq!(client.get_credit_line(&borrower).unwrap().interest_rate_bps, 3800);

    // Score 100: raw=300+7000=7300
    client.update_risk_parameters(&borrower, &10_000_i128, &0_u32, &100_u32);
    assert_eq!(client.get_credit_line(&borrower).unwrap().interest_rate_bps, 7300);
}

#[test]
fn existing_lines_unaffected_until_update() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    client.open_credit_line(&borrower, &10_000_i128, &300_u32, &50_u32);

    client.set_rate_formula_config(&200_u32, &50_u32, &100_u32, &5000_u32);

    // Existing line still has original rate until update_risk_parameters
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.interest_rate_bps, 300);
    assert_eq!(line.status, CreditStatus::Active);
}

#[test]
#[should_panic(expected = "rate change exceeds maximum allowed delta")]
fn formula_update_respects_rate_change_limits() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    
    // Initial rate = 300, score = 0
    client.open_credit_line(&borrower, &10_000_i128, &300_u32, &0_u32);

    // Set change limit to 50 bps
    client.set_rate_change_limits(&50_u32, &0_u64);

    // Enable formula: base=300, slope=100.
    // At score 1, rate = 300 + 1*100 = 400.
    // Delta = 400 - 300 = 100, which exceeds limit 50.
    client.set_rate_formula_config(&300_u32, &100_u32, &100_u32, &5000_u32);
    
    client.update_risk_parameters(&borrower, &10_000_i128, &0_u32, &1_u32);
}

#[test]
fn formula_update_within_rate_change_limits_succeeds() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    
    // Initial rate = 300, score = 0
    client.open_credit_line(&borrower, &10_000_i128, &300_u32, &0_u32);

    // Set change limit to 150 bps
    client.set_rate_change_limits(&150_u32, &0_u64);

    // Enable formula: base=300, slope=100.
    // At score 1, rate = 400. Delta = 100 <= 150.
    client.set_rate_formula_config(&300_u32, &100_u32, &100_u32, &5000_u32);
    
    client.update_risk_parameters(&borrower, &10_000_i128, &0_u32, &1_u32);
    
    assert_eq!(client.get_credit_line(&borrower).unwrap().interest_rate_bps, 400);
}
