// SPDX-License-Identifier: MIT

//! Cross-contract conservation test for settle_default_liquidation (Credit + Auction).
//!
//! This test verifies:
//! 1. Borrower's utilized amount decreases exactly by recovered amount.
//! 2. Recovered amount equals auction's highest bid.
//! 3. Replay of settle_default_liquidation fails.
//! 4. Auction's LiquidationSettled marker is set exactly once.
//!
//! Covers 3 scenarios as per requirements.

use creditra_credit::types::CreditStatus;
use creditra_credit::{Credit, CreditClient};
use gateway_auction::{Auction, AuctionClient};
use soroban_sdk::testutils::{Address as _, Events as _, Ledger};
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{contracttype, Address, Env, Symbol, TryFromVal, TryIntoVal};

const CREDIT_LIMIT: i128 = 10_000;
const INTEREST_RATE_BPS: u32 = 0;
const RISK_SCORE: u32 = 60;
const MIN_BID: i128 = 100;
const START_TS: u64 = 100;
const AUCTION_DURATION: u64 = 1_000;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
enum AuctionKey {
    LiquidationSettled(Symbol),
}

struct Deployment {
    credit_id: Address,
    auction_id: Address,
    borrower: Address,
}

fn setup_defaulted_credit(env: &Env, draw_amount: i128) -> Deployment {
    env.mock_all_auths_allowing_non_root_auth();
    env.ledger().with_mut(|ledger| {
        ledger.timestamp = START_TS;
    });

    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let credit_id = env.register(Credit, ());
    let auction_id = env.register(Auction, ());
    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token_address = token_id.address();

    let credit = CreditClient::new(env, &credit_id);
    credit.init(&admin);
    credit.set_liquidity_token(&token_address);
    credit.set_liquidity_source(&credit_id);

    StellarAssetClient::new(env, &token_address).mint(&credit_id, &CREDIT_LIMIT);

    credit.open_credit_line(&borrower, &CREDIT_LIMIT, &INTEREST_RATE_BPS, &RISK_SCORE);
    credit.draw_credit(&borrower, &draw_amount);

    let drawn = credit.get_credit_line(&borrower).unwrap();
    assert_eq!(drawn.status, CreditStatus::Active);
    assert_eq!(drawn.utilized_amount, draw_amount);

    credit.default_credit_line(&borrower);

    let defaulted = credit.get_credit_line(&borrower).unwrap();
    assert_eq!(defaulted.status, CreditStatus::Defaulted);
    assert_eq!(defaulted.utilized_amount, draw_amount);

    Deployment {
        credit_id,
        auction_id,
        borrower,
    }
}

fn run_auction(env: &Env, deployment: &Deployment, settlement_id: &Symbol, highest_bid: i128) {
    let auction = AuctionClient::new(env, &deployment.auction_id);
    let bidder = Address::generate(env);
    let winner = Address::generate(env);
    let start_time = env.ledger().timestamp();
    let end_time = start_time + AUCTION_DURATION;

    auction.set_factory_contract(&deployment.credit_id);
    auction.init_auction(
        settlement_id,
        &gateway_auction::types::AuctionMode::English,
        &start_time,
        &end_time,
        &MIN_BID,
        &0_u32,
        &None,
        &None,
    );
    auction.place_bid(settlement_id, &bidder, &(highest_bid / 2));
    auction.place_bid(settlement_id, &winner, &highest_bid);

    env.ledger().with_mut(|ledger| {
        ledger.timestamp = end_time;
    });

    auction.close_auction(settlement_id);
}

fn get_auction_state(
    env: &Env,
    auction_id: &Address,
    settlement_id: &Symbol,
) -> gateway_auction::types::AuctionState {
    env.as_contract(auction_id, || {
        env.storage().persistent().get(settlement_id).unwrap()
    })
}

fn is_settlement_marker_set(env: &Env, auction_id: &Address, settlement_id: &Symbol) -> bool {
    env.as_contract(auction_id, || {
        let key = AuctionKey::LiquidationSettled(settlement_id.clone());
        env.storage().persistent().get(&key).unwrap_or(false)
    })
}

fn run_conservation_test(env: &Env, draw_amount: i128, highest_bid: i128) {
    let deployment = setup_defaulted_credit(env, draw_amount);
    let settlement_id = Symbol::new(env, "auc_test");
    let credit = CreditClient::new(env, &deployment.credit_id);
    credit.set_auction_contract(&deployment.auction_id);

    let pre_line = credit.get_credit_line(&deployment.borrower).unwrap();
    let pre_utilized = pre_line.utilized_amount;

    // Check that settlement marker isn't set yet
    assert!(!is_settlement_marker_set(
        env,
        &deployment.auction_id,
        &settlement_id
    ));

    // Run auction
    run_auction(env, &deployment, &settlement_id, highest_bid);

    // Check auction state highest bid
    let auction_state = get_auction_state(env, &deployment.auction_id, &settlement_id);
    assert_eq!(auction_state.highest_bid, highest_bid);

    // Settle default liquidation (credit contract calls auction contract)
    credit.settle_default_liquidation(
        &deployment.borrower,
        &highest_bid,
        &settlement_id,
        &10_000_u32,
        &None,
    );

    // Check conservation
    let post_line = credit.get_credit_line(&deployment.borrower).unwrap();
    let post_utilized = post_line.utilized_amount;
    assert_eq!(pre_utilized - post_utilized, highest_bid);

    // Check auction settlement marker is set exactly once
    assert!(is_settlement_marker_set(
        env,
        &deployment.auction_id,
        &settlement_id
    ));

    // Verify replay fails
    let replay_result = std::panic::catch_unwind(|| {
        credit.settle_default_liquidation(
            &deployment.borrower,
            &highest_bid,
            &settlement_id,
            &None,
        );
    });
    assert!(replay_result.is_err(), "replay should panic");
}

#[test]
fn conservation_test_full_recovery() {
    let env = Env::default();
    let draw_amount = 1_500;
    let highest_bid = 1_500;
    run_conservation_test(&env, draw_amount, highest_bid);
}

#[test]
fn conservation_test_partial_recovery_1() {
    let env = Env::default();
    let draw_amount = 2_000;
    let highest_bid = 1_200;
    run_conservation_test(&env, draw_amount, highest_bid);
}

#[test]
fn conservation_test_partial_recovery_2() {
    let env = Env::default();
    let draw_amount = 5_000;
    let highest_bid = 3_500;
    run_conservation_test(&env, draw_amount, highest_bid);
}
