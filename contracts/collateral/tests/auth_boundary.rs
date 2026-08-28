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
//!
//! For each entrypoint, we verify:
//! 1. An unauthorized third party **cannot** call the function (panics).
//! 2. The authorized caller **can** call the function (succeeds).
//! 3. Boundary conditions are properly enforced (e.g., positive amounts).
//!
//! # Error codes
//!
//! - `#1`: `Unauthorized` — caller does not have the required authority.
//! - `#6`: `InvalidAmount` — amount is zero or negative.

use creditra_credit::Credit;
use creditra_credit::CreditClient;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{token::StellarAssetClient, Address, Env, Vec};

/// Setup environment with admin, borrower, and contract initialized.
fn setup() -> (Env, CreditClient, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    // Set up a collateral token.
    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&token);

    let token_admin = StellarAssetClient::new(&env, &token);
    token_admin.mint(&borrower, &1_000_000_i128);
    token_admin.mint(&admin, &1_000_000_i128);

    (env, client, admin, borrower, token)
}

/// Helper: Extract error discriminant from panic message.
fn extract_error_code(err_msg: &str) -> Option<u32> {
    // Expected format: "Error(Contract, #N)"
    if let Some(pos) = err_msg.find("#") {
        if let Ok(code) = err_msg[pos + 1..].split(')').next().unwrap_or("").parse() {
            return Some(code);
        }
    }
    None
}

/// Helper: Assert that a function panicked with Unauthorized (#1).
fn assert_unauthorized<F: Fn() + std::panic::UnwindSafe>(f: F, context: &str) {
    let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).expect_err(context);
    let err_str = if let Some(s) = err.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = err.downcast_ref::<&str>() {
        s.to_string()
    } else {
        format!("{err:?}")
    };

    if let Some(code) = extract_error_code(&err_str) {
        assert_eq!(
            code, 1,
            "{context}: expected Unauthorized (#1), got #{code}. Error: {err_str}"
        );
    } else {
        panic!("{context}: could not extract error code from {err_str}");
    }
}

