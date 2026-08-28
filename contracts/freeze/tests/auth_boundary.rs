// SPDX-License-Identifier: MIT

//! Per-entrypoint auth boundary tests for every freeze-related entrypoint (#835).
//!
//! # What
//!
//! Proves that each state-changing freeze entrypoint requires admin
//! authorization (no signer / wrong signer revert), that an admin-signed call
//! succeeds and records exactly one admin authorization, and that read-only
//! freeze queries require no authorization.
//!
//! # Freeze surface covered
//!
//! | Entrypoint                 | Auth           | Covered |
//! |----------------------------|----------------|---------|
//! | `freeze_draws`             | admin          | ✅      |
//! | `unfreeze_draws`           | admin          | ✅      |
//! | `freeze_credit_line`       | admin          | ✅      |
//! | `unfreeze_credit_line`     | admin          | ✅      |
//! | `freeze_borrower_until`    | admin (+ role) | ✅      |
//! | `unfreeze_borrower`        | admin (+ role) | ✅      |
//! | `is_draws_frozen`          | none           | ✅      |
//! | `get_draws_freeze_reason`  | none           | ✅      |
//! | `is_credit_line_frozen`    | none           | ✅      |
//! | `get_credit_line_freeze_reason` | none      | ✅      |
//! | `is_borrower_frozen`       | none           | ✅      |
//! | `get_borrower_frozen_until`| none           | ✅      |
//!
//! # See also
//! - `contracts/credit/src/freeze.rs` — freeze implementation.
//! - `contracts/credit/tests/freeze_auth_snap.rs` — auth *shape* snapshots.
//! - `contracts/credit/tests/unauthorized_matrix.rs` — broader negative matrix.

use creditra_credit::{Credit, CreditClient, FreezeReason};
use soroban_sdk::testutils::{Address as _, Ledger, MockAuth, MockAuthInvoke};
use soroban_sdk::{Address, Env, IntoVal};

const START_TS: u64 = 10_000;

/// Deploy + `init` + open a credit line with `mock_all_auths` enabled.
///
/// `mock_all_auths` still records what was authorized, so `env.auths()` after
/// the call under test reflects that call's requirements (setup auths are
/// cleared by reading after the call of interest only when we freeze setup
/// separately — for positive snapshots we re-read after the single call).
fn setup(env: &Env) -> (CreditClient<'_>, Address, Address) {
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = START_TS);
    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &50_u32);
    (client, admin, borrower)
}

/// Same as [`setup`] without `mock_all_auths`, for negative boundary tests.
///
/// `init` / `open_credit_line` currently succeed without mocked signers in this
/// harness, matching `freeze_auth_snap.rs`.
fn setup_no_mock(env: &Env) -> (CreditClient<'_>, Address, Address, Address) {
    env.ledger().with_mut(|li| li.timestamp = START_TS);
    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &50_u32);
    (client, contract_id, admin, borrower)
}

fn mock_admin_freeze_draws<'a>(
    env: &'a Env,
    client: &CreditClient<'a>,
    contract_id: &Address,
    admin: &Address,
) {
    client
        .mock_auths(&[MockAuth {
            address: admin,
            invoke: &MockAuthInvoke {
                contract: contract_id,
                fn_name: "freeze_draws",
                args: (FreezeReason::LiquidityReserve,).into_val(env),
                sub_invokes: &[],
            },
        }])
        .freeze_draws(&FreezeReason::LiquidityReserve);
}

fn mock_admin_freeze_credit_line<'a>(
    env: &'a Env,
    client: &CreditClient<'a>,
    contract_id: &Address,
    admin: &Address,
    borrower: &Address,
    reason: FreezeReason,
) {
    client
        .mock_auths(&[MockAuth {
            address: admin,
            invoke: &MockAuthInvoke {
                contract: contract_id,
                fn_name: "freeze_credit_line",
                args: (borrower.clone(), reason).into_val(env),
                sub_invokes: &[],
            },
        }])
        .freeze_credit_line(borrower, &reason);
}

