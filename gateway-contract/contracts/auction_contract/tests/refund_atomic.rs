//! Atomic-refund invariants during auction close.
//!
//! In English mode every outbid immediately and atomically refunds the
//! displaced bidder under the reentrancy guard — there is no loser-list
//! batch at close time. By the time `close_auction` is called every
//! prior bidder has already been repaid exactly once. These tests verify:
//!
//! | Test | Invariant |
//! |------|-----------|
//! | `close_auction_emits_no_refund_events` | `close_auction` emits **zero** `BID_RFDN` |
//! | `each_outbid_emits_exactly_one_refund_event` | N bids → N − 1 `BID_RFDN`, one per outbid |
//! | `close_with_no_bids_is_safe` | zero-bid close: no panic, no refund event |
//! | `close_with_single_bid_no_refund_event` | sole bidder never displaced, no refund |
//! | `refund_event_names_correct_prev_bidder_and_amount` | `BID_RFDN` payload is prev-bidder + prev-amount |
//! | `close_auction_does_not_mutate_winner_or_bid` | `close_auction` is a pure status flip |
//! | `round_robin_refund_per_outbid_step` | A→B→C cycling; each step names the correct displaced holder |
//!
//! # API change
//!
//! `BidRefundedEvent` is now re-exported from the crate root so integration
//! tests can deserialise the raw event payload without accessing private
//! modules. The re-export is additive and backwards-compatible.
//!
//! # Running
//!
//! ```bash
//! cargo test -p gateway-auction --test refund_atomic
//! ```

use gateway_auction::{
    Auction, AuctionClient, AuctionMode, AuctionState, AuctionStatus, BidRefundedEvent,
};
use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{Address, Env, Symbol, TryFromVal, TryIntoVal};

const BID_RFDN: &str = "BID_RFDN";
const AUC_CLOSE: &str = "AUC_CLOSE";

// ── Helpers ──────────────────────────────────────────────────────────────────

fn fresh_env() -> Env {
    let env = Env::default();
    env.mock_all_auths();
    env
}

/// Register a fresh auction contract and return its address.
///
/// The caller creates the `AuctionClient` locally:
/// ```ignore
/// let id = register(&env);
/// let client = AuctionClient::new(&env, &id);
/// ```
/// This avoids a borrow conflict: `AuctionClient<'a>` holds `&'a Address`,
/// so `id` must outlive `client` in the same scope.
fn register(env: &Env) -> Address {
    env.register(Auction, ())
}

/// Initialise a long-lived English auction (min_bid = 1, no increment requirement).
fn open_english(client: &AuctionClient<'_>, id: &Symbol) {
    client.init_auction(
        id,
        &AuctionMode::English,
        &0_u64,
        &u64::MAX,
        &1_i128,
        &0_u32,
        &None,
        &None,
        &gateway_auction::DutchAuctionDecay::None,
        &None,
    );
}

/// Count events whose first topic matches `topic` in the most-recent call.
///
/// In soroban-sdk v22 `env.events().all()` returns only the events emitted
/// by the last successful host-function invocation.
fn count_topic(env: &Env, topic: &str) -> usize {
    env.events()
        .all()
        .iter()
        .filter(|(_, topics, _)| {
            topics
                .get(0)
                .and_then(|v| Symbol::try_from_val(env, &v).ok())
                .map(|t: Symbol| t == Symbol::new(env, topic))
                .unwrap_or(false)
        })
        .count()
}

/// Collect and deserialise every `BID_RFDN` payload from the most-recent call.
fn collect_refund_events(env: &Env) -> std::vec::Vec<BidRefundedEvent> {
    let mut out = std::vec::Vec::new();
    for (_, topics, data) in env.events().all().iter() {
        let t0: Symbol = Symbol::try_from_val(env, &topics.get(0).unwrap()).unwrap();
        if t0 == Symbol::new(env, BID_RFDN) {
            let evt: BidRefundedEvent = data.try_into_val(env).unwrap();
            out.push(evt);
        }
    }
    out
}

