// SPDX-License-Identifier: MIT

//! Per-entrypoint auth snapshot for the query (v7) subsystem (#876).
//!
//! Every entrypoint on the v7 query surface is a pure, read-only view: no
//! token CPIs, no storage mutation, and — critically — no `require_auth`.
//! This file pins that shape so a future change that silently adds an
//! authorization requirement to a query entrypoint (breaking off-chain
//! indexers, keepers, and dashboards that call these without a signer) is
//! caught immediately, even though `mock_all_auths` would otherwise paper
//! over the regression.
//!
//! # Snapshot (query v7 surface)
//!
//! | Entrypoint                     | Required signer   | Auths recorded |
//! |----------------------------------|--------------------|-----------------|
//! | `get_credit_line`                | none (read-only)   | 0               |
//! | `get_credit_line_summary`        | none (read-only)   | 0               |
//! | `get_protocol_summary`           | none (read-only)   | 0               |
//! | `get_repayment_schedule`         | none (read-only)   | 0               |
//! | `get_health_factor`              | none (read-only)   | 0               |
//! | `is_delinquent`                  | none (read-only)   | 0               |
//! | `get_credit_lines_paginated`     | none (read-only)   | 0               |
//! | `borrow_capabilities`            | none (read-only)   | 0               |
//! | `query_capabilities`             | none (read-only)   | 0               |
//!
//! # Rules
//! - Never weaken an existing assertion (e.g. dropping an
//!   `env.auths().is_empty()` check).
//! - If a query entrypoint ever gains a `require_auth` call, update the
//!   table above alongside the assertion — that is a deliberate, reviewed
//!   API change, not a silent regression.
//!
//! # See also
//! - `creditra_credit::query` — the read-only query implementation.
//! - `contracts/query/tests/capabilities.rs` — functional coverage of the
//!   `query_capabilities` bitmap.
//! - `contracts/freeze/tests/auth_snap.rs` — the analogous snapshot for the
//!   freeze subsystem's state-changing entrypoints.

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{token, Address, Env};

const START_TS: u64 = 10_000;

/// Deploys a fresh contract, initializes `admin`, opens a credit line for
/// `borrower`, draws against it, and configures a repayment schedule — with
/// `mock_all_auths` enabled for the whole env.
///
/// Because `mock_all_auths` still records what was authorized (it only skips
/// signature verification), `env.auths()` after the call under test reflects
/// exactly what that call — and nothing from setup — required.
fn setup_active(env: &Env) -> (CreditClient<'_>, Address, Address) {
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = START_TS);

    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    client.open_credit_line(&borrower, &1_000_i128, &500_u32, &50_u32);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);
    token::StellarAssetClient::new(env, &token_address).mint(&contract_id, &1_000_000_i128);
    client.set_min_collateral_ratio_bps(&0);
    client.draw_credit(&borrower, &500_i128);
    client.set_repayment_schedule(
        &borrower,
        &100_i128,
        &2_592_000_u64,
        &(START_TS + 2_592_000),
    );

    (client, admin, borrower)
}

/// Deploys a fresh contract and initializes `admin`, but opens no credit
/// line — used to snapshot the "no data" branch of each entrypoint.
fn setup_empty(env: &Env) -> (CreditClient<'_>, Address, Address) {
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = START_TS);

    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    (client, admin, borrower)
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 1 — Active-line state: every query entrypoint records zero auths
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn get_credit_line_requires_no_auth() {
    let env = Env::default();
    let (client, _admin, borrower) = setup_active(&env);

    let line = client.get_credit_line(&borrower);

    assert!(line.is_some());
    assert!(
        env.auths().is_empty(),
        "get_credit_line must not require any authorization"
    );
}

#[test]
fn get_credit_line_summary_requires_no_auth() {
    let env = Env::default();
    let (client, _admin, borrower) = setup_active(&env);

    let summary = client.get_credit_line_summary(&borrower);

    assert!(summary.is_some());
    assert!(
        env.auths().is_empty(),
        "get_credit_line_summary must not require any authorization"
    );
}

#[test]
fn get_protocol_summary_requires_no_auth() {
    let env = Env::default();
    let (client, _admin, _borrower) = setup_active(&env);

    let _ = client.get_protocol_summary();

    assert!(
        env.auths().is_empty(),
        "get_protocol_summary must not require any authorization"
    );
}

#[test]
fn get_repayment_schedule_requires_no_auth() {
    let env = Env::default();
    let (client, _admin, borrower) = setup_active(&env);

    let schedule = client.get_repayment_schedule(&borrower);

    assert!(schedule.is_some());
    assert!(
        env.auths().is_empty(),
        "get_repayment_schedule must not require any authorization"
    );
}

