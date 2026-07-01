//! Auction contract type definitions.
//!
//! # What
//!
//! ABI types shared between the auction `#[contractimpl]` block and storage:
//!
//! - [`AuctionMode`] — English (ascending) or Dutch (descending) bid model.
//! - [`AuctionStatus`] — Open → Closed → Claimed terminal lifecycle.
//! - [`AuctionConfig`] — immutable per-auction parameters set at init.
//! - [`DutchAuctionDecay`] — Dutch price-curve shape (linear or stepped).
//! - [`AuctionState`] — mutable bid state (highest bidder & bid amount).
//! - [`Bid`] — single-bid record (currently informational, not persisted
//!   per-bid).
//! - [`DataKey`] — instance-storage keys used by the auction contract.
//! - [`AuctionKey`] — id-scoped persistent keys for the alternate storage
//!   API exposed by [`crate::storage`].
//!
//! # How
//!
//! All types are `#[contracttype]`-tagged so they cross the Soroban host ABI
//! boundary as structured values. Discriminants are ABI-stable; new variants
//! must be appended (see `gateway-contract/contracts/auction_contract/src/errors.rs`
//! for the same discipline applied to [`crate::errors::AuctionError`]).
//!
//! # Why
//!
//! The English mode is the protocol's default for asset disposal: bidders
//! atomically refund the previous highest bidder under the reentrancy guard
//! when outbid. The Dutch mode is included so the credit contract's default-
//! liquidation handoff can settle on a known-bounded timeline — first
//! qualifying bid wins and closes the auction in the same transaction.
//!
//! # Storage tier
//!
//! The instance `DataKey` variants store the *current* auction's
//! configuration and state in instance storage (small, hot). The persistent
//! [`AuctionKey`] variants — `Seller(id)`, `Asset(id)`, etc. — encode an
//! id-scoped namespace used by the alternate API in [`crate::storage`] when
//! the contract serves multiple auctions concurrently.
//!
//! See [`docs/default-liquidation-auction-hook.md`](../../../../docs/default-liquidation-auction-hook.md)
//! for the cross-contract settlement protocol.

use soroban_sdk::{contracttype, Address, BytesN};

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuctionMode {
    /// English auction: ascending price, highest bidder wins at end
    English,
    /// Dutch auction: descending price, first qualifying bid wins
    Dutch,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuctionStatus {
    Open,
    Closed,
    Claimed,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DutchAuctionDecay {
    /// No decay configured — used for English auctions or Dutch auctions
    /// that default to linear decay.
    None,
    /// Continuous linear interpolation from start price to floor price.
    Linear,
    /// Piecewise-constant staircase decay with `dutch_step_count` equal drops.
    Stepped,
    /// Multiplicative ~1%-per-step exponential decay.
    Exponential,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    Status,
    HighestBidder,
    FactoryContract,
    EndTime,
    HighestBid,
    /// Contract-level grace window (in seconds) that must elapse after
    /// auction creation before the first bid can be placed.
    LiquidationGraceWindow,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuctionKey {
    Seller(u32),
    Asset(u32),
    MinBid(u32),
    EndTime(u32),
    HighestBidder(u32),
    HighestBid(u32),
    Status(u32),
    Claimed(u32),
}

#[contracttype]
#[derive(Clone)]
pub struct AuctionConfig {
    pub mode: AuctionMode,
    pub username_hash: BytesN<32>,
    pub start_time: u64,
    pub end_time: u64,
    pub min_bid: i128,
    /// Minimum outbid increment expressed in basis points (1 bps = 0.01%).
    /// Each new bid must be at least `highest * (1 + min_increment_bps / 10_000)`.
    /// Capped at 10_000 (100%) on init. Use 0 to require only a 1-stroop increment.
    pub min_increment_bps: u32,
    /// Starting price for Dutch auction (only used in Dutch mode).
    pub dutch_start_price: Option<i128>,
    /// Floor price for Dutch auction (only used in Dutch mode).
    pub dutch_floor_price: Option<i128>,
    /// Dutch decay shape. `DutchAuctionDecay::None` means linear (default).
    pub dutch_decay: DutchAuctionDecay,
    /// Number of equal time buckets used by [`DutchAuctionDecay::Stepped`].
    /// Required for stepped Dutch auctions; ignored for all other decay kinds.
    pub dutch_step_count: Option<u32>,
}

#[contracttype]
#[derive(Clone)]
pub struct AuctionState {
    pub config: AuctionConfig,
    pub status: AuctionStatus,
    pub highest_bidder: Option<Address>,
    pub highest_bid: i128,
}

#[contracttype]
#[derive(Clone)]
pub struct Bid {
    pub bidder: Address,
    pub amount: i128,
    pub timestamp: u64,
}
