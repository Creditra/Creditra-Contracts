// SPDX-License-Identifier: MIT

//! Focused event tests for the freeze (v7) structured event module.
//!
//! # Coverage
//!
//! Validates every publisher function in `events.rs`:
//!
//! - Correct topic tuple published to the Soroban event ledger.
//! - Payload struct fields match the supplied arguments.
//! - Timestamp field is set from `env.ledger().timestamp()`.
//! - Freeze + unfreeze pairs emit separate events with correct `frozen` flag.
//!
//! # See also
//!
//! - `contracts/freeze/src/events.rs` — event definitions.
//! - `contracts/freeze/tests/auth_boundary.rs` — auth boundary tests.

use creditra_freeze::events::{
    publish_borrower_frozen, publish_borrower_unfrozen, publish_credit_line_frozen,
    publish_draws_frozen, BorrowerFrozenEvent, BorrowerUnfrozenEvent, CreditLineFrozenEvent,
    DrawsFrozenEvent,
};
use creditra_freeze::FreezeReason;
use soroban_sdk::{
    testutils::{Address as _, Events as _, Ledger},
    Address, Env, Symbol, TryFromVal, TryIntoVal, Val, Vec,
};

use creditra_credit::Credit;

const TEST_TS: u64 = 50_000;

fn make_env() -> (Env, Address) {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = TEST_TS);
    let contract_id = env.register(Credit, ());
    (env, contract_id)
}

// ── Helper: extract first event from log ────────────────────────────────

fn first_event(env: &Env) -> (Address, Vec<Val>, Val) {
    env.events()
        .all()
        .get(0)
        .expect("at least one event must be published")
}

// ── DrawsFrozenEvent ──────────────────────────────────────────────────────

/// publish_draws_frozen emits on topic `("freeze", "drw_frz")`.
#[test]
fn publish_draws_frozen_topic() {
    let (env, contract_id) = make_env();

    env.as_contract(&contract_id, || {
        publish_draws_frozen(&env, true, FreezeReason::LiquidityReserve);
    });

    let ev = first_event(&env);
    let topics = &ev.1;
    assert_eq!(topics.len(), 2, "topic must be a 2-tuple");
    assert_eq!(
        Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap(),
        Symbol::new(&env, "freeze"),
        "first topic must be 'freeze'"
    );
    assert_eq!(
        Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
        Symbol::new(&env, "drw_frz"),
        "second topic must be 'drw_frz'"
    );
}

/// publish_draws_frozen payload contains frozen=true, correct reason and timestamp.
#[test]
fn publish_draws_frozen_payload_frozen_true() {
    let (env, contract_id) = make_env();

    env.as_contract(&contract_id, || {
        publish_draws_frozen(&env, true, FreezeReason::LiquidityReserve);
    });

    let ev = first_event(&env);
    let payload: DrawsFrozenEvent = ev.2.try_into_val(&env).unwrap();
    assert!(payload.frozen, "frozen must be true");
    assert_eq!(
        payload.reason,
        FreezeReason::LiquidityReserve,
        "reason must match"
    );
    assert_eq!(payload.timestamp, TEST_TS, "timestamp must match ledger");
}

/// publish_draws_frozen with frozen=false represents an unfreeze action.
#[test]
fn publish_draws_frozen_payload_frozen_false() {
    let (env, contract_id) = make_env();

    env.as_contract(&contract_id, || {
        publish_draws_frozen(&env, false, FreezeReason::OperationalMaintenance);
    });

    let ev = first_event(&env);
    let payload: DrawsFrozenEvent = ev.2.try_into_val(&env).unwrap();
    assert!(!payload.frozen, "frozen must be false for unfreeze");
    assert_eq!(payload.reason, FreezeReason::OperationalMaintenance);
}

