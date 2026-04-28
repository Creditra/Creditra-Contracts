// SPDX-License-Identifier: MIT

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, Address, Env};

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    (env, admin, contract_id)
}

fn setup_with_credit_line() -> (Env, Address, Address, Address) {
    let (env, admin, contract_id) = setup();
    let borrower = Address::generate(&env);
    let client = CreditClient::new(&env, &contract_id);
    client.open_credit_line(&borrower, &10_000, &500, &50);
    (env, admin, contract_id, borrower)
}

// ── draw_credit error paths ────────────────────────────────────────────────

#[test]
#[should_panic(expected = "amount must be positive")]
fn draw_credit_zero_amount_panics() {
    let (env, _admin, contract_id, borrower) = setup_with_credit_line();
    let client = CreditClient::new(&env, &contract_id);
    client.draw_credit(&borrower, &0);
}

#[test]
#[should_panic(expected = "amount must be positive")]
fn draw_credit_negative_amount_panics() {
    let (env, _admin, contract_id, borrower) = setup_with_credit_line();
    let client = CreditClient::new(&env, &contract_id);
    client.draw_credit(&borrower, &-100);
}

#[test]
#[should_panic(expected = "Error(Contract, #19)")]
fn draw_credit_when_draws_frozen() {
    let (env, _admin, contract_id, borrower) = setup_with_credit_line();
    let client = CreditClient::new(&env, &contract_id);
    client.freeze_draws();
    client.draw_credit(&borrower, &100);
}

#[test]
#[should_panic(expected = "Error(Contract, #17)")]
fn draw_credit_exceeds_max_draw_amount() {
    let (env, _admin, contract_id, borrower) = setup_with_credit_line();
    let client = CreditClient::new(&env, &contract_id);
    client.set_max_draw_amount(&500);
    client.draw_credit(&borrower, &600);
}

#[test]
fn draw_credit_within_max_draw_amount() {
    let (env, _admin, contract_id, borrower) = setup_with_credit_line();
    let client = CreditClient::new(&env, &contract_id);
    client.set_max_draw_amount(&500);
    client.draw_credit(&borrower, &500);
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 500);
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn draw_credit_nonexistent_borrower() {
    let (env, _admin, contract_id) = setup();
    let stranger = Address::generate(&env);
    let client = CreditClient::new(&env, &contract_id);
    client.draw_credit(&stranger, &100);
}

#[test]
#[should_panic(expected = "Error(Contract, #20)")]
fn draw_credit_on_suspended_line() {
    let (env, _admin, contract_id, borrower) = setup_with_credit_line();
    let client = CreditClient::new(&env, &contract_id);
    client.suspend_credit_line(&borrower);
    client.draw_credit(&borrower, &100);
}

#[test]
#[should_panic(expected = "Error(Contract, #21)")]
fn draw_credit_on_defaulted_line() {
    let (env, _admin, contract_id, borrower) = setup_with_credit_line();
    let client = CreditClient::new(&env, &contract_id);
    client.default_credit_line(&borrower);
    client.draw_credit(&borrower, &100);
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn draw_credit_on_closed_line() {
    let (env, admin, contract_id, borrower) = setup_with_credit_line();
    let client = CreditClient::new(&env, &contract_id);
    client.close_credit_line(&borrower, &admin);
    client.draw_credit(&borrower, &100);
}

#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn draw_credit_over_limit() {
    let (env, _admin, contract_id, borrower) = setup_with_credit_line();
    let client = CreditClient::new(&env, &contract_id);
    client.draw_credit(&borrower, &10_001);
}

// ── repay_credit error paths ───────────────────────────────────────────────

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn repay_credit_zero_amount_panics() {
    let (env, _admin, contract_id, borrower) = setup_with_credit_line();
    let client = CreditClient::new(&env, &contract_id);
    client.draw_credit(&borrower, &100);
    client.repay_credit(&borrower, &0);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn repay_credit_negative_amount_panics() {
    let (env, _admin, contract_id, borrower) = setup_with_credit_line();
    let client = CreditClient::new(&env, &contract_id);
    client.draw_credit(&borrower, &100);
    client.repay_credit(&borrower, &-50);
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn repay_credit_nonexistent_borrower() {
    let (env, _admin, contract_id) = setup();
    let stranger = Address::generate(&env);
    let client = CreditClient::new(&env, &contract_id);
    client.repay_credit(&stranger, &100);
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn repay_credit_on_closed_line() {
    let (env, admin, contract_id, borrower) = setup_with_credit_line();
    let client = CreditClient::new(&env, &contract_id);
    client.close_credit_line(&borrower, &admin);
    client.repay_credit(&borrower, &100);
}

#[test]
fn repay_credit_overpayment_capped() {
    let (env, _admin, contract_id, borrower) = setup_with_credit_line();
    let client = CreditClient::new(&env, &contract_id);
    client.draw_credit(&borrower, &100);
    client.repay_credit(&borrower, &500);
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 0);
}

// ── getter coverage ────────────────────────────────────────────────────────

#[test]
fn get_liquidity_source_returns_contract_when_unset() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let source = client.get_liquidity_source();
    assert_eq!(source, contract_id);
}

#[test]
fn get_liquidity_source_returns_configured_source() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let reserve = Address::generate(&env);
    client.set_liquidity_source(&reserve);
    let source = client.get_liquidity_source();
    assert_eq!(source, reserve);
}

#[test]
fn get_schema_version_returns_none_initially() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);
    assert!(client.get_schema_version().is_none());
}

