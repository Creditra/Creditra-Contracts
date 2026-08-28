// SPDX-License-Identifier: MIT

//! Focused rustdoc-coverage tests for `creditra-query` public entrypoints.
//!
//! # Purpose
//!
//! Validates that every public API documented in the query crate behaves
//! exactly as its NatSpec-style rustdoc specifies, covering:
//!
//! - [`capabilities`] — every [`QueryCapabilities`] flag, all short-circuit
//!   paths, and the `delinquency_applicable` gate.
//! - [`publish_credit_line_queried`] — topic, payload fields, `found` flag.
//! - [`publish_health_factor_queried`] — topic, `health_bps` encoding, sentinel.
//! - [`publish_delinquency_checked`] — topic, `is_delinquent` flag.
//! - [`publish_protocol_summary_queried`] — topic, aggregate payload.
//! - Cross-cutting: determinism, no-mutation guarantee, zero-state edge cases.
//!
//! # Test categories
//!
//! | Section | What it covers |
//! |---------|---------------|
//! | §1 `capabilities` — no line | All-false bitmap when borrower is unknown |
//! | §2 `capabilities` — zero utilization | `has_credit_line` true, health/delinquency false |
//! | §3 `capabilities` — with utilization | `health_factor_applicable` gating |
//! | §4 `capabilities` — closed line | Delinquency cannot apply to closed lines |
//! | §5 `capabilities` — determinism | Same inputs → same outputs |
//! | §6 Event: CreditLineQueriedEvent | Topic, payload, `found` flag |
//! | §7 Event: HealthFactorQueriedEvent | Topic, `health_bps`, `u32::MAX` sentinel |
//! | §8 Event: DelinquencyCheckedEvent | Topic, boolean flag |
//! | §9 Event: ProtocolSummaryQueriedEvent | Topic, aggregate fields |
//! | §10 Event determinism | Same publish → same payload |
//! | §11 Events: no storage mutation | Events do not change credit line state |

use creditra_credit::{Credit, CreditClient};
use creditra_query::events::{
    publish_credit_line_queried, publish_delinquency_checked, publish_health_factor_queried,
    publish_protocol_summary_queried, CreditLineQueriedEvent, DelinquencyCheckedEvent,
    HealthFactorQueriedEvent, ProtocolSummaryQueriedEvent,
};
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events, Ledger},
    token, Address, Env, TryIntoVal,
};

// ── Shared test helpers ───────────────────────────────────────────────────────

/// Spin up a minimal test environment with a registered Credit contract and
/// an initialized admin. Returns `(env, client, contract_id, admin)`.
fn setup_env() -> (Env, CreditClient<'static>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    (env, client, contract_id, admin)
}

/// Extend `setup_env` with a minted liquidity token so `draw_credit` can
/// be called. Returns `(env, client, contract_id, admin)` with a token
/// registered and 1_000_000 units minted into the contract.
fn setup_with_token() -> (Env, CreditClient<'static>, Address, Address) {
    let (env, client, contract_id, admin) = setup_env();

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&contract_id);
    token::StellarAssetClient::new(&env, &token).mint(&contract_id, &1_000_000_i128);

    (env, client, contract_id, admin)
}

// ═══════════════════════════════════════════════════════════════════════════
// §1 — capabilities(): no credit line
// ═══════════════════════════════════════════════════════════════════════════

/// Rustdoc says: "any address may be passed; a missing credit line is handled
/// gracefully (all flags are `false`)."
#[test]
fn capabilities_no_credit_line_all_flags_false() {
    let (env, client, _, _) = setup_env();
    let borrower = Address::generate(&env);

    let caps = client.query_capabilities(&borrower);

    assert!(
        !caps.has_credit_line,
        "no line → has_credit_line must be false"
    );
    assert!(
        !caps.has_repayment_schedule,
        "no line → has_repayment_schedule must be false"
    );
    assert!(
        !caps.health_factor_applicable,
        "no line → health_factor_applicable must be false"
    );
    assert!(
        !caps.delinquency_applicable,
        "no line → delinquency_applicable must be false"
    );
    assert!(!caps.is_delinquent, "no line → is_delinquent must be false");
}

// ═══════════════════════════════════════════════════════════════════════════
// §2 — capabilities(): active line, zero utilization
// ═══════════════════════════════════════════════════════════════════════════

/// Rustdoc says: "`health_factor_applicable = utilized_amount > 0`; otherwise
/// `get_health_factor` returns `u32::MAX`."
/// With zero utilization: `has_credit_line = true`, health/delinquency = false.
#[test]
fn capabilities_active_line_zero_utilization() {
    let (env, client, _, _) = setup_env();
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000_i128, &500_u32, &50_u32);

    let caps = client.query_capabilities(&borrower);

    assert!(
        caps.has_credit_line,
        "open line → has_credit_line must be true"
    );
    assert!(
        !caps.health_factor_applicable,
        "zero utilization → health_factor_applicable must be false"
    );
    assert!(
        !caps.delinquency_applicable,
        "zero utilization → delinquency_applicable must be false"
    );
    assert!(!caps.is_delinquent);
}

