// SPDX-License-Identifier: MIT

//! Cross-contract conservation test for Credit + Auction settlement.
//!
//! This test verifies that funds are conserved across the credit ↔ auction
//! settlement flow. It ensures that the total amount of funds before and after
//! the liquidation process remains equal, accounting for all transfers between
//! the credit contract, auction contract, and external parties.

use creditra_credit::types::CreditStatus;
use creditra_credit::{Credit, CreditClient};
use gateway_auction::{Auction, AuctionClient, AuctionMode};
use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{token, Address, Env, Symbol, TryFromVal, TryIntoVal};

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
    token_id: Address,
}

/// Setup a defaulted credit line with auction contract deployed.
fn setup_conservative_test(env: &Env, draw_amount: i128) -> Deployment {
    env.mock_all_auths_allowing_non_root_auth();
    env.ledger().set_timestamp(START_TS);

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

    // Mint tokens to credit contract to fund draws
    token::StellarAssetClient::new(env, &token_address).mint(&credit_id, &CREDIT_LIMIT);

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
        token_id,
    }
}

/// Get the total token balance across all relevant accounts.
fn get_total_balance(env: &Env, token_id: &Address, deployment: &Deployment) -> i128 {
    let token_client = token::Client::new(env, token_id);
    
    let credit_balance = token_client.balance(&deployment.credit_id);
    let borrower_balance = token_client.balance(&deployment.borrower);
    let auction_balance = token_client.balance(&deployment.auction_id);
    
    credit_balance + borrower_balance + auction_balance
}

/// Run an auction and return the recovered amount.
fn run_auction(
    env: &Env,
    deployment: &Deployment,
    settlement_id: &Symbol,
    recovered_amount: i128,
) -> i128 {
    let auction = AuctionClient::new(env, &deployment.auction_id);
    let bidder = Address::generate(env);
    let winner = Address::generate(env);
    let start_time = env.ledger().timestamp();
    let end_time = start_time + AUCTION_DURATION;

    // Mint tokens to bidders for the auction
    token::StellarAssetClient::new(env, &deployment.token_id)
        .mint(&bidder, &(recovered_amount / 2));
    token::StellarAssetClient::new(env, &deployment.token_id)
        .mint(&winner, &recovered_amount);

    auction.init_auction(
        settlement_id,
        &AuctionMode::English,
        &start_time,
        &end_time,
        &MIN_BID,
        &0_u32,
        &None,
        &None,
        &None,
        &None,
    );
    
    let first_bid = recovered_amount / 2;
    auction.place_bid(settlement_id, &bidder, &first_bid);
    auction.place_bid(settlement_id, &winner, &recovered_amount);

    env.ledger().set_timestamp(end_time);

    auction.close_auction(settlement_id);
    auction.settle_default_liquidation(settlement_id, &deployment.credit_id, &deployment.borrower);

    recovered_amount
}

#[test]
fn test_full_recovery_funds_conserved() {
    let env = Env::default();
    let draw_amount = 1_200;
    let recovered_amount = draw_amount;
    let deployment = setup_conservative_test(&env, draw_amount);
    let settlement_id = Symbol::new(&env, "cons_full");

    // Record initial total balance
    let initial_total = get_total_balance(&env, &deployment.token_id, &deployment);

    // Run auction and settle
    let auction_recovery = run_auction(&env, &deployment, &settlement_id, recovered_amount);
    
    let credit = CreditClient::new(&env, &deployment.credit_id);
    credit.settle_default_liquidation(
        &deployment.borrower,
        &auction_recovery,
        &settlement_id,
        &None,
    );

    // Record final total balance
    let final_total = get_total_balance(&env, &deployment.token_id, &deployment);

    // Verify conservation: total funds should be equal
    assert_eq!(
        initial_total, final_total,
        "Total funds not conserved: initial={}, final={}",
        initial_total, final_total
    );

    // Verify credit line state
    let line = credit.get_credit_line(&deployment.borrower).unwrap();
    assert_eq!(line.utilized_amount, 0);
    assert_eq!(line.status, CreditStatus::Closed);
}

