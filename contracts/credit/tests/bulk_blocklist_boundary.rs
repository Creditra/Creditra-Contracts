// SPDX-License-Identifier: MIT

//! Boundary and event-count tests for the admin bulk blocklist entrypoint.
//!
//! Issue #1331: `bulk_block_borrowers` enforces `BULK_BLOCK_MAX = 50` after
//! authorization and emits exactly one `BorrowerBlockedEvent` per list entry.
//! These tests pin the contract that previously had no coverage:
//!
//! | Case | Expectation |
//! |------|-------------|
//! | exactly 50 unique entries | succeeds; one event per entry |
//! | 51 entries | reverts `InvalidAmount`; no writes, no events |
//! | duplicates | idempotent; one event per *entry*, not per unique borrower |
//! | empty list | succeeds; zero events |
//! | every accepted entry | `is_borrower_blocked` returns `true` |
//!
//! The batch cap protects the contract's bounded-gas guarantee; a regression
//! that let 51+ entries through would allow an unbounded loop, while one that
//! rejected 50 would break valid administrative batches.

use creditra_credit::events::BorrowerBlockedEvent;
use creditra_credit::types::ContractError;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Events};
use soroban_sdk::{Address, Env, Symbol, TryFromVal, TryIntoVal, Vec};

/// Mirrors `BULK_BLOCK_MAX` in `contracts/credit/src/lib.rs`.
const BULK_BLOCK_MAX: u32 = 50;

fn setup(env: &Env) -> (CreditClient<'_>, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    (client, admin)
}

/// Build a `soroban_sdk::Vec` of `n` fresh (unique) borrower addresses.
fn unique_borrowers(env: &Env, n: u32) -> Vec<Address> {
    let mut borrowers = Vec::new(env);
    for _ in 0..n {
        borrowers.push_back(Address::generate(env));
    }
    borrowers
}

/// Collect every `("blk_chg",)` blocklist event in the current event buffer.
///
/// Returns an owned `soroban_sdk::Vec` so callers may read the payloads after
/// subsequent contract calls (which clear the host event buffer).
fn blk_chg_events(env: &Env) -> Vec<BorrowerBlockedEvent> {
    let topic = Symbol::new(env, "blk_chg");
    let mut events = Vec::new(env);

    for (_contract, topics, data) in env.events().all().iter() {
        // Blocklist events use a single-element topic tuple.
        if topics.len() != 1 {
            continue;
        }

        let Some(t0) = Symbol::try_from_val(env, &topics.get(0).unwrap()).ok() else {
            continue;
        };
        if t0 != topic {
            continue;
        }

        let event: BorrowerBlockedEvent = data
            .try_into_val(env)
            .unwrap_or_else(|_| unreachable!("blk_chg payload must decode"));
        events.push_back(event);
    }

    events
}

#[test]
fn blocklist_bulk_exactly_max_succeeds_with_one_event_per_entry() {
    let env = Env::default();
    let (client, admin) = setup(&env);
    let borrowers = unique_borrowers(&env, BULK_BLOCK_MAX);

    client.bulk_block_borrowers(&admin, &borrowers);

    // Event count equals list length, and each event mirrors its list entry.
    let events = blk_chg_events(&env);
    assert_eq!(
        events.len(),
        BULK_BLOCK_MAX,
        "one BorrowerBlockedEvent per entry"
    );

    for (index, event) in events.iter().enumerate() {
        assert_eq!(
            event.borrower,
            borrowers.get(index as u32).unwrap(),
            "event borrower must follow list order"
        );
        assert!(event.blocked, "bulk block must emit blocked = true");
    }

    // Every accepted borrower is readable as blocked.
    for borrower in borrowers.iter() {
        assert!(
            client.is_borrower_blocked(&borrower),
            "every accepted borrower must be blocked"
        );
    }
}

#[test]
fn blocklist_bulk_above_max_reverts_with_invalid_amount_and_changes_nothing() {
    let env = Env::default();
    let (client, admin) = setup(&env);
    let borrowers = unique_borrowers(&env, BULK_BLOCK_MAX + 1);

    let rejected = client
        .try_bulk_block_borrowers(&admin, &borrowers)
        .err()
        .expect("51-entry batch must revert")
        .expect("the revert must be a typed contract error");

    assert_eq!(rejected, ContractError::InvalidAmount.into());

    // The guard runs before any write, so the rejected batch leaves no trace:
    // no events and no blocklist entries.
    assert_eq!(
        blk_chg_events(&env).len(),
        0_u32,
        "rejected batch must emit no events"
    );
    for borrower in borrowers.iter() {
        assert!(
            !client.is_borrower_blocked(&borrower),
            "rejected batch must not block any borrower"
        );
    }
}

#[test]
fn blocklist_bulk_duplicates_are_idempotent_with_one_event_per_entry() {
    let env = Env::default();
    let (client, admin) = setup(&env);

    let first = Address::generate(&env);
    let second = Address::generate(&env);

    // A full-sized batch that alternates between two already-known addresses.
    let mut list = Vec::new(&env);
    for index in 0..BULK_BLOCK_MAX {
        if index % 2 == 0 {
            list.push_back(first.clone());
        } else {
            list.push_back(second.clone());
        }
    }

    client.bulk_block_borrowers(&admin, &list);

    // One event per list entry — duplicates are not collapsed.
    assert_eq!(
        blk_chg_events(&env).len(),
        BULK_BLOCK_MAX,
        "duplicates still produce one event each"
    );
    assert!(client.is_borrower_blocked(&first));
    assert!(client.is_borrower_blocked(&second));

    // Re-blocking the same list is idempotent: state is unchanged and the
    // event stream still mirrors the request size.
    client.bulk_block_borrowers(&admin, &list);
    assert_eq!(blk_chg_events(&env).len(), BULK_BLOCK_MAX);
    assert!(client.is_borrower_blocked(&first));
    assert!(client.is_borrower_blocked(&second));
}

#[test]
fn blocklist_bulk_empty_list_succeeds_with_no_events() {
    let env = Env::default();
    let (client, admin) = setup(&env);

    let empty: Vec<Address> = Vec::new(&env);
    client.bulk_block_borrowers(&admin, &empty);

    assert_eq!(blk_chg_events(&env).len(), 0_u32);
}
