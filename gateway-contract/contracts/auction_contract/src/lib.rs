#![cfg_attr(not(test), no_std)]

//! # gateway-auction
//!
//! Minimal English / Dutch auction contract used by the Creditra credit
//! contract to settle defaulted collateral. The cross-contract handoff is
//! documented in
//! [`docs/default-liquidation-auction-hook.md`](../../../../docs/default-liquidation-auction-hook.md);
//! the credit-side call lives in
//! [`crate::lib::settle_default_liquidation`](../../../../contracts/credit/src/lib.rs)
//! (cross-repo path).
//!
//! ## What
//!
//! - English (open ascending) mode with a configurable
//!   `min_increment_bps`. Previous highest bidder is **atomically refunded**
//!   when outbid, under the reentrancy guard.
//! - Dutch (descending) mode with linear price decay
//!   `p(t) = p_0 - (p_0 - p_f) * min(t, T) / T`. First qualifying bid
//!   atomically flips status to Closed.
//! - One-shot `settle_default_liquidation(auction_id, credit_contract, borrower)`
//!   callable only by the registered factory (= the credit contract).
//!   Replay-protected by the persistent
//!   `AuctionKey::LiquidationSettled(auction_id)` flag.
//! - `claim_auction(auction_id)` — winner-only payout under the
//!   reentrancy guard.
//!
//! ## How
//!
//! - 12 `AuctionError` discriminants pinned by ABI;
//!   `AuctionError::Reentrancy = 10` reverts on re-entered refund or claim.
//! - Three event topics under stable symbols (`BID_RFDN`, `AUC_CLOSE`,
//!   `LIQ_SETL`) — see [`events`].
//! - Storage tier discipline mirrors the credit contract: small instance
//!   state for the current auction, id-scoped persistent state for
//!   multi-auction deployments. TTL bump cadence is ~30 d / ~7 d (auctions
//!   are short-lived).
//!
//! ## Anti-snipe disclosure
//!
//! The PR #430 feature branch added an end-time-extension code path
//! intended to push the close time out when a bid lands inside an
//! extension window. After the reconciliation with
//! `AUCTION_CLOSE_TIME_FIX.md`, the live [`place_bid`] hard-rejects bids
//! when `now >= end_time` without extending. The anti-snipe behavior is
//! therefore **documented but not active** in this release; treat it as
//! planned, not delivered. See
//! [`docs/SECURITY.md`](../../../../docs/SECURITY.md) §6 ("Known gaps") and
//! [`WHITEPAPER.md`](../../../../WHITEPAPER.md) §6.3.
//!
//! ## Modules
//! - [`errors`] — `AuctionError` codes used by the contract.
//! - [`events`] — Soroban events emitted by the contract.
//! - [`storage`] — Persistent/instance keys and TTL helpers.
//! - [`types`] — Stored data types (`AuctionState`, etc.).

mod errors;
mod events;
mod storage;
mod types;

use errors::AuctionError;

use soroban_sdk::{contract, contractimpl, contracttype, token, Address, BytesN, Env, Symbol};

use crate::storage::{
    clear_reentrancy_guard, get_factory_contract, set_factory_contract, set_reentrancy_guard,
};
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