#[test]
fn test_partial_recovery_funds_conserved() {
    let env = Env::default();
    let draw_amount = 1_000;
    let recovered_amount = 400;
    let deployment = setup_conservative_test(&env, draw_amount);
    let settlement_id = Symbol::new(&env, "cons_part");

    // Record initial total balance
    let initial_total = get_total_balance(&env, &deployment.token_id, &deployment);

    // Run auction and settle
    let auction_recovery = run_auction(&env, &deployment, &settlement_id, recovered_amount);
    
    let credit = CreditClient::new(&env, &deployment.credit_id);
    credit.settle_default_liquidation(
        &deployment.borrower,
        &auction_recovery,
        &settlement_id,
        &None,
    );

    // Record final total balance
    let final_total = get_total_balance(&env, &deployment.token_id, &deployment);

    // Verify conservation: total funds should be equal
    assert_eq!(
        initial_total, final_total,
        "Total funds not conserved: initial={}, final={}",
        initial_total, final_total
    );

    // Verify credit line state
    let line = credit.get_credit_line(&deployment.borrower).unwrap();
    assert_eq!(line.utilized_amount, draw_amount - recovered_amount);
    assert_eq!(line.status, CreditStatus::Defaulted);
}

#[test]
fn test_atomic_settlement_funds_conserved() {
    let env = Env::default();
    let draw_amount = 1_500;
    let recovered_amount = 1_500;
    let deployment = setup_conservative_test(&env, draw_amount);
    let settlement_id = Symbol::new(&env, "cons_atomic");

    // Configure the auction contract in the credit contract
    let credit = CreditClient::new(&env, &deployment.credit_id);
    credit.set_auction_contract(&deployment.auction_id);

    let auction = AuctionClient::new(&env, &deployment.auction_id);
    auction.set_factory_contract(&deployment.credit_id);

    // Record initial total balance
    let initial_total = get_total_balance(&env, &deployment.token_id, &deployment);

    // Setup auction
    let winner = Address::generate(&env);
    token::StellarAssetClient::new(&env, &deployment.token_id)
        .mint(&winner, &recovered_amount);

    let start_time = env.ledger().timestamp();
    let end_time = start_time + AUCTION_DURATION;

    auction.init_auction(
        &settlement_id,
        &AuctionMode::English,
        &start_time,
        &end_time,
        &MIN_BID,
        &0_u32,
        &None,
        &None,
        &None,
        &None,
    );
    
    auction.place_bid(
        &settlement_id,
        &Address::generate(&env),
        &(recovered_amount / 2),
    );
    auction.place_bid(&settlement_id, &winner, &recovered_amount);

    env.ledger().set_timestamp(end_time);
    auction.close_auction(&settlement_id);

    // Atomic settlement via credit contract
    credit.settle_default_liquidation(
        &deployment.borrower,
        &recovered_amount,
        &settlement_id,
        &None,
    );

    // Record final total balance
    let final_total = get_total_balance(&env, &deployment.token_id, &deployment);

    // Verify conservation: total funds should be equal
    assert_eq!(
        initial_total, final_total,
        "Total funds not conserved: initial={}, final={}",
        initial_total, final_total
    );

    // Verify credit line state
    let line = credit.get_credit_line(&deployment.borrower).unwrap();
    assert_eq!(line.utilized_amount, 0);
    assert_eq!(line.status, CreditStatus::Closed);
}

#[test]
fn test_multiple_partial_settlements_funds_conserved() {
    let env = Env::default();
    let draw_amount = 2_000;
    let first_recovery = 500;
    let second_recovery = 500;
    let deployment = setup_conservative_test(&env, draw_amount);
    
    // Record initial total balance
    let initial_total = get_total_balance(&env, &deployment.token_id, &deployment);

    // First settlement
    let settlement_id_1 = Symbol::new(&env, "cons_multi_1");
    let auction_recovery_1 = run_auction(&env, &deployment, &settlement_id_1, first_recovery);
    
    let credit = CreditClient::new(&env, &deployment.credit_id);
    credit.settle_default_liquidation(
        &deployment.borrower,
        &auction_recovery_1,
        &settlement_id_1,
        &None,
    );

    // Second settlement
    let settlement_id_2 = Symbol::new(&env, "cons_multi_2");
    let auction_recovery_2 = run_auction(&env, &deployment, &settlement_id_2, second_recovery);
    
    credit.settle_default_liquidation(
        &deployment.borrower,
        &auction_recovery_2,
        &settlement_id_2,
        &None,
    );

    // Record final total balance
    let final_total = get_total_balance(&env, &deployment.token_id, &deployment);

    // Verify conservation: total funds should be equal
    assert_eq!(
        initial_total, final_total,
        "Total funds not conserved: initial={}, final={}",
        initial_total, final_total
    );

    // Verify credit line state
    let line = credit.get_credit_line(&deployment.borrower).unwrap();
    assert_eq!(line.utilized_amount, draw_amount - first_recovery - second_recovery);
    assert_eq!(line.status, CreditStatus::Defaulted);
}
