// SPDX-License-Identifier: MIT

//! Per-entrypoint auth boundary tests for the borrow subsystem (buffer2 #1).
//!
//! This file tests that every state-changing borrow entrypoint requires
//! proper authentication from the borrower. It verifies:
//! 1. Each entrypoint records exactly one authorization (the borrower)
//! 2. Each entrypoint reverts when called without authentication
//! 3. Each entrypoint reverts when called with a wrong signer
//! 4. Read-only borrow views require no authentication
//!
//! # Snapshot (borrow surface)
//!
//! | Entrypoint                    | Required signer | Auths recorded | Sub-invocations |
//! |-------------------------------|-----------------|----------------|------------------|
//! | `draw_credit`                 | borrower        | 1              | 1 (token transfer)|
//! | `repay_credit`                | borrower        | 1              | 1 (token transfer)|
//! | `repay_and_release_collateral`| borrower        | 1              | 2 (token + collateral)|
//! | `get_borrow_state`            | none (read-only)| 0             | —                |
//!
//! # Rules
//! - Never weaken an existing assertion (e.g. loosening `auths().len()`).
//! - If a borrow entrypoint gains a second required signer or a
//!   sub-invocation, update the table above alongside the assertion.
//!
//! # See also
//! - `creditra_credit::borrow` — the borrow/repay implementation.
//! - `contracts/credit/tests/freeze_auth_snap.rs` — similar auth boundary tests for freeze.

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{token, Address, Env, IntoVal};

/// Deploys a fresh contract, initializes `admin`, configures a token, and opens a credit line
/// for `borrower`, with `mock_all_auths` enabled for the whole env.
///
/// Because `mock_all_auths` still records what was authorized (it only
/// skips signature verification), `env.auths()` after the call under test
/// reflects exactly what that call — and nothing from setup — required.
fn setup_with_token(env: &Env) -> (CreditClient<'_>, Address, Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);
    client.set_liquidity_source(&contract_id);

    // Mint reserves to the contract and borrower
    token::StellarAssetClient::new(env, &token_address).mint(&contract_id, &1_000_000_i128);
    token::StellarAssetClient::new(env, &token_address).mint(&borrower, &1_000_000_i128);
    token::Client::new(env, &token_address).approve(
        &borrower,
        &contract_id,
        &1_000_000_i128,
        &100_000_u32,
    );

    client.open_credit_line(&borrower, &10_000_i128, &500_u32, &50_u32);

    (client, contract_id, admin, borrower)
}

/// Same as [`setup_with_token`] but *without* `mock_all_auths`, for negative tests.
fn setup_no_mock(env: &Env) -> (CreditClient<'_>, Address, Address, Address, Address) {
    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);
    client.set_liquidity_source(&contract_id);

    token::StellarAssetClient::new(env, &token_address).mint(&contract_id, &1_000_000_i128);

    client.open_credit_line(&borrower, &10_000_i128, &500_u32, &50_u32);

    (client, contract_id, token_address, admin, borrower)
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 1 — Positive snapshot: exactly one auth, held by borrower
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn draw_credit_auth_snapshot() {
    let env = Env::default();
    let (client, _contract_id, _admin, borrower) = setup_with_token(&env);

    client.draw_credit(&borrower, &1_000_i128);

    let auths = env.auths();
    assert_eq!(
        auths.len(),
        1,
        "draw_credit must record exactly one authorization"
    );
    assert_eq!(
        auths[0].0, borrower,
        "draw_credit must be authorized by the borrower"
    );
}

#[test]
fn repay_credit_auth_snapshot() {
    let env = Env::default();
    let (client, _contract_id, _admin, borrower) = setup_with_token(&env);
    client.draw_credit(&borrower, &1_000_i128);

    client.repay_credit(&borrower, &500_i128);

    let auths = env.auths();
    assert_eq!(
        auths.len(),
        1,
        "repay_credit must record exactly one authorization"
    );
    assert_eq!(
        auths[0].0, borrower,
        "repay_credit must be authorized by the borrower"
    );
}

#[test]
fn repay_and_release_collateral_auth_snapshot() {
    let env = Env::default();
    let (client, _contract_id, _admin, borrower) = setup_with_token(&env);

    // Deposit collateral first
    client.deposit_collateral(&borrower, &5_000_i128);

    // Draw credit
    client.draw_credit(&borrower, &1_000_i128);

    client.repay_and_release_collateral(&borrower, &500_i128);

    let auths = env.auths();
    assert_eq!(
        auths.len(),
        1,
        "repay_and_release_collateral must record exactly one authorization"
    );
    assert_eq!(
        auths[0].0, borrower,
        "repay_and_release_collateral must be authorized by the borrower"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 2 — Negative: entrypoint reverts with zero signers mocked
// ═══════════════════════════════════════════════════════════════════════════

#[test]
#[should_panic]
fn draw_credit_reverts_without_auth() {
    let env = Env::default();
    let (client, _contract_id, _token_address, _admin, borrower) = setup_no_mock(&env);
    client.draw_credit(&borrower, &1_000_i128);
}

#[test]
#[should_panic]
fn repay_credit_reverts_without_auth() {
    let env = Env::default();
    let (client, _contract_id, _token_address, _admin, borrower) = setup_no_mock(&env);
    client.repay_credit(&borrower, &500_i128);
}

#[test]
#[should_panic]
fn repay_and_release_collateral_reverts_without_auth() {
    let env = Env::default();
    let (client, _contract_id, _token_address, _admin, borrower) = setup_no_mock(&env);
    client.repay_and_release_collateral(&borrower, &500_i128);
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 3 — Edge case: a non-borrower signer is rejected, not just "no signer"
// ═══════════════════════════════════════════════════════════════════════════

#[test]
#[should_panic]
fn draw_credit_wrong_signer_reverts() {
    let env = Env::default();
    let (client, contract_id, _token_address, _admin, borrower) = setup_no_mock(&env);
    let attacker = Address::generate(&env);

    client
        .mock_auths(&[MockAuth {
            address: &attacker,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "draw_credit",
                args: (borrower.clone(), 1_000_i128).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .draw_credit(&borrower, &1_000_i128);
}

#[test]
#[should_panic]
fn repay_credit_wrong_signer_reverts() {
    let env = Env::default();
    let (client, contract_id, _token_address, _admin, borrower) = setup_no_mock(&env);
    let attacker = Address::generate(&env);

    client
        .mock_auths(&[MockAuth {
            address: &attacker,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "repay_credit",
                args: (borrower.clone(), 500_i128).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .repay_credit(&borrower, &500_i128);
}

#[test]
#[should_panic]
fn repay_and_release_collateral_wrong_signer_reverts() {
    let env = Env::default();
    let (client, contract_id, _token_address, _admin, borrower) = setup_no_mock(&env);
    let attacker = Address::generate(&env);

    client
        .mock_auths(&[MockAuth {
            address: &attacker,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "repay_and_release_collateral",
                args: (borrower.clone(), 500_i128).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .repay_and_release_collateral(&borrower, &500_i128);
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 4 — Edge case: read-only borrow queries require no authorization
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn get_borrow_state_requires_no_auth() {
    let env = Env::default();
    let (client, _contract_id, _admin, borrower) = setup_with_token(&env);

    let _ = client.get_credit_line(&borrower);

    assert!(
        env.auths().is_empty(),
        "get_credit_line must not require any authorization"
    );
}
