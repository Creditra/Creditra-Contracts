// SPDX-License-Identifier: MIT

//! Per-entrypoint authentication boundary tests for collateral contract.
//!
//! # Scope
//!
//! Tests verify that every state-changing collateral entrypoint correctly enforces
//! the expected authentication boundary:
//!
//! - Borrower-facing operations (`deposit_collateral`, `withdraw_collateral`, etc.)
//!   require the **borrower's** `require_auth()`.
//! - Admin-only operations (`set_min_collateral_ratio_bps`, etc.)
//!   require the **admin's** `require_auth()`.

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token::StellarAssetClient, Address, Env, Vec};

/// Setup environment with admin, borrower, and contract initialized.
fn setup(env: &Env) -> (CreditClient<'_>, Address, Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let borrower = Address::generate(env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    // Set up a collateral token.
    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&token);

    let token_admin = StellarAssetClient::new(env, &token);
    token_admin.mint(&borrower, &1_000_000_i128);
    token_admin.mint(&admin, &1_000_000_i128);

    (client, admin, borrower, token)
}

// ─────────────────────────────────────────────────────────────────────────────
// Borrower-facing entrypoint tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn deposit_collateral_requires_borrower_auth() {
    let env = Env::default();
    let (client, _admin, borrower, _token) = setup(&env);

    client.deposit_collateral(&borrower, &1000);
    let auths = env.auths();
    assert_eq!(auths.len(), 1);
    assert_eq!(auths[0].0, borrower);
    assert_eq!(client.get_collateral(&borrower), 1000);
}

#[test]
fn deposit_collateral_rejects_zero_amount() {
    let env = Env::default();
    let (client, _admin, borrower, _token) = setup(&env);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.deposit_collateral(&borrower, &0);
    }));
    assert!(result.is_err());
}

#[test]
fn deposit_collateral_rejects_negative_amount() {
    let env = Env::default();
    let (client, _admin, borrower, _token) = setup(&env);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.deposit_collateral(&borrower, &-100);
    }));
    assert!(result.is_err());
}

#[test]
fn withdraw_collateral_requires_borrower_auth() {
    let env = Env::default();
    let (client, _admin, borrower, _token) = setup(&env);

    client.deposit_collateral(&borrower, &5000);
    client.withdraw_collateral(&borrower, &1000);

    let auths = env.auths();
    assert_eq!(auths.len(), 1);
    assert_eq!(auths[0].0, borrower);
    assert_eq!(client.get_collateral(&borrower), 4000);
}

#[test]
fn withdraw_collateral_rejects_zero_amount() {
    let env = Env::default();
    let (client, _admin, borrower, _token) = setup(&env);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.withdraw_collateral(&borrower, &0);
    }));
    assert!(result.is_err());
}

#[test]
fn withdraw_collateral_rejects_negative_amount() {
    let env = Env::default();
    let (client, _admin, borrower, _token) = setup(&env);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.withdraw_collateral(&borrower, &-100);
    }));
    assert!(result.is_err());
}

#[test]
fn partial_release_collateral_requires_borrower_auth() {
    let env = Env::default();
    let (client, _admin, borrower, _token) = setup(&env);

    client.deposit_collateral(&borrower, &5000);
    client.partial_release_collateral(&borrower, &1000);

    let auths = env.auths();
    assert_eq!(auths.len(), 1);
    assert_eq!(auths[0].0, borrower);
    assert_eq!(client.get_collateral(&borrower), 4000);
}

#[test]
fn partial_release_collateral_rejects_zero_amount() {
    let env = Env::default();
    let (client, _admin, borrower, _token) = setup(&env);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.partial_release_collateral(&borrower, &0);
    }));
    assert!(result.is_err());
}

#[test]
fn partial_release_collateral_rejects_negative_amount() {
    let env = Env::default();
    let (client, _admin, borrower, _token) = setup(&env);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.partial_release_collateral(&borrower, &-100);
    }));
    assert!(result.is_err());
}

// ─────────────────────────────────────────────────────────────────────────────
// Multi-token collateral entrypoint tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn deposit_collateral_token_requires_borrower_auth() {
    let env = Env::default();
    let (client, _admin, borrower, token) = setup(&env);

    let mut tokens = Vec::new(&env);
    tokens.push_back(token.clone());
    client.set_collateral_token_allowlist(&tokens);

    client.deposit_collateral_token(&borrower, &token, &1000);

    let auths = env.auths();
    assert_eq!(auths[0].0, borrower);
    assert_eq!(client.get_collateral_for_token(&borrower, &token), 1000);
}

#[test]
fn deposit_collateral_token_rejects_zero_amount() {
    let env = Env::default();
    let (client, _admin, borrower, token) = setup(&env);

    let mut tokens = Vec::new(&env);
    tokens.push_back(token.clone());
    client.set_collateral_token_allowlist(&tokens);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.deposit_collateral_token(&borrower, &token, &0);
    }));
    assert!(result.is_err());
}

