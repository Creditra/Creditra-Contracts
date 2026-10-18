//! Tests for `require_auth` coverage on `settle_default_liquidation` factory caller.
//!
//! The auction-side [`settle_default_liquidation`] checks the caller against the
//! registered factory address but must also cryptographically verify the factory's
//! authorization via [`Address::require_auth`]. These tests prove that a forged
//! invoker with mocked auth entries cannot bypass the check.
//!
//! # Tests
//!
//! | Test | Auth mock | `credit_contract` | Expected |
//! |------|-----------|-------------------|----------|
//! | `non_factory_invoker_reverts` | intruder only | factory address | revert |
//! | `factory_invoker_succeeds` | factory only | factory address | `Ok(420)` |
//! | `replay_settle_reverts` | factory (twice) | factory address | `Err(AlreadySettled)` |
//! | `test_race_settle_then_claim` | factory then winner | factory address | settle ok, claim reverts |
//! | `test_race_claim_then_settle` | winner then factory | factory address | settle ok, claim reverts |
//!
//! # Running
//!
//! ```bash
//! cargo test -p gateway-auction --test auth_settle
//! ```
//!
//! [`settle_default_liquidation`]: ../src/lib.rs
use gateway_auction::DutchAuctionDecay;
use gateway_auction::{Auction, AuctionClient, AuctionMode};
use soroban_sdk::testutils::{Address as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{Address, Env, IntoVal, Symbol};

/// Deploy the auction contract, register the factory, create and close an
/// auction so that `settle_default_liquidation` can be exercised.
///
/// Returns `(env, contract_id, auction_id, factory, borrower, expected_recovered)`.
/// Callers create the `AuctionClient` locally to keep borrow lifetimes simple.
fn setup_auction() -> (Env, Address, Symbol, Address, Address, i128) {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);

    let factory = Address::generate(&env);
    let bidder = Address::generate(&env);
    let borrower = Address::generate(&env);
    let auction_id = Symbol::new(&env, "auth_stl");

    client.set_factory_contract(&factory);
    client.init_auction(
        &auction_id,
        &AuctionMode::English,
        &0_u64,
        &u64::MAX,
        &50_i128,
        &0_u32,
        &None,
        &None,
        &Some(DutchAuctionDecay::None),
        &None,
    );
    client.place_bid(&auction_id, &bidder, &420_i128);
    client.close_auction(&auction_id);

    (env, contract_id, auction_id, factory, borrower, 420_i128)
}

// ── Negative ────────────────────────────────────────────────────────────────

