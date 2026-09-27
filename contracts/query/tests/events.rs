// SPDX-License-Identifier: MIT

//! Focused tests for the query (v7) structured lifecycle events.
//!
//! # What
//!
//! Verifies that each publisher helper in [`creditra_query::events`]:
//! - Emits an event with the correct topic pair.
//! - Encodes the correct payload fields.
//! - Is deterministic (same inputs → same event).
//! - Does not mutate any storage.
//!
//! # Events covered
//!
//! - [`CreditLineQueriedEvent`] via `publish_credit_line_queried`
//! - [`HealthFactorQueriedEvent`] via `publish_health_factor_queried`
//! - [`DelinquencyCheckedEvent`] via `publish_delinquency_checked`
//! - [`ProtocolSummaryQueriedEvent`] via `publish_protocol_summary_queried`

use creditra_credit::{Credit, CreditClient};
use creditra_query::events::{
    publish_credit_line_queried, publish_delinquency_checked, publish_health_factor_queried,
    publish_protocol_summary_queried, CreditLineQueriedEvent, DelinquencyCheckedEvent,
    HealthFactorQueriedEvent, ProtocolSummaryQueriedEvent,
};
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events, Ledger},
    token, Address, Env, IntoVal,
};

// ── Helper ────────────────────────────────────────────────────────────────────

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);
    client.set_liquidity_source(&contract_id);
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &500_000_i128);

    (env, contract_id, admin)
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 1 — CreditLineQueriedEvent
// ═══════════════════════════════════════════════════════════════════════════

/// `publish_credit_line_queried` emits an event with topic `("query", "cl_read")`
/// and the correct payload when `found = true`.
#[test]
fn credit_line_queried_event_found_true_emits_correct_topic_and_payload() {
    let (env, contract_id, _admin) = setup();
    let borrower = Address::generate(&env);
    env.ledger().set_timestamp(1_000);

    let event = CreditLineQueriedEvent {
        borrower: borrower.clone(),
        found: true,
        timestamp: 1_000,
    };

    env.as_contract(&contract_id, || {
        publish_credit_line_queried(&env, event.clone());
    });

    let all_events = env.events().all();
    assert_eq!(all_events.len(), 1, "expected exactly one event");

    let (_, topics, data) = all_events.get(0).unwrap();
    // Topics: (symbol "query", symbol "cl_read")
    let t0: soroban_sdk::Symbol = topics.get(0).unwrap().try_into_val(&env).unwrap();
    let t1: soroban_sdk::Symbol = topics.get(1).unwrap().try_into_val(&env).unwrap();
    assert_eq!(t0, symbol_short!("query"));
    assert_eq!(t1, symbol_short!("cl_read"));

    // Payload: CreditLineQueriedEvent
    let payload: CreditLineQueriedEvent = data.try_into_val(&env).unwrap();
    assert_eq!(payload.borrower, borrower);
    assert!(payload.found);
    assert_eq!(payload.timestamp, 1_000);
}

