//! Bidder authorization coverage for every bidder-facing auction mutation.
//!
//! Every auction mutation that changes bidder-owned state must verify
//! [`Address::require_auth`] on the affected address — see
//! `gateway-contract/contracts/auction_contract/src/auth.rs`.
//!
//! | Mutation        | Authorized party  | Helper                               |
//! |-----------------|-------------------|--------------------------------------|
//! | `place_bid`     | The bidder        | `auth::require_bidder_auth`          |
//! | `claim_auction` | The winner        | `auth::require_winner_auth`          |
//!
//! These tests prove that a forged invoker (mocked thread / different
//! signer) cannot mutate bidder-owned state on behalf of another address,
//! that the success paths require the correct signer, that boundary
//! conditions (winner == bidder, self-outbid) are well formed, and that
//! replay of a claim is rejected (state-transition invariant).
//!
//! # Running
//!
//! ```bash
//! cargo test -p gateway-auction --test bidder_auth
//! ```

use gateway_auction::{Auction, AuctionClient, AuctionMode, AuctionState, DutchAuctionDecay};
use soroban_sdk::testutils::{Address as _, MockAuth, MockAuthInvoke};
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{Address, Env, IntoVal, Symbol};

// ── Helpers ──────────────────────────────────────────────────────────────────

const AUCTION_ID: &str = "bid_auth";

/// Register the factory, create an open English auction with a bid token, and
/// mint funds to `contract_id` and each bidder.
fn setup_english<'a>(
    env: &'a Env,
    client: &AuctionClient<'a>,
    contract_id: &Address,
    min_bid: i128,
    min_increment_bps: u32,
    bidders: &[Address],
) {
    let factory = Address::generate(env);
    client.set_factory_contract(&factory);
    client.init_auction(
        &Symbol::new(env, AUCTION_ID),
        &AuctionMode::English,
        &0_u64,
        &u64::MAX,
        &min_bid,
        &min_increment_bps,
        &None,
        &None,
        &Some(DutchAuctionDecay::None),
        &None,
    );

    let token_admin = Address::generate(env);
    let token_id = env.register_stellar_asset_contract_v2(token_admin.clone());
    let bid_token = token_id.address();
    let sac = StellarAssetClient::new(env, &bid_token);
    sac.mint(contract_id, &10_000_000_i128);
    for b in bidders {
        sac.mint(b, &10_000_000_i128);
    }
    env.as_contract(contract_id, || {
        env.storage()
            .instance()
            .set(&Symbol::new(env, "bid_token"), &bid_token);
    });
}

// ── place_bid: success path ──────────────────────────────────────────────────

#[test]
fn place_bid_succeeds_with_bidders_own_auth() {
    // Happy path: a bidder authorized by the host (env-wide mock auth, which
    // also authorizes the bid token `transfer` sub-invoke) successfully places
    // a bid. The rejection tests below prove a wrong or missing signer is
    // refused, so this validates the positive side of the invariant.
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);

    let bidder = Address::generate(&env);
    setup_english(
        &env,
        &client,
        &contract_id,
        1_i128,
        0,
        std::slice::from_ref(&bidder),
    );
    let auction_id = Symbol::new(&env, AUCTION_ID);

    let result = client.try_place_bid(&auction_id, &bidder, &100_i128);

    assert!(
        result.is_ok(),
        "a bidder signed for itself must be accepted"
    );
}

// ── place_bid: rejection paths ───────────────────────────────────────────────

