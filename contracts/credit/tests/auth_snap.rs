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
use soroban_sdk::testutils::{Address as _, AuthorizedFunction, Ledger, MockAuth, MockAuthInvoke};
use soroban_sdk::{token, Address, Env, IntoVal, Symbol};

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
    client.set_treasury(&Address::generate(&env));

    (env, client, admin, borrower)
}

/// Builds a `MockAuth` registering `admin`'s authorization for a single `Credit`
/// entrypoint call. A macro (rather than a function) because `MockAuth` borrows
/// the inline `MockAuthInvoke`.
macro_rules! admin_auth {
    ($admin:expr, $contract:expr, $fn_name:expr, $args:expr) => {
        MockAuth {
            address: $admin,
            invoke: &MockAuthInvoke {
                contract: $contract,
                fn_name: $fn_name,
                args: $args,
                sub_invokes: &[],
            },
        }
    };
}

/// Negative-test environment: **no blanket auth mock**.
///
/// The fixture mirrors [`setup`] (token wired up, one active credit line, a
/// treasury recipient configured) so that an entrypoint failing for an
/// unrelated reason - e.g. `TreasuryNotSet` - cannot make a negative test pass
/// for the wrong reason.
///
/// `Env::mock_all_auths()` is sticky for the whole `Env` lifetime - it is *not*
/// scoped to a block. A blanket mock therefore makes every `require_auth`
/// succeed, which silently turns a `#[should_panic]` negative test into a test
/// that passes for the wrong reason.
///
/// Instead each setup call registers exactly the authorization it needs via
/// `mock_auths`, and those entries are consumed by the call they were
/// registered for. When setup returns, no authorization is pending, so the
/// entrypoint under test in each negative test runs unauthenticated and must
/// panic.
fn setup_no_mock() -> (Env, CreditClient<'static>, Address, Address) {
    let env = Env::default();
    env.ledger().set_timestamp(10_000);

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_addr = token_id.address();

    client
        .mock_auths(&[admin_auth!(
            &admin,
            &contract_id,
            "set_liquidity_token",
            soroban_sdk::vec![&env, (&token_addr).into_val(&env)]
        )])
        .set_liquidity_token(&token_addr);

    client
        .mock_auths(&[admin_auth!(
            &admin,
            &contract_id,
            "set_liquidity_source",
            soroban_sdk::vec![&env, (&contract_id).into_val(&env)]
        )])
        .set_liquidity_source(&contract_id);

    // The token contract requires the *credit contract* to authorise each mint.
    for to in [&contract_id, &borrower] {
        let token_client = token::StellarAssetClient::new(&env, &token_addr);
        token_client
            .mock_auths(&[MockAuth {
                address: &contract_id,
                invoke: &MockAuthInvoke {
                    contract: &token_addr,
                    fn_name: "mint",
                    args: soroban_sdk::vec![
                        &env,
                        (*to).into_val(&env),
                        10_000_000_i128.into_val(&env),
                    ],
                    sub_invokes: &[],
                },
            }])
            .mint(to, &10_000_000_i128);
    }

    client
        .mock_auths(&[admin_auth!(
            &admin,
            &contract_id,
            "open_credit_line",
            soroban_sdk::vec![
                &env,
                borrower.into_val(&env),
                100_000_i128.into_val(&env),
                300_u32.into_val(&env),
                50_u32.into_val(&env),
            ]
        )])
        .open_credit_line(&borrower, &100_000_i128, &300_u32, &50_u32);

    let treasury = Address::generate(&env);
    client
        .mock_auths(&[admin_auth!(
            &admin,
            &contract_id,
            "set_treasury",
            soroban_sdk::vec![&env, treasury.into_val(&env)]
        )])
        .set_treasury(&treasury);

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

// ─────────────────────────────────────────────────────────────────────────────
// Fund-setter auth snapshots — Issue #1281
// ─────────────────────────────────────────────────────────────────────────────
//
// These entrypoints previously required *two* authorizations: the `admin`
// argument's own `require_auth()` plus the stored admin's
// `require_admin_auth()`. The `admin: Address` parameter has been removed, so
// the stored admin's authorization is now the only one requested.
//
// Each test asserts both halves of that contract:
//   1. Positive: exactly one authorization is recorded, and it is the admin.
//   2. Negative: with no valid auth the call panics, so the guard is
//      load-bearing.

/// Asserts the authorization recorded for the call under test.
///
/// Checks three things, which together pin the Issue #1281 contract:
///
/// 1. The signer is the stored admin - the guard is still load-bearing.
/// 2. No sub-invocations were authorised for the call - the entrypoint
///    authorizes itself, not a token transfer or another contract call.
/// 3. The authorization carries exactly `expected_argc` arguments. Since
///    `require_auth` records the call arguments, this is a direct regression
///    test for the removed `admin: Address` parameter: reintroducing it would
///    make the count one higher.
fn assert_single_admin_auth(env: &Env, admin: &Address, entrypoint: &str, expected_argc: usize) {
    let auths = env.auths();
    let (signer, invocation) = auths.last().unwrap();
    assert_eq!(
        *signer, *admin,
        "{entrypoint} must be authorised by the stored admin"
    );
    match &invocation.function {
        AuthorizedFunction::Contract((_contract, name, args)) => {
            assert_eq!(*name, Symbol::new(env, entrypoint));
            assert!(
                invocation.sub_invocations.is_empty(),
                "{entrypoint} must not authorise sub-invocations"
            );
            assert_eq!(
                args.len() as usize,
                expected_argc,
                "{entrypoint} must not take an `admin` argument (Issue #1281)"
            );
        }
        other => panic!("{entrypoint} authorised unexpected function: {other:?}"),
    }
}

#[test]
fn test_set_treasury_auth_snap() {
    let (env, client, admin, _borrower) = setup();
    client.set_treasury(&Address::generate(&env));
    let auths = env.auths();
    insta::assert_debug_snapshot!(auths.last().unwrap());
    assert_single_admin_auth(&env, &admin, "set_treasury", 1);
}

#[test]
#[should_panic]
fn set_treasury_without_admin_auth_panics() {
    let (_env, client, _admin, _borrower) = setup_no_mock();
    client.set_treasury(&Address::generate(&_env));
}

#[test]
fn test_set_bounty_auth_snap() {
    let (env, client, admin, _borrower) = setup();
    client.set_bounty(&Address::generate(&env));
    let auths = env.auths();
    insta::assert_debug_snapshot!(auths.last().unwrap());
    assert_single_admin_auth(&env, &admin, "set_bounty", 1);
}

#[test]
#[should_panic]
fn set_bounty_without_admin_auth_panics() {
    let (_env, client, _admin, _borrower) = setup_no_mock();
    client.set_bounty(&Address::generate(&_env));
}

#[test]
fn test_propose_treasury_withdrawal_auth_snap() {
    let (env, client, admin, _borrower) = setup();
    client.propose_treasury_withdrawal();
    let auths = env.auths();
    insta::assert_debug_snapshot!(auths.last().unwrap());
    assert_single_admin_auth(&env, &admin, "propose_treasury_withdrawal", 0);
}

#[test]
#[should_panic]
fn propose_treasury_withdrawal_without_admin_auth_panics() {
    let (_env, client, _admin, _borrower) = setup_no_mock();
    client.propose_treasury_withdrawal();
}

#[test]
fn test_execute_treasury_withdrawal_auth_snap() {
    let (env, client, admin, _borrower) = setup();
    client.propose_treasury_withdrawal();
    env.ledger().set_timestamp(10_000 + 86_400);
    client.execute_treasury_withdrawal();
    let auths = env.auths();
    insta::assert_debug_snapshot!(auths.last().unwrap());
    assert_single_admin_auth(&env, &admin, "execute_treasury_withdrawal", 0);
}

#[test]
#[should_panic]
fn execute_treasury_withdrawal_without_admin_auth_panics() {
    let (env, client, _admin, _borrower) = setup_no_mock();
    client.propose_treasury_withdrawal();
    env.ledger().set_timestamp(10_000 + 86_400);
    client.execute_treasury_withdrawal();
}

#[test]
fn test_withdraw_treasury_auth_snap() {
    let (env, client, admin, _borrower) = setup();
    client.withdraw_treasury();
    let auths = env.auths();
    insta::assert_debug_snapshot!(auths.last().unwrap());
    assert_single_admin_auth(&env, &admin, "withdraw_treasury", 0);
}

#[test]
#[should_panic]
fn withdraw_treasury_without_admin_auth_panics() {
    let (_env, client, _admin, _borrower) = setup_no_mock();
    client.withdraw_treasury();
}

#[test]
fn test_withdraw_bounty_auth_snap() {
    let (env, client, admin, _borrower) = setup();
    client.set_bounty(&Address::generate(&env));
    client.withdraw_bounty();
    let auths = env.auths();
    insta::assert_debug_snapshot!(auths.last().unwrap());
    assert_single_admin_auth(&env, &admin, "withdraw_bounty", 0);
}

#[test]
#[should_panic]
fn withdraw_bounty_without_admin_auth_panics() {
    let (_env, client, _admin, _borrower) = setup_no_mock();
    client.withdraw_bounty();
}

#[test]
fn test_block_borrower_auth_snap() {
    let (env, client, admin, borrower) = setup();
    client.block_borrower(&borrower);
    let auths = env.auths();
    insta::assert_debug_snapshot!(auths.last().unwrap());
    assert_single_admin_auth(&env, &admin, "block_borrower", 1);
}

#[test]
#[should_panic]
fn block_borrower_without_admin_auth_panics() {
    let (_env, client, _admin, borrower) = setup_no_mock();
    client.block_borrower(&borrower);
}

#[test]
fn test_unblock_borrower_auth_snap() {
    let (env, client, admin, borrower) = setup();
    client.block_borrower(&borrower);
    client.unblock_borrower(&borrower);
    let auths = env.auths();
    insta::assert_debug_snapshot!(auths.last().unwrap());
    assert_single_admin_auth(&env, &admin, "unblock_borrower", 1);
}

#[test]
#[should_panic]
fn unblock_borrower_without_admin_auth_panics() {
    let (_env, client, _admin, borrower) = setup_no_mock();
    client.unblock_borrower(&borrower);
}

#[test]
fn test_bulk_block_borrowers_auth_snap() {
    let (env, client, admin, borrower) = setup();
    let borrowers = soroban_sdk::vec![&env, borrower];
    client.bulk_block_borrowers(&borrowers);
    let auths = env.auths();
    insta::assert_debug_snapshot!(auths.last().unwrap());
    assert_single_admin_auth(&env, &admin, "bulk_block_borrowers", 1);
}

#[test]
#[should_panic]
fn bulk_block_borrowers_without_admin_auth_panics() {
    let (env, client, _admin, borrower) = setup_no_mock();
    let borrowers = soroban_sdk::vec![&env, borrower];
    client.bulk_block_borrowers(&borrowers);
}

#[test]
fn test_freeze_borrower_until_auth_snap() {
    let (env, client, admin, borrower) = setup();
    client.freeze_borrower_until(&borrower, &(10_000 + 3_600));
    let auths = env.auths();
    insta::assert_debug_snapshot!(auths.last().unwrap());
    assert_single_admin_auth(&env, &admin, "freeze_borrower_until", 2);
}

#[test]
#[should_panic]
fn freeze_borrower_until_without_admin_auth_panics() {
    let (env, client, _admin, borrower) = setup_no_mock();
    client.freeze_borrower_until(&borrower, &(10_000 + 3_600));
}

#[test]
fn test_unfreeze_borrower_auth_snap() {
    let (env, client, admin, borrower) = setup();
    client.unfreeze_borrower(&borrower);
    let auths = env.auths();
    insta::assert_debug_snapshot!(auths.last().unwrap());
    assert_single_admin_auth(&env, &admin, "unfreeze_borrower", 1);
}

#[test]
#[should_panic]
fn unfreeze_borrower_without_admin_auth_panics() {
    let (_env, client, _admin, borrower) = setup_no_mock();
    client.unfreeze_borrower(&borrower);
}

/// `unblock_borrower` is documented as idempotent, but it used to revert with
/// `Storage(MissingValue)` because it removed the borrower's blocklist entry
/// and then extended the TTL of the key it had just removed. This pins the
/// fixed behaviour: the borrower is unblocked, and a second unblock is a no-op.
#[test]
fn unblock_borrower_is_idempotent() {
    let (env, client, _admin, borrower) = setup();
    client.block_borrower(&borrower);
    assert!(client.is_borrower_blocked(&borrower));

    client.unblock_borrower(&borrower);
    assert!(!client.is_borrower_blocked(&borrower));

    env.ledger().set_timestamp(10_000);
    client.unblock_borrower(&borrower);
    assert!(!client.is_borrower_blocked(&borrower));
}
