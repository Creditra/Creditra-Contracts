// SPDX-License-Identifier: MIT

//! Per-entrypoint auth snapshot for the freeze subsystem (#866, v7).
//!
//! `unauthorized_matrix.rs` proves a *wrong* signer reverts. This file goes
//! one step further and pins the exact authorization *shape* the Soroban
//! host records for a **successful**, admin-signed call to every
//! state-changing freeze entrypoint. A future change that silently drops
//! `require_auth`, requires an extra signer, or authorizes the wrong
//! address will fail one of the snapshot assertions below even though
//! `mock_all_auths` would otherwise paper over the regression.
//!
//! # Snapshot (v7 freeze surface)
//!
//! | Entrypoint              | Required signer | Auths recorded | Sub-invocations |
//! |--------------------------|------------------|----------------|------------------|
//! | `freeze_draws`           | admin            | 1              | 0                |
//! | `unfreeze_draws`         | admin            | 1              | 0                |
//! | `freeze_credit_line`     | admin            | 1              | 0                |
//! | `unfreeze_credit_line`   | admin            | 1              | 0                |
//! | `is_draws_frozen`        | none (read-only) | 0              | —                |
//! | `is_credit_line_frozen`  | none (read-only) | 0              | —                |
//!
//! # Rules
//! - Never weaken an existing assertion (e.g. loosening `auths().len()`).
//! - If a freeze entrypoint gains a second required signer or a
//!   sub-invocation, update the table above alongside the assertion.
//!
//! # See also
//! - `creditra_credit::freeze` — the freeze/unfreeze implementation.
//! - `contracts/credit/tests/unauthorized_matrix.rs` — the negative-only
//!   caller matrix this file complements.

use creditra_credit::{Credit, CreditClient, FreezeReason};
use soroban_sdk::testutils::{Address as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{Address, Env, IntoVal};

/// Deploys a fresh contract, initializes `admin`, and opens a credit line
/// for `borrower`, with `mock_all_auths` enabled for the whole env.
///
/// Because `mock_all_auths` still records what was authorized (it only
/// skips signature verification), `env.auths()` after the call under test
/// reflects exactly what that call — and nothing from setup — required.
fn setup(env: &Env) -> (CreditClient<'_>, Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &50_u32);
    (client, admin, borrower)
}

/// Same as [`setup`] but *without* `mock_all_auths`, for negative tests.
/// `init` and `open_credit_line` do not currently enforce `require_auth`,
/// so both calls succeed here without any mocked signer.
fn setup_no_mock(env: &Env) -> (CreditClient<'_>, Address, Address, Address) {
    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &50_u32);
    (client, contract_id, admin, borrower)
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 1 — Positive snapshot: exactly one auth, held by admin
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn freeze_draws_auth_snapshot() {
    let env = Env::default();
    let (client, admin, _borrower) = setup(&env);

    client.freeze_draws(&FreezeReason::Compliance);

    let auths = env.auths();
    assert_eq!(
        auths.len(),
        1,
        "freeze_draws must record exactly one authorization"
    );
    assert_eq!(
        auths[0].0, admin,
        "freeze_draws must be authorized by the admin"
    );
}

#[test]
fn unfreeze_draws_auth_snapshot() {
    let env = Env::default();
    let (client, admin, _borrower) = setup(&env);
    client.freeze_draws(&FreezeReason::Compliance);

    client.unfreeze_draws();

    let auths = env.auths();
    assert_eq!(
        auths.len(),
        1,
        "unfreeze_draws must record exactly one authorization"
    );
    assert_eq!(
        auths[0].0, admin,
        "unfreeze_draws must be authorized by the admin"
    );
}

#[test]
fn freeze_credit_line_auth_snapshot() {
    let env = Env::default();
    let (client, admin, borrower) = setup(&env);

    client.freeze_credit_line(&borrower, &FreezeReason::Compliance);

    let auths = env.auths();
    assert_eq!(
        auths.len(),
        1,
        "freeze_credit_line must record exactly one authorization"
    );
    assert_eq!(
        auths[0].0, admin,
        "freeze_credit_line must be authorized by the admin"
    );
}

#[test]
fn unfreeze_credit_line_auth_snapshot() {
    let env = Env::default();
    let (client, admin, borrower) = setup(&env);
    client.freeze_credit_line(&borrower, &FreezeReason::RiskInvestigation);

    client.unfreeze_credit_line(&borrower);

    let auths = env.auths();
    assert_eq!(
        auths.len(),
        1,
        "unfreeze_credit_line must record exactly one authorization"
    );
    assert_eq!(
        auths[0].0, admin,
        "unfreeze_credit_line must be authorized by the admin"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 2 — Negative: entrypoint reverts with zero signers mocked
// ═══════════════════════════════════════════════════════════════════════════

#[test]
#[should_panic]
fn freeze_draws_reverts_without_auth() {
    let env = Env::default();
    let (client, _contract_id, _admin, _borrower) = setup_no_mock(&env);
    client.freeze_draws(&FreezeReason::Compliance);
}

#[test]
#[should_panic]
fn unfreeze_draws_reverts_without_auth() {
    let env = Env::default();
    let (client, contract_id, admin, _borrower) = setup_no_mock(&env);

    // Legitimately freeze first, authorizing only this one call.
    client
        .mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "freeze_draws",
                args: (FreezeReason::Compliance,).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .freeze_draws(&FreezeReason::Compliance);

    // No signer mocked for this call — must revert.
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

    client
        .mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "freeze_credit_line",
                args: (borrower.clone(), FreezeReason::Compliance).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .freeze_credit_line(&borrower, &FreezeReason::Compliance);

    client.unfreeze_credit_line(&borrower);
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 3 — Edge case: a non-admin signer is rejected, not just "no signer"
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
                args: (FreezeReason::Compliance,).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .freeze_draws(&FreezeReason::Compliance);
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

// ═══════════════════════════════════════════════════════════════════════════
// Section 4 — Edge case: read-only freeze queries require no authorization
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn is_draws_frozen_requires_no_auth() {
    let env = Env::default();
    let (client, _admin, _borrower) = setup(&env);
    client.freeze_draws(&FreezeReason::Compliance);

    let _ = client.is_draws_frozen();

    assert!(
        env.auths().is_empty(),
        "is_draws_frozen must not require any authorization"
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
        "is_credit_line_frozen must not require any authorization"
    );
}
