//! Auction event payloads and publishers.
//!
//! # What
//!
//! Three `#[contracttype]` event payload structs and their topic-publishing
//! helpers:
//!
//! - [`BidRefundedEvent`] on topic `(BID_RFDN, auction)` — emitted when an
//!   English-mode auction atomically refunds the previous highest bidder.
//!   This event is emitted *before* the refund token CPI under the
//!   reentrancy guard, so indexers can pair it with the on-chain transfer.
//! - [`AuctionClosedEvent`] on topic `(AUC_CLOSE, auction)` — emitted on
//!   English manual close and on Dutch auto-close from a qualifying bid.
//! - [`DefaultLiquidationSettlementEvent`] on topic
//!   `(LIQ_SETL, auction)` — emitted once per auction when the credit
//!   contract calls `settle_default_liquidation`. Replay-protected by a
//!   persistent marker (see [`crate::storage`]).
//!
//! # How
//!
//! All topics are `symbol_short!` (≤ 9 characters) so encoding is cheap and
//! deterministic. Publishers take `&Env` plus the event-specific payload
//! fields and call `env.events().publish(topic, payload)`. No mutation of
//! contract state happens in this module.
//!
//! # Why
//!
//! These three events are the auction contract's entire outward
//! signaling surface — together with the credit contract's
//! `("credit","liq_req")` and `("credit","liq_setl")` topics, they let an
//! off-chain orchestrator deterministically reconstruct the cross-contract
//! default-liquidation flow. See
//! [`docs/indexer-integration.md`](../../../../docs/indexer-integration.md)
//! for the indexer schema and
//! [`docs/ARCHITECTURE.md`](../../../../docs/ARCHITECTURE.md) for the
//! sequence diagram.
//!
//! # Stability
//!
//! Topic strings and payload field layouts are ABI-stable. Breaking changes
//! require a new event topic suffix (e.g. `LIQ_SETL2`) and a major version
//! bump.

use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BidRefundedEvent {
    pub prev_bidder: Address,
    pub amount: i128,
}

pub fn publish_bid_refunded_event(env: &Env, prev_bidder: Address, amount: i128) {
    env.events().publish(
        (symbol_short!("BID_RFDN"), symbol_short!("auction")),
        BidRefundedEvent {
            prev_bidder,
            amount,
        },
    );
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuctionClosedEvent {
    pub auction_id: Symbol,
    pub winner: Option<Address>,
    pub amount: i128,
}

pub fn publish_auction_closed_event(
    env: &Env,
    auction_id: Symbol,
    winner: Option<Address>,
    amount: i128,
) {
    env.events().publish(
        (symbol_short!("AUC_CLOSE"), symbol_short!("auction")),
        AuctionClosedEvent {
            auction_id,
            winner,
            amount,
        },
    );
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DefaultLiquidationSettlementEvent {
    pub auction_id: Symbol,
    pub credit_contract: Address,
    pub borrower: Address,
    pub winner: Address,
    pub recovered_amount: i128,
}

pub fn publish_default_liquidation_settlement_event(
    env: &Env,
    auction_id: Symbol,
    credit_contract: Address,
    borrower: Address,
    winner: Address,
    recovered_amount: i128,
) {
    env.events().publish(
        (symbol_short!("LIQ_SETL"), symbol_short!("auction")),
        DefaultLiquidationSettlementEvent {
            auction_id,
            credit_contract,
            borrower,
            winner,
            recovered_amount,
        },
    );
}
