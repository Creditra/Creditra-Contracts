#![cfg_attr(not(test), no_std)]

pub mod curves;
mod errors;
mod events;
mod storage;
mod types;

pub use curves::{calculate_price, CurveError, DecayCurve};
pub use errors::AuctionError;
pub use events::BidRefundedEvent;
pub use types::{AuctionMode, AuctionState, AuctionStatus, DutchAuctionDecay};

use soroban_sdk::{contract, contractimpl, contracttype, token, Address, BytesN, Env, Symbol};

use crate::storage::{
    bump_auction_state_ttl, bump_instance_ttl, bump_settlement_marker_ttl, clear_reentrancy_guard,
    get_factory_contract, set_liquidation_grace_window, set_reentrancy_guard,
};
use crate::types::*;
use events::{
    publish_auction_closed_event, publish_bid_refunded_event,
    publish_default_liquidation_settlement_event,
};

/// Returns the minimum bid amount that satisfies the `min_increment_bps`
/// requirement over `highest_bid`.
///
/// The threshold is `highest_bid + ceil(highest_bid * bps / 10_000)`, with a
/// floor increment of 1 stroop so there is always forward progress.
///
/// # Errors
/// Panics with [`AuctionError::BidTooLow`] on i128 overflow (requires a bid
/// in the quintillion-strobe range; effectively unreachable in practice).
fn min_next_bid(env: &Env, highest_bid: i128, min_increment_bps: u32) -> i128 {
    let bps = min_increment_bps as i128;
    let product = highest_bid
        .checked_mul(bps)
        .unwrap_or_else(|| env.panic_with_error(AuctionError::BidTooLow));
    let bps_increment = product / 10_000 + i128::from(product % 10_000 != 0);
    let increment = bps_increment.max(1);
    highest_bid
        .checked_add(increment)
        .unwrap_or_else(|| env.panic_with_error(AuctionError::BidTooLow))
}

fn validate_auction_curve_params(
    mode: AuctionMode,
    min_bid: i128,
    dutch_start_price: Option<i128>,
    dutch_floor_price: Option<i128>,
    dutch_decay: DutchAuctionDecay,
    dutch_step_count: Option<u32>,
) {
    if mode != AuctionMode::Dutch {
        if dutch_start_price.is_some()
            || dutch_floor_price.is_some()
            || dutch_step_count.is_some()
        {
            panic!("dutch curve parameters are only valid in Dutch mode");
        }
        return;
    }

    let start = dutch_start_price.expect("dutch_start_price required for Dutch mode");
    let floor = dutch_floor_price.expect("dutch_floor_price required for Dutch mode");

    if start < floor {
        panic!("dutch_start_price must be >= dutch_floor_price");
    }
    if start < min_bid {
        panic!("dutch_start_price must be >= min_bid");
    }

    match dutch_decay {
        DutchAuctionDecay::None | DutchAuctionDecay::Linear | DutchAuctionDecay::Exponential => {
            if dutch_step_count.is_some() {
                panic!("dutch_step_count must be None unless DutchAuctionDecay::Stepped");
            }
        }
        DutchAuctionDecay::Stepped => {
            let step_count = dutch_step_count
                .unwrap_or_else(|| panic!("dutch_step_count required for stepped Dutch auctions"));
            if step_count == 0 {
                panic!("dutch_step_count must be > 0 for stepped Dutch auctions");
            }
        }
    }
}

