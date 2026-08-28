// SPDX-License-Identifier: MIT

//! Auth snapshot for the query subsystem (#947). Every query is read-only
//! and must not require auth. Tests verify zero auths are recorded when
//! `mock_all_auths` is active, and that calls succeed with no signer.

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

/// Deploys a fresh contract with `mock_all_auths` enabled.
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
fn setup_no_mock(env: &Env) -> (CreditClient<'_>, Address) {
    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &50_u32);
    (client, borrower)
}

// ── Positive snapshot: zero auths recorded ───────────────────────────────

#[test]
fn get_credit_line_auth_snapshot() {
    let env = Env::default();
    let (client, _admin, _borrower) = setup(&env);
    let _ = client.get_credit_line(&_borrower);
    assert!(
        env.auths().is_empty(),
        "get_credit_line must not require auth"
    );
}

#[test]
fn get_protocol_summary_auth_snapshot() {
    let env = Env::default();
    let (client, _admin, _borrower) = setup(&env);
    let _ = client.get_protocol_summary();
    assert!(
        env.auths().is_empty(),
        "get_protocol_summary must not require auth"
    );
}

#[test]
fn get_repayment_schedule_auth_snapshot() {
    let env = Env::default();
    let (client, _admin, _borrower) = setup(&env);
    let _ = client.get_repayment_schedule(&_borrower);
    assert!(
        env.auths().is_empty(),
        "get_repayment_schedule must not require auth"
    );
}

#[test]
fn get_health_factor_auth_snapshot() {
    let env = Env::default();
    let (client, _admin, _borrower) = setup(&env);
    let _ = client.get_health_factor(&_borrower);
    assert!(
        env.auths().is_empty(),
        "get_health_factor must not require auth"
    );
}

#[test]
fn is_delinquent_auth_snapshot() {
    let env = Env::default();
    let (client, _admin, _borrower) = setup(&env);
    let _ = client.is_delinquent(&_borrower);
    assert!(
        env.auths().is_empty(),
        "is_delinquent must not require auth"
    );
}

// ── Zero-signer verification ─────────────────────────────────────────────

#[test]
fn get_credit_line_requires_no_auth() {
    let env = Env::default();
    let (client, borrower) = setup_no_mock(&env);
    assert!(client.get_credit_line(&borrower).is_some());
}

#[test]
fn get_protocol_summary_requires_no_auth() {
    let env = Env::default();
    let (client, _borrower) = setup_no_mock(&env);
    let _ = client.get_protocol_summary();
}

#[test]
fn get_repayment_schedule_requires_no_auth() {
    let env = Env::default();
    let (client, borrower) = setup_no_mock(&env);
    let _ = client.get_repayment_schedule(&borrower);
}

#[test]
fn get_health_factor_requires_no_auth() {
    let env = Env::default();
    let (client, borrower) = setup_no_mock(&env);
    assert_eq!(client.get_health_factor(&borrower), u32::MAX);
}

#[test]
fn is_delinquent_requires_no_auth() {
    let env = Env::default();
    let (client, borrower) = setup_no_mock(&env);
    assert!(!client.is_delinquent(&borrower));
}

// ── Edge: nonexistent borrower ───────────────────────────────────────────

#[test]
fn get_credit_line_nonexistent_borrower_auth_snapshot() {
    let env = Env::default();
    let (client, _admin, _borrower) = setup(&env);
    let stranger = Address::generate(&env);
    let _ = client.get_credit_line(&stranger);
    assert!(
        env.auths().is_empty(),
        "get_credit_line must not require auth for unknown borrower"
    );
}

#[test]
fn get_health_factor_nonexistent_borrower_auth_snapshot() {
    let env = Env::default();
    let (client, _admin, _borrower) = setup(&env);
    let stranger = Address::generate(&env);
    let health = client.get_health_factor(&stranger);
    assert_eq!(health, u32::MAX);
    assert!(
        env.auths().is_empty(),
        "get_health_factor must not require auth for unknown borrower"
    );
}

#[test]
fn is_delinquent_nonexistent_borrower_auth_snapshot() {
    let env = Env::default();
    let (client, _admin, _borrower) = setup(&env);
    let stranger = Address::generate(&env);
    assert!(!client.is_delinquent(&stranger));
    assert!(
        env.auths().is_empty(),
        "is_delinquent must not require auth for unknown borrower"
    );
}
