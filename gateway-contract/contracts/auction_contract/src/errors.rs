//! Auction contract error codes.
//!
//! # Stability
//! Discriminants are part of the contract ABI. Existing variants must not be
//! reordered or renumbered; new variants must be appended at the end.

use soroban_sdk::contracterror;

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
    /// The liquidation grace window has not yet elapsed; bidding is blocked.
    GracePeriodActive = 14,
    UnknownError = 200,
}

impl AuctionError {
    pub fn from_u32_safe(code: u32) -> Self {
        match code {
            1 => Self::NotWinner,
            2 => Self::AlreadyClaimed,
            3 => Self::NotClosed,
            4 => Self::NoFactoryContract,
            5 => Self::Unauthorized,
            6 => Self::InvalidState,
            7 => Self::BidTooLow,
            8 => Self::AuctionNotOpen,
            9 => Self::AuctionNotClosed,
            10 => Self::Reentrancy,
            11 => Self::NoWinner,
            12 => Self::NotFound,
            13 => Self::AlreadySettled,
            14 => Self::GracePeriodActive,
            200 => Self::UnknownError,
            _ => Self::UnknownError,
        }
    }
}

impl From<soroban_sdk::Error> for AuctionError {
    fn from(err: soroban_sdk::Error) -> Self {
        if err.is_type(soroban_sdk::xdr::ScErrorType::Contract) {
            Self::from_u32_safe(err.get_code())
        } else {
            Self::UnknownError
        }
    }
}

impl<'a> From<&'a AuctionError> for soroban_sdk::Error {
    fn from(err: &'a AuctionError) -> Self {
        soroban_sdk::Error::from_contract_error(*err as u32)
    }
}

impl From<AuctionError> for soroban_sdk::Error {
    fn from(err: AuctionError) -> Self {
        soroban_sdk::Error::from_contract_error(err as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_golden_vector_encodings() {
        assert_eq!(AuctionError::from_u32_safe(1), AuctionError::NotWinner);
        assert_eq!(
            AuctionError::from_u32_safe(14),
            AuctionError::GracePeriodActive
        );
        assert_eq!(AuctionError::from_u32_safe(200), AuctionError::UnknownError);
        assert_eq!(AuctionError::from_u32_safe(999), AuctionError::UnknownError);
    }
}
