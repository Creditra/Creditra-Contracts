//! Integration tests for `env.panic_with_error(AuctionError::…)` on public paths (Issue #609).
//!
//! Contract failures must surface stable `AuctionError` discriminants — not host
//! string panics — so indexers and cross-contract callers can decode reverts.
//!
//! # Running
//!
//! ```bash
//! cargo test -p gateway-auction --test panic_with_error
//! ```

use soroban_sdk::testutils::Ledger as _;

use gateway_auction::{Auction, AuctionClient, AuctionError, AuctionMode};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, Symbol};

fn init_open_auction(client: &AuctionClient<'_>, auction_id: &Symbol, end_time: u64) {
    client.init_auction(
        auction_id,
        &AuctionMode::English,
        &0,
        &end_time,
        &50_i128,
        &0_u32,
        &None,
        &None,
        &gateway_auction::DutchAuctionDecay::None,
        &None,
    );
}

#[test]
fn close_auction_missing_id_returns_not_found() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);
    let missing = Symbol::new(&env, "missing_close");

    let err = client.try_close_auction(&missing).unwrap_err().unwrap();
    assert_eq!(
        err,
        AuctionError::NotFound.into(),
        "missing auction must revert with NotFound, not a string panic"
    );
}

#[test]
fn place_bid_missing_id_returns_not_found() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);
    let bidder = Address::generate(&env);
    let missing = Symbol::new(&env, "missing_bid");

    let err = client
        .try_place_bid(&missing, &bidder, &100_i128)
        .unwrap_err()
        .unwrap();
    assert_eq!(
        err,
        AuctionError::NotFound.into(),
        "bid on missing auction must revert with NotFound"
    );
}

#[test]
fn place_bid_after_end_time_returns_auction_not_open() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1001);

    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);
    let bidder = Address::generate(&env);
    let auction_id = Symbol::new(&env, "timed_out");

    init_open_auction(&client, &auction_id, 1000);

    let err = client
        .try_place_bid(&auction_id, &bidder, &100_i128)
        .unwrap_err()
        .unwrap();
    assert_eq!(
        err,
        AuctionError::AuctionNotOpen.into(),
        "late bid must revert with AuctionNotOpen, not a string panic"
    );
}

#[test]
fn place_bid_non_positive_amount_returns_bid_too_low() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);
    let bidder = Address::generate(&env);
    let auction_id = Symbol::new(&env, "non_positive");

    init_open_auction(&client, &auction_id, u64::MAX);

    for amount in [0_i128, -1_i128] {
        let err = client
            .try_place_bid(&auction_id, &bidder, &amount)
            .unwrap_err()
            .unwrap();
        assert_eq!(
            err,
            AuctionError::BidTooLow.into(),
            "amount {amount} must revert with BidTooLow"
        );
    }
}
