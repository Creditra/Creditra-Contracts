#![cfg_attr(not(test), no_std)]

mod errors;
mod events;
mod storage;
mod types;

use errors::AuctionError;

use soroban_sdk::{contract, contractimpl, contracttype, token, Address, BytesN, Env, Symbol};

use crate::types::*;
use events::{
    publish_auction_closed_event, publish_bid_refunded_event,
    publish_default_liquidation_settlement_event,
};
use storage::{bump_auction_state_ttl, bump_settlement_marker_ttl};

/// Returns the minimum bid that must be placed to outbid `highest_bid`.
///
/// increment = ceil(highest_bid * min_increment_bps / 10_000)
///
/// Ceiling is computed as `q / d + (q % d != 0)` to avoid the `q + (d-1)`
/// addition that would overflow when q is near i128::MAX.
///
/// A floor of 1 stroop is applied so that a zero-bps config still requires
/// a strictly higher bid, preventing equal-amount griefing.
fn min_next_bid(highest_bid: i128, min_increment_bps: u32) -> i128 {
    let bps = min_increment_bps as i128;
    let product = highest_bid
        .checked_mul(bps)
        .expect("overflow in bid increment calculation");
    let bps_increment = product / 10_000 + i128::from(product % 10_000 != 0);
    // Always require at least +1 stroop even when bps == 0
    let increment = bps_increment.max(1);
    highest_bid
        .checked_add(increment)
        .expect("overflow computing minimum next bid threshold")
}

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
    pub fn init_auction(
        env: Env,
        auction_id: Symbol,
        start_time: u64,
        end_time: u64,
        min_bid: i128,
    ) {
        if start_time >= end_time {
            panic!("invalid times");
        }
        // Cap at 100% (10_000 bps) — a requirement higher than that is nonsensical
        if min_increment_bps > 10_000 {
            panic!("min_increment_bps exceeds maximum of 10000 (100%)");
        }
        let config = AuctionConfig {
            username_hash: BytesN::from_array(&env, &[0; 32]),
            start_time,
            end_time,
            min_bid,
            min_increment_bps,
        };
        let state = AuctionState {
            config,
            status: AuctionStatus::Open,
            highest_bidder: None,
            highest_bid: 0,
        };
        env.storage().persistent().set(&auction_id, &state);
        bump_auction_state_ttl(&env, &auction_id);
    }

    pub fn close_auction(env: Env, auction_id: Symbol) {
        let mut state: AuctionState = env
            .storage()
            .persistent()
            .get(&auction_id)
            .unwrap_or_else(|| panic!("auction not found"));
        bump_auction_state_ttl(&env, &auction_id);
        if state.status == AuctionStatus::Closed {
            panic!("already closed");
        }
        state.status = AuctionStatus::Closed;
        env.storage().persistent().set(&auction_id, &state);
        bump_auction_state_ttl(&env, &auction_id);
        publish_auction_closed_event(&env, auction_id, state.highest_bidder, state.highest_bid);
    }

    /// Place a bid for an auction identified by `auction_id`.
    ///
    /// Bid floor: `amount` must be strictly greater than `max(min_bid - 1, highest_bid)`.
    /// Equivalently, the first bid must be at least `min_bid`, and every later bid must
    /// exceed the current highest. Equal-to-highest bids abort with `AuctionError::BidTooLow`.
    ///
    /// When outbidding, the previous highest bidder is refunded exactly `highest_bid`
    /// (event first, then token transfer when `bid_token` is configured).
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
        bump_auction_state_ttl(&env, &auction_id);

        if state.status != AuctionStatus::Open {
            panic!("auction not open");
        }

        if env.ledger().timestamp() >= state.config.end_time {
            panic!("auction closed");
        }

        let min_floor = state.config.min_bid.saturating_sub(1);
        let required_floor = if state.highest_bid > min_floor {
            state.highest_bid
        } else {
            min_floor
        };
        if amount <= required_floor {
            env.panic_with_error(AuctionError::BidTooLow);
        }

        let token_addr: Option<Address> = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, "bid_token"));

            // Emit refund event before performing token transfer
            publish_bid_refunded_event(&env, prev_bidder.clone(), state.highest_bid);

            if let Some(tkn) = token_addr {
                let token_client = token::Client::new(&env, &tkn);
                token_client.transfer(
                    &env.current_contract_address(),
                    &prev_bidder,
                    &refund_amount,
                );
            }
        }

        state.highest_bidder = Some(bidder);
        state.highest_bid = amount;
        env.storage().persistent().set(&auction_id, &state);
        bump_auction_state_ttl(&env, &auction_id);
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
        bump_auction_state_ttl(&env, &auction_id);

        if state.status != AuctionStatus::Closed {
            panic!("auction not closed");
        }

        let settlement_key = AuctionKey::LiquidationSettled(auction_id.clone());
        bump_settlement_marker_ttl(&env, &settlement_key);
        let already_settled = env
            .storage()
            .persistent()
            .get::<AuctionKey, bool>(&settlement_key)
            .unwrap_or(false);
        if already_settled {
            panic!("liquidation already settled");
        }

        env.storage().persistent().set(&settlement_key, &true);
        bump_settlement_marker_ttl(&env, &settlement_key);

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
        bump_auction_state_ttl(&env, &auction_id);

        if state.status != AuctionStatus::Closed {
            panic!("auction not closed");
        }

        let winner = state.highest_bidder.clone().unwrap_or_else(|| panic!("no winner"));
        winner.require_auth();

        if state.status == AuctionStatus::Claimed {
            panic!("already claimed");
        }

        let mut updated_state = state;
        updated_state.status = AuctionStatus::Claimed;
        env.storage().persistent().set(&auction_id, &updated_state);
        bump_auction_state_ttl(&env, &auction_id);
    }
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod test;
