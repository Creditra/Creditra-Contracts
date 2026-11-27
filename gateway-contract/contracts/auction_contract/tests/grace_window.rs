//! Comprehensive tests for the auction liquidation grace window feature.
//!
//! # Coverage map
//!
//! | Test | Scenario | Expected |
//! |------|----------|----------|
//! | `bid_at_start_plus_grace_minus_one_rejected` | Early bid at `start + grace - 1` | Revert `GracePeriodActive` |
//! | `bid_at_start_plus_grace_accepted` | Boundary bid at `start + grace` | Success |
//! | `set_grace_window_without_factory_auth_rejected` | Setter without factory authorization | Revert `NoFactoryContract` / `Unauthorized` |
//! | `zero_grace_window_disables_feature` | Grace window set to `0` | Feature disabled, bids accepted |
//!
//! # Running
//!
//! ```bash
//! cargo test -p gateway-auction --test grace_window
//! ```

use gateway_auction::{Auction, AuctionClient, AuctionError, AuctionMode, DutchAuctionDecay};
use soroban_sdk::testutils::{Address as _, MockAuth, MockAuthInvoke};
use soroban_sdk::testutils::{Ledger as _};
use soroban_sdk::{Address, Env, IntoVal, Symbol};

// ── Helpers ──────────────────────────────────────────────────────────

/// Deploy the auction contract, register a factory, set a grace window,
/// and create an auction with the given `start_time` and `end_time`.
///
/// Returns `(env, client, factory, contract_id, auction_id)`.
fn setup_grace_test(
    env: &Env,
    start_time: u64,
    end_time: u64,
    grace_window: u64,
) -> (Env, AuctionClient<'_>, Address, Address, Symbol) {
    env.mock_all_auths();
    let factory = Address::generate(env);
    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(env, &contract_id);
    let auction_id = Symbol::new(env, "grace_auc");

    client.set_factory_contract(&factory);
    client.set_liquidation_grace_window(&grace_window);
    client.init_auction(
        &auction_id,
        &AuctionMode::English,
        &start_time,
        &end_time,
        &50_i128,
        &0_u32,
        &None,
        &None,
        &Some(DutchAuctionDecay::None),
        &None,
    );

    (env.clone(), client, factory, contract_id, auction_id)
}

/// Create an auction without setting a grace window (defaults to 0).
fn setup_no_grace_test(env: &Env, start_time: u64, end_time: u64) -> (Env, AuctionClient<'_>, Address, Symbol) {
    env.mock_all_auths();
    let factory = Address::generate(env);
    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(env, &contract_id);
    let auction_id = Symbol::new(env, "no_grace_auc");

    client.set_factory_contract(&factory);
    client.init_auction(
        &auction_id,
        &AuctionMode::English,
        &start_time,
        &end_time,
        &50_i128,
        &0_u32,
        &None,
        &None,
        &Some(DutchAuctionDecay::None),
        &None,
    );

    (env.clone(), client, factory, auction_id)
}

// ── Test 1: Early bid at start + grace - 1 reverts with GracePeriodActive ──

#[test]
fn bid_at_start_plus_grace_minus_one_rejected() {
    let env = Env::default();
    let start_time = 1000;
    let end_time = 2000;
    let grace_window = 60;
    let (_, client, _factory, _contract_id, auction_id) =
        setup_grace_test(&env, start_time, end_time, grace_window);

    let bidder = Address::generate(&env);
    // Timestamp = start_time + grace_window - 1 = 1059 → still in grace period
    env.ledger().set_timestamp(start_time + grace_window - 1);

    let result = client.try_place_bid(&auction_id, &bidder, &100_i128);
    assert!(
        result.is_err(),
        "bid at start + grace - 1 must be rejected during grace period"
    );
    assert_eq!(
        result.unwrap_err().unwrap(),
        AuctionError::GracePeriodActive.into(),
        "must return GracePeriodActive error code"
    );
}

// ── Test 2: Boundary bid at start + grace is accepted ──

#[test]
fn bid_at_start_plus_grace_accepted() {
    let env = Env::default();
    let start_time = 1000;
    let end_time = 2000;
    let grace_window = 60;
    let (_, client, _factory, _contract_id, auction_id) =
        setup_grace_test(&env, start_time, end_time, grace_window);

    let bidder = Address::generate(&env);
    // Timestamp = start_time + grace_window = 1060 → grace period has elapsed
    env.ledger().set_timestamp(start_time + grace_window);

    let result = client.try_place_bid(&auction_id, &bidder, &100_i128);
    assert!(
        result.is_ok(),
        "bid at exact start + grace boundary must be accepted"
    );
}

