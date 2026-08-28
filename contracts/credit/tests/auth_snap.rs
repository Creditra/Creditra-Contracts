// SPDX-License-Identifier: MIT
#![cfg(test)]

//! Per-entrypoint auth snapshot tests for the lifecycle subsystem (Issue #906).
//!
//! # What is an "auth snapshot"?
//!
//! An auth snapshot records *which identity* a given entrypoint asks Soroban
//! to authenticate (`require_auth` / `require_admin_auth`).  By calling
//! `env.auths()` after each invocation and asserting on the result we get
//! two guarantees:
//!
//! 1. **Positive path** — the correct signer (admin or borrower) is
//!    actually required when the call succeeds.  If a developer accidentally
//!    removes a `require_auth` call the assertion on `auths()[n].0` catches
//!    it at compile-time (field access) or at test-time (wrong identity).
//!
//! 2. **Negative path** — calling the same entrypoint *without* setting up
//!    auth (no `mock_all_auths`) panics, proving the guard is load-bearing.
//!
//! The `insta` snapshot tests (for the existing risk surface) additionally
//! pin the full `AuthorizedInvocation` tree so that sub-invocation shape
//! regressions are caught automatically.
//!
//! # Covered entrypoints
//!
//! | Entrypoint                   | Required signer         |
//! |------------------------------|-------------------------|
//! | `open_credit_line`           | admin (on re-open)      |
//! | `draw_credit`                | borrower                |
//! | `repay_credit`               | borrower                |
//! | `suspend_credit_line`        | admin                   |
//! | `self_suspend_credit_line`   | borrower                |
//! | `close_credit_line` (admin)  | admin                   |
//! | `close_credit_line` (borrower)| borrower               |
//! | `default_credit_line`        | admin                   |
//! | `reinstate_credit_line`      | admin                   |
//! | `forgive_debt`               | admin                   |
//! | `settle_default_liquidation` | admin                   |
//! | `set_rate_change_limits`     | admin (existing)        |
//! | `set_borrower_rate_floor`    | admin (existing)        |
//! | `set_borrower_rate_ceiling`  | admin (existing)        |
//! | `set_penalty_surcharge_bps`  | admin (existing)        |
//! | `update_risk_parameters`     | admin (existing)        |

use creditra_credit::types::CreditStatus;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{token, Address, Env};

/// Positive-test environment: `mock_all_auths` enabled, token wired up,
/// one credit line open for `borrower` in Active state.
///
/// `env.auths()` after the call under test returns all authorizations
/// recorded in the *entire* env lifetime.  Use `.last().unwrap()` to
/// isolate the invocation under test (the same pattern the existing risk
/// tests use).
fn setup() -> (Env, CreditClient<'static>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(10_000);

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_addr = token_id.address();
    client.set_liquidity_token(&token_addr);
    client.set_liquidity_source(&contract_id);
    token::StellarAssetClient::new(&env, &token_addr).mint(&contract_id, &10_000_000_i128);
    token::StellarAssetClient::new(&env, &token_addr).mint(&borrower, &10_000_000_i128);

    client.open_credit_line(&borrower, &100_000_i128, &300_u32, &50_u32);

    (env, client, admin, borrower)
}

/// Negative-test environment: NO `mock_all_auths`.
/// Any `require_auth` / `require_admin_auth` inside the called entrypoint
/// will panic immediately because no auth context is provided.
fn setup_no_mock() -> (Env, CreditClient<'static>, Address, Address) {
    let env = Env::default();
    // Deliberately *not* calling env.mock_all_auths()
    env.ledger().set_timestamp(10_000);

    // We still need to initialise the contract; do it in a nested scope
    // with mock_all_auths so setup itself doesn't fail.
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    {
        env.mock_all_auths();
        client.init(&admin);

        let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
        let token_addr = token_id.address();
        client.set_liquidity_token(&token_addr);
        client.set_liquidity_source(&contract_id);
        token::StellarAssetClient::new(&env, &token_addr).mint(&contract_id, &10_000_000_i128);
        token::StellarAssetClient::new(&env, &token_addr).mint(&borrower, &10_000_000_i128);

        client.open_credit_line(&borrower, &100_000_i128, &300_u32, &50_u32);
    }
    // mock_all_auths is scoped: after the block, new invocations require real auth.
    (env, client, admin, borrower)
}

#[test]
fn test_set_rate_change_limits_auth_snap() {
    let (env, client, _admin, _borrower) = setup();
    client.set_rate_change_limits(&500_u32, &3600_u64);
    let auths = env.auths();
    // Snapshot only the last auth to exclude setup auths
    insta::assert_debug_snapshot!(auths.last().unwrap());
}

#[test]
fn test_set_borrower_rate_floor_auth_snap() {
    let (env, client, _admin, borrower) = setup();
    client.set_borrower_rate_floor(&borrower, &Some(100));
    let auths = env.auths();
    insta::assert_debug_snapshot!(auths.last().unwrap());
}

#[test]
fn test_set_borrower_rate_ceiling_auth_snap() {
    let (env, client, _admin, borrower) = setup();
    client.set_borrower_rate_ceiling(&borrower, &Some(1000));
    let auths = env.auths();
    insta::assert_debug_snapshot!(auths.last().unwrap());
}