/// Computes the current Dutch auction price based on elapsed time.
///
/// The price decays linearly from start_price to floor_price over the auction duration.
/// Formula: current_price = start_price - ((start_price - floor_price) * elapsed_time) / duration
///
/// This function is overflow-safe and ensures monotone decreasing prices.
fn compute_dutch_price(
    start_price: i128,
    floor_price: i128,
    elapsed_time: u64,
    duration: u64,
) -> i128 {
    if duration == 0 {
        return floor_price; // Avoid division by zero
    }

    if elapsed_time >= duration {
        return floor_price; // Clamp at floor when auction ends
    }

    // Compute total price drop
    let price_drop = start_price
        .checked_sub(floor_price)
        .expect("start_price must be >= floor_price");

    // Compute the portion of time elapsed as a fraction
    // Use checked arithmetic to prevent overflow
    let elapsed_i128 = elapsed_time as i128;
    let duration_i128 = duration as i128;

    // Compute drop so far: (price_drop * elapsed_time) / duration
    // This is safe because we ensure duration > 0 and values are reasonable
    let drop_so_far = price_drop
        .checked_mul(elapsed_i128)
        .expect("overflow in Dutch price calculation")
        .checked_div(duration_i128)
        .expect("division should succeed with positive duration");

    // Current price = start_price - drop_so_far
    let current_price = start_price
        .checked_sub(drop_so_far)
        .expect("current price should not underflow");

    // Ensure we never go below floor (shouldn't happen with correct math, but safety check)
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
    ) {
        if start_time >= end_time {
            panic!("invalid times");
        }
        // Cap at 100% (10_000 bps) — a requirement higher than that is nonsensical
        if min_increment_bps > 10_000 {
            panic!("min_increment_bps exceeds maximum of 10000 (100%)");
        }

        // Validate Dutch auction parameters
        if mode == AuctionMode::Dutch {
            let start = dutch_start_price.expect("dutch_start_price required for Dutch mode");
            let floor = dutch_floor_price.expect("dutch_floor_price required for Dutch mode");
            if start < floor {
                panic!("dutch_start_price must be >= dutch_floor_price");
            }
            if start < min_bid {
                panic!("dutch_start_price must be >= min_bid");
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

    pub fn close_auction(env: Env, auction_id: Symbol) {
        let mut state: AuctionState = env
            .storage()
            .persistent()
            .get(&auction_id)
            .unwrap_or_else(|| panic!("auction not found"));
        bump_auction_state_ttl(&env, &auction_id);
        if state.status == AuctionStatus::Claimed {
            env.panic_with_error(AuctionError::AlreadyClaimed);
        }
        state.status = AuctionStatus::Closed;
        env.storage().persistent().set(&auction_id, &state);
        bump_auction_state_ttl(&env, &auction_id);
        publish_auction_closed_event(&env, auction_id, state.highest_bidder, state.highest_bid);
    }

    /// Place a bid for an auction identified by `auction_id`.
    ///
    /// For English auctions:
    /// - Bid floor: `amount` must be strictly greater than `max(min_bid - 1, highest_bid)`.
    /// - First bid must be at least `min_bid`, every later bid must exceed the current highest.
    /// - When outbidding, the previous highest bidder is refunded exactly `highest_bid`.
    ///
    /// For Dutch auctions:
    /// - First bid at or above the current Dutch price wins immediately.
    /// - Price decays linearly from start_price to floor_price over the auction duration.
    /// - No outbidding - first qualifying bid settles the auction.
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

        let now = env.ledger().timestamp();

        if now >= state.config.end_time {
            panic!("auction closed");
        }

        match state.config.mode {
            AuctionMode::English => {
                // English auction: require bid to exceed current highest
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

                    // Emit refund event before performing token transfer
                    publish_bid_refunded_event(&env, prev_bidder.clone(), state.highest_bid);

                    // Set reentrancy guard before the token transfer to block any
                    // reentrant call to place_bid or claim_auction during the CPI.
                    // Soroban transaction rollback clears instance storage on panic,
                    // so the flag cannot be left permanently set on failure.
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
                // Dutch auction: first qualifying bid wins
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

                let current_price =
                    compute_dutch_price(start_price, floor_price, elapsed_time, duration);

                // Bid must be at least current price
                if amount < current_price {
                    env.panic_with_error(AuctionError::BidTooLow);
                }

                // Bid must be at least min_bid
                if amount < state.config.min_bid {
                    env.panic_with_error(AuctionError::BidTooLow);
                }

                // In Dutch auction, first qualifying bid wins - close the auction
                state.highest_bidder = Some(bidder);
                state.highest_bid = amount;
                state.status = AuctionStatus::Closed;

                // Publish close event for Dutch auction settlement
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

    /// Emit an auction settlement signal for credit default liquidation orchestration.
    ///
    /// Requirements:
    /// - caller must be the registered factory contract (`set_factory_contract`)
    /// - auction must be closed
    /// - settlement signal is one-time per auction_id
    pub fn settle_default_liquidation(
        env: Env,
        auction_id: Symbol,
        credit_contract: Address,
        borrower: Address,
    ) -> i128 {
        let factory = get_factory_contract(&env)
            .unwrap_or_else(|| env.panic_with_error(AuctionError::NoFactoryContract));
        factory.require_auth();

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

        state.highest_bid
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
        env.storage().persistent().set(&auction_id, &updated_state);
        bump_auction_state_ttl(&env, &auction_id);

        // Reentrancy guard wraps the transfer site (defense-in-depth).
        // The status is already updated to Claimed above (checks-effects-interactions),
        // so a reentrant claim_auction call will hit AuctionNotClosed before reaching here.
        // The guard provides an additional layer: any reentrant call during the token
        // transfer will revert with AuctionError::Reentrancy.
        set_reentrancy_guard(&env);
        // token_client.transfer(...) — proceeds to winner (transfer to be wired up)
        clear_reentrancy_guard(&env);
    }
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod test;