#[test]
fn deposit_collateral_token_rejects_negative_amount() {
    let env = Env::default();
    let (client, _admin, borrower, token) = setup(&env);

    let mut tokens = Vec::new(&env);
    tokens.push_back(token.clone());
    client.set_collateral_token_allowlist(&tokens);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.deposit_collateral_token(&borrower, &token, &-100);
    }));
    assert!(result.is_err());
}

#[test]
fn withdraw_collateral_token_requires_borrower_auth() {
    let env = Env::default();
    let (client, _admin, borrower, token) = setup(&env);

    let mut tokens = Vec::new(&env);
    tokens.push_back(token.clone());
    client.set_collateral_token_allowlist(&tokens);

    client.deposit_collateral_token(&borrower, &token, &5000);
    client.withdraw_collateral_token(&borrower, &token, &1000);

    let auths = env.auths();
    assert_eq!(auths[0].0, borrower);
    assert_eq!(client.get_collateral_for_token(&borrower, &token), 4000);
}

#[test]
fn withdraw_collateral_token_rejects_zero_amount() {
    let env = Env::default();
    let (client, _admin, borrower, token) = setup(&env);

    let mut tokens = Vec::new(&env);
    tokens.push_back(token.clone());
    client.set_collateral_token_allowlist(&tokens);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.withdraw_collateral_token(&borrower, &token, &0);
    }));
    assert!(result.is_err());
}

#[test]
fn withdraw_collateral_token_rejects_negative_amount() {
    let env = Env::default();
    let (client, _admin, borrower, token) = setup(&env);

    let mut tokens = Vec::new(&env);
    tokens.push_back(token.clone());
    client.set_collateral_token_allowlist(&tokens);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.withdraw_collateral_token(&borrower, &token, &-100);
    }));
    assert!(result.is_err());
}

// ─────────────────────────────────────────────────────────────────────────────
// Admin-only collateral configuration entrypoint tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn set_min_collateral_ratio_bps_requires_admin_auth() {
    let env = Env::default();
    let (client, admin, _borrower, _token) = setup(&env);

    client.set_min_collateral_ratio_bps(&12_000_u32);

    let auths = env.auths();
    assert_eq!(auths.len(), 1);
    assert_eq!(auths[0].0, admin);
    assert_eq!(client.get_min_collateral_ratio_bps(), Some(12_000));
}

#[test]
fn set_collateral_risk_weight_requires_admin_auth() {
    let env = Env::default();
    let (client, admin, _borrower, _token) = setup(&env);
    let asset = Address::generate(&env);

    client.set_collateral_risk_weight(&asset, &5_000_u32);

    let auths = env.auths();
    assert_eq!(auths.len(), 1);
    assert_eq!(auths[0].0, admin);
}

#[test]
fn set_collateral_token_allowlist_requires_admin_auth() {
    let env = Env::default();
    let (client, admin, _borrower, token) = setup(&env);

    let mut tokens = Vec::new(&env);
    tokens.push_back(token);
    client.set_collateral_token_allowlist(&tokens);

    let auths = env.auths();
    assert_eq!(auths.len(), 1);
    assert_eq!(auths[0].0, admin);
}

#[test]
fn set_admin_collateral_cooldown_seconds_requires_admin_auth() {
    let env = Env::default();
    let (client, admin, _borrower, _token) = setup(&env);

    client.set_col_admin_cooldown_secs(&120_u64);

    let auths = env.auths();
    assert_eq!(auths.len(), 1);
    assert_eq!(auths[0].0, admin);
    assert_eq!(client.get_col_admin_cooldown_secs(), Some(120));
}

// ─────────────────────────────────────────────────────────────────────────────
// Boundary tests: Read-only functions should not require auth
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn get_collateral_does_not_require_auth() {
    let env = Env::default();
    let (client, _admin, borrower, _token) = setup(&env);

    client.deposit_collateral(&borrower, &1000);
    let balance = client.get_collateral(&borrower);
    assert_eq!(balance, 1000);
}

#[test]
fn get_collateral_for_token_does_not_require_auth() {
    let env = Env::default();
    let (client, _admin, borrower, token) = setup(&env);

    let mut tokens = Vec::new(&env);
    tokens.push_back(token.clone());
    client.set_collateral_token_allowlist(&tokens);

    client.deposit_collateral_token(&borrower, &token, &2000);
    let balance = client.get_collateral_for_token(&borrower, &token);
    assert_eq!(balance, 2000);
}

#[test]
fn get_admin_collateral_cooldown_seconds_does_not_require_auth() {
    let env = Env::default();
    let (client, _admin, _borrower, _token) = setup(&env);

    client.set_col_admin_cooldown_secs(&240_u64);
    let cooldown = client.get_col_admin_cooldown_secs();
    assert_eq!(cooldown, Some(240));
}

#[test]
fn get_last_admin_collateral_critical_action_ts_does_not_require_auth() {
    let env = Env::default();
    let (client, _admin, _borrower, _token) = setup(&env);

    client.set_min_collateral_ratio_bps(&13_000_u32);
    let ts = client.get_last_col_admin_action_ts();
    assert!(ts.is_some());
}
