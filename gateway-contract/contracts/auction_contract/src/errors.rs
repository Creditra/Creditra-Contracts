//! Auction contract error codes.
//!
//! # Stability
//! Discriminants are part of the contract ABI. Existing variants must not be
//! reordered or renumbered; new variants must be appended at the end.

use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum AuctionError {
    /// Caller is not the winning bidder for the auction being claimed.
    NotWinner = 1,
    /// The winning bidder has already claimed the auction proceeds.
    AlreadyClaimed = 2,
    /// Operation requires the auction to be in the `Closed` state.
    NotClosed = 3,
    /// The credit / factory contract address has not been configured.
    NoFactoryContract = 4,
    /// Caller is not authorized to perform this admin-only operation.
    Unauthorized = 5,
    /// Auction is in a state incompatible with the requested operation.
    InvalidState = 6,
    /// Submitted bid does not meet the minimum next-bid threshold.
    BidTooLow = 7,
    /// Operation requires the auction to be in the `Open` state.
    AuctionNotOpen = 8,
    /// Operation requires the auction to be in the `Closed` state (settlement).
    AuctionNotClosed = 9,
    /// Reentrant call detected through the reentrancy guard.
    Reentrancy = 10,
    /// Auction closed without a valid winning bid.
    NoWinner = 11,
    /// Auction with the requested id was not found.
    NotFound = 12,
    /// `settle_default_liquidation` was called a second time for the same auction.
    AlreadySettled = 13,
}