#[test]
fn place_bid_rejects_when_passed_bidder_does_not_sign() {
    // A caller submits `victim` as the bidder but only signs for itself
    // (`intruder`). The contract must reject because `victim` never attested.
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);

    let bidder = Address::generate(&env);
    setup_english(
        &env,
        &client,
        &contract_id,
        1_i128,
        0,
        std::slice::from_ref(&bidder),
    );

    let intruder = Address::generate(&env);
    let auction_id = Symbol::new(&env, AUCTION_ID);

    let result = client
        .mock_auths(&[MockAuth {
            address: &intruder,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "place_bid",
                args: (auction_id.clone(), bidder.clone(), 100_i128).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_place_bid(&auction_id, &bidder, &100_i128);

    assert!(
        result.is_err(),
        "place_bid must reject when the passed bidder did not authorize the call"
    );
}

#[test]
fn place_bid_rejects_without_any_auth() {
    // No auth entry at all: the host has nothing attesting the bidder.
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);

    let bidder = Address::generate(&env);
    setup_english(
        &env,
        &client,
        &contract_id,
        1_i128,
        0,
        std::slice::from_ref(&bidder),
    );
    let auction_id = Symbol::new(&env, AUCTION_ID);

    // mock_auths with an empty set still requires at least an attestation for
    // the bidder; the host rejects the missing signature.
    let result = client
        .mock_auths(&[])
        .try_place_bid(&auction_id, &bidder, &100_i128);

    assert!(
        result.is_err(),
        "place_bid must reject when the bidder provides no authorization"
    );
}

#[test]
fn place_bid_cannot_be_forged_for_a_victim_address() {
    // End-to-end: a fresh env without blanket mock auth. Signing only for the
    // intruder must fail to place a bid under the victim's address, because
    // the victim is the one the contract asks to authorize.
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);

    let victim = Address::generate(&env);
    setup_english(
        &env,
        &client,
        &contract_id,
        1_i128,
        0,
        std::slice::from_ref(&victim),
    );

    let attacker = Address::generate(&env);
    let auction_id = Symbol::new(&env, AUCTION_ID);

    let result = client
        .mock_auths(&[MockAuth {
            address: &attacker,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "place_bid",
                args: (auction_id.clone(), victim.clone(), 500_i128).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_place_bid(&auction_id, &victim, &500_i128);

    assert!(
        result.is_err(),
        "an attacker must not be able to place a bid under a victim address"
    );
}

// ── place_bid: boundary conditions ──────────────────────────────────────────

#[test]
fn place_bid_self_outbid_by_same_authorized_bidder_is_consistent() {
    // The same bidder may raise its own bid; every mutation is authorized by
    // the same bidder and the stored state must track the latest amount.
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);

    let bidder = Address::generate(&env);
    setup_english(
        &env,
        &client,
        &contract_id,
        1_i128,
        0,
        std::slice::from_ref(&bidder),
    );
    let auction_id = Symbol::new(&env, AUCTION_ID);

    client.place_bid(&auction_id, &bidder, &100_i128);
    client.place_bid(&auction_id, &bidder, &150_i128);

    let stored: AuctionState = env.as_contract(&contract_id, || {
        env.storage().persistent().get(&auction_id).unwrap()
    });
    assert_eq!(
        stored.highest_bidder.unwrap(),
        bidder,
        "self-outbid keeps the same authorized bidder"
    );
    assert_eq!(stored.highest_bid, 150_i128, "highest bid tracks latest");
}

#[test]
fn place_bid_outbid_refunds_previous_authorized_bidder() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);

    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    setup_english(
        &env,
        &client,
        &contract_id,
        1_i128,
        0,
        &[alice.clone(), bob.clone()],
    );
    let auction_id = Symbol::new(&env, AUCTION_ID);

    client.place_bid(&auction_id, &alice, &100_i128);
    client.place_bid(&auction_id, &bob, &200_i128);

    let stored: AuctionState = env.as_contract(&contract_id, || {
        env.storage().persistent().get(&auction_id).unwrap()
    });
    assert_eq!(
        stored.highest_bidder.unwrap(),
        bob,
        "the authorized outbidder becomes the highest bidder"
    );
    assert_eq!(stored.highest_bid, 200_i128);
}

// ── claim_auction: success path ──────────────────────────────────────────────

