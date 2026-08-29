use cosmwasm_std::StdError;
use thiserror::Error;

/// Domain category for a [`ContractError`] variant.
///
/// Each variant groups related contract errors, enabling callers to
/// match on high-level categories without inspecting every individual
/// error variant.
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
#[repr(u8)]
pub enum ContractErrorCategory {
    /// Wrapper around a standard CosmWasm error.
    Std,
    /// The requested resource (credit line, draw, …) was not found.
    NotFound,
    /// Caller lacks permission for the operation.
    Auth,
    /// Collateral-related constraint violation.
    Collateral,
    /// Input validation failure.
    Validation,
    /// State-machine or lifecycle violation (e.g. double settlement).
    State,
    /// Oracle price-feed or quorum error.
    Oracle,
}

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

    #[error("OracleNotFound")]
    OracleNotFound,

    /// Rate or surcharge exceeds the protocol maximum (10 000 bps = 100 %).
    #[error("RateTooHigh")]
    RateTooHigh,

    /// Arithmetic overflow detected in checked computation.
    #[error("Overflow")]
    Overflow,

    #[error("CollateralTokenNotAllowed")]
    CollateralTokenNotAllowed,

    #[error("TreasuryAddressNotSet")]
    TreasuryAddressNotSet,

    #[error("BountyAddressNotSet")]
    BountyAddressNotSet,

    #[error("LateFeeConfigInvalid")]
    LateFeeConfigInvalid,

    #[error("ProtocolFeeBpsExceeded")]
    ProtocolFeeBpsExceeded,

    /// A configured interest rate exceeds the applicable ceiling.
    #[error("RateCeilingExceeded")]
    RateCeilingExceeded,

    /// A fee-share ratio (in basis points) exceeds [`crate::fees::MAX_FEE_SHARE_BPS`].
    #[error("InvalidFeeShareBps")]
    InvalidFeeShareBps,

    /// Requested treasury withdrawal exceeds the accumulated treasury balance.
    #[error("InsufficientTreasuryBalance")]
    InsufficientTreasuryBalance,

    /// Requested bounty withdrawal exceeds the accumulated bounty balance.
    #[error("InsufficientBountyBalance")]
    InsufficientBountyBalance,

    /// A draw would push unrepaid utilization above the committed credit amount.
    #[error("OverLimit")]
    OverLimit,
    /// The collateral token allowlist is at its maximum size and cannot
    /// accept another denomination.
    ///
    /// Raised when an admin attempts to add a collateral token to a full
    /// allowlist (see [`crate::state::MAX_COLLATERAL_TOKENS`]). The cap
    /// protects transaction resource limits: the allowlist is scanned
    /// linearly on every collateral deposit and returned wholesale by
    /// queries, so an unbounded list would grow storage and gas costs
    /// without bound.
    #[error("TooManyCollateralTokens")]
    TooManyCollateralTokens,
}