#[test]
fn get_health_factor_requires_no_auth() {
    let env = Env::default();
    let (client, _admin, borrower) = setup_active(&env);

    let hf = client.get_health_factor(&borrower);

    assert_ne!(
        hf,
        u32::MAX,
        "borrower has utilization; expect a finite health factor"
    );
    assert!(
        env.auths().is_empty(),
        "get_health_factor must not require any authorization"
    );
}

#[test]
fn is_delinquent_requires_no_auth() {
    let env = Env::default();
    let (client, _admin, borrower) = setup_active(&env);

    let _ = client.is_delinquent(&borrower);

    assert!(
        env.auths().is_empty(),
        "is_delinquent must not require any authorization"
    );
}

#[test]
fn get_credit_lines_paginated_requires_no_auth() {
    let env = Env::default();
    let (client, _admin, _borrower) = setup_active(&env);

    let page = client.get_credit_lines_paginated(&None, &10_u32);

    assert_eq!(page.lines.len(), 1);
    assert!(
        env.auths().is_empty(),
        "get_credit_lines_paginated must not require any authorization"
    );
}

#[test]
fn borrow_capabilities_requires_no_auth() {
    let env = Env::default();
    let (client, _admin, borrower) = setup_active(&env);

    let _ = client.borrow_capabilities(&borrower);

    assert!(
        env.auths().is_empty(),
        "borrow_capabilities must not require any authorization"
    );
}

#[test]
fn query_capabilities_requires_no_auth() {
    let env = Env::default();
    let (client, _admin, borrower) = setup_active(&env);

    let caps = client.query_capabilities(&borrower);

    assert!(caps.has_credit_line);
    assert!(
        env.auths().is_empty(),
        "query_capabilities must not require any authorization"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 2 — Empty state: no credit line still records zero auths
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn get_credit_line_on_missing_borrower_requires_no_auth() {
    let env = Env::default();
    let (client, _admin, borrower) = setup_empty(&env);

    let line = client.get_credit_line(&borrower);

    assert!(line.is_none());
    assert!(
        env.auths().is_empty(),
        "get_credit_line must not require any authorization even for an unknown borrower"
    );
}

#[test]
fn get_health_factor_on_missing_borrower_requires_no_auth() {
    let env = Env::default();
    let (client, _admin, borrower) = setup_empty(&env);

    let hf = client.get_health_factor(&borrower);

    assert_eq!(hf, u32::MAX);
    assert!(
        env.auths().is_empty(),
        "get_health_factor must not require any authorization even for an unknown borrower"
    );
}

#[test]
fn is_delinquent_on_missing_borrower_requires_no_auth() {
    let env = Env::default();
    let (client, _admin, borrower) = setup_empty(&env);

    let delinquent = client.is_delinquent(&borrower);

    assert!(!delinquent);
    assert!(
        env.auths().is_empty(),
        "is_delinquent must not require any authorization even for an unknown borrower"
    );
}

#[test]
fn query_capabilities_on_missing_borrower_requires_no_auth() {
    let env = Env::default();
    let (client, _admin, borrower) = setup_empty(&env);

    let caps = client.query_capabilities(&borrower);

    assert!(!caps.has_credit_line);
    assert!(
        env.auths().is_empty(),
        "query_capabilities must not require any authorization even for an unknown borrower"
    );
}

#[test]
fn get_protocol_summary_zero_state_requires_no_auth() {
    let env = Env::default();
    let (client, _admin, _borrower) = setup_empty(&env);

    let summary = client.get_protocol_summary();

    assert_eq!(summary.count, 0);
    assert!(
        env.auths().is_empty(),
        "get_protocol_summary must not require any authorization on an empty protocol"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 3 — Edge case: zero mocked signers never satisfies a query call
// ═══════════════════════════════════════════════════════════════════════════
//
// These calls succeed with *no* signer at all (not even a mocked one),
// proving the zero-auth snapshot above isn't an artifact of
// `mock_all_auths` — the entrypoints genuinely never invoke `require_auth`.

#[test]
fn query_entrypoints_succeed_with_zero_signers_mocked() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = START_TS);

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    // `init` and `open_credit_line` do not currently enforce `require_auth`
    // (see `contracts/freeze/tests/auth_snap.rs` for the same observation on
    // the freeze surface), so both succeed here without any mocked signer.
    client.init(&admin);
    client.open_credit_line(&borrower, &1_000_i128, &500_u32, &50_u32);

    let _ = client.get_credit_line(&borrower);
    let _ = client.get_credit_line_summary(&borrower);
    let _ = client.get_protocol_summary();
    let _ = client.get_repayment_schedule(&borrower);
    let _ = client.get_health_factor(&borrower);
    let _ = client.is_delinquent(&borrower);
    let _ = client.get_credit_lines_paginated(&None, &10_u32);
    let _ = client.borrow_capabilities(&borrower);
    let _ = client.query_capabilities(&borrower);

    assert!(
        env.auths().is_empty(),
        "the last query call must not have required any authorization"
    );
}
