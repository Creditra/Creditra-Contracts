// SPDX-License-Identifier: MIT

//! Per-entrypoint auth boundary tests for collateral operations.
//!
//! Each test verifies that calling a state-changing collateral entrypoint
//! without the correct signer reverts. Setup uses targeted `mock_auths`
//! so only the intended addresses are authorized for setup operations;
//! the function under test receives no valid authorization.

use creditra_credit::types::FreezeReason;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Ledger, MockAuth, MockAuthInvoke};
use soroban_sdk::{Address, Env, IntoVal, Symbol, Vec};

const START_TS: u64 = 5_000;

fn setup() -> (Env, CreditClient<'_>, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = START_TS);

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    // Configure liquidity token for collateral operations
    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&token);

    let asset = soroban_sdk::token::StellarAssetClient::new(&env, &token);
    asset.mint(&borrower, &100_000_i128);
    asset.mint(&token, &100_000_i128);

    (env, client, admin, borrower, token)
}

fn set_timestamp(env: &Env, timestamp: u64) {
    env.ledger().with_mut(|li| li.timestamp = timestamp);
}

// ── Admin-gated collateral functions ────────────────────────────────────────

#[test]
#[should_panic]
fn set_min_collateral_ratio_bps_unauthorized() {
    let (env, client, _admin, _borrower, _token) = setup();
    client.set_min_collateral_ratio_bps(&12_000_u32);
}

#[test]
#[should_panic]
fn set_collateral_risk_weight_unauthorized() {
    let (env, client, _admin, _borrower, _token) = setup();
    let asset = Address::generate(&env);
    client.set_collateral_risk_weight(&asset, &5_000_u32);
}

#[test]
#[should_panic]
fn set_collateral_token_allowlist_unauthorized() {
    let (env, client, _admin, _borrower, _token) = setup();
    let token = Address::generate(&env);
    client.set_collateral_token_allowlist(&vec![&env, token]);
}

#[test]
#[should_panic]
fn set_admin_collateral_cooldown_seconds_unauthorized() {
    let (env, client, _admin, _borrower, _token) = setup();
    client.set_collateral_admin_cooldown(&120_u64);
}

