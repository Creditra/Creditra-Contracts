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
}
