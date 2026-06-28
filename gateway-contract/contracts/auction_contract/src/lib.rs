#![cfg_attr(not(test), no_std)]

mod errors;
mod events;
mod storage;
mod types;

pub use errors::AuctionError;
pub use events::BidRefundedEvent;
pub use types::{AuctionMode, AuctionState, AuctionStatus, DutchAuctionDecay};

use soroban_sdk::{contract, contractimpl, contracttype, token, Address, BytesN, Env, Symbol};

use crate::storage::{clear_reentrancy_guard, get_factory_contract, set_reentrancy_guard};
use crate::types::*;
use events::{
    publish_auction_closed_event, publish_bid_refunded_event,
    publish_default_liquidation_settlement_event,
};
use storage::{bump_auction_state_ttl, bump_settlement_marker_ttl};

fn min_next_bid(highest_bid: i128, min_increment_bps: u32) -> i128 {
    let bps = min_increment_bps as i128;
    let product = highest_bid
        .checked_mul(bps)
        .expect("overflow in bid increment calculation");
    let bps_increment = product / 10_000 + i128::from(product % 10_000 != 0);
    let increment = bps_increment.max(1);
    highest_bid
        .checked_add(increment)
        .expect("overflow computing minimum next bid threshold")
}

/// Computes the current Dutch auction price based on elapsed time.
///
/// - [`DutchAuctionDecay::None`] / [`DutchAuctionDecay::Linear`]: `p(t) = start - (start - floor) * t / T`
/// - [`DutchAuctionDecay::Stepped`]: equal time buckets, discrete downward steps.
/// - [`DutchAuctionDecay::Exponential`]: ~1% multiplicative decay per time unit,
///   capped at 100 iterations for safety.
pub fn compute_dutch_price(
    start_price: i128,
    floor_price: i128,
    elapsed_time: u64,
    duration: u64,
    decay: &DutchAuctionDecay,
    step_count: Option<u32>,
) -> i128 {
    if duration == 0 {
        return floor_price;
    }
    if elapsed_time >= duration {
        return floor_price;
    }

    let price_drop = start_price
        .checked_sub(floor_price)
        .expect("start_price must be >= floor_price");

    let elapsed_i128 = elapsed_time as i128;
    let duration_i128 = duration as i128;

    let drop_so_far = match decay {
        DutchAuctionDecay::None | DutchAuctionDecay::Linear => price_drop
            .checked_mul(elapsed_i128)
            .expect("overflow in Dutch price calculation")
            .checked_div(duration_i128)
            .expect("division should succeed with positive duration"),

        DutchAuctionDecay::Stepped => {
            let steps = match step_count {
                Some(s) if s > 0 => i128::from(s),
                Some(_) => panic!("dutch_step_count must be > 0 for stepped Dutch auctions"),
                None => panic!("dutch_step_count required for stepped Dutch auctions"),
            };
            let elapsed_steps = i128::from(
                elapsed_time
                    .checked_mul(steps as u64)
                    .expect("overflow in stepped Dutch step calculation")
                    / duration,
            );
            price_drop
                .checked_mul(elapsed_steps)
                .expect("overflow in Dutch price calculation")
                .checked_div(steps)
                .expect("division should succeed with positive step count")
        }

        DutchAuctionDecay::Exponential => {
            let t = elapsed_i128.min(100);
            let mut factor = 10_000i128;
            for _ in 0..t {
                factor = factor
                    .checked_mul(9_900)
                    .expect("overflow in exponential factor")
                    / 10_000;
            }
            price_drop
                .checked_mul(10_000 - factor)
                .expect("overflow in exponential drop calculation")
                / 10_000
        }
    };

    let current_price = start_price
        .checked_sub(drop_so_far)
        .expect("current price should not underflow");

    current_price.max(floor_price)
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
        mode: AuctionMode,
        start_time: u64,
        end_time: u64,
        min_bid: i128,
        min_increment_bps: u32,
        dutch_start_price: Option<i128>,
        dutch_floor_price: Option<i128>,
        dutch_decay: DutchAuctionDecay,
        dutch_step_count: Option<u32>,
    ) {
        if start_time >= end_time {
            panic!("invalid times");
        }
        if min_increment_bps > 10_000 {
            panic!("min_increment_bps exceeds maximum of 10000 (100%)");
        }

        if mode == AuctionMode::Dutch {
            let start = dutch_start_price.expect("dutch_start_price required for Dutch mode");
            let floor = dutch_floor_price.expect("dutch_floor_price required for Dutch mode");
            if start < floor {
                panic!("dutch_start_price must be >= dutch_floor_price");
            }
            if start < min_bid {
                panic!("dutch_start_price must be >= min_bid");
            }

            match &dutch_decay {
                DutchAuctionDecay::None | DutchAuctionDecay::Linear => {}
                DutchAuctionDecay::Stepped => match dutch_step_count {
                    Some(0) => {
                        panic!("dutch_step_count must be > 0 for stepped Dutch auctions")
                    }
                    Some(_) => {}
                    None => panic!("dutch_step_count required for stepped Dutch auctions"),
                },
                DutchAuctionDecay::Exponential => {}
            }
        }

        let config = AuctionConfig {
            mode,
            username_hash: BytesN::from_array(&env, &[0; 32]),
            start_time,
            end_time,
            min_bid,
            min_increment_bps,
            dutch_start_price,
            dutch_floor_price,
            dutch_decay,
            dutch_step_count,
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

    /// Register the factory/credit contract address that is permitted to call
    /// `settle_default_liquidation`. Must be called once after deployment.
    pub fn set_factory_contract(env: Env, factory: Address) {
        factory.require_auth();
        storage::set_factory_contract(&env, &factory);
    }

    /// Set the liquidation auction grace window duration in seconds (admin only).
    ///
    /// This is the minimum time that must elapse between auction creation
    /// (`start_time` in `init_auction`) and when the first bid can be placed.
    /// During the grace period, calls to `place_bid` will fail with
    /// [`AuctionError::GracePeriodActive`]. After the grace window expires,
    /// existing auction behavior is preserved.
    ///
    /// Pass `0` to disable the grace window (default).
    ///
    /// # Authorization
    /// Requires auth from the configured factory/credit contract.
    pub fn set_liquidation_grace_window(env: Env, seconds: u64) {
        let factory = get_factory_contract(&env)
            .unwrap_or_else(|| env.panic_with_error(AuctionError::NoFactoryContract));
        factory.require_auth();
        storage::set_liquidation_grace_window(&env, seconds);
    }

    /// Return the configured liquidation auction grace window in seconds.
    ///
    /// Returns `0` when never configured (no grace period enforced).
    pub fn get_liquidation_grace_window(env: Env) -> u64 {
        storage::get_liquidation_grace_window(&env)
    }

    pub fn close_auction(env: Env, auction_id: Symbol) {
        let mut state: AuctionState = env
            .storage()
            .persistent()
            .get(&auction_id)
            .unwrap_or_else(|| env.panic_with_error(AuctionError::NotFound));
        bump_auction_state_ttl(&env, &auction_id);
        if state.status == AuctionStatus::Claimed {
            env.panic_with_error(AuctionError::AlreadyClaimed);
        }
        if state.status != AuctionStatus::Open {
            env.panic_with_error(AuctionError::AuctionNotOpen);
        }
        state.status = AuctionStatus::Closed;
        env.storage().persistent().set(&auction_id, &state);
        bump_auction_state_ttl(&env, &auction_id);
        publish_auction_closed_event(&env, auction_id, state.highest_bidder, state.highest_bid);
    }

    pub fn place_bid(env: Env, auction_id: Symbol, bidder: Address, amount: i128) {
        bidder.require_auth();

        if amount <= 0 {
            env.panic_with_error(AuctionError::BidTooLow);
        }

        let mut state: AuctionState = env
            .storage()
            .persistent()
            .get(&auction_id)
            .unwrap_or_else(|| env.panic_with_error(AuctionError::NotFound));
        bump_auction_state_ttl(&env, &auction_id);

        if state.status != AuctionStatus::Open {
            env.panic_with_error(AuctionError::AuctionNotOpen);
        }

        let now = env.ledger().timestamp();
        if now >= state.config.end_time {
            env.panic_with_error(AuctionError::AuctionNotOpen);
        }

        // Enforce liquidation grace window: no bids until start_time + grace_window.
        let grace_window = storage::get_liquidation_grace_window(&env);
        if grace_window > 0 {
            let earliest_start = state.config.start_time.saturating_add(grace_window);
            if now < earliest_start {
                env.panic_with_error(AuctionError::GracePeriodActive);
            }
        }

        match state.config.mode {
            AuctionMode::English => {
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

                if let (Some(prev_bidder), Some(tkn)) =
                    (state.highest_bidder.clone(), token_addr)
                {
                    let refund_amount = state.highest_bid;
                    publish_bid_refunded_event(&env, prev_bidder.clone(), state.highest_bid);
                    set_reentrancy_guard(&env);
                    let token_client = token::Client::new(&env, &tkn);
                    token_client.transfer(
                        &env.current_contract_address(),
                        &prev_bidder,
                        &refund_amount,
                    );
                    clear_reentrancy_guard(&env);
                }

                state.highest_bidder = Some(bidder);
                state.highest_bid = amount;
            }

            AuctionMode::Dutch => {
                let current_time = env.ledger().timestamp();
                let elapsed_time = current_time
                    .checked_sub(state.config.start_time)
                    .unwrap_or(0);
                let duration = state
                    .config
                    .end_time
                    .checked_sub(state.config.start_time)
                    .unwrap_or(1);

                let start_price = state
                    .config
                    .dutch_start_price
                    .unwrap_or(state.config.min_bid);
                let floor_price = state
                    .config
                    .dutch_floor_price
                    .unwrap_or(state.config.min_bid);

                let decay = state.config.dutch_decay.clone();

                let current_price = compute_dutch_price(
                    start_price,
                    floor_price,
                    elapsed_time,
                    duration,
                    &decay,
                    state.config.dutch_step_count,
                );

                if amount < current_price {
                    env.panic_with_error(AuctionError::BidTooLow);
                }
                if amount < state.config.min_bid {
                    env.panic_with_error(AuctionError::BidTooLow);
                }

                state.highest_bidder = Some(bidder);
                state.highest_bid = amount;
                state.status = AuctionStatus::Closed;

                publish_auction_closed_event(
                    &env,
                    auction_id.clone(),
                    state.highest_bidder.clone(),
                    state.highest_bid,
                );
            }
        }

        env.storage().persistent().set(&auction_id, &state);
        bump_auction_state_ttl(&env, &auction_id);
    }

    pub fn settle_default_liquidation(
        env: Env,
        auction_id: Symbol,
        credit_contract: Address,
        borrower: Address,
    ) -> i128 {
        let factory = get_factory_contract(&env)
            .unwrap_or_else(|| env.panic_with_error(AuctionError::NoFactoryContract));
        factory.require_auth();
        if credit_contract != factory {
            env.panic_with_error(AuctionError::Unauthorized);
        }

        let state: AuctionState = env
            .storage()
            .persistent()
            .get(&auction_id)
            .unwrap_or_else(|| env.panic_with_error(AuctionError::NotFound));
        bump_auction_state_ttl(&env, &auction_id);

        if state.status != AuctionStatus::Closed {
            env.panic_with_error(AuctionError::NotClosed);
        }

        let settlement_key = AuctionKey::LiquidationSettled(auction_id.clone());
        bump_settlement_marker_ttl(&env, &settlement_key);
        let already_settled = env
            .storage()
            .persistent()
            .get::<AuctionKey, bool>(&settlement_key)
            .unwrap_or(false);
        if already_settled {
            env.panic_with_error(AuctionError::AlreadySettled);
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

        state.highest_bid
    }

    pub fn claim_auction(env: Env, auction_id: Symbol) {
        let state: AuctionState = env
            .storage()
            .persistent()
            .get(&auction_id)
            .unwrap_or_else(|| env.panic_with_error(AuctionError::NotFound));
        bump_auction_state_ttl(&env, &auction_id);

        if state.status != AuctionStatus::Closed {
            env.panic_with_error(AuctionError::AuctionNotClosed);
        }

        let winner = state
            .highest_bidder
            .clone()
            .unwrap_or_else(|| env.panic_with_error(AuctionError::NoWinner));
        winner.require_auth();

        if state.status == AuctionStatus::Claimed {
            env.panic_with_error(AuctionError::AlreadyClaimed);
        }

        let mut updated_state = state;
        updated_state.status = AuctionStatus::Claimed;
        env.storage()
            .persistent()
            .set(&auction_id, &updated_state);
        bump_auction_state_ttl(&env, &auction_id);

        set_reentrancy_guard(&env);
        // token_client.transfer(...) — proceeds to winner (to be wired up)
        clear_reentrancy_guard(&env);
    }
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod test;