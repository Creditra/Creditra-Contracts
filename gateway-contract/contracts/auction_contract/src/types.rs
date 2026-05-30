use soroban_sdk::{contracttype, Address, BytesN};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
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
pub enum DataKey {
    Status,
    HighestBidder,
    FactoryContract,
    EndTime,
    HighestBid,
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
    /// Starting price for Dutch auction (only used in Dutch mode)
    pub dutch_start_price: Option<i128>,
    /// Floor price for Dutch auction (only used in Dutch mode)
    pub dutch_floor_price: Option<i128>,
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