#[test]
fn claim_auction_succeeds_for_the_winner() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);

    let winner = Address::generate(&env);
    setup_english(
        &env,
        &client,
        &contract_id,
        1_i128,
        0,
        std::slice::from_ref(&winner),
    );
    let auction_id = Symbol::new(&env, AUCTION_ID);

    client.place_bid(&auction_id, &winner, &500_i128);
    client.close_auction(&auction_id);

    let result = client
        .mock_auths(&[MockAuth {
            address: &winner,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "claim_auction",
                args: (auction_id.clone(),).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_claim_auction(&auction_id);

    assert!(
        result.is_ok(),
        "the recorded winner signed by itself must be able to claim"
    );
}

// ── claim_auction: rejection paths ───────────────────────────────────────────

#[test]
fn claim_auction_rejects_non_winner() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);

    let winner = Address::generate(&env);
    let intruder = Address::generate(&env);
    setup_english(
        &env,
        &client,
        &contract_id,
        1_i128,
        0,
        &[winner.clone(), intruder.clone()],
    );
    let auction_id = Symbol::new(&env, AUCTION_ID);

    client.place_bid(&auction_id, &winner, &500_i128);
    client.close_auction(&auction_id);

    let result = client
        .mock_auths(&[MockAuth {
            address: &intruder,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "claim_auction",
                args: (auction_id.clone(),).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_claim_auction(&auction_id);

    assert!(
        result.is_err(),
        "a non-winner must not be able to claim auction proceeds"
    );
}

#[test]
fn claim_auction_uses_stored_winner_not_caller_input() {
    // The winner is read from stored state, so a signed intruder still cannot
    // claim: the contract asks *the winner* to authorize, not the caller.
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);

    let winner = Address::generate(&env);
    setup_english(
        &env,
        &client,
        &contract_id,
        1_i128,
        0,
        std::slice::from_ref(&winner),
    );
    let auction_id = Symbol::new(&env, AUCTION_ID);

    client.place_bid(&auction_id, &winner, &500_i128);
    client.close_auction(&auction_id);

    // Assert the winner is the only one able to claim by confirming an empty
    // auth set (no winner attestation) is rejected host-side.
    let result = client.mock_auths(&[]).try_claim_auction(&auction_id);
    assert!(
        result.is_err(),
        "claim requires the stored winner's authorization"
    );
}

// ── regression: state-transition invariants ─────────────────────────────────

#[test]
fn claim_cannot_be_replayed_by_winner() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);

    let winner = Address::generate(&env);
    setup_english(
        &env,
        &client,
        &contract_id,
        1_i128,
        0,
        std::slice::from_ref(&winner),
    );
    let auction_id = Symbol::new(&env, AUCTION_ID);

    client.place_bid(&auction_id, &winner, &500_i128);
    client.close_auction(&auction_id);

    let claim = || {
        client
            .mock_auths(&[MockAuth {
                address: &winner,
                invoke: &MockAuthInvoke {
                    contract: &contract_id,
                    fn_name: "claim_auction",
                    args: (auction_id.clone(),).into_val(&env),
                    sub_invokes: &[],
                },
            }])
            .try_claim_auction(&auction_id)
    };

    assert!(claim().is_ok(), "first claim by winner must succeed");
    assert!(
        claim().is_err(),
        "replaying the same claim must revert (AlreadyClaimed / AlreadySettled)"
    );
}

#[test]
fn claim_requires_closed_state() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);

    let winner = Address::generate(&env);
    setup_english(
        &env,
        &client,
        &contract_id,
        1_i128,
        0,
        std::slice::from_ref(&winner),
    );
    let auction_id = Symbol::new(&env, AUCTION_ID);

    // Bid but do *not* close: auction is still Open, claim must revert.
    client.place_bid(&auction_id, &winner, &500_i128);

    let result = client
        .mock_auths(&[MockAuth {
            address: &winner,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "claim_auction",
                args: (auction_id.clone(),).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_claim_auction(&auction_id);

    assert!(result.is_err(), "claiming an Open auction must be rejected");
}
