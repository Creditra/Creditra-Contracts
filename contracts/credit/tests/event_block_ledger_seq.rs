// SPDX-License-Identifier: MIT

//! Pin `BorrowerBlockedEvent.ledger` to `env.ledger().sequence()` at emission.
//!
//! These tests advance the ledger sequence between calls and assert the
//! `ledger` field of every emitted `BorrowerBlockedEvent` matches the
//! sequence number that was live when the event was published. This guards
//! against off-by-one regressions and ensures off-chain indexers can rely
//! on the field for chronological ordering.

use creditra_credit::events::BorrowerBlockedEvent;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Events};
use soroban_sdk::{Address, Env, Symbol};

fn setup(env: &Env) -> (CreditClient<'_>, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    (client, admin)
}

/// Advance the ledger sequence by `delta` ticks.
fn advance_ledger(env: &Env, delta: u32) {
    env.ledger().with_mut(|li| {
        li.sequence_number = li.sequence_number.saturating_add(delta);
    });
}

/// Return the last emitted `BorrowerBlockedEvent` (data only).
fn last_blocked_event(env: &Env) -> BorrowerBlockedEvent {
    let kind = Symbol::new(env, "blk_chg");
    for (_contract, topics, data) in env.events().all().iter().rev() {
        let t0 = Symbol::try_from_val(env, &topics.get(0).unwrap()).unwrap();
        if t0 == kind {
            return BorrowerBlockedEvent::try_from_val(env, &data).unwrap();
        }
    }
    panic!("No blk_chg event found");
}

// ── block_borrower ────────────────────────────────────────────────────────────

#[test]
fn block_emits_ledger_matching_sequence() {
    let env = Env::default();
    let (client, _admin) = setup(&env);
    let borrower = Address::generate(&env);

    advance_ledger(&env, 5);
    let seq = env.ledger().sequence();
    client.block_borrower(&_admin, &borrower);

    let event = last_blocked_event(&env);
    assert_eq!(event.ledger, seq, "block ledger must equal env sequence");
    assert!(event.blocked);
    assert_eq!(event.borrower, borrower);
}

#[test]
fn block_after_advancing_twice_matches_newer_sequence() {
    let env = Env::default();
    let (client, _admin) = setup(&env);
    let borrower = Address::generate(&env);

    advance_ledger(&env, 3);
    let seq1 = env.ledger().sequence();
    client.block_borrower(&_admin, &borrower);

    let ev1 = last_blocked_event(&env);
    assert_eq!(ev1.ledger, seq1);

    advance_ledger(&env, 10);
    let borrower2 = Address::generate(&env);
    let seq2 = env.ledger().sequence();
    client.block_borrower(&_admin, &borrower2);

    let ev2 = last_blocked_event(&env);
    assert_eq!(ev2.ledger, seq2, "second block must reflect later sequence");
    assert!(
        ev2.ledger > ev1.ledger,
        "ledger must be strictly increasing"
    );
}

// ── unblock_borrower ──────────────────────────────────────────────────────────

#[test]
fn unblock_emits_ledger_matching_sequence() {
    let env = Env::default();
    let (client, _admin) = setup(&env);
    let borrower = Address::generate(&env);

    client.block_borrower(&_admin, &borrower);

    advance_ledger(&env, 7);
    let seq = env.ledger().sequence();
    client.unblock_borrower(&_admin, &borrower);

    let event = last_blocked_event(&env);
    assert_eq!(event.ledger, seq, "unblock ledger must equal env sequence");
    assert!(!event.blocked, "unblock event must have blocked = false");
}

// ── block → unblock round-trip ────────────────────────────────────────────────

#[test]
fn block_then_unblock_ledger_values_differ() {
    let env = Env::default();
    let (client, _admin) = setup(&env);
    let borrower = Address::generate(&env);

    let seq_block = env.ledger().sequence();
    client.block_borrower(&_admin, &borrower);
    let ev_block = last_blocked_event(&env);
    assert_eq!(ev_block.ledger, seq_block);

    advance_ledger(&env, 1);
    let seq_unblock = env.ledger().sequence();
    client.unblock_borrower(&_admin, &borrower);
    let ev_unblock = last_blocked_event(&env);
    assert_eq!(ev_unblock.ledger, seq_unblock);
    assert!(ev_unblock.ledger > ev_block.ledger);
}

// ── bulk_block_borrowers ──────────────────────────────────────────────────────

#[test]
fn bulk_block_emits_one_event_per_borrower_all_with_same_ledger() {
    let env = Env::default();
    let (client, _admin) = setup(&env);

    let b1 = Address::generate(&env);
    let b2 = Address::generate(&env);
    let b3 = Address::generate(&env);

    advance_ledger(&env, 4);
    let seq = env.ledger().sequence();

    client.bulk_block_borrowers(
        &_admin,
        &soroban_sdk::vec![&env, b1.clone(), b2.clone(), b3.clone()],
    );

    // Collect all blk_chg events emitted after the clear point
    let kind = Symbol::new(&env, "blk_chg");
    let mut blocked_events: Vec<BorrowerBlockedEvent> = Vec::new();
    for (_contract, topics, data) in env.events().all().iter() {
        let t0 = Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap();
        if t0 == kind {
            blocked_events.push(BorrowerBlockedEvent::try_from_val(&env, &data).unwrap());
        }
    }

    assert_eq!(
        blocked_events.len(),
        3,
        "bulk block must emit exactly 3 events"
    );

    for ev in blocked_events.iter() {
        assert_eq!(
            ev.ledger, seq,
            "every bulk event must share the same ledger"
        );
        assert!(ev.blocked);
    }
}

#[test]
fn bulk_block_ledger_matches_at_emission_not_call_time() {
    let env = Env::default();
    let (client, _admin) = setup(&env);

    // Advance ledger before calling bulk_block
    advance_ledger(&env, 20);
    let seq = env.ledger().sequence();

    let b1 = Address::generate(&env);
    let b2 = Address::generate(&env);

    client.bulk_block_borrowers(&_admin, &soroban_sdk::vec![&env, b1.clone(), b2.clone()]);

    let kind = Symbol::new(&env, "blk_chg");
    let mut events = Vec::new();
    for (_contract, topics, data) in env.events().all().iter() {
        let t0 = Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap();
        if t0 == kind {
            events.push(BorrowerBlockedEvent::try_from_val(&env, &data).unwrap());
        }
    }

    assert_eq!(events.len(), 2);
    for ev in events.iter() {
        assert_eq!(
            ev.ledger, seq,
            "ledger must reflect the sequence at emission, not some earlier default"
        );
    }
}

// ── edge case: no advancement (default sequence) ──────────────────────────────

#[test]
fn block_at_default_sequence_matches() {
    let env = Env::default();
    let (client, _admin) = setup(&env);
    let borrower = Address::generate(&env);

    let seq = env.ledger().sequence();
    client.block_borrower(&_admin, &borrower);

    let event = last_blocked_event(&env);
    assert_eq!(
        event.ledger, seq,
        "must match even at the default ledger sequence"
    );
}
