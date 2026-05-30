use soroban_sdk::{contracttype, Address, BytesN};

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
    pub username_hash: BytesN<32>,
    pub start_time: u64,
    pub end_time: u64,
    pub min_bid: i128,
    /// Minimum outbid increment expressed in basis points (1 bps = 0.01%).
    /// Each new bid must be at least `highest * (1 + min_increment_bps / 10_000)`.
    /// Capped at 10_000 (100%) on init. Use 0 to require only a 1-stroop increment.
    pub min_increment_bps: u32,
    /// Anti-snipe: final window in seconds before end_time where bids trigger extensions.
    /// Set to 0 to disable anti-snipe mechanism.
    pub extension_window: u64,
    /// Anti-snipe: duration in seconds added to end_time when a late bid is placed.
    pub extension_amount: u64,
    /// Anti-snipe: maximum number of extensions allowed to prevent infinite auctions.
    pub max_extensions: u32,
    /// Anti-snipe: current count of extensions that have been applied.
    pub extensions_count: u32,
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
