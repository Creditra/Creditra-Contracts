// SPDX-License-Identifier: MIT

//! Focused tests for the v7 `query_capabilities` / `capabilities()` read-only view.
//!
//! Covers every [`QueryCapabilities`] field across the "no credit line",
//! active line (with/without utilization), schedule presence, and closed-line
//! edge cases.

use creditra_credit::types::CreditStatus;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, Address, Env};

fn setup() -> (Env, CreditClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    (env, client, admin)
}

#[test]
fn no_credit_line_all_query_capabilities_false() {
    let (env, client, _admin) = setup();
    let borrower = Address::generate(&env);

    let caps = client.query_capabilities(&borrower);
    assert!(!caps.has_credit_line);
    assert!(!caps.has_repayment_schedule);
    assert!(!caps.health_factor_applicable);
    assert!(!caps.delinquency_applicable);
    assert!(!caps.is_delinquent);
}

#[test]
fn active_line_zero_utilization_capabilities() {
    let (env, client, _admin) = setup();
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000_i128, &500_u32, &50_u32);

    let caps = client.query_capabilities(&borrower);
    assert!(caps.has_credit_line);
    assert!(!caps.has_repayment_schedule);
    assert!(
        !caps.health_factor_applicable,
        "zero utilization → health factor is u32::MAX"
    );
    assert!(!caps.delinquency_applicable);
    assert!(!caps.is_delinquent);
}

#[test]
fn active_line_with_utilization_health_factor_applicable() {
    let (env, client, _admin) = setup();
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000_i128, &500_u32, &50_u32);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    token::StellarAssetClient::new(&env, &token).mint(&client.address, &1_000_000_i128);
    client.set_min_collateral_ratio_bps(&0);

    client.draw_credit(&borrower, &500_i128);

    let caps = client.query_capabilities(&borrower);
    assert!(caps.has_credit_line);
    assert!(caps.health_factor_applicable);
    assert!(
        !caps.delinquency_applicable,
        "no repayment schedule → delinquency not applicable"
    );
    assert!(!caps.is_delinquent);
}

#[test]
fn closed_line_capabilities() {
    let (env, client, admin) = setup();
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000_i128, &500_u32, &50_u32);
    client.close_credit_line(&borrower, &admin);

    let caps = client.query_capabilities(&borrower);
    assert!(caps.has_credit_line);
    assert!(!caps.health_factor_applicable);
    assert!(
        !caps.delinquency_applicable,
        "closed lines cannot be delinquent"
    );
    assert!(!caps.is_delinquent);

    let line = client.get_credit_line(&borrower).expect("line exists");
    assert_eq!(line.status, CreditStatus::Closed);
}

#[test]
fn query_capabilities_deterministic_same_result_twice() {
    let (env, client, _admin) = setup();
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000_i128, &500_u32, &50_u32);

    let caps1 = client.query_capabilities(&borrower);
    let caps2 = client.query_capabilities(&borrower);
    assert_eq!(caps1, caps2, "query_capabilities must be deterministic");
}
