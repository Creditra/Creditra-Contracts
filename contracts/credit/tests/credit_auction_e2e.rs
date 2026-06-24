// SPDX-License-Identifier: MIT

//! Cross-contract default liquidation flow for Credit + Auction.
//!
//! These tests deploy both contracts into the same Soroban test environment and
//! exercise the operational path used by off-chain liquidation orchestration:
//! credit default emits the liquidation request, the auction contract runs bids
//! and emits its settlement hook, then the recovered amount is reconciled back
//! into the credit contract.

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

struct Deployment {
    credit_id: Address,
    auction_id: Address,
    borrower: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
struct AuctionSettlementEvent {
    auction_id: Symbol,
    credit_contract: Address,
    borrower: Address,
    winner: Address,
    recovered_amount: i128,
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
    assert_event_topic(env, &credit_id, "credit", "liq_req");

    let defaulted = credit.get_credit_line(&borrower).unwrap();
    assert_eq!(defaulted.status, CreditStatus::Defaulted);
    assert_eq!(defaulted.utilized_amount, draw_amount);

    Deployment {
        credit_id,
        auction_id,
        borrower,
    }
}

fn run_auction_to_settlement(
    env: &Env,
    deployment: &Deployment,
    settlement_id: &Symbol,
    recovered_amount: i128,
) -> i128 {
    let first_bid = recovered_amount / 2;
    assert!(first_bid >= MIN_BID);
    assert!(recovered_amount > first_bid);

    let auction = AuctionClient::new(env, &deployment.auction_id);
    let bidder = Address::generate(env);
    let winner = Address::generate(env);
    let start_time = env.ledger().timestamp();
    let end_time = start_time + AUCTION_DURATION;

    auction.init_auction(settlement_id, &start_time, &end_time, &MIN_BID);
    auction.place_bid(settlement_id, &bidder, &first_bid);
    auction.place_bid(settlement_id, &winner, &recovered_amount);

    env.ledger().with_mut(|ledger| {
        ledger.timestamp = end_time;
    });

    auction.close_auction(settlement_id);
    auction.settle_default_liquidation(settlement_id, &deployment.credit_id, &deployment.borrower);

    let settlement = auction_settlement_event(env, &deployment.auction_id);
    assert_eq!(settlement.auction_id, settlement_id.clone());
    assert_eq!(settlement.credit_contract, deployment.credit_id);
    assert_eq!(settlement.borrower, deployment.borrower);
    assert_eq!(settlement.winner, winner);
    assert_eq!(settlement.recovered_amount, recovered_amount);
    settlement.recovered_amount
}

fn settle_credit_from_auction(
    env: &Env,
    deployment: &Deployment,
    settlement_id: &Symbol,
    recovered_amount: i128,
) {
    let credit = CreditClient::new(env, &deployment.credit_id);
    credit.settle_default_liquidation(&deployment.borrower, &recovered_amount, settlement_id, &None);
    assert_event_topic(env, &deployment.credit_id, "credit", "liq_setl");
}

fn assert_event_topic(env: &Env, contract_id: &Address, topic0: &str, topic1: &str) {
    let expected0 = Symbol::new(env, topic0);
    let expected1 = Symbol::new(env, topic1);

    let matched = env.events().all().iter().any(|(contract, topics, _data)| {
        if contract != contract_id.clone() || topics.len() < 2 {
            return false;
        }

        let actual0: Symbol = Symbol::try_from_val(env, &topics.get(0).unwrap()).unwrap();
        let actual1: Symbol = Symbol::try_from_val(env, &topics.get(1).unwrap()).unwrap();
        actual0 == expected0 && actual1 == expected1
    });

    assert!(matched, "missing event topic ({topic0}, {topic1})");
}

fn auction_settlement_event(env: &Env, auction_id: &Address) -> AuctionSettlementEvent {
    for (contract, topics, data) in env.events().all().iter() {
        if contract != auction_id.clone() || topics.len() < 2 {
            continue;
        }

        let topic0: Symbol = Symbol::try_from_val(env, &topics.get(0).unwrap()).unwrap();
        let topic1: Symbol = Symbol::try_from_val(env, &topics.get(1).unwrap()).unwrap();
        if topic0 == Symbol::new(env, "LIQ_SETL") && topic1 == Symbol::new(env, "auction") {
            return data.try_into_val(env).unwrap();
        }
    }

    panic!("missing auction settlement event");
}

#[test]
fn e2e_full_recovery_closes_defaulted_credit_line() {
    let env = Env::default();
    let draw_amount = 1_200;
    let recovered_amount = draw_amount;
    let deployment = setup_defaulted_credit(&env, draw_amount);
    let settlement_id = Symbol::new(&env, "auc_full");

    let auction_recovery =
        run_auction_to_settlement(&env, &deployment, &settlement_id, recovered_amount);
    settle_credit_from_auction(&env, &deployment, &settlement_id, auction_recovery);

    let credit = CreditClient::new(&env, &deployment.credit_id);
    let line = credit.get_credit_line(&deployment.borrower).unwrap();
    assert_eq!(line.utilized_amount, 0);
    assert_eq!(line.status, CreditStatus::Closed);
}

#[test]
fn e2e_partial_recovery_keeps_remaining_defaulted_debt() {
    let env = Env::default();
    let draw_amount = 1_000;
    let recovered_amount = 400;
    let deployment = setup_defaulted_credit(&env, draw_amount);
    let settlement_id = Symbol::new(&env, "auc_part");

    let auction_recovery =
        run_auction_to_settlement(&env, &deployment, &settlement_id, recovered_amount);
    settle_credit_from_auction(&env, &deployment, &settlement_id, auction_recovery);

    let credit = CreditClient::new(&env, &deployment.credit_id);
    let line = credit.get_credit_line(&deployment.borrower).unwrap();
    assert_eq!(line.utilized_amount, draw_amount - recovered_amount);
    assert_eq!(line.status, CreditStatus::Defaulted);
}

#[test]
fn e2e_atomic_settlement_with_configured_auction() {
    let env = Env::default();
    let draw_amount = 1_500;
    let recovered_amount = 1_500;
    let deployment = setup_defaulted_credit(&env, draw_amount);
    let settlement_id = Symbol::new(&env, "auc_atomic");

    // Configure the auction contract in the credit contract
    let credit = CreditClient::new(&env, &deployment.credit_id);
    credit.set_auction_contract(&deployment.auction_id);

    // Run the auction but do NOT call settle_default_liquidation manually!
    let auction = AuctionClient::new(&env, &deployment.auction_id);
    auction.set_factory_contract(&deployment.credit_id);

    let start_time = env.ledger().timestamp();
    let end_time = start_time + AUCTION_DURATION;

    auction.init_auction(&settlement_id, &start_time, &end_time, &MIN_BID);
    auction.place_bid(&settlement_id, &Address::generate(&env), &(recovered_amount / 2));
    let winner = Address::generate(&env);
    auction.place_bid(&settlement_id, &winner, &recovered_amount);

    env.ledger().with_mut(|ledger| {
        ledger.timestamp = end_time;
    });

    auction.close_auction(&settlement_id);

    // Call settle_default_liquidation on the credit contract!
    // It should atomically call settle_default_liquidation on the auction contract,
    // reconcile the bid amount, and close the defaulted line!
    credit.settle_default_liquidation(&deployment.borrower, &recovered_amount, &settlement_id, &None);

    let line = credit.get_credit_line(&deployment.borrower).unwrap();
    assert_eq!(line.utilized_amount, 0);
    assert_eq!(line.status, CreditStatus::Closed);

    // Also assert that the auction event is still emitted and marker is set
    assert_event_topic(&env, &deployment.auction_id, "LIQ_SETL", "auction");
}