// ═══════════════════════════════════════════════════════════════════════════
// §3 — capabilities(): line with utilization, no schedule
// ═══════════════════════════════════════════════════════════════════════════

/// Rustdoc says: `delinquency_applicable = open && utilized_amount > 0 && schedule exists`.
/// Without a schedule, `delinquency_applicable` must be false even with utilization.
#[test]
fn capabilities_line_with_utilization_no_schedule() {
    let (env, client, _, _) = setup_with_token();
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000_i128, &500_u32, &50_u32);
    // Bypass collateral ratio for draw test
    client.set_min_collateral_ratio_bps(&0);
    client.draw_credit(&borrower, &300_i128);

    let caps = client.query_capabilities(&borrower);

    assert!(caps.has_credit_line);
    assert!(
        caps.health_factor_applicable,
        "utilized > 0 → health_factor_applicable must be true"
    );
    assert!(
        !caps.delinquency_applicable,
        "no schedule → delinquency_applicable must be false"
    );
    assert!(!caps.is_delinquent);
}

// ═══════════════════════════════════════════════════════════════════════════
// §4 — capabilities(): closed line
// ═══════════════════════════════════════════════════════════════════════════

/// Rustdoc says: "`delinquency_applicable = open && …`; closed lines cannot
/// be delinquent."
#[test]
fn capabilities_closed_line_delinquency_not_applicable() {
    let (env, client, _, admin) = setup_env();
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000_i128, &500_u32, &50_u32);
    client.close_credit_line(&borrower, &admin);

    let caps = client.query_capabilities(&borrower);

    assert!(caps.has_credit_line, "closed line still present in storage");
    assert!(
        !caps.delinquency_applicable,
        "closed lines cannot be delinquent"
    );
    assert!(!caps.is_delinquent, "closed lines are never delinquent");
}

// ═══════════════════════════════════════════════════════════════════════════
// §5 — capabilities(): determinism
// ═══════════════════════════════════════════════════════════════════════════

