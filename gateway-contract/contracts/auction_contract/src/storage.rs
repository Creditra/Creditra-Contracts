//! Auction storage helpers and TTL primitives.
//!
//! # What
//!
//! Typed getters / setters for the auction contract's instance state
//! (current auction config, status, highest bidder, highest bid, factory
//! pointer) plus an id-scoped alternate API for multi-auction deployments
//! (`auction_*` family operating on [`crate::types::AuctionKey`]).
//!
//! Also owns the reentrancy guard primitive
//! ([`set_reentrancy_guard`] / [`clear_reentrancy_guard`]) which wraps the
//! prior-bid refund in English mode and the (placeholder) winner payout in
//! `claim_auction`.
//!
//! # How
//!
//! **Persistent storage** — both reads and writes go through helpers that bump
//! TTL when remaining lifetime drops below
//! [`PERSISTENT_LIFETIME_THRESHOLD`] (~7 days), extending the entry by
//! [`PERSISTENT_BUMP_AMOUNT`] (~30 days). Auction state is short-lived by
//! nature, so the cadence is more aggressive than the credit contract's
//! ~3 / ~6 month cycle.
//!
//! **Instance storage** — every hot entrypoint calls [`bump_instance_ttl`]
//! at the top of its body, extending the contract instance ledger entry by
//! [`INSTANCE_BUMP_AMOUNT`] (~30 days) whenever the remaining TTL drops below
//! [`INSTANCE_LIFETIME_THRESHOLD`] (~7 days). This prevents the instance from
//! being archived mid-auction.
//!
//! # Why
//!
//! Concentrating storage access here lets the auction contract enforce two
//! invariants:
//!
//! 1. **Single-shot settlement** — the persistent flag
//!    `AuctionKey::LiquidationSettled(auction_id)` is set on the first
//!    `settle_default_liquidation` and consulted on subsequent calls,
//!    making replay return `AuctionError::AlreadyClaimed = 2`.
//! 2. **CEI ordering on refund** — the reentrancy guard ensures a
//!    malicious bid token cannot re-enter `place_bid` during the refund
//!    CPI.

use crate::errors::AuctionError;
use crate::types::{AuctionStatus, DataKey};
use soroban_sdk::{Address, Env, Symbol};

/// TTL constants for persistent storage entries.
/// Bump amount: ~30 days (at ~5 s per ledger close).
pub(crate) const PERSISTENT_BUMP_AMOUNT: u32 = 518_400;
/// Lifetime threshold: ~7 days — entries are extended when remaining TTL drops below this.
pub(crate) const PERSISTENT_LIFETIME_THRESHOLD: u32 = 120_960;

/// TTL constants for the contract instance storage entry.
/// Bump amount: ~30 days (at ~5 s per ledger close).
pub(crate) const INSTANCE_BUMP_AMOUNT: u32 = 518_400;
/// Lifetime threshold: ~7 days — instance is extended when remaining TTL drops below this.
pub(crate) const INSTANCE_LIFETIME_THRESHOLD: u32 = 120_960;

/// Extend the contract instance TTL on every hot read path.
///
/// Called at the top of every [`#[contractimpl]`] entrypoint body so that the
/// instance ledger entry is never archived while an auction is in flight.
///
/// # Storage
/// - **Type**: Instance storage (the contract's own instance entry)
/// - **TTL extended when**: remaining lifetime < [`INSTANCE_LIFETIME_THRESHOLD`]
/// - **Extended to**: [`INSTANCE_BUMP_AMOUNT`] ledgers from the current ledger
pub(crate) fn bump_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
}

/// Extend TTL for an `AuctionState` entry stored under `auction_id`.
///
/// Called on every read/write path that may be followed by `claim_auction` so
/// in-flight auctions are not archived mid-lifecycle. Uses `PERSISTENT_BUMP_AMOUNT`
/// as the threshold so freshly created entries (short default TTL) are extended
/// on first touch.
pub(crate) fn bump_auction_state_ttl(env: &Env, auction_id: &Symbol) {
    if env.storage().persistent().has(auction_id) {
        env.storage().persistent().extend_ttl(
            auction_id,
            PERSISTENT_BUMP_AMOUNT,
            PERSISTENT_BUMP_AMOUNT,
        );
    }
}

/// Extend TTL for settlement replay-protection markers (only when the key exists).
pub(crate) fn bump_settlement_marker_ttl(env: &Env, key: &crate::AuctionKey) {
    if env.storage().persistent().has(key) {
        env.storage()
            .persistent()
            .extend_ttl(key, PERSISTENT_BUMP_AMOUNT, PERSISTENT_BUMP_AMOUNT);
    }
}

