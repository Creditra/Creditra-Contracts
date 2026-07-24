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
}