/// Freeze followed by unfreeze emits two separate DrawsFrozenEvents.
#[test]
fn publish_draws_frozen_two_events_for_freeze_unfreeze_pair() {
    let (env, contract_id) = make_env();

    env.as_contract(&contract_id, || {
        publish_draws_frozen(&env, true, FreezeReason::LiquidityReserve);
        env.ledger().with_mut(|li| li.timestamp = TEST_TS + 100);
        publish_draws_frozen(&env, false, FreezeReason::LiquidityReserve);
    });

    let all = env.events().all();
    assert_eq!(all.len(), 2, "must emit exactly two events");

    let freeze_ev: DrawsFrozenEvent = all.get(0).unwrap().2.try_into_val(&env).unwrap();
    let unfreeze_ev: DrawsFrozenEvent = all.get(1).unwrap().2.try_into_val(&env).unwrap();
    assert!(freeze_ev.frozen);
    assert!(!unfreeze_ev.frozen);
    assert_ne!(
        freeze_ev.timestamp, unfreeze_ev.timestamp,
        "timestamps must differ"
    );
}

/// DrawsFrozenEvent supports all FreezeReason variants.
#[test]
fn publish_draws_frozen_all_freeze_reasons() {
    for reason in [
        FreezeReason::LiquidityReserve,
        FreezeReason::RiskInvestigation,
        FreezeReason::Compliance,
        FreezeReason::OperationalMaintenance,
        FreezeReason::BorrowerRequest,
    ] {
        let (env, contract_id) = make_env();
        env.as_contract(&contract_id, || {
            publish_draws_frozen(&env, true, reason);
        });
        let ev = first_event(&env);
        let payload: DrawsFrozenEvent = ev.2.try_into_val(&env).unwrap();
        assert_eq!(
            payload.reason, reason,
            "reason must round-trip for {reason:?}"
        );
    }
}

// ── CreditLineFrozenEvent ─────────────────────────────────────────────────

/// publish_credit_line_frozen emits on topic `("freeze", "ln_frz")`.
#[test]
fn publish_credit_line_frozen_topic() {
    let (env, contract_id) = make_env();
    let borrower = Address::generate(&env);

    env.as_contract(&contract_id, || {
        publish_credit_line_frozen(&env, &borrower, true, FreezeReason::Compliance);
    });

    let ev = first_event(&env);
    let topics = &ev.1;
    assert_eq!(topics.len(), 2);
    assert_eq!(
        Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap(),
        Symbol::new(&env, "freeze")
    );
    assert_eq!(
        Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
        Symbol::new(&env, "ln_frz")
    );
}

/// publish_credit_line_frozen payload is correct for a freeze action.
#[test]
fn publish_credit_line_frozen_payload_freeze() {
    let (env, contract_id) = make_env();
    let borrower = Address::generate(&env);

    env.as_contract(&contract_id, || {
        publish_credit_line_frozen(&env, &borrower, true, FreezeReason::Compliance);
    });

    let ev = first_event(&env);
    let payload: CreditLineFrozenEvent = ev.2.try_into_val(&env).unwrap();
    assert_eq!(payload.borrower, borrower, "borrower must match");
    assert!(payload.frozen, "frozen must be true");
    assert_eq!(payload.reason, FreezeReason::Compliance);
    assert_eq!(payload.timestamp, TEST_TS);
}

/// publish_credit_line_frozen payload is correct for an unfreeze action.
#[test]
fn publish_credit_line_frozen_payload_unfreeze() {
    let (env, contract_id) = make_env();
    let borrower = Address::generate(&env);

    env.as_contract(&contract_id, || {
        publish_credit_line_frozen(&env, &borrower, false, FreezeReason::RiskInvestigation);
    });

    let ev = first_event(&env);
    let payload: CreditLineFrozenEvent = ev.2.try_into_val(&env).unwrap();
    assert!(!payload.frozen, "frozen must be false for unfreeze");
    assert_eq!(payload.reason, FreezeReason::RiskInvestigation);
}

/// Freeze + unfreeze for a credit line emits two independent events.
#[test]
fn publish_credit_line_frozen_two_events_for_pair() {
    let (env, contract_id) = make_env();
    let borrower = Address::generate(&env);

    env.as_contract(&contract_id, || {
        publish_credit_line_frozen(&env, &borrower, true, FreezeReason::Compliance);
        publish_credit_line_frozen(&env, &borrower, false, FreezeReason::Compliance);
    });

    let all = env.events().all();
    assert_eq!(all.len(), 2);
    let ev0: CreditLineFrozenEvent = all.get(0).unwrap().2.try_into_val(&env).unwrap();
    let ev1: CreditLineFrozenEvent = all.get(1).unwrap().2.try_into_val(&env).unwrap();
    assert!(ev0.frozen);
    assert!(!ev1.frozen);
}