impl ContractError {
    /// Return the [`ContractErrorCategory`] this error belongs to.
    pub fn category(&self) -> ContractErrorCategory {
        match self {
            ContractError::Std(_) => ContractErrorCategory::Std,
            ContractError::CreditLineNotFound(_) | ContractError::DrawNotFound(_, _) => {
                ContractErrorCategory::NotFound
            }
            ContractError::Unauthorized => ContractErrorCategory::Auth,
            ContractError::CollateralInsufficient => ContractErrorCategory::Collateral,
            ContractError::InsufficientCollateralBalance => ContractErrorCategory::Collateral,
            ContractError::CollateralTokenNotAllowed => ContractErrorCategory::Collateral,
            ContractError::InvalidAmount => ContractErrorCategory::Validation,
            ContractError::AlreadySettled => ContractErrorCategory::State,
            ContractError::OraclePriceInvalid => ContractErrorCategory::Oracle,
            ContractError::OracleQuorumNotMet => ContractErrorCategory::Oracle,
            ContractError::OracleNotFound => ContractErrorCategory::Oracle,
            ContractError::RateTooHigh => ContractErrorCategory::Validation,
            ContractError::Overflow => ContractErrorCategory::State,
            ContractError::RateCeilingExceeded => ContractErrorCategory::Validation,
            ContractError::InvalidFeeShareBps => ContractErrorCategory::Validation,
            ContractError::InsufficientTreasuryBalance => ContractErrorCategory::Collateral,
            ContractError::InsufficientBountyBalance => ContractErrorCategory::Collateral,
            ContractError::TreasuryAddressNotSet => ContractErrorCategory::State,
            ContractError::BountyAddressNotSet => ContractErrorCategory::State,
            ContractError::LateFeeConfigInvalid => ContractErrorCategory::Validation,
            ContractError::ProtocolFeeBpsExceeded => ContractErrorCategory::Validation,
            ContractError::OverLimit => ContractErrorCategory::Validation,
            ContractError::TooManyCollateralTokens => ContractErrorCategory::Validation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ContractErrorCategory unit tests ────────────────────────────────

    #[test]
    fn category_variants_are_distinct() {
        let std = ContractErrorCategory::Std;
        let nf = ContractErrorCategory::NotFound;
        let auth = ContractErrorCategory::Auth;
        let coll = ContractErrorCategory::Collateral;
        let val = ContractErrorCategory::Validation;
        let state = ContractErrorCategory::State;
        let oracle = ContractErrorCategory::Oracle;

        assert_ne!(std as u8, nf as u8);
        assert_ne!(nf as u8, auth as u8);
        assert_ne!(auth as u8, coll as u8);
        assert_ne!(coll as u8, val as u8);
        assert_ne!(val as u8, state as u8);
        assert_ne!(state as u8, oracle as u8);
        assert_ne!(oracle as u8, std as u8);
    }

    #[test]
    fn category_debug_format() {
        let oracle = ContractErrorCategory::Oracle;
        let debug = format!("{:?}", oracle);
        assert_eq!(debug, "Oracle");
    }

    #[test]
    fn category_copy_and_clone() {
        let a = ContractErrorCategory::Auth;
        let b = a;
        assert_eq!(a, b);
    }

    // ── ContractError::category() mapping tests ─────────────────────────

    #[test]
    fn std_error_category() {
        let err = ContractError::Std(StdError::generic_err("test"));
        assert_eq!(err.category(), ContractErrorCategory::Std);
    }

    #[test]
    fn credit_line_not_found_category() {
        let err = ContractError::CreditLineNotFound(42);
        assert_eq!(err.category(), ContractErrorCategory::NotFound);
    }

    #[test]
    fn draw_not_found_category() {
        let err = ContractError::DrawNotFound(1, 42);
        assert_eq!(err.category(), ContractErrorCategory::NotFound);
    }

    #[test]
    fn unauthorized_category() {
        let err = ContractError::Unauthorized;
        assert_eq!(err.category(), ContractErrorCategory::Auth);
    }

    #[test]
    fn collateral_insufficient_category() {
        let err = ContractError::CollateralInsufficient;
        assert_eq!(err.category(), ContractErrorCategory::Collateral);
    }

    #[test]
    fn insufficient_collateral_balance_category() {
        let err = ContractError::InsufficientCollateralBalance;
        assert_eq!(err.category(), ContractErrorCategory::Collateral);
    }

    #[test]
    fn invalid_amount_category() {
        let err = ContractError::InvalidAmount;
        assert_eq!(err.category(), ContractErrorCategory::Validation);
    }

    #[test]
    fn already_settled_category() {
        let err = ContractError::AlreadySettled;
        assert_eq!(err.category(), ContractErrorCategory::State);
    }

    #[test]
    fn oracle_price_invalid_category() {
        let err = ContractError::OraclePriceInvalid;
        assert_eq!(err.category(), ContractErrorCategory::Oracle);
    }

    #[test]
    fn oracle_quorum_not_met_category() {
        let err = ContractError::OracleQuorumNotMet;
        assert_eq!(err.category(), ContractErrorCategory::Oracle);
    }

    // ── Existing display & equality tests (preserved) ───────────────────

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
    fn too_many_collateral_tokens_category() {
        let err = ContractError::TooManyCollateralTokens;
        assert_eq!(err.category(), ContractErrorCategory::Validation);
    }

    #[test]
    fn too_many_collateral_tokens_display_and_equality() {
        let err = ContractError::TooManyCollateralTokens;
        assert_eq!(err.to_string(), "TooManyCollateralTokens");
        assert_eq!(err, ContractError::TooManyCollateralTokens);
        assert_ne!(err, ContractError::InvalidAmount);
        assert_ne!(err, ContractError::Unauthorized);
        assert_ne!(err, ContractError::AlreadySettled);
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
    fn over_limit_display_and_equality() {
        let err = ContractError::OverLimit;
        assert_eq!(err.to_string(), "OverLimit");
        assert_eq!(err, ContractError::OverLimit);
        assert_ne!(err, ContractError::InvalidAmount);
        assert_eq!(err.category(), ContractErrorCategory::Validation);
    }
}