/// Computes the current Dutch auction price based on elapsed time.
///
/// # Overview
///
/// In a Dutch (descending) auction the price starts at `start_price` and
/// decreases over time until it reaches `floor_price` at the end of the
/// auction window.  This function returns the price that a qualifying bid
/// must meet (or exceed) at a given point in time.
///
/// # Parameters
///
/// | Parameter      | Description |
/// |----------------|-------------|
/// | `start_price`  | Price at the beginning of the auction (`t = 0`). Must be ≥ `floor_price`. |
/// | `floor_price`  | Minimum price the auction can reach. The returned price is clamped to this value. |
/// | `elapsed_time` | Seconds elapsed since the auction started. |
/// | `duration`     | Total auction duration in seconds. |
/// | `decay`        | Shape of the price-decay curve (see [`DutchAuctionDecay`]). |
/// | `step_count`   | Required only for [`DutchAuctionDecay::Stepped`]; ignored for all other decay kinds. |
///
/// # Decay curves
///
/// ## Linear (`DutchAuctionDecay::Linear` or `DutchAuctionDecay::None`)
///
/// ```text
/// p(t) = start_price − ⌊(start_price − floor_price) × t / duration⌋
/// ```
///
/// The price drops at a constant rate from `start_price` to `floor_price`.
/// `DutchAuctionDecay::None` is treated identically to `Linear` (the
/// default when no explicit decay is configured).
///
/// ## Stepped (`DutchAuctionDecay::Stepped`)
///
/// ```text
/// p(t) = start_price − ⌊(start_price − floor_price) × ⌊t × steps / duration⌋ / steps⌋
/// ```
///
/// This is a step-down curve: the price remains constant within each of the
/// `step_count` equal-duration buckets and only drops at bucket boundaries.
/// The total drop from `start_price` to `floor_price` is split across those
/// buckets.  `step_count` **must** be `Some(n)` where `n > 0`.
///
/// ## Exponential (`DutchAuctionDecay::Exponential`)
///
/// ```text
/// factor(t)  = 0.99 ^ min(t, 100)
/// drop(t)    = (start_price − floor_price) × (1 − factor(t))
/// p(t)       = start_price − drop(t)
/// ```
///
/// Approximately 1 % multiplicative decay per time unit.  The
/// iteration count is capped at 100 to bound gas consumption.
///
/// # Return value
///
/// The current Dutch auction price, guaranteed to be ≥ `floor_price`.
///
/// # Edge cases
///
/// * `duration == 0` → returns `floor_price` immediately (avoids division by zero).
/// * `elapsed_time >= duration` → returns `floor_price` (auction window expired).
/// * If `start_price < floor_price`, the function **panics** — callers
///   must validate parameters at auction creation time.
///
/// # Examples
///
/// ```
/// use gateway_auction::DutchAuctionDecay;
///
/// // Linear: price at start
/// assert_eq!(compute_dutch_price(1000, 500, 0, 100, &DutchAuctionDecay::Linear, None), 1000);
/// // Linear: price halfway
/// assert_eq!(compute_dutch_price(1000, 500, 50, 100, &DutchAuctionDecay::Linear, None), 750);
/// // Linear: price at end
/// assert_eq!(compute_dutch_price(1000, 500, 100, 100, &DutchAuctionDecay::Linear, None), 500);
/// ```
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
    /// Replay barrier for a successful bid acceptance. The identity is the
    /// auction id plus the bidder address and exact bid amount. Reusing the same
    /// identity is treated as a no-op so a retried transaction cannot re-apply
    /// the same state transition or refund path.
    BidAccepted(Symbol, Address, i128),
}

#[contractimpl]
impl Auction {
    /// Initializes a new auction.
    ///
    /// # Parameters
    /// - `env`: The execution environment.
    /// - `auction_id`: The unique identifier for the auction.
    /// - `mode`: The mode of the auction (e.g., English or Dutch).
    /// - `start_time`: The timestamp when the auction starts.
    /// - `end_time`: The timestamp when the auction ends.
    /// - `min_bid`: The minimum initial bid (English) or floor price equivalent logic.
    /// - `min_increment_bps`: The minimum bid increment in basis points (max 10000).
    /// - `dutch_start_price`: The starting price for a Dutch auction.
    /// - `dutch_floor_price`: The lowest possible price for a Dutch auction.
    /// - `dutch_decay`: The price decay configuration for a Dutch auction.
    /// - `dutch_step_count`: Required steps if decay is `Stepped`.
    ///
    /// # Panics
    /// Panics if `start_time >= end_time`, if `min_increment_bps > 10_000`, or if Dutch auction parameters are invalid.
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
        bump_instance_ttl(&env);
        if start_time >= end_time {
            panic!("invalid times");
        }
        if min_increment_bps > 10_000 {
            panic!("min_increment_bps exceeds maximum of 10000 (100%)");
        }

        validate_auction_curve_params(
            mode,
            min_bid,
            dutch_start_price,
            dutch_floor_price,
            dutch_decay,
            dutch_step_count,
        );

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