#[test]
fn get_max_draw_amount_returns_none_initially() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);
    assert!(client.get_max_draw_amount().is_none());
}

#[test]
fn get_max_draw_amount_returns_value_after_set() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);
    client.set_max_draw_amount(&5_000);
    assert_eq!(client.get_max_draw_amount(), Some(5_000));
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn set_max_draw_amount_zero_panics() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);
    client.set_max_draw_amount(&0);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn set_max_draw_amount_negative_panics() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);
    client.set_max_draw_amount(&-100);
}

#[test]
fn get_grace_period_config_returns_none_initially() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);
    assert!(client.get_grace_period_config().is_none());
}

#[test]
fn get_rate_change_limits_returns_none_initially() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);
    assert!(client.get_rate_change_limits().is_none());
}

#[test]
fn is_draws_frozen_default_false() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);
    assert!(!client.is_draws_frozen());
}

// ── config error paths ─────────────────────────────────────────────────────

#[test]
#[should_panic(expected = "Error(Contract, #14)")]
fn init_already_initialized_panics() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let new_admin = Address::generate(&env);
    client.init(&new_admin);
}

// ── draw with liquidity token configured ───────────────────────────────────

#[test]
fn draw_and_repay_with_liquidity_token() {
    let (env, _admin, contract_id, borrower) = setup_with_credit_line();
    let client = CreditClient::new(&env, &contract_id);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);

    let sac = token::StellarAssetClient::new(&env, &token_address);
    sac.mint(&contract_id, &10_000);

    client.draw_credit(&borrower, &1_000);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 1_000);

    let borrower_balance = token::Client::new(&env, &token_address).balance(&borrower);
    assert_eq!(borrower_balance, 1_000);

    sac.mint(&borrower, &1_000);
    token::Client::new(&env, &token_address).approve(&borrower, &contract_id, &1_000, &1_000);
    client.repay_credit(&borrower, &500);

    let line_after = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line_after.utilized_amount, 500);
}

#[test]
fn set_and_get_liquidity_source() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    let reserve = Address::generate(&env);
    client.set_liquidity_source(&reserve);

    let source = client.get_liquidity_source();
    assert_eq!(source, reserve);
}

#[test]
#[should_panic(expected = "Error(Contract, #24)")]
fn draw_with_insufficient_liquidity_reserve() {
    let (env, _admin, contract_id, borrower) = setup_with_credit_line();
    let client = CreditClient::new(&env, &contract_id);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);

    let sac = token::StellarAssetClient::new(&env, &token_address);
    sac.mint(&contract_id, &100);

    client.draw_credit(&borrower, &500);
}

// ── repay with liquidity token ─────────────────────────────────────────────

#[test]
fn repay_with_zero_effective_amount() {
    let (env, _admin, contract_id, borrower) = setup_with_credit_line();
    let client = CreditClient::new(&env, &contract_id);

    client.repay_credit(&borrower, &100);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 0);
}

// ── successful draw without token (no transfer path) ──────────────────────

#[test]
fn draw_without_liquidity_token_skips_transfer() {
    let (env, _admin, contract_id, borrower) = setup_with_credit_line();
    let client = CreditClient::new(&env, &contract_id);

    client.draw_credit(&borrower, &5_000);
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 5_000);

    client.draw_credit(&borrower, &3_000);
    let line2 = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line2.utilized_amount, 8_000);
}

// ── risk parameter edge cases ──────────────────────────────────────────────

#[test]
#[should_panic]
fn update_risk_limit_below_utilization_panics() {
    let (env, _admin, contract_id, borrower) = setup_with_credit_line();
    let client = CreditClient::new(&env, &contract_id);
    client.draw_credit(&borrower, &5_000);
    client.update_risk_parameters(&borrower, &4_000, &500, &50);
}

#[test]
#[should_panic]
fn update_risk_negative_limit_panics() {
    let (env, _admin, contract_id, borrower) = setup_with_credit_line();
    let client = CreditClient::new(&env, &contract_id);
    client.update_risk_parameters(&borrower, &-100, &500, &50);
}

#[test]
#[should_panic]
fn update_risk_score_too_high_panics() {
    let (env, _admin, contract_id, borrower) = setup_with_credit_line();
    let client = CreditClient::new(&env, &contract_id);
    client.update_risk_parameters(&borrower, &10_000, &500, &101);
}

#[test]
#[should_panic]
fn update_risk_rate_too_high_panics() {
    let (env, _admin, contract_id, borrower) = setup_with_credit_line();
    let client = CreditClient::new(&env, &contract_id);
    client.update_risk_parameters(&borrower, &10_000, &10_001, &50);
}

// ── rate change limits enforcement ─────────────────────────────────────────

#[test]
#[should_panic(expected = "rate change exceeds maximum allowed delta")]
fn update_risk_rate_change_exceeds_limit() {
    let (env, _admin, contract_id, borrower) = setup_with_credit_line();
    let client = CreditClient::new(&env, &contract_id);
    client.set_rate_change_limits(&100, &0);
    client.update_risk_parameters(&borrower, &10_000, &700, &50);
}

#[test]
fn update_risk_rate_change_within_limit() {
    let (env, _admin, contract_id, borrower) = setup_with_credit_line();
    let client = CreditClient::new(&env, &contract_id);
    client.set_rate_change_limits(&200, &0);
    client.update_risk_parameters(&borrower, &10_000, &600, &50);
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.interest_rate_bps, 600);
}