/// publish_credit_line_frozen emits independent events for distinct borrowers.
#[test]
fn publish_credit_line_frozen_distinct_borrowers() {
    let (env, contract_id) = make_env();
    let borrower_a = Address::generate(&env);
    let borrower_b = Address::generate(&env);

    env.as_contract(&contract_id, || {
        publish_credit_line_frozen(&env, &borrower_a, true, FreezeReason::Compliance);
        publish_credit_line_frozen(
            &env,
            &borrower_b,
            true,
            FreezeReason::OperationalMaintenance,
        );
    });

    let all = env.events().all();
    assert_eq!(all.len(), 2);
    let ev_a: CreditLineFrozenEvent = all.get(0).unwrap().2.try_into_val(&env).unwrap();
    let ev_b: CreditLineFrozenEvent = all.get(1).unwrap().2.try_into_val(&env).unwrap();
    assert_eq!(ev_a.borrower, borrower_a);
    assert_eq!(ev_b.borrower, borrower_b);
    assert_ne!(ev_a.borrower, ev_b.borrower);
}

// ── BorrowerFrozenEvent ───────────────────────────────────────────────────

/// publish_borrower_frozen emits on topic `("freeze", "brw_frz")`.
#[test]
fn publish_borrower_frozen_topic() {
    let (env, contract_id) = make_env();
    let borrower = Address::generate(&env);

    env.as_contract(&contract_id, || {
        publish_borrower_frozen(&env, &borrower, TEST_TS + 3_600);
    });

    let ev = first_event(&env);
    let topics = &ev.1;
    assert_eq!(topics.len(), 2);
    assert_eq!(
        Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap(),
        Symbol::new(&env, "freeze")
    );
    assert_eq!(
        Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
        Symbol::new(&env, "brw_frz")
    );
}

/// publish_borrower_frozen payload contains correct borrower, expiry, and timestamp.
#[test]
fn publish_borrower_frozen_payload() {
    let (env, contract_id) = make_env();
    let borrower = Address::generate(&env);
    let expiry = TEST_TS + 86_400;

    env.as_contract(&contract_id, || {
        publish_borrower_frozen(&env, &borrower, expiry);
    });

    let ev = first_event(&env);
    let payload: BorrowerFrozenEvent = ev.2.try_into_val(&env).unwrap();
    assert_eq!(payload.borrower, borrower, "borrower must match");
    assert_eq!(payload.frozen_until, expiry, "frozen_until must match");
    assert_eq!(payload.timestamp, TEST_TS, "timestamp must match ledger");
}

/// publish_borrower_frozen with same-as-now expiry records correctly.
#[test]
fn publish_borrower_frozen_expiry_equals_now() {
    let (env, contract_id) = make_env();
    let borrower = Address::generate(&env);

    // frozen_until == now means already expired — still records the event.
    env.as_contract(&contract_id, || {
        publish_borrower_frozen(&env, &borrower, TEST_TS);
    });

    let ev = first_event(&env);
    let payload: BorrowerFrozenEvent = ev.2.try_into_val(&env).unwrap();
    assert_eq!(payload.frozen_until, TEST_TS);
}

// ── BorrowerUnfrozenEvent ─────────────────────────────────────────────────

/// publish_borrower_unfrozen emits on topic `("freeze", "brw_ufz")`.
#[test]
fn publish_borrower_unfrozen_topic() {
    let (env, contract_id) = make_env();
    let borrower = Address::generate(&env);

    env.as_contract(&contract_id, || {
        publish_borrower_unfrozen(&env, &borrower);
    });

    let ev = first_event(&env);
    let topics = &ev.1;
    assert_eq!(topics.len(), 2);
    assert_eq!(
        Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap(),
        Symbol::new(&env, "freeze")
    );
    assert_eq!(
        Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
        Symbol::new(&env, "brw_ufz")
    );
}