/// Read the persisted `AuctionState` directly from contract storage.
fn read_state(env: &Env, contract_id: &Address, auction_id: &Symbol) -> AuctionState {
    env.as_contract(contract_id, || env.storage().persistent().get(auction_id))
        .expect("auction state must exist in persistent storage")
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// `close_auction` must not emit any `BID_RFDN` events.
///
/// Refunds are emitted atomically at outbid time; `close_auction` is a
/// pure status transition from `Open` to `Closed`.
#[test]
fn close_auction_emits_no_refund_events() {
    let env = fresh_env();
    let id = register(&env);
    let client = AuctionClient::new(&env, &id);
    let aid = Symbol::new(&env, "ra_close1");
    open_english(&client, &aid);

    let b0 = Address::generate(&env);
    let b1 = Address::generate(&env);
    let b2 = Address::generate(&env);
    let b3 = Address::generate(&env);

    client.place_bid(&aid, &b0, &100_i128);
    client.place_bid(&aid, &b1, &200_i128);
    client.place_bid(&aid, &b2, &400_i128);
    client.place_bid(&aid, &b3, &700_i128);

    // Only events emitted by this specific call are visible.
    client.close_auction(&aid);

    assert_eq!(
        count_topic(&env, BID_RFDN),
        0,
        "close_auction must emit zero BID_RFDN events — refunds are per-outbid, not batched at close"
    );
    assert_eq!(
        count_topic(&env, AUC_CLOSE),
        1,
        "close_auction must emit exactly one AUC_CLOSE event"
    );
}

/// The first bid has no prior holder and must emit zero `BID_RFDN` events.
/// Every subsequent bid displaces exactly one holder: one `BID_RFDN` per call.
#[test]
fn each_outbid_emits_exactly_one_refund_event() {
    let env = fresh_env();
    let id = register(&env);
    let client = AuctionClient::new(&env, &id);
    let aid = Symbol::new(&env, "ra_peroutbid");
    open_english(&client, &aid);

    let bidders: [Address; 5] = [
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
    ];
    let amounts: [i128; 5] = [100, 200, 400, 700, 1_100];

    for (i, (bidder, &amount)) in bidders.iter().zip(amounts.iter()).enumerate() {
        client.place_bid(&aid, bidder, &amount);
        let expected = if i == 0 { 0 } else { 1 };
        assert_eq!(
            count_topic(&env, BID_RFDN),
            expected,
            "bid {i}: expected {expected} BID_RFDN event(s) from this call"
        );
    }
}

/// Closing an auction that received zero bids must not panic and must not emit
/// any `BID_RFDN` event.
#[test]
fn close_with_no_bids_is_safe() {
    let env = fresh_env();
    let id = register(&env);
    let client = AuctionClient::new(&env, &id);
    let aid = Symbol::new(&env, "ra_zerobid");
    open_english(&client, &aid);

    client.close_auction(&aid);

    assert_eq!(
        count_topic(&env, BID_RFDN),
        0,
        "zero-bid close must not emit a refund event"
    );
    assert_eq!(
        count_topic(&env, AUC_CLOSE),
        1,
        "zero-bid close must still emit AUC_CLOSE"
    );
}

/// A single bidder was never outbid, so `close_auction` must emit no `BID_RFDN`.
#[test]
fn close_with_single_bid_no_refund_event() {
    let env = fresh_env();
    let id = register(&env);
    let client = AuctionClient::new(&env, &id);
    let aid = Symbol::new(&env, "ra_onebid");
    open_english(&client, &aid);

    client.place_bid(&aid, &Address::generate(&env), &500_i128);
    client.close_auction(&aid);

    assert_eq!(
        count_topic(&env, BID_RFDN),
        0,
        "sole bidder is never displaced — close_auction must not emit BID_RFDN"
    );
}

/// Each `BID_RFDN` payload must carry the **previous** bidder's address and
/// the **previous** bid amount, not the new bidder or the new amount.
#[test]
fn refund_event_names_correct_prev_bidder_and_amount() {
    let env = fresh_env();
    let id = register(&env);
    let client = AuctionClient::new(&env, &id);
    let aid = Symbol::new(&env, "ra_evtcorrect");
    open_english(&client, &aid);

    let alice = Address::generate(&env);
    let bob = Address::generate(&env);

    client.place_bid(&aid, &alice, &300_i128);
    client.place_bid(&aid, &bob, &700_i128);

    let events = collect_refund_events(&env);
    assert_eq!(
        events.len(),
        1,
        "bob outbidding alice must produce exactly one BID_RFDN"
    );
    assert_eq!(
        events[0].prev_bidder, alice,
        "BID_RFDN must name the displaced bidder (alice)"
    );
    assert_eq!(
        events[0].amount, 300_i128,
        "BID_RFDN must carry alice's original bid amount"
    );
}

/// `close_auction` must not alter `highest_bidder` or `highest_bid` — it is a
/// status flip only (`Open` → `Closed`).
#[test]
fn close_auction_does_not_mutate_winner_or_bid() {
    let env = fresh_env();
    let id = register(&env);
    let client = AuctionClient::new(&env, &id);
    let aid = Symbol::new(&env, "ra_nomutate");
    open_english(&client, &aid);

    let winner = Address::generate(&env);
    client.place_bid(&aid, &winner, &888_i128);

    let before = read_state(&env, &id, &aid);
    client.close_auction(&aid);
    let after = read_state(&env, &id, &aid);

    assert_eq!(
        after.highest_bidder, before.highest_bidder,
        "close_auction must not change the winner"
    );
    assert_eq!(
        after.highest_bid, before.highest_bid,
        "close_auction must not change the highest bid"
    );
    assert_eq!(
        after.status,
        AuctionStatus::Closed,
        "status must transition to Closed"
    );
}

/// Round-robin bidding (A→B→C→A→B→C). At each outbidding step the event
/// must name the **immediately** preceding holder with their exact amount.
///
/// This is the primary proof that the per-outbid refund is atomic and correctly
/// targets the current loser, not some historical bidder.
#[test]
fn round_robin_refund_per_outbid_step() {
    let env = fresh_env();
    let id = register(&env);
    let client = AuctionClient::new(&env, &id);
    let aid = Symbol::new(&env, "ra_roundrbn");
    open_english(&client, &aid);

    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let c = Address::generate(&env);
    let bidders: [Address; 3] = [a, b, c];

    // (bidder_index, amount) — each step strictly outbids the previous holder.
    let steps: [(usize, i128); 6] =
        [(0, 50), (1, 100), (2, 200), (0, 350), (1, 600), (2, 900)];

    let mut prev: Option<(Address, i128)> = None;

    for &(idx, amount) in &steps {
        let bidder = &bidders[idx];
        client.place_bid(&aid, bidder, &amount);

        match &prev {
            None => {
                // First bid — no prior holder, no refund expected.
                assert_eq!(
                    count_topic(&env, BID_RFDN),
                    0,
                    "first bid must not emit BID_RFDN"
                );
            }
            Some((displaced_addr, displaced_amount)) => {
                let events = collect_refund_events(&env);
                assert_eq!(
                    events.len(),
                    1,
                    "each outbid step must emit exactly one BID_RFDN"
                );
                assert_eq!(
                    &events[0].prev_bidder, displaced_addr,
                    "BID_RFDN must name the immediately-displaced holder"
                );
                assert_eq!(
                    events[0].amount, *displaced_amount,
                    "BID_RFDN must carry the displaced holder's exact bid amount"
                );
            }
        }

        prev = Some((bidder.clone(), amount));
    }
}
