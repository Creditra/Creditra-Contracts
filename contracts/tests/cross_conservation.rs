// SPDX-License-Identifier: MIT

//! Focused cross-contract conservation tests for Credit + Auction settlement.

use creditra_credit::types::CreditStatus;
use creditra_credit::{Credit, CreditClient};
use gateway_auction::{Auction, AuctionClient, AuctionMode, AuctionError};
use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{token, Address, Env, Symbol};

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

fn setup_test(env: &Env, draw_amount: i128) -> Deployment {
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
    credit.set_auction_contract(&auction_id);

    let auction = AuctionClient::new(env, &auction_id);
    auction.set_factory_contract(&credit_id);
    
    // Set bid_token in auction contract
    env.as_contract(&auction_id, || {
        env.storage()
            .instance()
            .set(&Symbol::new(env, "bid_token"), &token_address);
    });

    // Mint tokens to credit contract
    token::StellarAssetClient::new(env, &token_address).mint(&credit_id, &CREDIT_LIMIT);

    credit.open_credit_line(&borrower, &CREDIT_LIMIT, &INTEREST_RATE_BPS, &RISK_SCORE);
    credit.draw_credit(&borrower, &draw_amount);
    credit.default_credit_line(&borrower);

    Deployment {
        credit_id,
        auction_id,
        borrower,
        token_id,
    }
}

fn get_total_balance(env: &Env, token_id: &Address, deployment: &Deployment, bidder: &Address) -> i128 {
    let token_client = token::Client::new(env, token_id);
    
    let credit_balance = token_client.balance(&deployment.credit_id);
    let borrower_balance = token_client.balance(&deployment.borrower);
    let auction_balance = token_client.balance(&deployment.auction_id);
    let bidder_balance = token_client.balance(bidder);
    
    credit_balance + borrower_balance + auction_balance + bidder_balance
}

#[test]
fn test_conservation_under_full_liquidation() {
    let env = Env::default();
    let draw_amount = 1_500;
    let deployment = setup_test(&env, draw_amount);
    let settlement_id = Symbol::new(&env, "liq_full");

    let bidder = Address::generate(&env);
    token::StellarAssetClient::new(&env, &deployment.token_id).mint(&bidder, &2_000);

    let initial_total = get_total_balance(&env, &deployment.token_id, &deployment, &bidder);

    let auction = AuctionClient::new(&env, &deployment.auction_id);
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

    // Bidder places bid of 1500
    auction.place_bid(&settlement_id, &bidder, &1_500);

    env.ledger().set_timestamp(end_time);
    auction.close_auction(&settlement_id);

    // Settle default liquidation via credit contract
    let credit = CreditClient::new(&env, &deployment.credit_id);
    credit.settle_default_liquidation(
        &deployment.borrower,
        &1_500,
        &settlement_id,
        &None,
    );

    let final_total = get_total_balance(&env, &deployment.token_id, &deployment, &bidder);
    assert_eq!(initial_total, final_total, "Balances not conserved after default liquidation");

    // Verify bidder paid, credit contract received the funds
    let token_client = token::Client::new(&env, &deployment.token_id);
    assert_eq!(token_client.balance(&deployment.auction_id), 0);
    assert_eq!(token_client.balance(&bidder), 500); // 2000 - 1500
    assert_eq!(token_client.balance(&deployment.credit_id), 8_500 + 1_500); // 8500 (10000-1500) + 1500 recovered
}

#[test]
fn test_conservation_under_partial_liquidation() {
    let env = Env::default();
    let draw_amount = 2_000;
    let deployment = setup_test(&env, draw_amount);
    let settlement_id = Symbol::new(&env, "liq_part");

    let bidder = Address::generate(&env);
    token::StellarAssetClient::new(&env, &deployment.token_id).mint(&bidder, &1_000);

    let initial_total = get_total_balance(&env, &deployment.token_id, &deployment, &bidder);

    let auction = AuctionClient::new(&env, &deployment.auction_id);
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

    // Bidder places bid of 800
    auction.place_bid(&settlement_id, &bidder, &800);

    env.ledger().set_timestamp(end_time);
    auction.close_auction(&settlement_id);

    // Settle default liquidation via credit contract
    let credit = CreditClient::new(&env, &deployment.credit_id);
    credit.settle_default_liquidation(
        &deployment.borrower,
        &800,
        &settlement_id,
        &None,
    );

    let final_total = get_total_balance(&env, &deployment.token_id, &deployment, &bidder);
    assert_eq!(initial_total, final_total, "Balances not conserved after partial liquidation");

    let token_client = token::Client::new(&env, &deployment.token_id);
    assert_eq!(token_client.balance(&deployment.auction_id), 0);
    assert_eq!(token_client.balance(&bidder), 200); // 1000 - 800
    assert_eq!(token_client.balance(&deployment.credit_id), 8_000 + 800); // 8000 + 800 recovered
}

#[test]
fn test_claim_settled_liquidation_is_prevented() {
    let env = Env::default();
    let draw_amount = 1_000;
    let deployment = setup_test(&env, draw_amount);
    let settlement_id = Symbol::new(&env, "liq_prevent_claim");

    let bidder = Address::generate(&env);
    token::StellarAssetClient::new(&env, &deployment.token_id).mint(&bidder, &1_500);

    let auction = AuctionClient::new(&env, &deployment.auction_id);
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

    auction.place_bid(&settlement_id, &bidder, &1_000);

    env.ledger().set_timestamp(end_time);
    auction.close_auction(&settlement_id);

    // Settle default liquidation
    let credit = CreditClient::new(&env, &deployment.credit_id);
    credit.settle_default_liquidation(
        &deployment.borrower,
        &1_000,
        &settlement_id,
        &None,
    );

    // Winner attempts to claim the auction - should fail because it has already been settled via default liquidation
    let res = env.as_contract(&deployment.auction_id, || {
        soroban_sdk::std::panic::catch_unwind(soroban_sdk::std::panic::AssertUnwindSafe(|| {
            auction.claim_auction(&settlement_id);
        }))
    });
    assert!(res.is_err());
}
