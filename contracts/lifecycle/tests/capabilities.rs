// SPDX-License-Identifier: MIT

//! Focused tests for the v7 `lifecycle_capabilities` read-only view.
//!
//! Covers every [`LifecycleCapabilities`] field across every [`CreditStatus`]
//! reachable from the public entrypoints, plus the "no credit line" and
//! "protocol paused" edge cases.

use creditra_credit::types::CreditStatus;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

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
fn no_credit_line_all_capabilities_false() {
    let (_env, client, _admin) = setup();
    let borrower = Address::generate(&_env);

    let caps = client.lifecycle_capabilities(&borrower);
    assert!(!caps.can_suspend);
    assert!(!caps.can_self_suspend);
    assert!(!caps.can_close_admin);
    assert!(!caps.can_close_borrower);
    assert!(!caps.can_default);
    assert!(!caps.can_reinstate);
}

#[test]
fn active_line_zero_utilization_capabilities() {
    let (env, client, _admin) = setup();
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000_i128, &500_u32, &50_u32);

    let caps = client.lifecycle_capabilities(&borrower);
    assert!(caps.can_suspend);
    assert!(caps.can_self_suspend);
    assert!(caps.can_close_admin);
    assert!(
        caps.can_close_borrower,
        "zero utilization allows borrower self-close"
    );
    assert!(caps.can_default);
    assert!(
        !caps.can_reinstate,
        "Active line is never reinstate-eligible"
    );
}

#[test]
fn active_line_with_utilization_blocks_borrower_close_only() {
    let (env, client, _admin) = setup();
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000_i128, &500_u32, &50_u32);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    soroban_sdk::token::StellarAssetClient::new(&env, &token)
        .mint(&client.address, &1_000_000_i128);
    client.set_min_collateral_ratio_bps(&0);

    client.draw_credit(&borrower, &500_i128);

    let caps = client.lifecycle_capabilities(&borrower);
    assert!(
        caps.can_close_admin,
        "admin force-close ignores utilization"
    );
    assert!(
        !caps.can_close_borrower,
        "borrower self-close requires zero utilization"
    );
}

#[test]
fn suspended_line_capabilities() {
    let (env, client, _admin) = setup();
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000_i128, &500_u32, &50_u32);
    client.suspend_credit_line(&borrower);

    let caps = client.lifecycle_capabilities(&borrower);
    assert!(!caps.can_suspend, "already suspended");
    assert!(!caps.can_self_suspend, "already suspended");
    assert!(caps.can_close_admin);
    assert!(caps.can_close_borrower, "zero utilization");
    assert!(
        caps.can_default,
        "Suspended -> Defaulted is a valid transition"
    );
    assert!(!caps.can_reinstate);
}

#[test]
fn defaulted_line_capabilities() {
    let (env, client, _admin) = setup();
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000_i128, &500_u32, &50_u32);
    client.default_credit_line(&borrower);

    let caps = client.lifecycle_capabilities(&borrower);
    assert!(!caps.can_suspend);
    assert!(!caps.can_self_suspend);
    assert!(
        caps.can_close_admin,
        "admin can force-close a defaulted line"
    );
    assert!(caps.can_close_borrower, "zero utilization");
    assert!(!caps.can_default, "already Defaulted");
    assert!(
        caps.can_reinstate,
        "only Defaulted lines are reinstate-eligible"
    );
}

#[test]
fn reinstated_active_line_capabilities_match_active() {
    let (env, client, _admin) = setup();
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000_i128, &500_u32, &50_u32);
    client.default_credit_line(&borrower);
    client.reinstate_credit_line(&borrower, &CreditStatus::Active);

    let caps = client.lifecycle_capabilities(&borrower);
    assert!(caps.can_suspend);
    assert!(caps.can_self_suspend);
    assert!(caps.can_default);
    assert!(!caps.can_reinstate);
}

#[test]
fn closed_line_all_capabilities_false() {
    let (env, client, admin) = setup();
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000_i128, &500_u32, &50_u32);
    client.close_credit_line(&borrower, &admin);

    let caps = client.lifecycle_capabilities(&borrower);
    assert!(!caps.can_suspend);
    assert!(!caps.can_self_suspend);
    assert!(!caps.can_close_admin, "already Closed");
    assert!(!caps.can_close_borrower, "already Closed");
    assert!(!caps.can_default, "Closed lines cannot default");
    assert!(!caps.can_reinstate);
}

#[test]
fn paused_protocol_forces_all_capabilities_false() {
    let (env, client, _admin) = setup();
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000_i128, &500_u32, &50_u32);

    client.set_protocol_paused(&true);

    let caps = client.lifecycle_capabilities(&borrower);
    assert!(!caps.can_suspend);
    assert!(!caps.can_self_suspend);
    assert!(!caps.can_close_admin);
    assert!(!caps.can_close_borrower);
    assert!(!caps.can_default);
    assert!(!caps.can_reinstate);
}
