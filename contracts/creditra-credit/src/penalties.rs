use cosmwasm_schema::cw_serde;

use crate::error::ContractError;

/// Fee configuration for a flat amount charged per overdue installment.
#[cw_serde]
pub struct FlatFeeConfig {
    /// Token units charged once per overdue installment.
    /// Must be >= 0. Zero disables the fee (no-op).
    pub amount: i128,
}

/// Fee configuration for an APR-based surcharge applied while delinquent.
#[cw_serde]
pub struct AprFeeConfig {
    /// Extra basis points added to the base interest rate while delinquent.
    /// Must be in `0..=10_000`.
    pub surcharge_bps: u32,
}

/// Configuration for the late-fee penalty applied to overdue installments.
///
/// # Variants
///
/// | Variant    | Behaviour |
/// |------------|-----------|
/// | `Flat`     | A fixed `amount` in token units is applied once per missed installment. |
/// | `AprBased` | An additive basis-point surcharge on the periodic interest rate while the line is delinquent. |
#[cw_serde]
pub enum LateFeeConfig {
    /// Fixed flat amount charged once per overdue installment.
    Flat(FlatFeeConfig),
    /// Additive APR surcharge applied to delinquent lines during accrual.
    /// `surcharge_bps` must be in `0..=10_000`.
    AprBased(AprFeeConfig),
}

/// Compute the late fee for `missed_installments` overdue periods.
///
/// Returns the total fee amount in token units. All arithmetic is
/// overflow-safe: uses `checked_mul` and propagates `ContractError::Overflow`
/// on overflow.
///
/// # Errors
///
/// - `ContractError::LateFeeConfigInvalid` — `Flat` config with a negative `amount`.
/// - `ContractError::Overflow` — arithmetic overflow in multiplication.
pub fn compute_late_fee(
    config: &LateFeeConfig,
    missed_installments: u64,
) -> Result<i128, ContractError> {
    if missed_installments == 0 {
        return Ok(0);
    }

    match config {
        LateFeeConfig::Flat(FlatFeeConfig { amount }) => {
            if *amount < 0 {
                return Err(ContractError::LateFeeConfigInvalid);
            }
            if *amount == 0 {
                return Ok(0);
            }
            let count = i128::try_from(missed_installments)
                .map_err(|_| ContractError::Overflow)?;
            amount.checked_mul(count).ok_or(ContractError::Overflow)
        }
        LateFeeConfig::AprBased(_) => {
            Ok(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helpers ──────────────────────────────────────────────────────────

    fn flat(amount: i128) -> LateFeeConfig {
        LateFeeConfig::Flat(FlatFeeConfig { amount })
    }

    fn apr(surcharge_bps: u32) -> LateFeeConfig {
        LateFeeConfig::AprBased(AprFeeConfig { surcharge_bps })
    }

    // ── Flat surcharge mode ──────────────────────────────────────────────

    #[test]
    fn flat_single_installment() {
        let fee = compute_late_fee(&flat(50), 1).unwrap();
        assert_eq!(fee, 50);
    }

    #[test]
    fn flat_multiple_installments() {
        let fee = compute_late_fee(&flat(50), 3).unwrap();
        assert_eq!(fee, 150);
    }

    #[test]
    fn flat_zero_amount_is_noop() {
        let fee = compute_late_fee(&flat(0), 5).unwrap();
        assert_eq!(fee, 0);
    }

    #[test]
    fn flat_large_amount() {
        let fee = compute_late_fee(&flat(1_000_000), 100).unwrap();
        assert_eq!(fee, 100_000_000);
    }

    #[test]
    fn flat_zero_missed_installments_returns_zero() {
        let fee = compute_late_fee(&flat(999), 0).unwrap();
        assert_eq!(fee, 0);
    }

    #[test]
    fn flat_negative_amount_returns_error() {
        let err = compute_late_fee(&flat(-1), 1).unwrap_err();
        assert_eq!(err, ContractError::LateFeeConfigInvalid);
    }

    #[test]
    fn flat_negative_amount_with_zero_missed_is_ok() {
        let fee = compute_late_fee(&flat(-1), 0).unwrap();
        assert_eq!(fee, 0);
    }

    #[test]
    fn flat_overflow_returns_error() {
        let err = compute_late_fee(&flat(i128::MAX), 2).unwrap_err();
        assert_eq!(err, ContractError::Overflow);
    }

    #[test]
    fn flat_boundary_one_token_one_installment() {
        let fee = compute_late_fee(&flat(1), 1).unwrap();
        assert_eq!(fee, 1);
    }

    #[test]
    fn flat_boundary_max_safe_multiplication() {
        let half = i128::MAX / 2;
        let fee = compute_late_fee(&flat(half), 2).unwrap();
        assert_eq!(fee, half * 2);
    }

    // ── APR-based mode ───────────────────────────────────────────────────

    #[test]
    fn apr_always_returns_zero() {
        let fee = compute_late_fee(&apr(200), 5).unwrap();
        assert_eq!(fee, 0);
    }

    #[test]
    fn apr_zero_surcharge_returns_zero() {
        let fee = compute_late_fee(&apr(0), 10).unwrap();
        assert_eq!(fee, 0);
    }

    #[test]
    fn apr_max_surcharge_returns_zero() {
        let fee = compute_late_fee(&apr(10_000), 100).unwrap();
        assert_eq!(fee, 0);
    }

    #[test]
    fn apr_zero_missed_installments_returns_zero() {
        let fee = compute_late_fee(&apr(500), 0).unwrap();
        assert_eq!(fee, 0);
    }

    // ── Cross-mode independence ──────────────────────────────────────────

    #[test]
    fn flat_and_apr_produce_different_results() {
        let flat_fee = compute_late_fee(&flat(100), 3).unwrap();
        let apr_fee = compute_late_fee(&apr(500), 3).unwrap();
        assert_eq!(flat_fee, 300);
        assert_eq!(apr_fee, 0);
        assert_ne!(flat_fee, apr_fee);
    }

    #[test]
    fn switching_config_does_not_carry_state() {
        let apr = compute_late_fee(&apr(9_999), 10).unwrap();
        let flat = compute_late_fee(&flat(7), 10).unwrap();
        assert_eq!(apr, 0);
        assert_eq!(flat, 70);
    }
}