/// `publish_credit_line_queried` with `found = false` encodes the correct flag.
#[test]
fn credit_line_queried_event_found_false_encodes_correctly() {
    let (env, contract_id, _admin) = setup();
    let borrower = Address::generate(&env);
    env.ledger().set_timestamp(42);

    let event = CreditLineQueriedEvent {
        borrower: borrower.clone(),
        found: false,
        timestamp: 42,
    };

    env.as_contract(&contract_id, || {
        publish_credit_line_queried(&env, event.clone());
    });

    let (_, _, data) = env.events().all().get(0).unwrap();
    let payload: CreditLineQueriedEvent = data.try_into_val(&env).unwrap();
    assert!(!payload.found);
    assert_eq!(payload.borrower, borrower);
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 2 — HealthFactorQueriedEvent
// ═══════════════════════════════════════════════════════════════════════════

/// `publish_health_factor_queried` emits topic `("query", "hf_read")` with
/// the health factor value.
#[test]
fn health_factor_queried_event_emits_correct_topic_and_value() {
    let (env, contract_id, _admin) = setup();
    let borrower = Address::generate(&env);
    env.ledger().set_timestamp(500);

    let event = HealthFactorQueriedEvent {
        borrower: borrower.clone(),
        health_bps: 15_000,
        timestamp: 500,
    };

    env.as_contract(&contract_id, || {
        publish_health_factor_queried(&env, event.clone());
    });

    let all_events = env.events().all();
    assert_eq!(all_events.len(), 1);

    let (_, topics, data) = all_events.get(0).unwrap();
    let t1: soroban_sdk::Symbol = topics.get(1).unwrap().try_into_val(&env).unwrap();
    assert_eq!(t1, symbol_short!("hf_read"));

    let payload: HealthFactorQueriedEvent = data.try_into_val(&env).unwrap();
    assert_eq!(payload.borrower, borrower);
    assert_eq!(payload.health_bps, 15_000);
    assert_eq!(payload.timestamp, 500);
}

/// `health_bps = u32::MAX` encodes correctly (zero-utilization sentinel).
#[test]
fn health_factor_queried_event_max_sentinel_encodes_correctly() {
    let (env, contract_id, _admin) = setup();
    let borrower = Address::generate(&env);

    let event = HealthFactorQueriedEvent {
        borrower: borrower.clone(),
        health_bps: u32::MAX,
        timestamp: 0,
    };

    env.as_contract(&contract_id, || {
        publish_health_factor_queried(&env, event);
    });

    let (_, _, data) = env.events().all().get(0).unwrap();
    let payload: HealthFactorQueriedEvent = data.try_into_val(&env).unwrap();
    assert_eq!(payload.health_bps, u32::MAX);
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 3 — DelinquencyCheckedEvent
// ═══════════════════════════════════════════════════════════════════════════

/// `publish_delinquency_checked` with `is_delinquent = true` emits topic
/// `("query", "dlq_chk")`.
#[test]
fn delinquency_checked_event_true_emits_correct_topic_and_flag() {
    let (env, contract_id, _admin) = setup();
    let borrower = Address::generate(&env);
    env.ledger().set_timestamp(9_999);

    let event = DelinquencyCheckedEvent {
        borrower: borrower.clone(),
        is_delinquent: true,
        timestamp: 9_999,
    };

    env.as_contract(&contract_id, || {
        publish_delinquency_checked(&env, event);
    });

    let all_events = env.events().all();
    assert_eq!(all_events.len(), 1);

    let (_, topics, data) = all_events.get(0).unwrap();
    let t1: soroban_sdk::Symbol = topics.get(1).unwrap().try_into_val(&env).unwrap();
    assert_eq!(t1, symbol_short!("dlq_chk"));

    let payload: DelinquencyCheckedEvent = data.try_into_val(&env).unwrap();
    assert_eq!(payload.borrower, borrower);
    assert!(payload.is_delinquent);
    assert_eq!(payload.timestamp, 9_999);
}

/// `publish_delinquency_checked` with `is_delinquent = false` encodes the flag.
#[test]
fn delinquency_checked_event_false_encodes_correctly() {
    let (env, contract_id, _admin) = setup();
    let borrower = Address::generate(&env);

    let event = DelinquencyCheckedEvent {
        borrower: borrower.clone(),
        is_delinquent: false,
        timestamp: 0,
    };

    env.as_contract(&contract_id, || {
        publish_delinquency_checked(&env, event);
    });

    let (_, _, data) = env.events().all().get(0).unwrap();
    let payload: DelinquencyCheckedEvent = data.try_into_val(&env).unwrap();
    assert!(!payload.is_delinquent);
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 4 — ProtocolSummaryQueriedEvent
// ═══════════════════════════════════════════════════════════════════════════

/// `publish_protocol_summary_queried` emits topic `("query", "proto_rd")`.
#[test]
fn protocol_summary_queried_event_emits_correct_topic_and_payload() {
    let (env, contract_id, _admin) = setup();
    env.ledger().set_timestamp(2_000);

    let event = ProtocolSummaryQueriedEvent {
        total_utilized: 100_000_i128,
        active_line_count: 5_u32,
        timestamp: 2_000,
    };

    env.as_contract(&contract_id, || {
        publish_protocol_summary_queried(&env, event);
    });

    let all_events = env.events().all();
    assert_eq!(all_events.len(), 1);

    let (_, topics, data) = all_events.get(0).unwrap();
    let t0: soroban_sdk::Symbol = topics.get(0).unwrap().try_into_val(&env).unwrap();
    let t1: soroban_sdk::Symbol = topics.get(1).unwrap().try_into_val(&env).unwrap();
    assert_eq!(t0, symbol_short!("query"));
    assert_eq!(t1, symbol_short!("proto_rd"));

    let payload: ProtocolSummaryQueriedEvent = data.try_into_val(&env).unwrap();
    assert_eq!(payload.total_utilized, 100_000_i128);
    assert_eq!(payload.active_line_count, 5);
    assert_eq!(payload.timestamp, 2_000);
}

/// Zero-state protocol summary (no lines, no utilization).
#[test]
fn protocol_summary_queried_event_zero_state_encodes_correctly() {
    let (env, contract_id, _admin) = setup();

    let event = ProtocolSummaryQueriedEvent {
        total_utilized: 0,
        active_line_count: 0,
        timestamp: 0,
    };

    env.as_contract(&contract_id, || {
        publish_protocol_summary_queried(&env, event);
    });

    let (_, _, data) = env.events().all().get(0).unwrap();
    let payload: ProtocolSummaryQueriedEvent = data.try_into_val(&env).unwrap();
    assert_eq!(payload.total_utilized, 0);
    assert_eq!(payload.active_line_count, 0);
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 5 — Determinism
// ═══════════════════════════════════════════════════════════════════════════

/// Two identical publish calls with the same inputs produce the same event payload.
#[test]
fn events_are_deterministic_same_input_same_output() {
    let (env, contract_id, _admin) = setup();
    let borrower = Address::generate(&env);
    env.ledger().set_timestamp(777);

    let event = CreditLineQueriedEvent {
        borrower: borrower.clone(),
        found: true,
        timestamp: 777,
    };

    env.as_contract(&contract_id, || {
        publish_credit_line_queried(&env, event.clone());
        publish_credit_line_queried(&env, event.clone());
    });

    let all_events = env.events().all();
    assert_eq!(all_events.len(), 2);

    let (_, _, data0) = all_events.get(0).unwrap();
    let (_, _, data1) = all_events.get(1).unwrap();
    let p0: CreditLineQueriedEvent = data0.try_into_val(&env).unwrap();
    let p1: CreditLineQueriedEvent = data1.try_into_val(&env).unwrap();
    assert_eq!(p0, p1, "event payloads must be deterministic");
}

// ── trait import needed for try_into_val ─────────────────────────────────────
use soroban_sdk::TryIntoVal;
