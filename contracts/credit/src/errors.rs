//! Centralized error types for the Creditra Credit contract.
//!
//! All contract entry-points revert by returning one of these variants.
//! Frontends, SDKs, and tests can match on the stable numeric codes rather
//! than fragile string messages.
//!
//! # Error codes
//!
//! | Code | Variant                    | When raised                                                  |
//! |------|----------------------------|--------------------------------------------------------------|
//! | 1    | `Unauthorized`             | Caller is not the contract admin (or not the borrower).      |
//! | 2    | `CreditLineNotFound`       | No credit line exists for the given borrower address.        |
//! | 3    | `InvalidAmount`            | Amount is zero or negative.                                  |
//! | 4    | `OverLimit`                | Draw would exceed the borrower's credit limit.               |
//! | 5    | `CreditLineClosed`         | Operation is not allowed on a closed credit line.            |
//! | 6    | `UtilizedAmountNotZero`    | Borrower tried to close a line that still has utilization.   |
//! | 7    | `InvalidCreditLimit`       | `credit_limit` is negative or below current utilization.     |
//! | 8    | `InterestRateExceedsMax`   | `interest_rate_bps` exceeds 10 000 (100 %).                  |
//! | 9    | `RiskScoreExceedsMax`      | `risk_score` exceeds 100.                                    |
//! | 10   | `Overflow`                 | Arithmetic overflow during utilization accounting.           |
//! | 11   | `ReentrancyGuard`          | Reentrant call detected; guard is already set.               |

use soroban_sdk::contracterror;

/// Stable, versioned error enum for the Credit contract.
///
/// Every variant maps to a unique `u32` discriminant so that callers can
/// match on the numeric code returned by the Soroban host.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum CreditError {
    /// Caller is not the contract admin (or not the expected borrower).
    Unauthorized = 1,

    /// No credit line exists for the given borrower address.
    CreditLineNotFound = 2,

    /// Amount is zero or negative; must be strictly positive.
    InvalidAmount = 3,

    /// Draw would push `utilized_amount` above `credit_limit`.
    OverLimit = 4,

    /// The credit line is closed; draws and repayments are not allowed.
    CreditLineClosed = 5,

    /// Borrower attempted to close a line whose `utilized_amount` is non-zero.
    UtilizedAmountNotZero = 6,

    /// `credit_limit` is negative, or it is below the current `utilized_amount`.
    InvalidCreditLimit = 7,

    /// `interest_rate_bps` exceeds `MAX_INTEREST_RATE_BPS` (10 000).
    InterestRateExceedsMax = 8,

    /// `risk_score` exceeds `MAX_RISK_SCORE` (100).
    RiskScoreExceedsMax = 9,

    /// Arithmetic overflow while computing new utilization.
    Overflow = 10,

    /// Reentrant call detected; the reentrancy guard is already active.
    ReentrancyGuard = 11,
}