pub fn get_status(env: &Env) -> AuctionStatus {
    env.storage()
        .instance()
        .get(&DataKey::Status)
        .unwrap_or(AuctionStatus::Open)
}

pub fn set_status(env: &Env, status: AuctionStatus) {
    env.storage().instance().set(&DataKey::Status, &status);
}

pub fn get_highest_bidder(env: &Env) -> Option<Address> {
    env.storage().instance().get(&DataKey::HighestBidder)
}

pub fn set_highest_bidder(env: &Env, bidder: &Address) {
    env.storage()
        .instance()
        .set(&DataKey::HighestBidder, bidder);
}

pub fn get_factory_contract(env: &Env) -> Option<Address> {
    env.storage().instance().get(&DataKey::FactoryContract)
}

pub fn set_factory_contract(env: &Env, factory: &Address) {
    env.storage()
        .instance()
        .set(&DataKey::FactoryContract, factory);
}

// ── Reentrancy guard ──────────────────────────────────────────────────────────

/// Returns the instance-storage key used for the reentrancy flag.
/// Mirrors the identical key used in `contracts/credit/src/storage.rs`.
pub fn reentrancy_key(env: &Env) -> Symbol {
    Symbol::new(env, "reentrancy")
}

/// Assert the reentrancy guard is not set, then set it.
///
/// Panics with [`AuctionError::Reentrancy`] if the guard is already active,
/// indicating a reentrant cross-contract callback. The caller **must** call
/// [`clear_reentrancy_guard`] on every exit path (success and failure) to
/// release the guard and prevent the contract from being permanently locked.
///
/// # Storage
/// - **Type**: Instance storage
/// - **Key**: `Symbol("reentrancy")`
/// - **Value**: `true` while a token transfer is in progress
pub fn set_reentrancy_guard(env: &Env) {
    let key = reentrancy_key(env);
    let current: bool = env.storage().instance().get(&key).unwrap_or(false);
    if current {
        env.panic_with_error(AuctionError::Reentrancy);
    }
    env.storage().instance().set(&key, &true);
}

/// Clear the reentrancy guard set by [`set_reentrancy_guard`].
///
/// Must be called on every exit path (success and failure) of any function
/// that called [`set_reentrancy_guard`]. Writing `false` is idempotent and
/// safe to call even if the guard was never set.
///
/// # Storage
/// - **Type**: Instance storage
/// - **Key**: `Symbol("reentrancy")`
/// - **Value**: `false` (guard released)
pub fn clear_reentrancy_guard(env: &Env) {
    env.storage().instance().set(&reentrancy_key(env), &false);
}

/// Return the configured liquidation grace window in seconds.
///
/// Returns `0` when never configured (no grace period enforced).
pub fn get_liquidation_grace_window(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&DataKey::LiquidationGraceWindow)
        .unwrap_or(0)
}

/// Set the liquidation grace window (in seconds) for all future auctions.
///
/// When non-zero, `place_bid` will reject any bid placed before
/// `start_time + grace_window` has elapsed.
pub fn set_liquidation_grace_window(env: &Env, seconds: u64) {
    env.storage()
        .instance()
        .set(&DataKey::LiquidationGraceWindow, &seconds);
}

pub fn get_end_time(env: &Env) -> u64 {
    env.storage().instance().get(&DataKey::EndTime).unwrap_or(0)
}

pub fn set_end_time(env: &Env, end_time: u64) {
    env.storage().instance().set(&DataKey::EndTime, &end_time);
}

pub fn get_highest_bid(env: &Env) -> u128 {
    env.storage()
        .instance()
        .get(&DataKey::HighestBid)
        .unwrap_or(0)
}

pub fn set_highest_bid(env: &Env, bid: u128) {
    env.storage().instance().set(&DataKey::HighestBid, &bid);
}

// --- id-scoped auction storage ---
use crate::types::AuctionKey;

/// Check whether an auction with the given `id` exists in persistent storage.
///
/// Bumps the TTL of `AuctionKey::Status(id)` when the key is present, so
/// a bare existence probe does not let live auctions drift toward expiry.
///
/// # Storage
/// - **Type**: Persistent
/// - **Key**: `AuctionKey::Status(id)`
/// - **TTL bumped when**: key exists and remaining lifetime < [`PERSISTENT_LIFETIME_THRESHOLD`]
pub fn auction_exists(env: &Env, id: u32) -> bool {
    let key = AuctionKey::Status(id);
    let exists = env.storage().persistent().has(&key);
    if exists {
        env.storage().persistent().extend_ttl(
            &key,
            PERSISTENT_LIFETIME_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );
    }
    exists
}