// ── Test 3: Setter called without factory authorization reverts ──

#[test]
fn set_grace_window_without_factory_auth_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);
    let factory = Address::generate(&env);
    client.set_factory_contract(&factory);

    let intruder = Address::generate(&env);
    let auction_id = Symbol::new(&env, "unauth_grace");

    let result = client
        .mock_auths(&[MockAuth {
            address: &intruder,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_liquidation_grace_window",
                args: (3600_u64,).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_set_liquidation_grace_window(&3600_u64);

    assert!(
        result.is_err(),
        "set_liquidation_grace_window without factory auth must be rejected"
    );
}

// ── Test 4: Zero grace window correctly disables the feature ──

#[test]
fn zero_grace_window_disables_feature() {
    let env = Env::default();
    let start_time = 1000;
    let end_time = 2000;
    let (_, client, _factory, _contract_id, auction_id) =
        setup_grace_test(&env, start_time, end_time, 0);

    // Verify the grace window is configured as 0
    assert_eq!(
        client.get_liquidation_grace_window(),
        0_u64,
        "grace window must be 0 (disabled)"
    );

    let bidder = Address::generate(&env);
    // Even at start_time (well before start_time + 0 = start_time), bid should succeed
    env.ledger().set_timestamp(start_time);

    let result = client.try_place_bid(&auction_id, &bidder, &100_i128);
    assert!(
        result.is_ok(),
        "bid at start_time must succeed when grace window is 0 (disabled)"
    );
}

// ── Additional edge cases ────────────────────────────────────────────

/// Confirms that setting grace window to 0 after it was previously non-zero
/// re-enables immediate bidding.
#[test]
fn set_grace_window_to_zero_enables_immediate_bids() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);
    let factory = Address::generate(&env);
    client.set_factory_contract(&factory);

    // Set a non-zero grace window
    client.set_liquidation_grace_window(&300_u64);

    let auction_id = Symbol::new(&env, "zero_after_nonzero");
    client.init_auction(
        &auction_id,
        &AuctionMode::English,
        &1000,
        &2000,
        &50_i128,
        &0_u32,
        &None,
        &None,
        &Some(DutchAuctionDecay::None),
        &None,
    );

    // Clear the grace window
    client.set_liquidation_grace_window(&0_u64);
    assert_eq!(client.get_liquidation_grace_window(), 0_u64);

    let bidder = Address::generate(&env);
    env.ledger().set_timestamp(1000);
    let result = client.try_place_bid(&auction_id, &bidder, &100_i128);
    assert!(
        result.is_ok(),
        "bid must succeed after grace window set to 0"
    );
}

/// Confirms that a bid one second before start + grace is still rejected
/// (strict inequality: now < earliest_start).
#[test]
fn bid_one_second_before_grace_expiry_rejected() {
    let env = Env::default();
    let start_time = 1000;
    let end_time = 2000;
    let grace_window = 60;
    let (_, client, _factory, _contract_id, auction_id) =
        setup_grace_test(&env, start_time, end_time, grace_window);

    let bidder = Address::generate(&env);
    env.ledger().set_timestamp(start_time + grace_window - 1);

    let result = client.try_place_bid(&auction_id, &bidder, &100_i128);
    assert!(result.is_err(), "bid before grace expiry must fail");
}

/// Confirms that the grace window does not affect close_auction.
#[test]
fn close_auction_unaffected_by_grace_window() {
    let env = Env::default();
    let start_time = 1000;
    let end_time = 2000;
    let (_, client, _factory, _contract_id, auction_id) =
        setup_grace_test(&env, start_time, end_time, 60);

    let bidder = Address::generate(&env);
    env.ledger().set_timestamp(start_time + 60);
    client.place_bid(&auction_id, &bidder, &100_i128);

    env.ledger().set_timestamp(end_time);
    let result = client.try_close_auction(&auction_id);
    assert!(
        result.is_ok(),
        "close_auction must not be blocked by grace window"
    );
}

/// Confirms that set_liquidation_grace_window requires factory to be set
/// first (NoFactoryContract).
#[test]
fn set_grace_window_requires_factory() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);

    let result = client.try_set_liquidation_grace_window(&60_u64);
    assert!(
        result.is_err(),
        "setting grace window without factory must fail"
    );
}