    /// Sets the factory contract address.
    ///
    /// # Authorization
    /// Requires `require_auth` from the factory itself.
    pub fn set_factory_contract(env: Env, factory: Address) {
        bump_instance_ttl(&env);
        factory.require_auth();
        storage::set_factory_contract(&env, &factory);
    }

    pub fn place_bid(env: Env, auction_id: Symbol, bidder: Address, amount: i128) {
        bump_instance_ttl(&env);
        bidder.require_auth();

        let bid_identity = AuctionKey::BidAccepted(auction_id.clone(), bidder.clone(), amount);
        if env.storage().persistent().has(&bid_identity) {
            env.storage().persistent().extend_ttl(
                &bid_identity,
                crate::storage::PERSISTENT_LIFETIME_THRESHOLD,
                crate::storage::PERSISTENT_BUMP_AMOUNT,
            );
            return;
        }

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

        let grace_window = storage::get_liquidation_grace_window(&env);
        if grace_window > 0 {
            let earliest_start = state.config.start_time.saturating_add(grace_window);
            if now < earliest_start {
                env.panic_with_error(AuctionError::GracePeriodActive);
            }
        }

        match state.config.mode {
            AuctionMode::English => {
                let threshold = if state.highest_bid > 0 {
                    min_next_bid(&env, state.highest_bid, state.config.min_increment_bps)
                        .max(state.config.min_bid)
                } else {
                    min_next_bid(&env, state.config.min_bid, state.config.min_increment_bps)
                };
                if amount < threshold {
                    env.panic_with_error(AuctionError::BidTooLow);
                }

                let token_addr: Option<Address> = env
                    .storage()
                    .instance()
                    .get(&Symbol::new(&env, "bid_token"));

                let previous_bidder = state.highest_bidder.clone();
                let previous_bid_amount = state.highest_bid;

                // The outbid refund and the incoming bid escrow must complete as one
                // atomic ledger transition. The state update only happens after both
                // token transfers succeed; otherwise the entire transaction reverts and
                // the auction remains unchanged.
                if let Some(ref tkn) = token_addr {
                    set_reentrancy_guard(&env);
                    let token_client = token::Client::new(&env, tkn);
                    token_client.transfer(&bidder, &env.current_contract_address(), &amount);

                    if let Some(prev_bidder) = previous_bidder.clone() {
                        publish_bid_refunded_event(&env, prev_bidder.clone(), previous_bid_amount);
                        token_client.transfer(
                            &env.current_contract_address(),
                            &prev_bidder,
                            &previous_bid_amount,
                        );
                    }

                    clear_reentrancy_guard(&env);
                }

                state.highest_bidder = Some(bidder);
                state.highest_bid = amount;
            }

            AuctionMode::Dutch => {
                let current_time = env.ledger().timestamp();
                let elapsed_time = current_time.saturating_sub(state.config.start_time);
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

                let token_addr: Option<Address> = env
                    .storage()
                    .instance()
                    .get(&Symbol::new(&env, "bid_token"));

                if let Some(ref tkn) = token_addr {
                    set_reentrancy_guard(&env);
                    let token_client = token::Client::new(&env, tkn);
                    token_client.transfer(&bidder, &env.current_contract_address(), &amount);
                    clear_reentrancy_guard(&env);
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

        env.storage().persistent().set(&bid_identity, &true);
        env.storage().persistent().extend_ttl(
            &bid_identity,
            crate::storage::PERSISTENT_LIFETIME_THRESHOLD,
            crate::storage::PERSISTENT_BUMP_AMOUNT,
        );
    }

    /// Closes an open auction, transitioning its status from `Open` to `Closed`.
    ///
    /// The caller must be the registered factory contract.  After closing, the
    /// auction is eligible for `settle_default_liquidation` (factory-only) or
    /// `claim_auction` (winner-only).
    ///
    /// # Authorization
    /// Requires [`Address::require_auth`] from the registered factory contract.
    ///
    /// # Parameters
    /// - `env`: The execution environment.
    /// - `auction_id`: The identifier of the auction to close.
    ///
    /// # Errors
    /// * [`AuctionError::NoFactoryContract`] — factory address not configured.
    /// * [`AuctionError::NotFound`] — no auction found for `auction_id`.
    /// * [`AuctionError::AuctionNotOpen`] — auction is already `Closed`.
    /// * [`AuctionError::AlreadyClaimed`] — auction is in `Claimed` terminal state.
    pub fn close_auction(env: Env, auction_id: Symbol) {
        let factory = get_factory_contract(&env)
            .unwrap_or_else(|| env.panic_with_error(AuctionError::NoFactoryContract));
        factory.require_auth();

        let mut state: AuctionState = env
            .storage()
            .persistent()
            .get(&auction_id)
            .unwrap_or_else(|| env.panic_with_error(AuctionError::NotFound));
        bump_auction_state_ttl(&env, &auction_id);

        match state.status {
            AuctionStatus::Claimed => env.panic_with_error(AuctionError::AlreadyClaimed),
            AuctionStatus::Closed => env.panic_with_error(AuctionError::AuctionNotOpen),
            AuctionStatus::Open => {}
        }

        state.status = AuctionStatus::Closed;
        env.storage().persistent().set(&auction_id, &state);
        bump_auction_state_ttl(&env, &auction_id);

        events::publish_auction_closed_event(
            &env,
            auction_id,
            state.highest_bidder,
            state.highest_bid,
        );
    }

    /// Settles an auction that ended in default or completes the liquidation process.
    ///
    /// Transfers the highest bid amount (if any) to the credit contract.
    ///
    /// # Authorization
    /// Requires `require_auth` from the factory contract.
    ///
    /// # Returns
    /// The `highest_bid` amount that was settled.
    ///
    /// # Panics
    /// * [`AuctionError::NoFactoryContract`] - Factory contract not set.
    /// * [`AuctionError::Unauthorized`] - Caller is not the factory contract.
    /// * [`AuctionError::NotFound`] - Auction not found.
    /// * [`AuctionError::NotClosed`] - Auction is not in the `Closed` state.
    /// * [`AuctionError::AlreadySettled`] - Auction has already been settled.
    pub fn settle_default_liquidation(
        env: Env,
        auction_id: Symbol,
        credit_contract: Address,
        borrower: Address,
    ) -> i128 {
        bump_instance_ttl(&env);
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
            credit_contract.clone(),
            borrower,
            winner,
            state.highest_bid,
        );

        let token_addr: Option<Address> = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, "bid_token"));
        if let Some(tkn) = token_addr {
            if state.highest_bid > 0 {
                set_reentrancy_guard(&env);
                let token_client = token::Client::new(&env, &tkn);
                token_client.transfer(
                    &env.current_contract_address(),
                    &credit_contract,
                    &state.highest_bid,
                );
                clear_reentrancy_guard(&env);
            }
        }

        state.highest_bid
    }