/// Get the [`AuctionStatus`] for the auction identified by `id`.
///
/// Returns [`AuctionStatus::Open`] when the key is absent (consistent with
/// the write-path default). Bumps persistent TTL on every successful read so
/// the entry is not archived between status transitions.
///
/// # Storage
/// - **Type**: Persistent
/// - **Key**: `AuctionKey::Status(id)`
/// - **TTL bumped when**: key exists
pub fn auction_get_status(env: &Env, id: u32) -> crate::types::AuctionStatus {
    let key = AuctionKey::Status(id);
    let value = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or(crate::types::AuctionStatus::Open);
    if env.storage().persistent().has(&key) {
        env.storage().persistent().extend_ttl(
            &key,
            PERSISTENT_LIFETIME_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );
    }
    value
}

pub fn auction_set_status(env: &Env, id: u32, status: crate::types::AuctionStatus) {
    let key = AuctionKey::Status(id);
    env.storage().persistent().set(&key, &status);
    env.storage().persistent().extend_ttl(
        &key,
        PERSISTENT_LIFETIME_THRESHOLD,
        PERSISTENT_BUMP_AMOUNT,
    );
}

/// Get the seller [`Address`] for the auction identified by `id`.
///
/// Returns `None` when no seller has been written. Bumps persistent TTL on
/// every read where the key exists.
///
/// # Storage
/// - **Type**: Persistent
/// - **Key**: `AuctionKey::Seller(id)`
/// - **TTL bumped when**: key exists
pub fn auction_get_seller(env: &Env, id: u32) -> Option<Address> {
    let key = AuctionKey::Seller(id);
    let value = env.storage().persistent().get(&key);
    if env.storage().persistent().has(&key) {
        env.storage().persistent().extend_ttl(
            &key,
            PERSISTENT_LIFETIME_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );
    }
    value
}

pub fn auction_set_seller(env: &Env, id: u32, seller: &Address) {
    let key = AuctionKey::Seller(id);
    env.storage().persistent().set(&key, seller);
    env.storage().persistent().extend_ttl(
        &key,
        PERSISTENT_LIFETIME_THRESHOLD,
        PERSISTENT_BUMP_AMOUNT,
    );
}

/// Get the asset [`Address`] for the auction identified by `id`.
///
/// Returns `None` when no asset has been written. Bumps persistent TTL on
/// every read where the key exists.
///
/// # Storage
/// - **Type**: Persistent
/// - **Key**: `AuctionKey::Asset(id)`
/// - **TTL bumped when**: key exists
pub fn auction_get_asset(env: &Env, id: u32) -> Option<Address> {
    let key = AuctionKey::Asset(id);
    let value = env.storage().persistent().get(&key);
    if env.storage().persistent().has(&key) {
        env.storage().persistent().extend_ttl(
            &key,
            PERSISTENT_LIFETIME_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );
    }
    value
}

pub fn auction_set_asset(env: &Env, id: u32, asset: &Address) {
    let key = AuctionKey::Asset(id);
    env.storage().persistent().set(&key, asset);
    env.storage().persistent().extend_ttl(
        &key,
        PERSISTENT_LIFETIME_THRESHOLD,
        PERSISTENT_BUMP_AMOUNT,
    );
}

/// Get the minimum bid amount for the auction identified by `id`.
///
/// Returns `0` when the key is absent. Bumps persistent TTL on every read
/// where the key exists.
///
/// # Storage
/// - **Type**: Persistent
/// - **Key**: `AuctionKey::MinBid(id)`
/// - **TTL bumped when**: key exists
pub fn auction_get_min_bid(env: &Env, id: u32) -> i128 {
    let key = AuctionKey::MinBid(id);
    let value = env.storage().persistent().get(&key).unwrap_or(0);
    if env.storage().persistent().has(&key) {
        env.storage().persistent().extend_ttl(
            &key,
            PERSISTENT_LIFETIME_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );
    }
    value
}

pub fn auction_set_min_bid(env: &Env, id: u32, min_bid: i128) {
    let key = AuctionKey::MinBid(id);
    env.storage().persistent().set(&key, &min_bid);
    env.storage().persistent().extend_ttl(
        &key,
        PERSISTENT_LIFETIME_THRESHOLD,
        PERSISTENT_BUMP_AMOUNT,
    );
}

