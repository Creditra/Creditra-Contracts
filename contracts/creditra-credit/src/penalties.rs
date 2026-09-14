//! Late-fee penalty model.
//!
//! # Overview
//!
//! Defines [`LateFeeConfig`], a two-variant enum representing the two
//! supported late-fee modes:
//!
//! - **[`LateFeeConfig::Flat`]** — a fixed token amount added once per missed
//!   installment, regardless of principal size or time elapsed.  The amount is
//!   stored in a [`FlatFeeConfig`] wrapper struct.
//! - **[`LateFeeConfig::AprBased`]** — an additive basis-point surcharge on
//!   the periodic interest rate, applied when the line is delinquent.  The
//!   surcharge is stored in an [`AprFeeConfig`] wrapper struct.
//!
//! # Calculation and Overflow Safety
//!
//! [`compute_late_fee`] is a pure, deterministic function with no
//! floating-point arithmetic, no `unwrap`, and no side effects.  All
//! arithmetic uses checked operations; any overflow returns
//! [`ContractError::Overflow`] before any state mutation can occur.
//!
//! # Rounding Policy
//!
//! - **Flat mode**: Discrete token amounts multiplied by whole installment
//!   counts yield exact integer values without fractional remainders.
//! - **APR-based mode**: Delinquency surcharges follow the protocol interest
//!   accrual policy using floor rounding (truncation toward zero), favoring
//!   the borrower.
//!
//! # Cleared Configuration
//!
//! When configuration is cleared (`None`), late fees evaluate to zero via
//! [`compute_late_fee_optional`], preserving neutral protocol behavior.
//!
//! # API change summary
//!
//! | Before | After |
//! |---|---|
//! | No late-fee configuration | Both APR-based and flat surcharge modes |
//! | No `SetLateFeeConfig` message | Execute message added |
//! | No `GetLateFeeConfig` query | Query message added |

use cosmwasm_schema::cw_serde;
use cosmwasm_std::Uint128;

use crate::error::ContractError;

/// Maximum allowed basis-point surcharge (10 000 bps = 100 %).
pub const MAX_SURCHARGE_BPS: u32 = 10_000;

/// Payload for the [`LateFeeConfig::Flat`] variant.
///
/// Wraps the flat surcharge amount so the enum can serialise as a tagged
/// union in JSON and Binary.
#[cw_serde]
#[derive(Copy)]
pub struct FlatFeeConfig {
    /// Token units charged once per overdue installment.
    ///
    /// Must be `>= 0`. Zero disables the fee (no-op).
    pub amount: Uint128,
}

