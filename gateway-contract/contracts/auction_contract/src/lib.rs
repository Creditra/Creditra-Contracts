#![no_std]

mod events;
mod types;

use soroban_sdk::{contract, contractimpl, contracttype, token, Address, BytesN, Env, Symbol};

use crate::types::*;
use events::{
    publish_auction_closed_event, publish_bid_refunded_event, publish_default_liquidation_settlement_event,
};

#[contract]
pub struct Auction;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuctionKey {
    Closed(Symbol),
    LiquidationSettled(Symbol),
}

#[contractimpl]
impl Auction {
    pub fn init_auction(env: Env, auction_id: Symbol, start_time: u64, end_time: u64, min_bid: i128) {
        if start_time >= end_time {
            panic!("invalid times");
        }
        let config = AuctionConfig {
            username_hash: BytesN::from_array(&env, &[0; 32]),
            start_time,
            end_time,
            min_bid,
        };
        let state = AuctionState {
            config,
            status: AuctionStatus::Open,
            highest_bidder: None,
            highest_bid: 0,
        };
        env.storage().persistent().set(&auction_id, &state);
    }

    pub fn close_auction(env: Env, auction_id: Symbol) {
        let mut state: AuctionState = env
            .storage()
            .persistent()
            .get(&auction_id)
            .unwrap_or_else(|| panic!("auction not found"));
        if state.status == AuctionStatus::Closed {
            panic!("already closed");
        }
        state.status = AuctionStatus::Closed;
        env.storage().persistent().set(&auction_id, &state);
        publish_auction_closed_event(&env, auction_id, state.highest_bidder, state.highest_bid);
    }

    /// Place a bid for an auction identified by `auction_id`.
    /// If there's a previous highest bidder, emit a `BID_RFDN` event
    /// before attempting the refund token transfer.
    pub fn place_bid(env: Env, auction_id: Symbol, bidder: Address, amount: i128) {
        bidder.require_auth();

        if amount <= 0 {
            panic!("amount must be positive");
        }

        let mut state: AuctionState = env
            .storage()
            .persistent()
            .get(&auction_id)
            .unwrap_or_else(|| panic!("auction not initialized"));

        if state.status != AuctionStatus::Open {
            panic!("auction not open");
        }

        if env.ledger().timestamp() >= state.config.end_time {
            panic!("auction closed");
        }

        if amount < state.config.min_bid {
            panic!("bid too low");
        }

        if let Some(prev_bidder) = state.highest_bidder.clone() {
            if amount <= state.highest_bid {
                panic!("bid must be higher than current highest bid");
            }

            // Emit refund event before performing token transfer
            publish_bid_refunded_event(&env, prev_bidder.clone(), state.highest_bid);

            // Attempt refund token transfer if token address configured in instance storage
            let token_addr: Option<Address> = env
                .storage()
                .instance()
                .get(&Symbol::new(&env, "bid_token"));
            if let Some(tkn) = token_addr {
                let token_client = token::Client::new(&env, &tkn);
                // Contract is the sender of refund transfers (for tests this will be mocked)
                token_client.transfer(
                    &env.current_contract_address(),
                    &prev_bidder,
                    &state.highest_bid,
                );
            }
        }

        // Store new highest bid
        state.highest_bidder = Some(bidder);
        state.highest_bid = amount;
        env.storage().persistent().set(&auction_id, &state);
    }

    /// Emit an auction settlement signal for credit default liquidation orchestration.
    ///
    /// Requirements:
    /// - auction must be closed
    /// - settlement signal is one-time per auction_id
    pub fn settle_default_liquidation(
        env: Env,
        auction_id: Symbol,
        credit_contract: Address,
        borrower: Address,
    ) {
        let state: AuctionState = env
            .storage()
            .persistent()
            .get(&auction_id)
            .unwrap_or_else(|| panic!("auction state not found"));

        if state.status != AuctionStatus::Closed {
            panic!("auction not closed");
        }

        let settlement_key = AuctionKey::LiquidationSettled(auction_id.clone());
        let already_settled = env
            .storage()
            .persistent()
            .get::<AuctionKey, bool>(&settlement_key)
            .unwrap_or(false);
        if already_settled {
            panic!("liquidation already settled");
        }

        env.storage().persistent().set(&settlement_key, &true);

        let winner = state.highest_bidder.unwrap_or_else(|| borrower.clone());
        publish_default_liquidation_settlement_event(
            &env,
            auction_id,
            credit_contract,
            borrower,
            winner,
            state.highest_bid,
        );
    }

    /// Claim the auction proceeds for the winner.
    /// Requirements:
    /// - auction must be closed
    /// - caller must be the winner
    /// - auction must have a bid
    pub fn claim_auction(env: Env, auction_id: Symbol) {
        let state: AuctionState = env
            .storage()
            .persistent()
            .get(&auction_id)
            .unwrap_or_else(|| panic!("auction state not found"));

        if state.status != AuctionStatus::Closed {
            panic!("auction not closed");
        }

        let winner = state
            .highest_bidder
            .clone()
            .unwrap_or_else(|| panic!("no winner"));
        winner.require_auth();

        if state.status == AuctionStatus::Claimed {
            panic!("already claimed");
        }

        // Mark as claimed
        let mut updated_state = state;
        updated_state.status = AuctionStatus::Claimed;
        env.storage().persistent().set(&auction_id, &updated_state);
    }
}

#[cfg(test)]
mod test;