fn mock_admin_freeze_borrower_until<'a>(
    env: &'a Env,
    client: &CreditClient<'a>,
    contract_id: &Address,
    admin: &Address,
    borrower: &Address,
    expiry_ts: u64,
) {
    client
        .mock_auths(&[MockAuth {
            address: admin,
            invoke: &MockAuthInvoke {
                contract: contract_id,
                fn_name: "freeze_borrower_until",
                args: (admin.clone(), borrower.clone(), expiry_ts).into_val(env),
                sub_invokes: &[],
            },
        }])
        .freeze_borrower_until(admin, borrower, &expiry_ts);
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 1 — Positive: admin-signed call records exactly one admin auth
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn freeze_draws_authorized_records_admin_auth() {
    let env = Env::default();
    let (client, admin, _borrower) = setup(&env);

    client.freeze_draws(&FreezeReason::LiquidityReserve);

    let auths = env.auths();
    assert_eq!(auths.len(), 1, "freeze_draws must record exactly one auth");
    assert_eq!(
        auths[0].0, admin,
        "freeze_draws must be authorized by admin"
    );
    assert!(client.is_draws_frozen());
}

#[test]
fn unfreeze_draws_authorized_records_admin_auth() {
    let env = Env::default();
    let (client, admin, _borrower) = setup(&env);
    client.freeze_draws(&FreezeReason::LiquidityReserve);

    client.unfreeze_draws();

    let auths = env.auths();
    assert_eq!(
        auths.len(),
        1,
        "unfreeze_draws must record exactly one auth"
    );
    assert_eq!(
        auths[0].0, admin,
        "unfreeze_draws must be authorized by admin"
    );
    assert!(!client.is_draws_frozen());
}

#[test]
fn freeze_credit_line_authorized_records_admin_auth() {
    let env = Env::default();
    let (client, admin, borrower) = setup(&env);

    client.freeze_credit_line(&borrower, &FreezeReason::Compliance);

    let auths = env.auths();
    assert_eq!(
        auths.len(),
        1,
        "freeze_credit_line must record exactly one auth"
    );
    assert_eq!(
        auths[0].0, admin,
        "freeze_credit_line must be authorized by admin"
    );
    assert!(client.is_credit_line_frozen(&borrower));
}

#[test]
fn unfreeze_credit_line_authorized_records_admin_auth() {
    let env = Env::default();
    let (client, admin, borrower) = setup(&env);
    client.freeze_credit_line(&borrower, &FreezeReason::RiskInvestigation);

    client.unfreeze_credit_line(&borrower);

    let auths = env.auths();
    assert_eq!(
        auths.len(),
        1,
        "unfreeze_credit_line must record exactly one auth"
    );
    assert_eq!(
        auths[0].0, admin,
        "unfreeze_credit_line must be authorized by admin"
    );
    assert!(!client.is_credit_line_frozen(&borrower));
}

#[test]
fn freeze_borrower_until_authorized_records_admin_auth() {
    let env = Env::default();
    let (client, admin, borrower) = setup(&env);
    let expiry = START_TS + 3_600;

    client.freeze_borrower_until(&admin, &borrower, &expiry);

    let auths = env.auths();
    assert!(
        !auths.is_empty(),
        "freeze_borrower_until must record at least one auth"
    );
    assert!(
        auths.iter().any(|(addr, _)| addr == &admin),
        "freeze_borrower_until must be authorized by admin"
    );
    assert!(client.is_borrower_frozen(&borrower));
}

#[test]
fn unfreeze_borrower_authorized_records_admin_auth() {
    let env = Env::default();
    let (client, admin, borrower) = setup(&env);
    let expiry = START_TS + 3_600;
    client.freeze_borrower_until(&admin, &borrower, &expiry);

    client.unfreeze_borrower(&admin, &borrower);

    let auths = env.auths();
    assert!(
        !auths.is_empty(),
        "unfreeze_borrower must record at least one auth"
    );
    assert!(
        auths.iter().any(|(addr, _)| addr == &admin),
        "unfreeze_borrower must be authorized by admin"
    );
    assert!(!client.is_borrower_frozen(&borrower));
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 2 — Negative: zero signers → revert
// ═══════════════════════════════════════════════════════════════════════════

#[test]
#[should_panic]
fn freeze_draws_reverts_without_auth() {
    let env = Env::default();
    let (client, _contract_id, _admin, _borrower) = setup_no_mock(&env);
    client.freeze_draws(&FreezeReason::LiquidityReserve);
}

#[test]
#[should_panic]
fn unfreeze_draws_reverts_without_auth() {
    let env = Env::default();
    let (client, contract_id, admin, _borrower) = setup_no_mock(&env);
    mock_admin_freeze_draws(&env, &client, &contract_id, &admin);
    client.unfreeze_draws();
}

#[test]
#[should_panic]
fn freeze_credit_line_reverts_without_auth() {
    let env = Env::default();
    let (client, _contract_id, _admin, borrower) = setup_no_mock(&env);
    client.freeze_credit_line(&borrower, &FreezeReason::Compliance);
}

#[test]
#[should_panic]
fn unfreeze_credit_line_reverts_without_auth() {
    let env = Env::default();
    let (client, contract_id, admin, borrower) = setup_no_mock(&env);
    mock_admin_freeze_credit_line(
        &env,
        &client,
        &contract_id,
        &admin,
        &borrower,
        FreezeReason::Compliance,
    );
    client.unfreeze_credit_line(&borrower);
}

#[test]
#[should_panic]
fn freeze_borrower_until_reverts_without_auth() {
    let env = Env::default();
    let (client, _contract_id, admin, borrower) = setup_no_mock(&env);
    client.freeze_borrower_until(&admin, &borrower, &(START_TS + 3_600));
}

#[test]
#[should_panic]
fn unfreeze_borrower_reverts_without_auth() {
    let env = Env::default();
    let (client, contract_id, admin, borrower) = setup_no_mock(&env);
    let expiry = START_TS + 3_600;
    mock_admin_freeze_borrower_until(&env, &client, &contract_id, &admin, &borrower, expiry);
    client.unfreeze_borrower(&admin, &borrower);
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 3 — Negative: wrong (non-admin) signer → revert
// ═══════════════════════════════════════════════════════════════════════════

#[test]
#[should_panic]
fn freeze_draws_wrong_signer_reverts() {
    let env = Env::default();
    let (client, contract_id, _admin, _borrower) = setup_no_mock(&env);
    let attacker = Address::generate(&env);

    client
        .mock_auths(&[MockAuth {
            address: &attacker,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "freeze_draws",
                args: (FreezeReason::LiquidityReserve,).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .freeze_draws(&FreezeReason::LiquidityReserve);
}

#[test]
#[should_panic]
fn unfreeze_draws_wrong_signer_reverts() {
    let env = Env::default();
    let (client, contract_id, admin, _borrower) = setup_no_mock(&env);
    mock_admin_freeze_draws(&env, &client, &contract_id, &admin);
    let attacker = Address::generate(&env);

    client
        .mock_auths(&[MockAuth {
            address: &attacker,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "unfreeze_draws",
                args: ().into_val(&env),
                sub_invokes: &[],
            },
        }])
        .unfreeze_draws();
}

#[test]
#[should_panic]
fn freeze_credit_line_wrong_signer_reverts() {
    let env = Env::default();
    let (client, contract_id, _admin, borrower) = setup_no_mock(&env);
    let attacker = Address::generate(&env);

    client
        .mock_auths(&[MockAuth {
            address: &attacker,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "freeze_credit_line",
                args: (borrower.clone(), FreezeReason::Compliance).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .freeze_credit_line(&borrower, &FreezeReason::Compliance);
}

#[test]
#[should_panic]
fn unfreeze_credit_line_wrong_signer_reverts() {
    let env = Env::default();
    let (client, contract_id, admin, borrower) = setup_no_mock(&env);
    mock_admin_freeze_credit_line(
        &env,
        &client,
        &contract_id,
        &admin,
        &borrower,
        FreezeReason::Compliance,
    );
    let attacker = Address::generate(&env);

    client
        .mock_auths(&[MockAuth {
            address: &attacker,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "unfreeze_credit_line",
                args: (borrower.clone(),).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .unfreeze_credit_line(&borrower);
}

#[test]
#[should_panic]
fn freeze_borrower_until_wrong_signer_reverts() {
    let env = Env::default();
    let (client, contract_id, admin, borrower) = setup_no_mock(&env);
    let attacker = Address::generate(&env);
    let expiry = START_TS + 3_600;

    client
        .mock_auths(&[MockAuth {
            address: &attacker,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "freeze_borrower_until",
                args: (admin.clone(), borrower.clone(), expiry).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .freeze_borrower_until(&admin, &borrower, &expiry);
}

#[test]
#[should_panic]
fn unfreeze_borrower_wrong_signer_reverts() {
    let env = Env::default();
    let (client, contract_id, admin, borrower) = setup_no_mock(&env);
    let expiry = START_TS + 3_600;
    mock_admin_freeze_borrower_until(&env, &client, &contract_id, &admin, &borrower, expiry);
    let attacker = Address::generate(&env);

    client
        .mock_auths(&[MockAuth {
            address: &attacker,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "unfreeze_borrower",
                args: (admin.clone(), borrower.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .unfreeze_borrower(&admin, &borrower);
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 4 — Read-only freeze queries require no authorization
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn is_draws_frozen_requires_no_auth() {
    let env = Env::default();
    let (client, _admin, _borrower) = setup(&env);
    client.freeze_draws(&FreezeReason::LiquidityReserve);

    let _ = client.is_draws_frozen();
    assert!(
        env.auths().is_empty(),
        "is_draws_frozen must not require authorization"
    );
}

#[test]
fn get_draws_freeze_reason_requires_no_auth() {
    let env = Env::default();
    let (client, _admin, _borrower) = setup(&env);
    client.freeze_draws(&FreezeReason::LiquidityReserve);

    let _ = client.get_draws_freeze_reason();
    assert!(
        env.auths().is_empty(),
        "get_draws_freeze_reason must not require authorization"
    );
}

#[test]
fn is_credit_line_frozen_requires_no_auth() {
    let env = Env::default();
    let (client, _admin, borrower) = setup(&env);
    client.freeze_credit_line(&borrower, &FreezeReason::Compliance);

    let _ = client.is_credit_line_frozen(&borrower);
    assert!(
        env.auths().is_empty(),
        "is_credit_line_frozen must not require authorization"
    );
}

#[test]
fn get_credit_line_freeze_reason_requires_no_auth() {
    let env = Env::default();
    let (client, _admin, borrower) = setup(&env);
    client.freeze_credit_line(&borrower, &FreezeReason::Compliance);

    let _ = client.get_credit_line_freeze_reason(&borrower);
    assert!(
        env.auths().is_empty(),
        "get_credit_line_freeze_reason must not require authorization"
    );
}

#[test]
fn is_borrower_frozen_requires_no_auth() {
    let env = Env::default();
    let (client, admin, borrower) = setup(&env);
    client.freeze_borrower_until(&admin, &borrower, &(START_TS + 3_600));

    let _ = client.is_borrower_frozen(&borrower);
    assert!(
        env.auths().is_empty(),
        "is_borrower_frozen must not require authorization"
    );
}

#[test]
fn get_borrower_frozen_until_requires_no_auth() {
    let env = Env::default();
    let (client, admin, borrower) = setup(&env);
    let expiry = START_TS + 3_600;
    client.freeze_borrower_until(&admin, &borrower, &expiry);

    let _ = client.get_borrower_frozen_until(&borrower);
    assert!(
        env.auths().is_empty(),
        "get_borrower_frozen_until must not require authorization"
    );
}