/// Rustdoc says (implicit from "pure read with no mutation"): same inputs
/// → same outputs. Two consecutive calls must return identical bitmaps.
#[test]
fn capabilities_deterministic_consecutive_calls() {
    let (env, client, _, _) = setup_env();
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &2_000_i128, &600_u32, &40_u32);

    let caps_a = client.query_capabilities(&borrower);
    let caps_b = client.query_capabilities(&borrower);

    assert_eq!(
        caps_a, caps_b,
        "query_capabilities must be deterministic for unchanged state"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// §6 — publish_credit_line_queried
// ═══════════════════════════════════════════════════════════════════════════

/// Rustdoc says: emits topic `("query", "cl_read")` with the correct
/// `borrower`, `found`, and `timestamp` fields.
#[test]
fn publish_credit_line_queried_emits_correct_topic_found_true() {
    let (env, _, contract_id, _) = setup_env();
    let borrower = Address::generate(&env);
    env.ledger().set_timestamp(1_234);

    let event = CreditLineQueriedEvent {
        borrower: borrower.clone(),
        found: true,
        timestamp: 1_234,
    };

    env.as_contract(&contract_id, || {
        publish_credit_line_queried(&env, event.clone());
    });

    let all = env.events().all();
    assert_eq!(all.len(), 1, "exactly one event must be emitted");

    let (_, topics, data) = all.get(0).unwrap();
    let t0: soroban_sdk::Symbol = topics.get(0).unwrap().try_into_val(&env).unwrap();
    let t1: soroban_sdk::Symbol = topics.get(1).unwrap().try_into_val(&env).unwrap();
    assert_eq!(t0, symbol_short!("query"), "first topic must be 'query'");
    assert_eq!(
        t1,
        symbol_short!("cl_read"),
        "second topic must be 'cl_read'"
    );

    let payload: CreditLineQueriedEvent = data.try_into_val(&env).unwrap();
    assert_eq!(payload.borrower, borrower);
    assert!(payload.found);
    assert_eq!(payload.timestamp, 1_234);
}

/// Rustdoc says `found = false` is a valid and distinct encoded state.
#[test]
fn publish_credit_line_queried_found_false_encodes_correctly() {
    let (env, _, contract_id, _) = setup_env();
    let borrower = Address::generate(&env);
    env.ledger().set_timestamp(99);

    let event = CreditLineQueriedEvent {
        borrower: borrower.clone(),
        found: false,
        timestamp: 99,
    };

    env.as_contract(&contract_id, || {
        publish_credit_line_queried(&env, event);
    });

    let (_, _, data) = env.events().all().get(0).unwrap();
    let payload: CreditLineQueriedEvent = data.try_into_val(&env).unwrap();
    assert!(!payload.found, "found=false must round-trip correctly");
    assert_eq!(payload.timestamp, 99);
}

// ═══════════════════════════════════════════════════════════════════════════
// §7 — publish_health_factor_queried
// ═══════════════════════════════════════════════════════════════════════════

/// Rustdoc says: emits topic `("query", "hf_read")` with `health_bps` and `timestamp`.
#[test]
fn publish_health_factor_queried_emits_correct_topic_and_value() {
    let (env, _, contract_id, _) = setup_env();
    let borrower = Address::generate(&env);
    env.ledger().set_timestamp(500);

    let event = HealthFactorQueriedEvent {
        borrower: borrower.clone(),
        health_bps: 15_000,
        timestamp: 500,
    };

    env.as_contract(&contract_id, || {
        publish_health_factor_queried(&env, event);
    });

    let (_, topics, data) = env.events().all().get(0).unwrap();
    let t1: soroban_sdk::Symbol = topics.get(1).unwrap().try_into_val(&env).unwrap();
    assert_eq!(
        t1,
        symbol_short!("hf_read"),
        "second topic must be 'hf_read'"
    );

    let payload: HealthFactorQueriedEvent = data.try_into_val(&env).unwrap();
    assert_eq!(payload.borrower, borrower);
    assert_eq!(payload.health_bps, 15_000);
    assert_eq!(payload.timestamp, 500);
}

/// Rustdoc says: "`u32::MAX` = zero utilization sentinel". Must encode correctly.
#[test]
fn publish_health_factor_queried_max_sentinel_encodes_correctly() {
    let (env, _, contract_id, _) = setup_env();
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
    assert_eq!(
        payload.health_bps,
        u32::MAX,
        "u32::MAX sentinel must round-trip through event encoding"
    );
}

/// Rustdoc says: `health_bps < 10_000` means under-collateralized.
/// Verify the field value round-trips at the boundary.
#[test]
fn publish_health_factor_queried_under_collateralized_boundary() {
    let (env, _, contract_id, _) = setup_env();
    let borrower = Address::generate(&env);

    let event = HealthFactorQueriedEvent {
        borrower: borrower.clone(),
        health_bps: 9_999, // just under the 10_000 minimum
        timestamp: 0,
    };

    env.as_contract(&contract_id, || {
        publish_health_factor_queried(&env, event);
    });

    let (_, _, data) = env.events().all().get(0).unwrap();
    let payload: HealthFactorQueriedEvent = data.try_into_val(&env).unwrap();
    assert_eq!(payload.health_bps, 9_999);
}

// ═══════════════════════════════════════════════════════════════════════════
// §8 — publish_delinquency_checked
// ═══════════════════════════════════════════════════════════════════════════

/// Rustdoc says: emits topic `("query", "dlq_chk")` with `is_delinquent = true`.
#[test]
fn publish_delinquency_checked_true_emits_correct_topic() {
    let (env, _, contract_id, _) = setup_env();
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

    let (_, topics, data) = env.events().all().get(0).unwrap();
    let t1: soroban_sdk::Symbol = topics.get(1).unwrap().try_into_val(&env).unwrap();
    assert_eq!(
        t1,
        symbol_short!("dlq_chk"),
        "second topic must be 'dlq_chk'"
    );

    let payload: DelinquencyCheckedEvent = data.try_into_val(&env).unwrap();
    assert_eq!(payload.borrower, borrower);
    assert!(payload.is_delinquent);
    assert_eq!(payload.timestamp, 9_999);
}

/// Rustdoc says: `is_delinquent = false` is safe to emit and is the
/// common case when short-circuit conditions are not met.
#[test]
fn publish_delinquency_checked_false_encodes_correctly() {
    let (env, _, contract_id, _) = setup_env();
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
    assert!(
        !payload.is_delinquent,
        "is_delinquent=false must round-trip"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// §9 — publish_protocol_summary_queried
// ═══════════════════════════════════════════════════════════════════════════

/// Rustdoc says: emits topic `("query", "proto_rd")` with `total_utilized`,
/// `active_line_count`, and `timestamp`.
#[test]
fn publish_protocol_summary_queried_emits_correct_topic_and_payload() {
    let (env, _, contract_id, _) = setup_env();
    env.ledger().set_timestamp(2_000);

    let event = ProtocolSummaryQueriedEvent {
        total_utilized: 500_000_i128,
        active_line_count: 7_u32,
        timestamp: 2_000,
    };

    env.as_contract(&contract_id, || {
        publish_protocol_summary_queried(&env, event);
    });

    let (_, topics, data) = env.events().all().get(0).unwrap();
    let t0: soroban_sdk::Symbol = topics.get(0).unwrap().try_into_val(&env).unwrap();
    let t1: soroban_sdk::Symbol = topics.get(1).unwrap().try_into_val(&env).unwrap();
    assert_eq!(t0, symbol_short!("query"));
    assert_eq!(
        t1,
        symbol_short!("proto_rd"),
        "second topic must be 'proto_rd'"
    );

    let payload: ProtocolSummaryQueriedEvent = data.try_into_val(&env).unwrap();
    assert_eq!(payload.total_utilized, 500_000_i128);
    assert_eq!(payload.active_line_count, 7);
    assert_eq!(payload.timestamp, 2_000);
}

/// Rustdoc says: zero-state (no lines, no utilization) is valid.
#[test]
fn publish_protocol_summary_queried_zero_state() {
    let (env, _, contract_id, _) = setup_env();

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
// §10 — Event determinism
// ═══════════════════════════════════════════════════════════════════════════

/// Rustdoc says (implicit): two publish calls with identical inputs produce
/// identical event payloads.
#[test]
fn events_deterministic_same_input_produces_same_output() {
    let (env, _, contract_id, _) = setup_env();
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

    let all = env.events().all();
    assert_eq!(all.len(), 2, "two publishes → two events");

    let (_, _, data0) = all.get(0).unwrap();
    let (_, _, data1) = all.get(1).unwrap();
    let p0: CreditLineQueriedEvent = data0.try_into_val(&env).unwrap();
    let p1: CreditLineQueriedEvent = data1.try_into_val(&env).unwrap();
    assert_eq!(p0, p1, "event payloads must be deterministic");
}

/// All four publisher helpers produce independent, non-interfering events.
#[test]
fn all_four_publishers_emit_independent_events() {
    let (env, _, contract_id, _) = setup_env();
    let borrower = Address::generate(&env);
    env.ledger().set_timestamp(100);

    env.as_contract(&contract_id, || {
        publish_credit_line_queried(
            &env,
            CreditLineQueriedEvent {
                borrower: borrower.clone(),
                found: true,
                timestamp: 100,
            },
        );
        publish_health_factor_queried(
            &env,
            HealthFactorQueriedEvent {
                borrower: borrower.clone(),
                health_bps: 20_000,
                timestamp: 100,
            },
        );
        publish_delinquency_checked(
            &env,
            DelinquencyCheckedEvent {
                borrower: borrower.clone(),
                is_delinquent: false,
                timestamp: 100,
            },
        );
        publish_protocol_summary_queried(
            &env,
            ProtocolSummaryQueriedEvent {
                total_utilized: 0,
                active_line_count: 0,
                timestamp: 100,
            },
        );
    });

    let all = env.events().all();
    assert_eq!(all.len(), 4, "four publishers → four distinct events");
}

// ═══════════════════════════════════════════════════════════════════════════
// §11 — Events do not mutate contract state
// ═══════════════════════════════════════════════════════════════════════════

/// Rustdoc says: events are "opt-in" and purely additive; they do not change
/// credit line state. Verify the credit line is unchanged after publishing.
#[test]
fn publishing_events_does_not_mutate_credit_line_state() {
    let (env, client, contract_id, _) = setup_env();
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000_i128, &500_u32, &50_u32);

    // Snapshot state before publishing events.
    let line_before = client.get_credit_line(&borrower).unwrap();

    // Publish all four event types.
    env.as_contract(&contract_id, || {
        publish_credit_line_queried(
            &env,
            CreditLineQueriedEvent {
                borrower: borrower.clone(),
                found: true,
                timestamp: 0,
            },
        );
        publish_health_factor_queried(
            &env,
            HealthFactorQueriedEvent {
                borrower: borrower.clone(),
                health_bps: u32::MAX,
                timestamp: 0,
            },
        );
        publish_delinquency_checked(
            &env,
            DelinquencyCheckedEvent {
                borrower: borrower.clone(),
                is_delinquent: false,
                timestamp: 0,
            },
        );
        publish_protocol_summary_queried(
            &env,
            ProtocolSummaryQueriedEvent {
                total_utilized: 0,
                active_line_count: 0,
                timestamp: 0,
            },
        );
    });

    // Credit line state must be identical after event emission.
    let line_after = client.get_credit_line(&borrower).unwrap();
    assert_eq!(
        line_before.credit_limit, line_after.credit_limit,
        "credit_limit must not change after event emission"
    );
    assert_eq!(
        line_before.status, line_after.status,
        "status must not change after event emission"
    );
    assert_eq!(
        line_before.utilized_amount, line_after.utilized_amount,
        "utilized_amount must not change after event emission"
    );
}
