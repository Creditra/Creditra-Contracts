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

    /// Arithmetic overflow occurred.
    #[error("Overflow")]
    Overflow,

    /// Late-fee configuration is invalid (e.g., negative flat amount or
    /// surcharge > 10_000 bps).
    #[error("LateFeeConfigInvalid")]
    LateFeeConfigInvalid,
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
    fn rate_ceiling_exceeded_display_and_equality() {
        let err = ContractError::RateCeilingExceeded;
        assert_eq!(err.to_string(), "RateCeilingExceeded");
        assert_eq!(err, ContractError::RateCeilingExceeded);
        assert_ne!(err, ContractError::InvalidAmount);
        assert_ne!(err, ContractError::Unauthorized);
    }

    #[test]
    fn overflow_display_and_equality() {
        let err = ContractError::Overflow;
        assert_eq!(err.to_string(), "Overflow");
        assert_eq!(err, ContractError::Overflow);
        assert_ne!(err, ContractError::InvalidAmount);
        assert_ne!(err, ContractError::RateCeilingExceeded);
    }

    #[test]
    fn insufficient_collateral_balance_is_distinct_from_collateral_insufficient() {
        let balance_err = ContractError::InsufficientCollateralBalance;
        let insufficient_err = ContractError::CollateralInsufficient;
        assert_ne!(balance_err, insufficient_err);
        assert_ne!(balance_err.to_string(), insufficient_err.to_string());
    }

    #[test]
    fn overflow_display_and_equality() {
        let err = ContractError::Overflow;
        assert_eq!(err.to_string(), "Overflow");
        assert_eq!(err, ContractError::Overflow);
        assert_ne!(err, ContractError::Unauthorized);
    }

    #[test]
    fn late_fee_config_invalid_display_and_equality() {
        let err = ContractError::LateFeeConfigInvalid;
        assert_eq!(err.to_string(), "LateFeeConfigInvalid");
        assert_eq!(err, ContractError::LateFeeConfigInvalid);
        assert_ne!(err, ContractError::Overflow);
    }
}