/// Get the end timestamp for the auction identified by `id`.
///
/// Returns `0` when the key is absent. Bumps persistent TTL on every read
/// where the key exists.
///
/// # Storage
/// - **Type**: Persistent
/// - **Key**: `AuctionKey::EndTime(id)`
/// - **TTL bumped when**: key exists
pub fn auction_get_end_time(env: &Env, id: u32) -> u64 {
    let key = AuctionKey::EndTime(id);
    let value = env.storage().persistent().get(&key).unwrap_or(0);
    if env.storage().persistent().has(&key) {
        env.storage().persistent().extend_ttl(
            &key,
            PERSISTENT_LIFETIME_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );
    }
    value
}

pub fn auction_set_end_time(env: &Env, id: u32, end_time: u64) {
    let key = AuctionKey::EndTime(id);
    env.storage().persistent().set(&key, &end_time);
    env.storage().persistent().extend_ttl(
        &key,
        PERSISTENT_LIFETIME_THRESHOLD,
        PERSISTENT_BUMP_AMOUNT,
    );
}

/// Get the current highest bidder [`Address`] for the auction identified by `id`.
///
/// Returns `None` when no bid has been placed. Bumps persistent TTL on every
/// read where the key exists.
///
/// # Storage
/// - **Type**: Persistent
/// - **Key**: `AuctionKey::HighestBidder(id)`
/// - **TTL bumped when**: key exists
pub fn auction_get_highest_bidder(env: &Env, id: u32) -> Option<Address> {
    let key = AuctionKey::HighestBidder(id);
    let value = env.storage().persistent().get(&key);
    if env.storage().persistent().has(&key) {
        env.storage().persistent().extend_ttl(
            &key,
            PERSISTENT_LIFETIME_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );
    }
    value
}

pub fn auction_set_highest_bidder(env: &Env, id: u32, bidder: &Address) {
    let key = AuctionKey::HighestBidder(id);
    env.storage().persistent().set(&key, bidder);
    env.storage().persistent().extend_ttl(
        &key,
        PERSISTENT_LIFETIME_THRESHOLD,
        PERSISTENT_BUMP_AMOUNT,
    );
}

/// Get the current highest bid amount for the auction identified by `id`.
///
/// Returns `0` when no bid has been placed. Bumps persistent TTL on every
/// read where the key exists.
///
/// # Storage
/// - **Type**: Persistent
/// - **Key**: `AuctionKey::HighestBid(id)`
/// - **TTL bumped when**: key exists
pub fn auction_get_highest_bid(env: &Env, id: u32) -> i128 {
    let key = AuctionKey::HighestBid(id);
    let value = env.storage().persistent().get(&key).unwrap_or(0);
    if env.storage().persistent().has(&key) {
        env.storage().persistent().extend_ttl(
            &key,
            PERSISTENT_LIFETIME_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );
    }
    value
}

pub fn auction_set_highest_bid(env: &Env, id: u32, bid: i128) {
    let key = AuctionKey::HighestBid(id);
    env.storage().persistent().set(&key, &bid);
    env.storage().persistent().extend_ttl(
        &key,
        PERSISTENT_LIFETIME_THRESHOLD,
        PERSISTENT_BUMP_AMOUNT,
    );
}

/// Check whether the auction identified by `id` has already been claimed.
///
/// Returns `false` when the key is absent (default: not claimed). Bumps
/// persistent TTL on every read where the key exists so the claimed marker
/// is not archived before settlement replay-protection is no longer needed.
///
/// # Storage
/// - **Type**: Persistent
/// - **Key**: `AuctionKey::Claimed(id)`
/// - **TTL bumped when**: key exists
pub fn auction_is_claimed(env: &Env, id: u32) -> bool {
    let key = AuctionKey::Claimed(id);
    let value = env.storage().persistent().get(&key).unwrap_or(false);
    if env.storage().persistent().has(&key) {
        env.storage().persistent().extend_ttl(
            &key,
            PERSISTENT_LIFETIME_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );
    }
    value
}

pub fn auction_set_claimed(env: &Env, id: u32) {
    let key = AuctionKey::Claimed(id);
    env.storage().persistent().set(&key, &true);
    env.storage().persistent().extend_ttl(
        &key,
        PERSISTENT_LIFETIME_THRESHOLD,
        PERSISTENT_BUMP_AMOUNT,
    );
}