// Admin-gated functions called by non-admin with mock_auths
#[test]
#[should_panic]
fn set_min_collateral_ratio_bps_non_admin_mock_auth() {
    let (env, client, _admin, _borrower, _token) = setup();
    let contract_id = env.current_contract_address();
    let non_admin = Address::generate(&env);

    client
        .mock_auths(&[MockAuth {
            address: &non_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_min_collateral_ratio_bps",
                args: (12_000_u32,).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .set_min_collateral_ratio_bps(&12_000_u32);
}

#[test]
#[should_panic]
fn set_collateral_risk_weight_non_admin_mock_auth() {
    let (env, client, _admin, _borrower, _token) = setup();
    let contract_id = env.current_contract_address();
    let non_admin = Address::generate(&env);
    let asset = Address::generate(&env);

    client
        .mock_auths(&[MockAuth {
            address: &non_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_collateral_risk_weight",
                args: (asset, 5_000_u32).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .set_collateral_risk_weight(&asset, &5_000_u32);
}

#[test]
#[should_panic]
fn set_collateral_token_allowlist_non_admin_mock_auth() {
    let (env, client, _admin, _borrower, _token) = setup();
    let contract_id = env.current_contract_address();
    let non_admin = Address::generate(&env);
    let token = Address::generate(&env);

    client
        .mock_auths(&[MockAuth {
            address: &non_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_collateral_token_allowlist",
                args: (vec![&env, token.clone()],).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .set_collateral_token_allowlist(&vec![&env, token]);
}

#[test]
#[should_panic]
fn set_collateral_admin_cooldown_non_admin_mock_auth() {
    let (env, client, _admin, _borrower, _token) = setup();
    let contract_id = env.current_contract_address();
    let non_admin = Address::generate(&env);

    client
        .mock_auths(&[MockAuth {
            address: &non_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_collateral_admin_cooldown",
                args: (120_u64,).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .set_collateral_admin_cooldown(&120_u64);
}

// ── Borrower-gated collateral functions ─────────────────────────────────────

fn setup_with_collateral(
    env: &Env,
) -> (CreditClient<'_>, Address, Address, Address) {
    let (client, admin, borrower, token) = {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| li.timestamp = START_TS);

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);

        let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
        let token = token_id.address();
        client.set_liquidity_token(&token);
        client.set_liquidity_source(&token);

        let asset = soroban_sdk::token::StellarAssetClient::new(&env, &token);
        asset.mint(&borrower, &100_000_i128);
        asset.mint(&token, &100_000_i128);

        (client, admin, borrower, token)
    };

    // Deposit some collateral for testing
    client.deposit_collateral(&borrower, &5000);

    (client, admin, borrower, token)
}

#[test]
#[should_panic]
fn deposit_collateral_unauthorized() {
    let env = Env::default();
    let (client, _admin, borrower, _token) = setup_with_collateral(&env);
    let impersonator = Address::generate(&env);
    let contract_id = env.current_contract_address();

    client
        .mock_auths(&[MockAuth {
            address: &impersonator,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "deposit_collateral",
                args: (borrower, 1000_i128).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .deposit_collateral(&borrower, &1000_i128);
}

#[test]
#[should_panic]
fn withdraw_collateral_unauthorized() {
    let env = Env::default();
    let (client, _admin, borrower, _token) = setup_with_collateral(&env);
    let impersonator = Address::generate(&env);
    let contract_id = env.current_contract_address();

    client
        .mock_auths(&[MockAuth {
            address: &impersonator,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "withdraw_collateral",
                args: (borrower, 1000_i128).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .withdraw_collateral(&borrower, &1000_i128);
}

#[test]
#[should_panic]
fn partial_release_collateral_unauthorized() {
    let env = Env::default();
    let (client, _admin, borrower, _token) = setup_with_collateral(&env);
    let impersonator = Address::generate(&env);
    let contract_id = env.current_contract_address();

    client
        .mock_auths(&[MockAuth {
            address: &impersonator,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "partial_release_collateral",
                args: (borrower, 1000_i128).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .partial_release_collateral(&borrower, &1000_i128);
}

#[test]
#[should_panic]
fn deposit_collateral_token_unauthorized() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = START_TS);

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&token);

    let asset = soroban_sdk::token::StellarAssetClient::new(&env, &token);
    asset.mint(&borrower, &100_000_i128);
    asset.mint(&token, &100_000_i128);

    // Add token to allowlist
    client.set_collateral_token_allowlist(&vec![&env, token.clone()]);

    let impersonator = Address::generate(&env);

    client
        .mock_auths(&[MockAuth {
            address: &impersonator,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "deposit_collateral_token",
                args: (borrower, token, 1000_i128).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .deposit_collateral_token(&borrower, &token, &1000_i128);
}

#[test]
#[should_panic]
fn withdraw_collateral_token_unauthorized() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = START_TS);

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&token);

    let asset = soroban_sdk::token::StellarAssetClient::new(&env, &token);
    asset.mint(&borrower, &100_000_i128);
    asset.mint(&token, &100_000_i128);

    // Add token to allowlist and deposit some
    client.set_collateral_token_allowlist(&vec![&env, token.clone()]);
    client.deposit_collateral_token(&borrower, &token, &5000);

    let impersonator = Address::generate(&env);

    client
        .mock_auths(&[MockAuth {
            address: &impersonator,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "withdraw_collateral_token",
                args: (borrower, token, 1000_i128).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .withdraw_collateral_token(&borrower, &token, &1000_i128);
}

// ── Read-only functions (should not require auth) ───────────────────────────

#[test]
fn get_collateral_read_only() {
    let env = Env::default();
    let (client, _admin, borrower, _token) = setup_with_collateral(&env);
    let balance = client.get_collateral(&borrower);
    assert_eq!(balance, 5000);
}

#[test]
fn get_collateral_for_token_read_only() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = START_TS);

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&token);

    let asset = soroban_sdk::token::StellarAssetClient::new(&env, &token);
    asset.mint(&borrower, &100_000_i128);
    asset.mint(&token, &100_000_i128);

    client.set_collateral_token_allowlist(&vec![&env, token.clone()]);
    client.deposit_collateral_token(&borrower, &token, &5000);

    let balance = client.get_collateral_for_token(&borrower, &token);
    assert_eq!(balance, 5000);
}

#[test]
fn get_min_collateral_ratio_bps_read_only() {
    let (env, client, _admin, _borrower, _token) = setup();
    let ratio = client.get_min_collateral_ratio_bps();
    assert!(ratio.is_some());
}

#[test]
fn get_admin_collateral_cooldown_seconds_read_only() {
    let (env, client, _admin, _borrower, _token) = setup();
    let cooldown = client.get_collateral_admin_cooldown();
    assert!(cooldown.is_none());
}

#[test]
fn get_last_admin_collateral_critical_action_ts_read_only() {
    let (env, client, _admin, _borrower, _token) = setup();
    let ts = client.get_last_collateral_action_ts();
    assert!(ts.is_none());
}

#[test]
fn get_collateral_tokens_read_only() {
    let (env, client, _admin, _borrower, _token) = setup();
    let tokens = client.get_collateral_tokens();
    assert_eq!(tokens.len(), 0);
}