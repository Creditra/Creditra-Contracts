// SPDX-License-Identifier: MIT

//! Per-entrypoint authorization snapshot for the borrow (v7) surface.
//!
//! These tests pin the signer and authorization count recorded by Soroban for
//! every state-changing borrow entrypoint. Any change in who is required to
//! sign — or how many signers are required — will fail CI, alerting developers
//! to unintended auth-surface regressions.
//!
//! # Auth surface covered
//!
//! | Entrypoint | Required signer | Auths recorded |
//! |---|---|---|
//! | `draw_credit` | borrower | 1 |
//! | `repay_credit` | borrower | 1 |
//! | `reverse_draw` | admin | 1 |
//! | `set_draw_min_interval` | admin | 1 |
//! | `set_borrow_admin_cooldown` | admin | 1 |
//! | `set_utilization_cap` | admin | 1 |
//!
//! Read-only borrow entrypoints require no authorization:
//!
//! | Entrypoint | Auth required |
//! |---|---|
//! | `get_draw_min_interval` | none |
//! | `get_borrow_admin_cooldown` | none |
//! | `get_utilization_cap` | none |
//!
//! # See also
//!
//! - `contracts/credit/src/lib.rs` — borrow entrypoint implementations.
//! - `contracts/collateral/tests/auth_snap.rs` — collateral auth snapshot.
//! - `contracts/borrow/tests/err_stab.rs` — borrow error discriminant stability.

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger, MockAuth, MockAuthInvoke},
    token::StellarAssetClient,
    Address, Env, IntoVal,
};

const CREDIT_LIMIT: i128 = 10_000;
const DRAW_AMOUNT: i128 = 500;
const START_TS: u64 = 10_000;

// ── Fixture ────────────────────────────────────────────────────────────────

struct Fixture<'a> {
    client: CreditClient<'a>,
    admin: Address,
    borrower: Address,
    contract_id: Address,
    token: Address,
}

/// Deploy and configure the credit contract with a funded reserve and an open
/// credit line ready for draw/repay operations.
fn setup(env: &Env) -> Fixture<'_> {
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = START_TS);

    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    let token = env
        .register_stellar_asset_contract_v2(Address::generate(env))
        .address();
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&contract_id);

    // Fund the contract reserve so draw_credit can transfer tokens out.
    StellarAssetClient::new(env, &token).mint(&contract_id, &(CREDIT_LIMIT * 10));
    // Fund the borrower so repay_credit can pull tokens in.
    StellarAssetClient::new(env, &token).mint(&borrower, &(CREDIT_LIMIT * 2));
    soroban_sdk::token::Client::new(env, &token).approve(
        &borrower,
        &contract_id,
        &(CREDIT_LIMIT * 10),
        &100_000_u32,
    );

    client.open_credit_line(&borrower, &CREDIT_LIMIT, &300_u32, &50_u32);

    Fixture {
        client,
        admin,
        borrower,
        contract_id,
        token,
    }
}

// ── Helper ────────────────────────────────────────────────────────────────

/// Assert that the most recent call recorded exactly one auth and it belongs
/// to `signer`. Token sub-invocations may be present in the auth tree but
/// must not add a second top-level signer entry.
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

// ═══════════════════════════════════════════════════════════════════════════
// Borrower-signed entrypoints
// ═══════════════════════════════════════════════════════════════════════════

/// `draw_credit` must require exactly one authorization from the borrower.
#[test]
fn draw_credit_auth_snapshot() {
    let env = Env::default();
    let f = setup(&env);

    f.client.draw_credit(&f.borrower, &DRAW_AMOUNT);

    assert_single_auth(&env, &f.borrower, "draw_credit");
}

/// `repay_credit` must require exactly one authorization from the borrower.
#[test]
fn repay_credit_auth_snapshot() {
    let env = Env::default();
    let f = setup(&env);
    // Draw first so there is outstanding utilization to repay.
    f.client.draw_credit(&f.borrower, &DRAW_AMOUNT);

    f.client.repay_credit(&f.borrower, &DRAW_AMOUNT);

    assert_single_auth(&env, &f.borrower, "repay_credit");
}

/// `repay_credit` for a partial amount must still require exactly one
/// borrower auth (partial-repay code path).
#[test]
fn repay_credit_partial_auth_snapshot() {
    let env = Env::default();
    let f = setup(&env);
    f.client.draw_credit(&f.borrower, &DRAW_AMOUNT);

    f.client.repay_credit(&f.borrower, &(DRAW_AMOUNT / 2));

    assert_single_auth(&env, &f.borrower, "repay_credit (partial)");
}

// ═══════════════════════════════════════════════════════════════════════════
// Admin-signed entrypoints
// ═══════════════════════════════════════════════════════════════════════════

/// `reverse_draw` must require exactly one authorization from the admin.
///
/// We draw first so there is a recorded draw audit entry to reverse, then
/// verify that only the admin auth is captured for the reversal itself.
#[test]
fn reverse_draw_auth_snapshot() {
    let env = Env::default();
    let f = setup(&env);
    // Perform a draw to create an audit entry.
    f.client.draw_credit(&f.borrower, &DRAW_AMOUNT);
    let draw_ts = START_TS;

    // Advance time slightly — still within the reversal window.
    env.ledger().with_mut(|li| li.timestamp = START_TS + 60);

    f.client
        .reverse_draw(&f.borrower, &DRAW_AMOUNT, &draw_ts, &0_u32);

    assert_single_auth(&env, &f.admin, "reverse_draw");
}

/// `set_draw_min_interval` must require exactly one authorization from the admin.
#[test]
fn set_draw_min_interval_auth_snapshot() {
    let env = Env::default();
    let f = setup(&env);

    f.client.set_draw_min_interval(&300_u64);

    assert_single_auth(&env, &f.admin, "set_draw_min_interval");
}