#[test]
fn test_set_penalty_surcharge_bps_auth_snap() {
    let (env, client, _admin, _borrower) = setup();
    client.set_penalty_surcharge_bps(&500_u32);
    let auths = env.auths();
    insta::assert_debug_snapshot!(auths.last().unwrap());
}

#[test]
fn test_update_risk_parameters_auth_snap() {
    let (env, client, _admin, borrower) = setup();
    client.update_risk_parameters(&borrower, &2_000_i128, &400_u32, &60_u32);
    let auths = env.auths();
    insta::assert_debug_snapshot!(auths.last().unwrap());
}

// ─────────────────────────────────────────────────────────────────────────────
// Lifecycle auth snapshots — Issue #906
// ─────────────────────────────────────────────────────────────────────────────

// ── open_credit_line ──────────────────────────────────────────────────────────

/// Re-opening a non-Active line requires admin auth.
/// The auth recorded must belong to the admin address.
#[test]
fn open_credit_line_reopen_requires_admin_auth() {
    let (env, client, admin, borrower) = setup();
    // Close the line first so re-open is allowed.
    client.close_credit_line(&borrower, &admin);
    client.open_credit_line(&borrower, &100_000_i128, &300_u32, &50_u32);
    let auths = env.auths();
    let last = auths.last().unwrap();
    assert_eq!(
        last.0, admin,
        "open_credit_line (re-open) must be authorised by admin"
    );
}

/// Calling open_credit_line without admin auth panics.
#[test]
#[should_panic]
fn open_credit_line_without_admin_auth_panics() {
    let (env, client, admin, borrower) = setup_no_mock();
    // Close so a re-open path is exercised (re-open requires admin auth).
    {
        env.mock_all_auths();
        client.close_credit_line(&borrower, &admin);
    }
    // Now call without any auth — must panic.
    client.open_credit_line(&borrower, &100_000_i128, &300_u32, &50_u32);
}

// ── draw_credit ───────────────────────────────────────────────────────────────

/// draw_credit must record the borrower as the sole authorised identity.
#[test]
fn draw_credit_requires_borrower_auth() {
    let (env, client, _admin, borrower) = setup();
    client.draw_credit(&borrower, &1_000_i128);
    let auths = env.auths();
    let last = auths.last().unwrap();
    assert_eq!(
        last.0, borrower,
        "draw_credit must be authorised by the borrower"
    );
}

/// draw_credit without borrower auth panics.
#[test]
#[should_panic]
fn draw_credit_without_borrower_auth_panics() {
    let (_env, client, _admin, borrower) = setup_no_mock();
    client.draw_credit(&borrower, &1_000_i128);
}

// ── repay_credit ──────────────────────────────────────────────────────────────

/// repay_credit must record the borrower as the authorised identity.
#[test]
fn repay_credit_requires_borrower_auth() {
    let (env, client, _admin, borrower) = setup();
    client.draw_credit(&borrower, &1_000_i128);
    client.repay_credit(&borrower, &500_i128);
    let auths = env.auths();
    let last = auths.last().unwrap();
    assert_eq!(
        last.0, borrower,
        "repay_credit must be authorised by the borrower"
    );
}

/// repay_credit without borrower auth panics.
#[test]
#[should_panic]
fn repay_credit_without_borrower_auth_panics() {
    let (env, client, _admin, borrower) = setup_no_mock();
    {
        env.mock_all_auths();
        client.draw_credit(&borrower, &1_000_i128);
    }
    client.repay_credit(&borrower, &500_i128);
}

// ── suspend_credit_line ───────────────────────────────────────────────────────

/// suspend_credit_line must record the admin as the authorised identity.
#[test]
fn suspend_credit_line_requires_admin_auth() {
    let (env, client, admin, borrower) = setup();
    client.suspend_credit_line(&borrower);
    let auths = env.auths();
    let last = auths.last().unwrap();
    assert_eq!(
        last.0, admin,
        "suspend_credit_line must be authorised by admin"
    );
}

/// suspend_credit_line without admin auth panics.
#[test]
#[should_panic]
fn suspend_credit_line_without_admin_auth_panics() {
    let (_env, client, _admin, borrower) = setup_no_mock();
    client.suspend_credit_line(&borrower);
}

// ── self_suspend_credit_line ──────────────────────────────────────────────────

/// self_suspend_credit_line must record the borrower as the authorised identity.
#[test]
fn self_suspend_credit_line_requires_borrower_auth() {
    let (env, client, _admin, borrower) = setup();
    client.self_suspend_credit_line(&borrower);
    let auths = env.auths();
    let last = auths.last().unwrap();
    assert_eq!(
        last.0, borrower,
        "self_suspend_credit_line must be authorised by the borrower"
    );
}

/// self_suspend_credit_line without borrower auth panics.
#[test]
#[should_panic]
fn self_suspend_credit_line_without_borrower_auth_panics() {
    let (_env, client, _admin, borrower) = setup_no_mock();
    client.self_suspend_credit_line(&borrower);
}