/// Payload for the [`LateFeeConfig::AprBased`] variant.
///
/// Wraps the APR surcharge in basis points.
#[cw_serde]
#[derive(Copy)]
pub struct AprFeeConfig {
    /// Extra basis points added to the base interest rate while delinquent.
    ///
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
///
/// # Storage
///
/// Stored in instance storage under `LATE_FEE_CONFIG`.  Admin-configurable
/// via `SetLateFeeConfig` / `GetLateFeeConfig` on the contract.
///
/// # Examples
///
/// ```ignore
/// // Flat: charge 50 tokens per missed installment
/// let cfg = LateFeeConfig::Flat(FlatFeeConfig { amount: Uint128::new(50) });
///
/// // APR-based: add 200 bps to the interest rate when delinquent
/// let cfg = LateFeeConfig::AprBased(AprFeeConfig { surcharge_bps: 200 });
/// ```
#[cw_serde]
#[derive(Copy)]
pub enum LateFeeConfig {
    /// Fixed flat amount charged once per overdue installment.
    ///
    /// Use [`FlatFeeConfig`] to supply the `amount`.  Zero disables the fee.
    Flat(FlatFeeConfig),
    /// Additive APR surcharge applied to delinquent lines during accrual.
    ///
    /// `surcharge_bps` must be in `0..=10_000`.
    AprBased(AprFeeConfig),
}

/// Compute the flat late fee for `missed_installments` overdue periods.
///
/// Returns the total fee amount in token units.  All arithmetic is
/// overflow-safe: the computation uses `checked_mul` and propagates
/// [`ContractError::Overflow`] on overflow.
///
/// # Arguments
///
/// * `config` — Fee configuration.
/// * `missed_installments` — Number of overdue installment periods.  If
///   zero the function returns `Ok(Uint128::zero())` immediately.
///
/// # Returns
///
/// * `Ok(fee)` — total fee in token units (`>= 0`).
/// * `Err(ContractError::Overflow)` — arithmetic overflow detected.
/// * `Err(ContractError::InvalidAmount)` — `config` is `Flat` with a
///   negative or invalid amount.
///
/// # APR-based mode
///
/// The APR surcharge is applied by the accrual module, not here.  For
/// `AprBased` configs this function always returns `Ok(Uint128::zero())` —
/// callers should read `surcharge_bps` separately when they need it for
/// accrual.
///
/// # Examples
///
/// ```ignore
/// let fee = compute_late_fee(
///     LateFeeConfig::Flat(FlatFeeConfig { amount: Uint128::new(50) }),
///     3,
/// ).unwrap();
/// assert_eq!(fee, Uint128::new(150));
///
/// let fee = compute_late_fee(
///     LateFeeConfig::AprBased(AprFeeConfig { surcharge_bps: 200 }),
///     3,
/// ).unwrap();
/// assert_eq!(fee, Uint128::zero());
/// ```
pub fn compute_late_fee(
    config: LateFeeConfig,
    missed_installments: u64,
) -> Result<Uint128, ContractError> {
    if missed_installments == 0 {
        return Ok(Uint128::zero());
    }

    match config {
        LateFeeConfig::Flat(FlatFeeConfig { amount }) => {
            if amount.is_zero() {
                return Ok(Uint128::zero());
            }
            let count = Uint128::from(missed_installments);
            amount
                .checked_mul(count)
                .map_err(|_| ContractError::Overflow)
        }
        LateFeeConfig::AprBased(_) => {
            // APR surcharge is handled by the accrual module; no flat amount here.
            Ok(Uint128::zero())
        }
    }
}

/// Compute late fee for an optional configuration (such as when loaded from storage).
///
/// When `config` is `None` (cleared or unconfigured), returns `Ok(Uint128::zero())`.
/// When `config` is `Some(cfg)`, delegates to [`compute_late_fee`].
///
/// # Examples
///
/// ```ignore
/// let fee = compute_late_fee_optional(None, 5)?;
/// assert_eq!(fee, Uint128::zero());
/// ```
pub fn compute_late_fee_optional(
    config: Option<LateFeeConfig>,
    missed_installments: u64,
) -> Result<Uint128, ContractError> {
    match config {
        Some(cfg) => compute_late_fee(cfg, missed_installments),
        None => Ok(Uint128::zero()),
    }
}

/// Compute the APR-based late fee for delinquent principal over an elapsed duration.
///
/// Applies floor rounding in accordance with the protocol interest accrual policy:
/// `fee = floor(principal * surcharge_bps * elapsed_seconds / (10_000 * SECONDS_PER_YEAR))`.
///
/// # Errors
///
/// Returns [`ContractError::Overflow`] if the intermediate multiplication overflows `Uint128`.
pub fn compute_apr_late_fee(
    principal: Uint128,
    config: &AprFeeConfig,
    elapsed_seconds: u64,
) -> Result<Uint128, ContractError> {
    crate::accrual::accrued_interest(principal, config.surcharge_bps, elapsed_seconds)
}

/// Validate a late-fee configuration.
///
/// Ensures:
/// - `Flat` amounts are non-zero (zero is a no-op and should not be stored).
/// - `AprBased` surcharge is within `0..=MAX_SURCHARGE_BPS`.
///
/// # Errors
///
/// Returns [`ContractError::InvalidAmount`] for a zero flat fee,
/// or [`ContractError::RateTooHigh`] when the APR surcharge exceeds the cap.
pub fn validate_late_fee_config(config: &LateFeeConfig) -> Result<(), ContractError> {
    match config {
        LateFeeConfig::Flat(FlatFeeConfig { amount }) => {
            if amount.is_zero() {
                return Err(ContractError::InvalidAmount);
            }
            Ok(())
        }
        LateFeeConfig::AprBased(AprFeeConfig { surcharge_bps }) => {
            if *surcharge_bps > MAX_SURCHARGE_BPS {
                return Err(ContractError::RateTooHigh);
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Flat surcharge mode ──────────────────────────────────────────────────

    #[test]
    fn flat_single_installment() {
        let fee = compute_late_fee(
            LateFeeConfig::Flat(FlatFeeConfig {
                amount: Uint128::new(50),
            }),
            1,
        )
        .unwrap();
        assert_eq!(fee, Uint128::new(50));
    }

    #[test]
    fn flat_multiple_installments() {
        let fee = compute_late_fee(
            LateFeeConfig::Flat(FlatFeeConfig {
                amount: Uint128::new(50),
            }),
            3,
        )
        .unwrap();
        assert_eq!(fee, Uint128::new(150));
    }

    #[test]
    fn flat_zero_amount_is_noop() {
        let fee = compute_late_fee(
            LateFeeConfig::Flat(FlatFeeConfig {
                amount: Uint128::zero(),
            }),
            5,
        )
        .unwrap();
        assert_eq!(fee, Uint128::zero());
    }

    #[test]
    fn flat_large_amount() {
        let fee = compute_late_fee(
            LateFeeConfig::Flat(FlatFeeConfig {
                amount: Uint128::new(1_000_000),
            }),
            100,
        )
        .unwrap();
        assert_eq!(fee, Uint128::new(100_000_000));
    }

    #[test]
    fn flat_zero_missed_installments_returns_zero() {
        let fee = compute_late_fee(
            LateFeeConfig::Flat(FlatFeeConfig {
                amount: Uint128::new(999),
            }),
            0,
        )
        .unwrap();
        assert_eq!(fee, Uint128::zero());
    }

    #[test]
    fn flat_boundary_one_token_one_installment() {
        let fee = compute_late_fee(
            LateFeeConfig::Flat(FlatFeeConfig {
                amount: Uint128::new(1),
            }),
            1,
        )
        .unwrap();
        assert_eq!(fee, Uint128::new(1));
    }

    #[test]
    fn flat_boundary_large_multiplication() {
        let amount = Uint128::new(u128::MAX / 2);
        let fee = compute_late_fee(LateFeeConfig::Flat(FlatFeeConfig { amount }), 2).unwrap();
        assert_eq!(fee, amount.checked_mul(Uint128::new(2)).unwrap());
    }

    #[test]
    fn flat_max_uint128_zero_installments_returns_zero() {
        let fee = compute_late_fee(
            LateFeeConfig::Flat(FlatFeeConfig {
                amount: Uint128::MAX,
            }),
            0,
        )
        .unwrap();
        assert_eq!(fee, Uint128::zero());
    }

    #[test]
    fn flat_max_uint128_single_installment_succeeds() {
        let fee = compute_late_fee(
            LateFeeConfig::Flat(FlatFeeConfig {
                amount: Uint128::MAX,
            }),
            1,
        )
        .unwrap();
        assert_eq!(fee, Uint128::MAX);
    }

    #[test]
    fn flat_max_uint128_overflow_returns_overflow() {
        let err = compute_late_fee(
            LateFeeConfig::Flat(FlatFeeConfig {
                amount: Uint128::MAX,
            }),
            2,
        )
        .unwrap_err();
        assert_eq!(err, ContractError::Overflow);
    }

    #[test]
    fn flat_max_installments_at_unit_amount() {
        let fee = compute_late_fee(
            LateFeeConfig::Flat(FlatFeeConfig {
                amount: Uint128::new(1),
            }),
            u64::MAX,
        )
        .unwrap();
        assert_eq!(fee, Uint128::from(u64::MAX));
    }

    #[test]
    fn flat_max_installments_overflow_with_large_amount() {
        let err = compute_late_fee(
            LateFeeConfig::Flat(FlatFeeConfig {
                amount: Uint128::MAX,
            }),
            u64::MAX,
        )
        .unwrap_err();
        assert_eq!(err, ContractError::Overflow);
    }

    #[test]
    fn flat_overflow_boundary_at_max() {
        let half = Uint128::MAX / Uint128::new(2);
        let fee = compute_late_fee(LateFeeConfig::Flat(FlatFeeConfig { amount: half }), 2).unwrap();
        assert_eq!(fee, half.checked_mul(Uint128::new(2)).unwrap());
    }

    #[test]
    fn flat_overflow_boundary_just_above_max() {
        let half_plus_one = (Uint128::MAX / Uint128::new(2)) + Uint128::new(1);
        let err = compute_late_fee(
            LateFeeConfig::Flat(FlatFeeConfig {
                amount: half_plus_one,
            }),
            2,
        )
        .unwrap_err();
        assert_eq!(err, ContractError::Overflow);
    }

    #[test]
    fn optional_config_none_returns_zero() {
        let fee = compute_late_fee_optional(None, 10).unwrap();
        assert_eq!(fee, Uint128::zero());
    }

    #[test]
    fn optional_config_some_computes_fee() {
        let fee = compute_late_fee_optional(
            Some(LateFeeConfig::Flat(FlatFeeConfig {
                amount: Uint128::new(50),
            })),
            3,
        )
        .unwrap();
        assert_eq!(fee, Uint128::new(150));
    }

    #[test]
    fn apr_late_fee_floor_rounding() {
        let config = AprFeeConfig { surcharge_bps: 500 };
        let fee = compute_apr_late_fee(
            Uint128::new(10_000),
            &config,
            crate::accrual::SECONDS_PER_YEAR,
        )
        .unwrap();
        assert_eq!(fee, Uint128::new(500));
    }

    #[test]
    fn apr_late_fee_overflow_returns_overflow() {
        let config = AprFeeConfig {
            surcharge_bps: MAX_SURCHARGE_BPS,
        };
        let err = compute_apr_late_fee(Uint128::MAX, &config, crate::accrual::SECONDS_PER_YEAR)
            .unwrap_err();
        assert_eq!(err, ContractError::Overflow);
    }

    #[test]
    fn apr_late_fee_zero_principal_or_duration_returns_zero() {
        let config = AprFeeConfig { surcharge_bps: 500 };
        let fee1 = compute_apr_late_fee(Uint128::zero(), &config, 1000).unwrap();
        let fee2 = compute_apr_late_fee(Uint128::new(10_000), &config, 0).unwrap();
        assert_eq!(fee1, Uint128::zero());
        assert_eq!(fee2, Uint128::zero());
    }

    // ── APR-based mode (existing behaviour preserved) ────────────────────────

    #[test]
    fn apr_always_returns_zero_for_any_installments() {
        let fee = compute_late_fee(
            LateFeeConfig::AprBased(AprFeeConfig { surcharge_bps: 200 }),
            5,
        )
        .unwrap();
        assert_eq!(fee, Uint128::zero());
    }

    #[test]
    fn apr_zero_surcharge_returns_zero() {
        let fee = compute_late_fee(
            LateFeeConfig::AprBased(AprFeeConfig { surcharge_bps: 0 }),
            10,
        )
        .unwrap();
        assert_eq!(fee, Uint128::zero());
    }

    #[test]
    fn apr_max_surcharge_returns_zero() {
        let fee = compute_late_fee(
            LateFeeConfig::AprBased(AprFeeConfig {
                surcharge_bps: MAX_SURCHARGE_BPS,
            }),
            100,
        )
        .unwrap();
        assert_eq!(fee, Uint128::zero());
    }

    #[test]
    fn apr_zero_missed_installments_returns_zero() {
        let fee = compute_late_fee(
            LateFeeConfig::AprBased(AprFeeConfig { surcharge_bps: 500 }),
            0,
        )
        .unwrap();
        assert_eq!(fee, Uint128::zero());
    }

    // ── Cross-mode independence ───────────────────────────────────────────────

    #[test]
    fn flat_and_apr_produce_different_results() {
        let flat_fee = compute_late_fee(
            LateFeeConfig::Flat(FlatFeeConfig {
                amount: Uint128::new(100),
            }),
            3,
        )
        .unwrap();
        let apr_fee = compute_late_fee(
            LateFeeConfig::AprBased(AprFeeConfig { surcharge_bps: 500 }),
            3,
        )
        .unwrap();
        assert_eq!(flat_fee, Uint128::new(300));
        assert_eq!(apr_fee, Uint128::zero());
        assert_ne!(flat_fee, apr_fee);
    }

    #[test]
    fn switching_config_from_apr_to_flat_does_not_carry_state() {
        let apr = compute_late_fee(
            LateFeeConfig::AprBased(AprFeeConfig {
                surcharge_bps: 9_999,
            }),
            10,
        )
        .unwrap();
        let flat = compute_late_fee(
            LateFeeConfig::Flat(FlatFeeConfig {
                amount: Uint128::new(7),
            }),
            10,
        )
        .unwrap();
        assert_eq!(apr, Uint128::zero());
        assert_eq!(flat, Uint128::new(70));
    }

    // ── Validation tests ─────────────────────────────────────────────────────

    #[test]
    fn validate_flat_config_with_positive_amount() {
        let config = LateFeeConfig::Flat(FlatFeeConfig {
            amount: Uint128::new(50),
        });
        assert!(validate_late_fee_config(&config).is_ok());
    }

    #[test]
    fn validate_flat_config_zero_amount_errors() {
        let config = LateFeeConfig::Flat(FlatFeeConfig {
            amount: Uint128::zero(),
        });
        assert_eq!(
            validate_late_fee_config(&config).unwrap_err(),
            ContractError::InvalidAmount
        );
    }

    #[test]
    fn validate_apr_config_within_bounds() {
        let config = LateFeeConfig::AprBased(AprFeeConfig {
            surcharge_bps: 5_000,
        });
        assert!(validate_late_fee_config(&config).is_ok());
    }

    #[test]
    fn validate_apr_config_at_max_bound() {
        let config = LateFeeConfig::AprBased(AprFeeConfig {
            surcharge_bps: MAX_SURCHARGE_BPS,
        });
        assert!(validate_late_fee_config(&config).is_ok());
    }

    #[test]
    fn validate_apr_config_exceeds_max_errors() {
        let config = LateFeeConfig::AprBased(AprFeeConfig {
            surcharge_bps: MAX_SURCHARGE_BPS + 1,
        });
        assert_eq!(
            validate_late_fee_config(&config).unwrap_err(),
            ContractError::RateTooHigh
        );
    }

    #[test]
    fn validate_apr_config_zero_bps_is_ok() {
        let config = LateFeeConfig::AprBased(AprFeeConfig { surcharge_bps: 0 });
        assert!(validate_late_fee_config(&config).is_ok());
    }
}