/// `set_borrow_admin_cooldown` must require exactly one authorization from the admin.
#[test]
fn set_borrow_admin_cooldown_auth_snapshot() {
    let env = Env::default();
    let f = setup(&env);

    f.client.set_borrow_admin_cooldown(&3_600_u64);

    assert_single_auth(&env, &f.admin, "set_borrow_admin_cooldown");
}

/// `set_utilization_cap` must require exactly one authorization from the admin.
#[test]
fn set_utilization_cap_auth_snapshot() {
    let env = Env::default();
    let f = setup(&env);

    f.client.set_utilization_cap(&f.borrower, &8_000_u32);

    assert_single_auth(&env, &f.admin, "set_utilization_cap");
}

// ═══════════════════════════════════════════════════════════════════════════
// Read-only entrypoints — must require no authorization
// ═══════════════════════════════════════════════════════════════════════════

/// Read-only borrow query entrypoints must not require any authorization.
#[test]
fn borrow_queries_require_no_auth() {
    let env = Env::default();
    let f = setup(&env);

    // Populate some state first so queries have data to return.
    f.client.set_draw_min_interval(&120_u64);
    f.client.set_borrow_admin_cooldown(&3_600_u64);
    f.client.set_utilization_cap(&f.borrower, &7_500_u32);

    // ── get_draw_min_interval ──────────────────────────────────────────────
    let _ = f.client.get_draw_min_interval();
    assert!(
        env.auths().is_empty(),
        "get_draw_min_interval must be auth-free"
    );

    // ── get_borrow_admin_cooldown ──────────────────────────────────────────
    let _ = f.client.get_borrow_admin_cooldown();
    assert!(
        env.auths().is_empty(),
        "get_borrow_admin_cooldown must be auth-free"
    );

    // ── get_utilization_cap ────────────────────────────────────────────────
    let _ = f.client.get_utilization_cap(&f.borrower);
    assert!(
        env.auths().is_empty(),
        "get_utilization_cap must be auth-free"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Negative: wrong signer must not satisfy auth requirement
// ═══════════════════════════════════════════════════════════════════════════

/// A non-admin caller must not be able to authorize `set_draw_min_interval`.
///
/// Uses `mock_auths` with an attacker address to simulate a wrong signer;
/// the contract must reject the call.
#[test]
#[should_panic]
fn set_draw_min_interval_wrong_signer_reverts() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = START_TS);
    let attacker = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let admin = Address::generate(&env);
    // Init with mock_all_auths so setup succeeds, then strip mocks.
    env.mock_all_auths();
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    // Now attempt without admin auth — should panic.
    client
        .mock_auths(&[MockAuth {
            address: &attacker,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_draw_min_interval",
                args: (300_u64,).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .set_draw_min_interval(&300_u64);
}

/// A non-borrower must not be able to authorize `draw_credit` on behalf of
/// another borrower.
#[test]
#[should_panic]
fn draw_credit_wrong_signer_reverts() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = START_TS);
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let attacker = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    env.mock_all_auths();
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&contract_id);
    StellarAssetClient::new(&env, &token).mint(&contract_id, &(CREDIT_LIMIT * 10));
    client.open_credit_line(&borrower, &CREDIT_LIMIT, &300_u32, &50_u32);

    // Attempt draw with attacker signing for borrower — must panic.
    client
        .mock_auths(&[MockAuth {
            address: &attacker,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "draw_credit",
                args: (borrower.clone(), DRAW_AMOUNT).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .draw_credit(&borrower, &DRAW_AMOUNT);
}

/// A non-borrower must not be able to authorize `repay_credit` on behalf of
/// another borrower.
#[test]
#[should_panic]
fn repay_credit_wrong_signer_reverts() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = START_TS);
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let attacker = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    env.mock_all_auths();
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&contract_id);
    StellarAssetClient::new(&env, &token).mint(&contract_id, &(CREDIT_LIMIT * 10));
    StellarAssetClient::new(&env, &token).mint(&borrower, &CREDIT_LIMIT);
    client.open_credit_line(&borrower, &CREDIT_LIMIT, &300_u32, &50_u32);
    client.draw_credit(&borrower, &DRAW_AMOUNT);

    // Attempt repay with attacker signing for borrower — must panic.
    client
        .mock_auths(&[MockAuth {
            address: &attacker,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "repay_credit",
                args: (borrower.clone(), DRAW_AMOUNT).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .repay_credit(&borrower, &DRAW_AMOUNT);
}

/// A non-admin must not be able to authorize `set_borrow_admin_cooldown`.
#[test]
#[should_panic]
fn set_borrow_admin_cooldown_wrong_signer_reverts() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = START_TS);
    let attacker = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let admin = Address::generate(&env);
    env.mock_all_auths();
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    client
        .mock_auths(&[MockAuth {
            address: &attacker,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_borrow_admin_cooldown",
                args: (3_600_u64,).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .set_borrow_admin_cooldown(&3_600_u64);
}

/// A non-admin must not be able to authorize `set_utilization_cap`.
#[test]
#[should_panic]
fn set_utilization_cap_wrong_signer_reverts() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = START_TS);
    let attacker = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let admin = Address::generate(&env);
    env.mock_all_auths();
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    client.open_credit_line(&borrower, &CREDIT_LIMIT, &300_u32, &50_u32);

    client
        .mock_auths(&[MockAuth {
            address: &attacker,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_utilization_cap",
                args: (borrower.clone(), 8_000_u32).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .set_utilization_cap(&borrower, &8_000_u32);
}