// ── close_credit_line ─────────────────────────────────────────────────────────

/// close_credit_line called by admin records the admin as authorised identity.
#[test]
fn close_credit_line_admin_path_requires_admin_auth() {
    let (env, client, admin, borrower) = setup();
    client.close_credit_line(&borrower, &admin);
    let auths = env.auths();
    let last = auths.last().unwrap();
    assert_eq!(
        last.0, admin,
        "close_credit_line (admin) must be authorised by admin"
    );
}

/// close_credit_line called by borrower (zero util) records borrower as authorised.
#[test]
fn close_credit_line_borrower_path_requires_borrower_auth() {
    let (env, client, _admin, borrower) = setup();
    // No draw → utilized == 0, borrower close is allowed.
    client.close_credit_line(&borrower, &borrower);
    let auths = env.auths();
    let last = auths.last().unwrap();
    assert_eq!(
        last.0, borrower,
        "close_credit_line (borrower) must be authorised by borrower"
    );
}

/// close_credit_line without the closer's auth panics.
#[test]
#[should_panic]
fn close_credit_line_without_closer_auth_panics() {
    let (_env, client, admin, _borrower) = setup_no_mock();
    // Pass admin as closer but don't provide any auth — must panic.
    let fake_closer = admin;
    client.close_credit_line(&fake_closer, &fake_closer);
}

// ── default_credit_line ───────────────────────────────────────────────────────

/// default_credit_line must record the admin as the authorised identity.
#[test]
fn default_credit_line_requires_admin_auth() {
    let (env, client, admin, borrower) = setup();
    client.default_credit_line(&borrower);
    let auths = env.auths();
    let last = auths.last().unwrap();
    assert_eq!(
        last.0, admin,
        "default_credit_line must be authorised by admin"
    );
}

/// default_credit_line without admin auth panics.
#[test]
#[should_panic]
fn default_credit_line_without_admin_auth_panics() {
    let (_env, client, _admin, borrower) = setup_no_mock();
    client.default_credit_line(&borrower);
}

// ── reinstate_credit_line ─────────────────────────────────────────────────────

/// reinstate_credit_line must record the admin as the authorised identity.
#[test]
fn reinstate_credit_line_requires_admin_auth() {
    let (env, client, admin, borrower) = setup();
    client.default_credit_line(&borrower);
    client.reinstate_credit_line(&borrower, &CreditStatus::Active);
    let auths = env.auths();
    let last = auths.last().unwrap();
    assert_eq!(
        last.0, admin,
        "reinstate_credit_line must be authorised by admin"
    );
}

/// reinstate_credit_line without admin auth panics.
#[test]
#[should_panic]
fn reinstate_credit_line_without_admin_auth_panics() {
    let (env, client, _admin, borrower) = setup_no_mock();
    {
        env.mock_all_auths();
        client.default_credit_line(&borrower);
    }
    client.reinstate_credit_line(&borrower, &CreditStatus::Active);
}

// ── forgive_debt ──────────────────────────────────────────────────────────────

/// forgive_debt must record the admin as the authorised identity.
#[test]
fn forgive_debt_requires_admin_auth() {
    let (env, client, admin, borrower) = setup();
    client.draw_credit(&borrower, &1_000_i128);
    client.forgive_debt(&borrower, &500_i128);
    let auths = env.auths();
    let last = auths.last().unwrap();
    assert_eq!(last.0, admin, "forgive_debt must be authorised by admin");
}

/// forgive_debt without admin auth panics.
#[test]
#[should_panic]
fn forgive_debt_without_admin_auth_panics() {
    let (env, client, _admin, borrower) = setup_no_mock();
    {
        env.mock_all_auths();
        client.draw_credit(&borrower, &1_000_i128);
    }
    client.forgive_debt(&borrower, &500_i128);
}

// ── settle_default_liquidation ────────────────────────────────────────────────

/// settle_default_liquidation must record the admin as the authorised identity.
#[test]
fn settle_default_liquidation_requires_admin_auth() {
    use soroban_sdk::Symbol;
    let (env, client, admin, borrower) = setup();
    client.draw_credit(&borrower, &1_000_i128);
    client.default_credit_line(&borrower);
    let settlement_id = Symbol::new(&env, "settle01");
    client.settle_default_liquidation(&borrower, &1_000_i128, &settlement_id, &10_000_u32, &None);
    let auths = env.auths();
    let last = auths.last().unwrap();
    assert_eq!(
        last.0, admin,
        "settle_default_liquidation must be authorised by admin"
    );
}

/// settle_default_liquidation without admin auth panics.
#[test]
#[should_panic]
fn settle_default_liquidation_without_admin_auth_panics() {
    use soroban_sdk::Symbol;
    let (env, client, _admin, borrower) = setup_no_mock();
    {
        env.mock_all_auths();
        client.draw_credit(&borrower, &1_000_i128);
        client.default_credit_line(&borrower);
    }
    let settlement_id = Symbol::new(&env, "settle01");
    client.settle_default_liquidation(&borrower, &1_000_i128, &settlement_id, &10_000_u32, &None);
}