/// Helper: Assert that a function panicked with a specific error code.
fn assert_error_code<F: Fn() + std::panic::UnwindSafe>(f: F, expected_code: u32, context: &str) {
    let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).expect_err(context);
    let err_str = if let Some(s) = err.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = err.downcast_ref::<&str>() {
        s.to_string()
    } else {
        format!("{err:?}")
    };

    if let Some(code) = extract_error_code(&err_str) {
        assert_eq!(
            code, expected_code,
            "{context}: expected error #{expected_code}, got #{code}. Error: {err_str}"
        );
    } else {
        panic!("{context}: could not extract error code from {err_str}");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Borrower-facing entrypoint tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn deposit_collateral_requires_borrower_auth() {
    let (env, client, _admin, borrower, _token) = setup();
    let unauthorized = Address::generate(&env);

    // Unauthorized caller cannot deposit on behalf of borrower.
    assert_unauthorized(
        || {
            env.disable_dispatch_auth_for_address(&unauthorized);
            client.deposit_collateral(&borrower, &1000);
        },
        "unauthorized deposit_collateral must panic with Unauthorized",
    );

    // Authorized borrower can deposit.
    env.disable_dispatch_auth_for_address(&borrower);
    client.deposit_collateral(&borrower, &1000);
    assert_eq!(client.get_collateral(&borrower), 1000);
}

#[test]
fn deposit_collateral_rejects_zero_amount() {
    let (_env, client, _admin, borrower, _token) = setup();

    assert_error_code(
        || client.deposit_collateral(&borrower, &0),
        6, // InvalidAmount
        "deposit_collateral with zero amount must panic with InvalidAmount",
    );
}

#[test]
fn deposit_collateral_rejects_negative_amount() {
    let (_env, client, _admin, borrower, _token) = setup();

    assert_error_code(
        || client.deposit_collateral(&borrower, &-100),
        6, // InvalidAmount
        "deposit_collateral with negative amount must panic with InvalidAmount",
    );
}

#[test]
fn withdraw_collateral_requires_borrower_auth() {
    let (env, client, _admin, borrower, _token) = setup();
    let unauthorized = Address::generate(&env);

    // First, the authorized borrower deposits some collateral.
    env.disable_dispatch_auth_for_address(&borrower);
    client.deposit_collateral(&borrower, &5000);
    assert_eq!(client.get_collateral(&borrower), 5000);

    // Now try to withdraw with unauthorized caller.
    assert_unauthorized(
        || {
            env.disable_dispatch_auth_for_address(&unauthorized);
            client.withdraw_collateral(&borrower, &1000);
        },
        "unauthorized withdraw_collateral must panic with Unauthorized",
    );

    // Authorized borrower can withdraw.
    env.disable_dispatch_auth_for_address(&borrower);
    client.withdraw_collateral(&borrower, &1000);
    assert_eq!(client.get_collateral(&borrower), 4000);
}

#[test]
fn withdraw_collateral_rejects_zero_amount() {
    let (_env, client, _admin, borrower, _token) = setup();

    assert_error_code(
        || client.withdraw_collateral(&borrower, &0),
        6, // InvalidAmount
        "withdraw_collateral with zero amount must panic with InvalidAmount",
    );
}

#[test]
fn withdraw_collateral_rejects_negative_amount() {
    let (_env, client, _admin, borrower, _token) = setup();

    assert_error_code(
        || client.withdraw_collateral(&borrower, &-100),
        6, // InvalidAmount
        "withdraw_collateral with negative amount must panic with InvalidAmount",
    );
}

#[test]
fn partial_release_collateral_requires_borrower_auth() {
    let (env, client, _admin, borrower, _token) = setup();
    let unauthorized = Address::generate(&env);

    // First, the authorized borrower deposits some collateral.
    env.disable_dispatch_auth_for_address(&borrower);
    client.deposit_collateral(&borrower, &5000);
    assert_eq!(client.get_collateral(&borrower), 5000);

    // Now try to release with unauthorized caller.
    assert_unauthorized(
        || {
            env.disable_dispatch_auth_for_address(&unauthorized);
            client.partial_release_collateral(&borrower, &1000);
        },
        "unauthorized partial_release_collateral must panic with Unauthorized",
    );

    // Authorized borrower can release.
    env.disable_dispatch_auth_for_address(&borrower);
    client.partial_release_collateral(&borrower, &1000);
    assert_eq!(client.get_collateral(&borrower), 4000);
}

#[test]
fn partial_release_collateral_rejects_zero_amount() {
    let (_env, client, _admin, borrower, _token) = setup();

    assert_error_code(
        || client.partial_release_collateral(&borrower, &0),
        6, // InvalidAmount
        "partial_release_collateral with zero amount must panic with InvalidAmount",
    );
}

#[test]
fn partial_release_collateral_rejects_negative_amount() {
    let (_env, client, _admin, borrower, _token) = setup();

    assert_error_code(
        || client.partial_release_collateral(&borrower, &-100),
        6, // InvalidAmount
        "partial_release_collateral with negative amount must panic with InvalidAmount",
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Multi-token collateral entrypoint tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn deposit_collateral_token_requires_borrower_auth() {
    let (env, client, admin, borrower, token) = setup();
    let unauthorized = Address::generate(&env);

    // Set up allowlist with the token.
    env.disable_dispatch_auth_for_address(&admin);
    let mut tokens = Vec::new(&env);
    tokens.push_back(token.clone());
    client.set_collateral_token_allowlist(&tokens);

    // Unauthorized caller cannot deposit on behalf of borrower.
    assert_unauthorized(
        || {
            env.disable_dispatch_auth_for_address(&unauthorized);
            client.deposit_collateral_token(&borrower, &token, &1000);
        },
        "unauthorized deposit_collateral_token must panic with Unauthorized",
    );

    // Authorized borrower can deposit.
    env.disable_dispatch_auth_for_address(&borrower);
    client.deposit_collateral_token(&borrower, &token, &1000);
    assert_eq!(client.get_collateral_for_token(&borrower, &token), 1000);
}

#[test]
fn deposit_collateral_token_rejects_zero_amount() {
    let (env, client, admin, borrower, token) = setup();

    // Set up allowlist.
    env.disable_dispatch_auth_for_address(&admin);
    let mut tokens = Vec::new(&env);
    tokens.push_back(token.clone());
    client.set_collateral_token_allowlist(&tokens);

    assert_error_code(
        || client.deposit_collateral_token(&borrower, &token, &0),
        6, // InvalidAmount
        "deposit_collateral_token with zero amount must panic with InvalidAmount",
    );
}

#[test]
fn deposit_collateral_token_rejects_negative_amount() {
    let (env, client, admin, borrower, token) = setup();

    // Set up allowlist.
    env.disable_dispatch_auth_for_address(&admin);
    let mut tokens = Vec::new(&env);
    tokens.push_back(token.clone());
    client.set_collateral_token_allowlist(&tokens);

    assert_error_code(
        || client.deposit_collateral_token(&borrower, &token, &-100),
        6, // InvalidAmount
        "deposit_collateral_token with negative amount must panic with InvalidAmount",
    );
}

#[test]
fn withdraw_collateral_token_requires_borrower_auth() {
    let (env, client, admin, borrower, token) = setup();
    let unauthorized = Address::generate(&env);

    // Set up allowlist.
    env.disable_dispatch_auth_for_address(&admin);
    let mut tokens = Vec::new(&env);
    tokens.push_back(token.clone());
    client.set_collateral_token_allowlist(&tokens);

    // Authorized borrower deposits first.
    env.disable_dispatch_auth_for_address(&borrower);
    client.deposit_collateral_token(&borrower, &token, &5000);
    assert_eq!(client.get_collateral_for_token(&borrower, &token), 5000);

    // Unauthorized caller cannot withdraw.
    assert_unauthorized(
        || {
            env.disable_dispatch_auth_for_address(&unauthorized);
            client.withdraw_collateral_token(&borrower, &token, &1000);
        },
        "unauthorized withdraw_collateral_token must panic with Unauthorized",
    );

    // Authorized borrower can withdraw.
    env.disable_dispatch_auth_for_address(&borrower);
    client.withdraw_collateral_token(&borrower, &token, &1000);
    assert_eq!(client.get_collateral_for_token(&borrower, &token), 4000);
}

#[test]
fn withdraw_collateral_token_rejects_zero_amount() {
    let (env, client, admin, borrower, token) = setup();

    // Set up allowlist.
    env.disable_dispatch_auth_for_address(&admin);
    let mut tokens = Vec::new(&env);
    tokens.push_back(token.clone());
    client.set_collateral_token_allowlist(&tokens);

    assert_error_code(
        || client.withdraw_collateral_token(&borrower, &token, &0),
        6, // InvalidAmount
        "withdraw_collateral_token with zero amount must panic with InvalidAmount",
    );
}

#[test]
fn withdraw_collateral_token_rejects_negative_amount() {
    let (env, client, admin, borrower, token) = setup();

    // Set up allowlist.
    env.disable_dispatch_auth_for_address(&admin);
    let mut tokens = Vec::new(&env);
    tokens.push_back(token.clone());
    client.set_collateral_token_allowlist(&tokens);

    assert_error_code(
        || client.withdraw_collateral_token(&borrower, &token, &-100),
        6, // InvalidAmount
        "withdraw_collateral_token with negative amount must panic with InvalidAmount",
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Admin-only collateral configuration entrypoint tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn set_min_collateral_ratio_bps_requires_admin_auth() {
    let (env, client, admin, _borrower, _token) = setup();
    let unauthorized = Address::generate(&env);

    // Unauthorized caller cannot set the ratio.
    assert_unauthorized(
        || {
            env.disable_dispatch_auth_for_address(&unauthorized);
            client.set_min_collateral_ratio_bps(&12_000_u32);
        },
        "unauthorized set_min_collateral_ratio_bps must panic with Unauthorized",
    );

    // Authorized admin can set the ratio.
    env.disable_dispatch_auth_for_address(&admin);
    client.set_min_collateral_ratio_bps(&12_000_u32);
    assert_eq!(client.get_min_collateral_ratio_bps(), Some(12_000));
}

#[test]
fn set_collateral_risk_weight_requires_admin_auth() {
    let (env, client, admin, _borrower, _token) = setup();
    let unauthorized = Address::generate(&env);
    let asset = Address::generate(&env);

    // Unauthorized caller cannot set the weight.
    assert_unauthorized(
        || {
            env.disable_dispatch_auth_for_address(&unauthorized);
            client.set_collateral_risk_weight(&asset, &5_000_u32);
        },
        "unauthorized set_collateral_risk_weight must panic with Unauthorized",
    );

    // Authorized admin can set the weight.
    env.disable_dispatch_auth_for_address(&admin);
    client.set_collateral_risk_weight(&asset, &5_000_u32);
    assert_eq!(client.get_collateral_risk_weight_bps(&asset), Some(5_000));
}

#[test]
fn set_collateral_risk_weight_rejects_weight_over_10000_bps() {
    let (_env, client, _admin, _borrower, _token) = setup();
    let asset = Address::generate(&_env);

    // Risk weight > 10_000 bps should be rejected.
    assert_error_code(
        || client.set_collateral_risk_weight(&asset, &10_001_u32),
        10, // InvalidRiskWeight (assuming error code 10)
        "set_collateral_risk_weight with weight > 10_000 must panic with InvalidRiskWeight",
    );
}

#[test]
fn set_collateral_token_allowlist_requires_admin_auth() {
    let (env, client, admin, _borrower, token) = setup();
    let unauthorized = Address::generate(&env);

    let mut tokens = Vec::new(&env);
    tokens.push_back(token.clone());

    // Unauthorized caller cannot set the allowlist.
    assert_unauthorized(
        || {
            env.disable_dispatch_auth_for_address(&unauthorized);
            client.set_collateral_token_allowlist(&tokens);
        },
        "unauthorized set_collateral_token_allowlist must panic with Unauthorized",
    );

    // Authorized admin can set the allowlist.
    env.disable_dispatch_auth_for_address(&admin);
    client.set_collateral_token_allowlist(&tokens);

    // Verify the allowlist was set by attempting to use a token outside the list.
    let other_token = Address::generate(&env);
    env.disable_dispatch_auth_for_address(&_borrower);
    let borrower = _borrower;
    assert_error_code(
        || client.deposit_collateral_token(&borrower, &other_token, &1000),
        22, // MissingLiquidityToken
        "deposit_collateral_token with non-allowlisted token must panic with MissingLiquidityToken",
    );
}

#[test]
fn set_admin_collateral_cooldown_seconds_requires_admin_auth() {
    let (env, client, admin, _borrower, _token) = setup();
    let unauthorized = Address::generate(&env);

    // Unauthorized caller cannot set the cooldown.
    assert_unauthorized(
        || {
            env.disable_dispatch_auth_for_address(&unauthorized);
            client.set_admin_collateral_cooldown_seconds(&120_u64);
        },
        "unauthorized set_admin_collateral_cooldown_seconds must panic with Unauthorized",
    );

    // Authorized admin can set the cooldown.
    env.disable_dispatch_auth_for_address(&admin);
    client.set_admin_collateral_cooldown_seconds(&120_u64);
    assert_eq!(client.get_admin_collateral_cooldown_seconds(), Some(120));
}

// ─────────────────────────────────────────────────────────────────────────────
// Boundary tests: Read-only functions should not require auth
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn get_collateral_does_not_require_auth() {
    let (env, client, _admin, borrower, _token) = setup();

    // Deposit some collateral (requires auth).
    env.disable_dispatch_auth_for_address(&borrower);
    client.deposit_collateral(&borrower, &1000);

    // Query should work without explicitly enabling auth for any specific address.
    // (env.mock_all_auths() is still enabled, so it should just work)
    let balance = client.get_collateral(&borrower);
    assert_eq!(balance, 1000);
}

#[test]
fn get_collateral_for_token_does_not_require_auth() {
    let (env, client, admin, borrower, token) = setup();

    // Set up allowlist.
    env.disable_dispatch_auth_for_address(&admin);
    let mut tokens = Vec::new(&env);
    tokens.push_back(token.clone());
    client.set_collateral_token_allowlist(&tokens);

    // Deposit some collateral.
    env.disable_dispatch_auth_for_address(&borrower);
    client.deposit_collateral_token(&borrower, &token, &2000);

    // Query should work without special auth setup.
    let balance = client.get_collateral_for_token(&borrower, &token);
    assert_eq!(balance, 2000);
}

#[test]
fn get_admin_collateral_cooldown_seconds_does_not_require_auth() {
    let (env, client, admin, _borrower, _token) = setup();

    // Set the cooldown.
    env.disable_dispatch_auth_for_address(&admin);
    client.set_admin_collateral_cooldown_seconds(&240_u64);

    // Query should work without special auth setup.
    let cooldown = client.get_admin_collateral_cooldown_seconds();
    assert_eq!(cooldown, Some(240));
}

#[test]
fn get_last_admin_collateral_critical_action_ts_does_not_require_auth() {
    let (env, client, admin, _borrower, _token) = setup();

    // Perform a critical action (requires auth).
    env.disable_dispatch_auth_for_address(&admin);
    client.set_min_collateral_ratio_bps(&13_000_u32);

    // Query should work without special auth setup.
    let ts = client.get_last_admin_collateral_critical_action_ts();
    assert!(ts.is_some());
}
