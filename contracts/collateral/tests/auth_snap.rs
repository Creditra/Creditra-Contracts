// SPDX-License-Identifier: MIT

//! Per-entrypoint authorization snapshot for the collateral v7 surface.
//!
//! These tests pin the signer and authorization count recorded by Soroban for
//! every state-changing collateral entrypoint. Token transfers may be nested
//! below the recorded authorization, but they must not add another signer.
//!
//! | Entrypoint | Required signer | Auths recorded |
//! |---|---|---|
//! | `deposit_collateral` | borrower | 1 |
//! | `withdraw_collateral` | borrower | 1 |
//! | `partial_release_collateral` | borrower | 1 |
//! | `repay_and_release_collateral` | borrower | 1 |
//! | `deposit_collateral_token` | borrower | 1 |
//! | `withdraw_collateral_token` | borrower | 1 |
//! | `set_min_collateral_ratio_bps` | admin | 1 |
//! | `set_collateral_risk_weight` | admin | 1 |
//! | `set_collateral_token_allowlist` | admin | 1 |
//! | `set_admin_collateral_cooldown_seconds` | admin | 1 |
//!
//! Read-only collateral entrypoints require no authorization.

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{
    testutils::Address as _,
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env, Vec,
};

const COLLATERAL_AMOUNT: i128 = 1_000;

struct Fixture<'a> {
    client: CreditClient<'a>,
    admin: Address,
    borrower: Address,
    contract_id: Address,
    token: Address,
}

/// Deploy a configured contract and fund a borrower with collateral tokens.
fn setup(env: &Env) -> Fixture<'_> {
    env.mock_all_auths();

    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    let token = env
        .register_stellar_asset_contract_v2(Address::generate(env))
        .address();
    client.set_liquidity_token(&token);
    StellarAssetClient::new(env, &token).mint(&borrower, &(COLLATERAL_AMOUNT * 2));

    Fixture {
        client,
        admin,
        borrower,
        contract_id,
        token,
    }
}

/// Assert that the immediately preceding call required exactly `signer`.
fn assert_single_auth(env: &Env, signer: &Address, entrypoint: &str) {
    let auths = env.auths();
    assert_eq!(
        auths.len(),
        1,
        "{entrypoint} must record exactly one authorization"
    );
    assert_eq!(
        &auths[0].0, signer,
        "{entrypoint} must require the documented signer"
    );
}

#[test]
fn deposit_collateral_auth_snapshot() {
    let env = Env::default();
    let fixture = setup(&env);

    fixture
        .client
        .deposit_collateral(&fixture.borrower, &COLLATERAL_AMOUNT);

    assert_single_auth(&env, &fixture.borrower, "deposit_collateral");
}

#[test]
fn withdraw_collateral_auth_snapshot() {
    let env = Env::default();
    let fixture = setup(&env);
    fixture
        .client
        .deposit_collateral(&fixture.borrower, &COLLATERAL_AMOUNT);

    fixture
        .client
        .withdraw_collateral(&fixture.borrower, &COLLATERAL_AMOUNT);

    assert_single_auth(&env, &fixture.borrower, "withdraw_collateral");
}

#[test]
fn partial_release_collateral_auth_snapshot() {
    let env = Env::default();
    let fixture = setup(&env);
    fixture
        .client
        .deposit_collateral(&fixture.borrower, &COLLATERAL_AMOUNT);

    fixture
        .client
        .partial_release_collateral(&fixture.borrower, &1);

    assert_single_auth(&env, &fixture.borrower, "partial_release_collateral");
}

#[test]
fn repay_and_release_collateral_auth_snapshot() {
    let env = Env::default();
    let fixture = setup(&env);
    StellarAssetClient::new(&env, &fixture.token).mint(&fixture.contract_id, &1_000);
    fixture
        .client
        .open_credit_line(&fixture.borrower, &1_000, &300, &50);
    fixture
        .client
        .deposit_collateral(&fixture.borrower, &COLLATERAL_AMOUNT);
    fixture.client.draw_credit(&fixture.borrower, &100);
    TokenClient::new(&env, &fixture.token).approve(
        &fixture.borrower,
        &fixture.contract_id,
        &100,
        &1_000,
    );

    fixture
        .client
        .repay_and_release_collateral(&fixture.borrower, &1);

    assert_single_auth(&env, &fixture.borrower, "repay_and_release_collateral");
}