    /// Claims the proceeds or assets of a closed auction by the winning bidder.
    ///
    /// # Authorization
    /// Requires `require_auth` from the winning bidder.
    ///
    /// # Panics
    /// * [`AuctionError::NotFound`] - Auction not found.
    /// * [`AuctionError::AuctionNotClosed`] - Auction is not in `Closed` status.
    /// * [`AuctionError::AlreadySettled`] - Auction has already been liquidated/settled.
    /// * [`AuctionError::NoWinner`] - There is no winning bidder.
    /// * [`AuctionError::AlreadyClaimed`] - Auction was already claimed.
    /// * [`AuctionError::InvalidState`] - Bid token not found in storage.
    pub fn claim_auction(env: Env, auction_id: Symbol) {
        bump_instance_ttl(&env);
        let state: AuctionState = env
            .storage()
            .persistent()
            .get(&auction_id)
            .unwrap_or_else(|| env.panic_with_error(AuctionError::NotFound));
        bump_auction_state_ttl(&env, &auction_id);

        if state.status != AuctionStatus::Closed {
            env.panic_with_error(AuctionError::AuctionNotClosed);
        }

        let settlement_key = AuctionKey::LiquidationSettled(auction_id.clone());
        let already_settled = env
            .storage()
            .persistent()
            .get::<AuctionKey, bool>(&settlement_key)
            .unwrap_or(false);
        if already_settled {
            env.panic_with_error(AuctionError::AlreadySettled);
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
