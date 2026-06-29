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

    let p_u128 = price_drop as u128;

    let drop_so_far = match decay {
        DutchAuctionDecay::None | DutchAuctionDecay::Linear => {
            let e_u128 = elapsed_time as u128;
            let d_u128 = duration as u128;
            
            let q = p_u128 / d_u128;
            let r = p_u128 % d_u128;
            
            let drop = (q * e_u128) + ((r * e_u128) / d_u128);
            drop as i128
        }

        DutchAuctionDecay::Stepped => {
            let steps = match step_count {
                Some(s) if s > 0 => s as u128,
                Some(_) => panic!("dutch_step_count must be > 0 for stepped Dutch auctions"),
                None => panic!("dutch_step_count required for stepped Dutch auctions"),
            };
            
            let e_u128 = elapsed_time as u128;
            let d_u128 = duration as u128;
            let elapsed_steps = (e_u128 * steps) / d_u128;
            
            let q = p_u128 / steps;
            let r = p_u128 % steps;
            
            let drop = (q * elapsed_steps) + ((r * elapsed_steps) / steps);
            drop as i128
        }

        DutchAuctionDecay::Exponential => {
            let t = elapsed_time.min(100);
            let mut factor = 10_000u128;
            for _ in 0..t {
                factor = (factor * 9_900) / 10_000;
            }
            let drop_factor = 10_000 - factor;
            let q = p_u128 / 10_000;
            let r = p_u128 % 10_000;
            
            let drop = (q * drop_factor) + ((r * drop_factor) / 10_000);
            drop as i128
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
    /// Place a bid in an English ascending-price auction.
    ///
    /// # Parameters
    ///
    /// * `env` — The Soroban contract environment.
    /// * `auction_id` — Symbol identifying the auction. Multiple auctions may coexist in persistent storage.
    /// * `bidder` — Address of the bidder. Must authorize this invocation; unauthorized calls panic.
    /// * `amount` — Bid amount in the auction's token denomination. Must be strictly positive and strictly greater than the current highest bid (if any).
    ///
    /// # Panics
    ///
    /// * If `amount <= 0` — bids must be positive.
    /// * If `amount <= previous_highest_bid.amount` — bids must strictly increase.
    ///
    /// # Behavior
    ///
    /// 1. **Authorization check**: `bidder.require_auth()` — panics if caller is not the bidder.
    /// 2. **Load previous highest bid** from persistent storage keyed by `auction_id`.
    /// 3. **If a previous bid exists**:
    ///    - Validate new bid is strictly higher (panics otherwise).
    ///    - **Emit** `BID_RFDN` event (via `publish_bid_refunded_event`) *before* transfer — ensures event ordering even if transfer fails.
    ///    - **Refund** previous bidder: if a `bid_token` address is stored in instance storage, transfers `prev.amount` from contract to `prev.bidder`. If no token is configured, refund is skipped (useful for test mocks).
    /// 4. **Store new highest bid**: overwrites `auction_id` key with `AuctionState { bidder, amount }`.
    ///
    /// # Refund Safety
    ///
    /// Event emission precedes the token transfer. If the transfer fails (e.g., insufficient contract balance, token panic), the event is already recorded, enabling off-chain reconciliation.
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Alice bids 100 on auction "rare_nft"
    /// place_bid(env.clone(), symbol_short!("rare_nft"), alice_addr.clone(), 100);
    /// // State: { bidder: alice_addr, amount: 100 } stored under "rare_nft"
    ///
    /// // Bob bids 150 — Alice receives refund of 100, BID_RFDN event emitted
    /// place_bid(env.clone(), symbol_short!("rare_nft"), bob_addr.clone(), 150);
    /// // State: { bidder: bob_addr, amount: 150 }
    ///
    /// // Charlie attempts to bid 120 — panics (120 <= 150)
    /// // place_bid(env, symbol_short!("rare_nft"), charlie_addr, 120); // ❌ panic
    /// ```
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

                if let (Some(prev_bidder), Some(tkn)) = (state.highest_bidder.clone(), token_addr) {
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

    /// Claim the escrowed auction proceeds, transferring `highest_bid` to the winner.
    ///
    /// # Authorization
    /// Requires auth from the configured winning bidder (stored as `highest_bidder`).
    ///
    /// # Panics
    /// Panics with one of the [`AuctionError`] variants when the auction is not
    /// in `Closed` state, already claimed, or has no winner.
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

        let recovered_amount = state.highest_bid;
        let token_addr: Option<Address> = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, "bid_token"));
        let token_addr =
            token_addr.unwrap_or_else(|| env.panic_with_error(AuctionError::InvalidState));

        let mut updated_state = state;
        updated_state.status = AuctionStatus::Claimed;
        env.storage().persistent().set(&auction_id, &updated_state);
        bump_auction_state_ttl(&env, &auction_id);

        set_reentrancy_guard(&env);
        let token_client = token::Client::new(&env, &token_addr);
        token_client.transfer(&env.current_contract_address(), &winner, &recovered_amount);
        clear_reentrancy_guard(&env);
    }
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod test;