/// publish_borrower_unfrozen payload contains correct borrower and timestamp.
#[test]
fn publish_borrower_unfrozen_payload() {
    let (env, contract_id) = make_env();
    let borrower = Address::generate(&env);

    env.as_contract(&contract_id, || {
        publish_borrower_unfrozen(&env, &borrower);
    });

    let ev = first_event(&env);
    let payload: BorrowerUnfrozenEvent = ev.2.try_into_val(&env).unwrap();
    assert_eq!(payload.borrower, borrower, "borrower must match");
    assert_eq!(payload.timestamp, TEST_TS, "timestamp must match ledger");
}

// ── Freeze/unfreeze borrower round-trip ─────────────────────────────────

/// freeze_borrower_until followed by unfreeze_borrower emits distinct events on
/// distinct topics (`brw_frz` then `brw_ufz`).
#[test]
fn borrower_freeze_unfreeze_roundtrip_emits_two_different_events() {
    let (env, contract_id) = make_env();
    let borrower = Address::generate(&env);
    let expiry = TEST_TS + 3_600;

    env.as_contract(&contract_id, || {
        publish_borrower_frozen(&env, &borrower, expiry);
        env.ledger().with_mut(|li| li.timestamp = TEST_TS + 100);
        publish_borrower_unfrozen(&env, &borrower);
    });

    let all = env.events().all();
    assert_eq!(all.len(), 2, "must emit exactly two events");

    // First event: brw_frz
    let topics_0 = &all.get(0).unwrap().1;
    assert_eq!(
        Symbol::try_from_val(&env, &topics_0.get(1).unwrap()).unwrap(),
        Symbol::new(&env, "brw_frz"),
        "first event must be brw_frz"
    );

    // Second event: brw_ufz
    let topics_1 = &all.get(1).unwrap().1;
    assert_eq!(
        Symbol::try_from_val(&env, &topics_1.get(1).unwrap()).unwrap(),
        Symbol::new(&env, "brw_ufz"),
        "second event must be brw_ufz"
    );
}

// ── Namespace isolation ──────────────────────────────────────────────────

/// All four freeze publishers emit under the "freeze" first-topic namespace.
#[test]
fn all_publishers_use_freeze_namespace() {
    let (env, contract_id) = make_env();
    let borrower = Address::generate(&env);

    env.as_contract(&contract_id, || {
        publish_draws_frozen(&env, true, FreezeReason::LiquidityReserve);
        publish_credit_line_frozen(&env, &borrower, true, FreezeReason::Compliance);
        publish_borrower_frozen(&env, &borrower, TEST_TS + 1_000);
        publish_borrower_unfrozen(&env, &borrower);
    });

    let all = env.events().all();
    assert_eq!(all.len(), 4, "must emit exactly four events");

    for (i, ev) in all.iter().enumerate() {
        let first_topic = Symbol::try_from_val(&env, &ev.1.get(0).unwrap()).unwrap();
        assert_eq!(
            first_topic,
            Symbol::new(&env, "freeze"),
            "event[{i}] must use 'freeze' namespace"
        );
    }
}

// ── Struct derive sanity ──────────────────────────────────────────────────

/// All event structs support Clone, Debug, Eq, PartialEq derives.
#[test]
fn event_structs_derive_sanity() {
    let env = Env::default();
    let borrower = Address::generate(&env);

    let draws_ev = DrawsFrozenEvent {
        frozen: true,
        reason: FreezeReason::Compliance,
        timestamp: TEST_TS,
    };
    assert_eq!(draws_ev.clone(), draws_ev);
    let _ = format!("{draws_ev:?}");

    let line_ev = CreditLineFrozenEvent {
        borrower: borrower.clone(),
        frozen: true,
        reason: FreezeReason::Compliance,
        timestamp: TEST_TS,
    };
    assert_eq!(line_ev.clone(), line_ev);
    let _ = format!("{line_ev:?}");

    let brw_frz_ev = BorrowerFrozenEvent {
        borrower: borrower.clone(),
        frozen_until: TEST_TS + 100,
        timestamp: TEST_TS,
    };
    assert_eq!(brw_frz_ev.clone(), brw_frz_ev);
    let _ = format!("{brw_frz_ev:?}");

    let brw_ufz_ev = BorrowerUnfrozenEvent {
        borrower: borrower.clone(),
        timestamp: TEST_TS,
    };
    assert_eq!(brw_ufz_ev.clone(), brw_ufz_ev);
    let _ = format!("{brw_ufz_ev:?}");
}