#[test]
fn deposit_collateral_token_auth_snapshot() {
    let env = Env::default();
    let fixture = setup(&env);
    let mut tokens = Vec::new(&env);
    tokens.push_back(fixture.token.clone());
    fixture.client.set_collateral_token_allowlist(&tokens);

    fixture
        .client
        .deposit_collateral_token(&fixture.borrower, &fixture.token, &COLLATERAL_AMOUNT);

    assert_single_auth(&env, &fixture.borrower, "deposit_collateral_token");
}

#[test]
fn withdraw_collateral_token_auth_snapshot() {
    let env = Env::default();
    let fixture = setup(&env);
    let mut tokens = Vec::new(&env);
    tokens.push_back(fixture.token.clone());
    fixture.client.set_collateral_token_allowlist(&tokens);
    fixture
        .client
        .deposit_collateral_token(&fixture.borrower, &fixture.token, &COLLATERAL_AMOUNT);

    fixture
        .client
        .withdraw_collateral_token(&fixture.borrower, &fixture.token, &COLLATERAL_AMOUNT);

    assert_single_auth(&env, &fixture.borrower, "withdraw_collateral_token");
}

#[test]
fn set_min_collateral_ratio_bps_auth_snapshot() {
    let env = Env::default();
    let fixture = setup(&env);

    fixture.client.set_min_collateral_ratio_bps(&15_000);

    assert_single_auth(&env, &fixture.admin, "set_min_collateral_ratio_bps");
}

#[test]
fn set_collateral_risk_weight_auth_snapshot() {
    let env = Env::default();
    let fixture = setup(&env);

    fixture
        .client
        .set_collateral_risk_weight(&fixture.token, &8_000);

    assert_single_auth(&env, &fixture.admin, "set_collateral_risk_weight");
}

#[test]
fn set_collateral_token_allowlist_auth_snapshot() {
    let env = Env::default();
    let fixture = setup(&env);
    let mut tokens = Vec::new(&env);
    tokens.push_back(fixture.token.clone());

    fixture.client.set_collateral_token_allowlist(&tokens);

    assert_single_auth(&env, &fixture.admin, "set_collateral_token_allowlist");
}

#[test]
fn set_admin_collateral_cooldown_seconds_auth_snapshot() {
    let env = Env::default();
    let fixture = setup(&env);

    fixture.client.set_col_admin_cooldown_secs(&120);

    assert_single_auth(&env, &fixture.admin, "set_col_admin_cooldown_secs");
}

#[test]
fn collateral_queries_require_no_auth() {
    let env = Env::default();
    let fixture = setup(&env);

    let _ = fixture.client.get_collateral(&fixture.borrower);
    assert!(env.auths().is_empty(), "get_collateral must be auth-free");

    let _ = fixture.client.get_min_collateral_ratio_bps();
    assert!(
        env.auths().is_empty(),
        "get_min_collateral_ratio_bps must be auth-free"
    );

    let _ = fixture.client.get_col_admin_cooldown_secs();
    assert!(
        env.auths().is_empty(),
        "get_col_admin_cooldown_secs must be auth-free"
    );

    let _ = fixture.client.get_last_col_admin_action_ts();
    assert!(
        env.auths().is_empty(),
        "get_last_col_admin_action_ts must be auth-free"
    );

    let _ = fixture.client.get_collateral_tokens();
    assert!(
        env.auths().is_empty(),
        "get_collateral_tokens must be auth-free"
    );

    let _ = fixture
        .client
        .get_collateral_for_token(&fixture.borrower, &fixture.token);
    assert!(
        env.auths().is_empty(),
        "get_collateral_for_token must be auth-free"
    );
}
