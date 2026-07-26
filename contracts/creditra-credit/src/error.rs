use cosmwasm_std::StdError;
use thiserror::Error;

/// Errors returned by the CosmWasm creditra-credit contract.
#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("CreditLine {0} not found")]
    CreditLineNotFound(u64),

    #[error("Draw {0} not found on credit line {1}")]
    DrawNotFound(u64, u64),

    #[error("Unauthorized")]
    Unauthorized,

    /// Collateral is insufficient for the requested operation.
    ///
    /// Semantic error raised when posted or available collateral cannot
    /// cover the operation (distinct from balance/ratio-specific Soroban
    /// codes `InsufficientCollateralBalance` / `CollateralRatioBelowMinimum`).
    #[error("CollateralInsufficient")]
    CollateralInsufficient,

    /// Borrower's collateral balance is below the requested withdrawal amount.
    ///
    /// Raised when a borrower attempts to withdraw more collateral than their
    /// current deposited balance. Distinct from `CollateralInsufficient` (which
    /// covers general insufficiency) and `CollateralRatioBelowMinimum` (which
    /// covers health-factor constraints).
    #[error("InsufficientCollateralBalance")]
    InsufficientCollateralBalance,

    /// The requested amount is invalid (e.g., zero or negative where positive is expected).
    #[error("InvalidAmount")]
    InvalidAmount,

    /// Liquidation settlement already processed for this (borrower, settlement_id) pair.
    ///
    /// Raised when a settlement is attempted for a combination that has
    /// already been recorded, preventing double-settlement replay.
    #[error("AlreadySettled")]
    AlreadySettled,

    /// Oracle price is invalid (zero, negative, or malformed).
    #[error("OraclePriceInvalid")]
    OraclePriceInvalid,

    /// Oracle quorum condition was not satisfied (too few agreeing feeds).
    #[error("OracleQuorumNotMet")]
    OracleQuorumNotMet,

    /// Treasury fee share in basis points exceeds the maximum (10_000).
    #[error("InvalidFeeShareBps")]
    InvalidFeeShareBps,

    /// Treasury balance is insufficient for the requested withdrawal.
    #[error("InsufficientTreasuryBalance")]
    InsufficientTreasuryBalance,

    /// Bounty pool balance is insufficient for the requested withdrawal.
    #[error("InsufficientBountyBalance")]
    InsufficientBountyBalance,

    /// Treasury address has not been configured.
    #[error("TreasuryAddressNotSet")]
    TreasuryAddressNotSet,

    /// Bounty address has not been configured.
    #[error("BountyAddressNotSet")]
    BountyAddressNotSet,
}

#[cfg(test)]
mod tests {
    use super::ContractError;

    #[test]
    fn collateral_insufficient_display_and_equality() {
        let err = ContractError::CollateralInsufficient;
        assert_eq!(err.to_string(), "CollateralInsufficient");
        assert_eq!(err, ContractError::CollateralInsufficient);
        assert_ne!(err, ContractError::Unauthorized);
    }

    #[test]
    fn already_settled_display_and_equality() {
        let err = ContractError::AlreadySettled;
        assert_eq!(err.to_string(), "AlreadySettled");
        assert_eq!(err, ContractError::AlreadySettled);
        assert_ne!(err, ContractError::Unauthorized);
        assert_ne!(err, ContractError::CollateralInsufficient);
    }

    #[test]
    fn oracle_price_invalid_display_and_equality() {
        let err = ContractError::OraclePriceInvalid;
        assert_eq!(err.to_string(), "OraclePriceInvalid");
        assert_eq!(err, ContractError::OraclePriceInvalid);
        assert_ne!(err, ContractError::Unauthorized);
    }

    #[test]
    fn oracle_quorum_not_met_display_and_equality() {
        let err = ContractError::OracleQuorumNotMet;
        assert_eq!(err.to_string(), "OracleQuorumNotMet");
        assert_eq!(err, ContractError::OracleQuorumNotMet);
        assert_ne!(err, ContractError::OraclePriceInvalid);
    }

    #[test]
    fn invalid_amount_display_and_equality() {
        let err = ContractError::InvalidAmount;
        assert_eq!(err.to_string(), "InvalidAmount");
        assert_eq!(err, ContractError::InvalidAmount);
        assert_ne!(err, ContractError::Unauthorized);
    }

    #[test]
    fn insufficient_collateral_balance_display_and_equality() {
        let err = ContractError::InsufficientCollateralBalance;
        assert_eq!(err.to_string(), "InsufficientCollateralBalance");
        assert_eq!(err, ContractError::InsufficientCollateralBalance);
        assert_ne!(err, ContractError::CollateralInsufficient);
        assert_ne!(err, ContractError::Unauthorized);
    }

    #[test]
    fn insufficient_collateral_balance_is_distinct_from_collateral_insufficient() {
        let balance_err = ContractError::InsufficientCollateralBalance;
        let insufficient_err = ContractError::CollateralInsufficient;
        assert_ne!(balance_err, insufficient_err);
        assert_ne!(balance_err.to_string(), insufficient_err.to_string());
    }

    #[test]
    fn invalid_fee_share_bps_display_and_equality() {
        let err = ContractError::InvalidFeeShareBps;
        assert_eq!(err.to_string(), "InvalidFeeShareBps");
        assert_eq!(err, ContractError::InvalidFeeShareBps);
        assert_ne!(err, ContractError::Unauthorized);
    }

    #[test]
    fn insufficient_treasury_balance_display_and_equality() {
        let err = ContractError::InsufficientTreasuryBalance;
        assert_eq!(err.to_string(), "InsufficientTreasuryBalance");
        assert_eq!(err, ContractError::InsufficientTreasuryBalance);
        assert_ne!(err, ContractError::InsufficientBountyBalance);
    }

    #[test]
    fn insufficient_bounty_balance_display_and_equality() {
        let err = ContractError::InsufficientBountyBalance;
        assert_eq!(err.to_string(), "InsufficientBountyBalance");
        assert_eq!(err, ContractError::InsufficientBountyBalance);
        assert_ne!(err, ContractError::InsufficientTreasuryBalance);
    }

    #[test]
    fn treasury_address_not_set_display_and_equality() {
        let err = ContractError::TreasuryAddressNotSet;
        assert_eq!(err.to_string(), "TreasuryAddressNotSet");
        assert_eq!(err, ContractError::TreasuryAddressNotSet);
        assert_ne!(err, ContractError::BountyAddressNotSet);
    }

    #[test]
    fn bounty_address_not_set_display_and_equality() {
        let err = ContractError::BountyAddressNotSet;
        assert_eq!(err.to_string(), "BountyAddressNotSet");
        assert_eq!(err, ContractError::BountyAddressNotSet);
        assert_ne!(err, ContractError::TreasuryAddressNotSet);
    }
}