#[test]
fn non_factory_invoker_reverts() {
    let (env, contract_id, auction_id, factory, borrower, _expected) = setup_auction();
    let client = AuctionClient::new(&env, &contract_id);

    let intruder = Address::generate(&env);

    let result = client
        .mock_auths(&[MockAuth {
            address: &intruder,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "settle_default_liquidation",
                args: (auction_id.clone(), factory.clone(), borrower.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_settle_default_liquidation(&auction_id, &factory, &borrower);

    assert!(
        result.is_err(),
        "non-factory invoker must be rejected (require_auth prevents bypass)"
    );
}

// ── Positive ────────────────────────────────────────────────────────────────

#[test]
fn factory_invoker_succeeds() {
    let (env, contract_id, auction_id, factory, borrower, expected) = setup_auction();
    let client = AuctionClient::new(&env, &contract_id);

    let result = client
        .mock_auths(&[MockAuth {
            address: &factory,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "settle_default_liquidation",
                args: (auction_id.clone(), factory.clone(), borrower.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_settle_default_liquidation(&auction_id, &factory, &borrower);

    let recovered = result
        .expect("factory-authorized call must not encounter host error")
        .expect("factory-authorized call must not encounter contract error");
    assert_eq!(recovered, expected, "must return the highest bid");
}

// ── Replay ──────────────────────────────────────────────────────────────────

#[test]
fn replay_settle_reverts() {
    let (env, contract_id, auction_id, factory, borrower, _expected) = setup_auction();
    let client = AuctionClient::new(&env, &contract_id);
    // First settlement — succeeds
    let first = client
        .mock_auths(&[MockAuth {
            address: &factory,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "settle_default_liquidation",
                args: (auction_id.clone(), factory.clone(), borrower.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_settle_default_liquidation(&auction_id, &factory, &borrower);
    assert!(first.is_ok(), "first settlement must succeed");

    // Second settlement — must revert with AlreadySettled
    let second = client
        .mock_auths(&[MockAuth {
            address: &factory,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "settle_default_liquidation",
                args: (auction_id.clone(), factory.clone(), borrower.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_settle_default_liquidation(&auction_id, &factory, &borrower);
    assert!(second.is_err(), "second settlement must revert");
}

// ── Claim vs Settlement Race (#1354) ────────────────────────────────────────

/// Helper to set up a closed auction with a real token and funded bidder.
///
/// Returns `(env, contract_id, auction_id, factory, winner, borrower, bid_token, bid_amount)`.
fn setup_auction_with_token(
    auction_name: &str,
) -> (
    Env,
    Address,
    Symbol,
    Address,
    Address,
    Address,
    Address,
    i128,
) {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);

    let factory = Address::generate(&env);
    let winner = Address::generate(&env);
    let borrower = Address::generate(&env);
    let auction_id = Symbol::new(&env, auction_name);

    client.set_factory_contract(&factory);

    // Register token, fund contract and winner, and set bid_token
    let token_admin = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract_v2(token_admin);
    let bid_token = token_id.address();
    let sac = soroban_sdk::token::StellarAssetClient::new(&env, &bid_token);
    sac.mint(&contract_id, &10_000_000_i128);
    sac.mint(&winner, &10_000_000_i128);

    env.as_contract(&contract_id, || {
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "bid_token"), &bid_token);
    });

    let bid_amount = 500_i128;
    client.init_auction(
        &auction_id,
        &AuctionMode::English,
        &0_u64,
        &u64::MAX,
        &50_i128,
        &0_u32,
        &None,
        &None,
        &Some(DutchAuctionDecay::None),
        &None,
    );
    client.place_bid(&auction_id, &winner, &bid_amount);
    client.close_auction(&auction_id);

    (
        env,
        contract_id,
        auction_id,
        factory,
        winner,
        borrower,
        bid_token,
        bid_amount,
    )
}

/// Ordering 1: `settle_default_liquidation` followed by `claim_auction`.
///
/// Intended semantics:
/// - Factory successfully settles default liquidation; `settle_default_liquidation` transfers
///   `highest_bid` to the credit contract (`factory`).
/// - Winner cannot reclaim the bid via `claim_auction` (settlement marker prevents claim,
///   reverting with `AlreadySettled`).
/// - Balances: factory receives `bid_amount`, winner does not receive bid back.
#[test]
fn test_race_settle_then_claim() {
    let (
        env,
        contract_id,
        auction_id,
        factory,
        winner,
        borrower,
        bid_token,
        bid_amount,
    ) = setup_auction_with_token("stl_claim");
    let client = AuctionClient::new(&env, &contract_id);

    let token_client = soroban_sdk::token::Client::new(&env, &bid_token);
    let winner_balance_before = token_client.balance(&winner);
    let factory_balance_before = token_client.balance(&factory);

    // 1. Factory settles default liquidation
    let settle_res = client
        .mock_auths(&[MockAuth {
            address: &factory,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "settle_default_liquidation",
                args: (auction_id.clone(), factory.clone(), borrower.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_settle_default_liquidation(&auction_id, &factory, &borrower);

    assert!(settle_res.is_ok(), "settlement must succeed");
    assert_eq!(
        settle_res.unwrap().unwrap(),
        bid_amount,
        "settled amount must equal highest bid"
    );

    // Factory received bid_amount
    let factory_balance_after = token_client.balance(&factory);
    assert_eq!(
        factory_balance_after - factory_balance_before,
        bid_amount,
        "credit contract must receive the recovered bid"
    );

    // 2. Winner attempts to claim auction proceeds
    let claim_res = client
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

    // Intended semantics: winner cannot reclaim the bid
    assert!(
        claim_res.is_err(),
        "claim_auction must fail after settlement has completed (cannot reclaim bid)"
    );

    let winner_balance_after = token_client.balance(&winner);
    assert_eq!(
        winner_balance_after, winner_balance_before,
        "winner token balance must remain unchanged"
    );
}

/// Ordering 2: `claim_auction` followed by `settle_default_liquidation`.
///
/// Under current code:
/// - `claim_auction` succeeds and returns `recovered_amount` to the winner.
/// - Status is set to `AuctionStatus::Claimed`.
/// - `settle_default_liquidation` then fails with `AuctionError::NotClosed` (code 3),
///   and the credit contract fails to recover liquidation proceeds.
///
/// Intended semantics:
/// - Settlement must always succeed, and the winner cannot reclaim the bid.
/// - This test asserts the intended outcome: settlement succeeds and winner cannot reclaim bid.
/// - Fails on current behaviour until claim semantics are fixed.
#[test]
fn test_race_claim_then_settle() {
    let (
        env,
        contract_id,
        auction_id,
        factory,
        winner,
        borrower,
        bid_token,
        bid_amount,
    ) = setup_auction_with_token("clm_settle");
    let client = AuctionClient::new(&env, &contract_id);

    let token_client = soroban_sdk::token::Client::new(&env, &bid_token);
    let winner_balance_before = token_client.balance(&winner);
    let factory_balance_before = token_client.balance(&factory);

    // 1. Winner attempts claim before factory settlement
    let claim_res = client
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

    // 2. Factory executes settle_default_liquidation
    let settle_res = client
        .mock_auths(&[MockAuth {
            address: &factory,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "settle_default_liquidation",
                args: (auction_id.clone(), factory.clone(), borrower.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_settle_default_liquidation(&auction_id, &factory, &borrower);

    // Intended outcome assertions:
    // - Settlement must always succeed
    assert!(
        settle_res.is_ok(),
        "settle_default_liquidation must succeed even if claim was attempted"
    );
    assert_eq!(
        settle_res.unwrap().unwrap(),
        bid_amount,
        "settlement must recover the winning bid amount"
    );

    // - Credit contract / factory must receive the bid tokens
    let factory_balance_after = token_client.balance(&factory);
    assert_eq!(
        factory_balance_after - factory_balance_before,
        bid_amount,
        "credit contract must receive the recovered bid tokens"
    );

    // - Winner cannot reclaim the bid
    assert!(
        claim_res.is_err(),
        "winner must not be able to reclaim the bid"
    );

    let winner_balance_after = token_client.balance(&winner);
    assert_eq!(
        winner_balance_after, winner_balance_before,
        "winner token balance must not increase"
    );
}
