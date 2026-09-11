// SPDX-License-Identifier: MIT

//! # Math Utilities
//!
//! Overflow-safe arithmetic helpers for the CosmWasm credit contract.
//! These mirror the Soroban `math_utils` module to ensure consistent behavior
//! across both runtimes.

use cosmwasm_std::{Uint128, Uint256};

/// Rounding direction for fixed-point division.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Rounding {
    /// Truncate the fractional part (round toward zero).
    Floor,
    /// Add one if there is any non-zero remainder (round away from zero).
    Ceil,
}

/// Multiply `a` by `b` expressed as a fraction `(numerator / denominator)`,
/// returning the result rounded according to `rounding`.
///
/// # Formula
///
/// ```text
/// result = (a × numerator) / denominator   [± 1 ulp depending on Rounding]
/// ```
///
/// # Errors
///
/// Returns `None` if:
/// - `denominator` is zero
/// - `a × numerator` overflows `Uint128`
/// - Ceil rounding would overflow `Uint128`
///
/// # Examples
///
/// ```rust
/// use creditra_credit::math_utils::{mul_div, Rounding};
/// use cosmwasm_std::Uint128;
///
/// // 1 000 × (3 / 10) = 300 (floor)
/// assert_eq!(
///     mul_div(Uint128::new(1_000), 3, 10, Rounding::Floor),
///     Some(Uint128::new(300))
/// );
///
/// // 1 001 × (3 / 10) = 300.3 → ceil → 301
/// assert_eq!(
///     mul_div(Uint128::new(1_001), 3, 10, Rounding::Ceil),
///     Some(Uint128::new(301))
/// );
/// ```
pub fn mul_div(
    a: Uint128,
    numerator: u128,
    denominator: u128,
    rounding: Rounding,
) -> Option<Uint128> {
    if denominator == 0 {
        return None;
    }

    let product = a.checked_mul(Uint128::from(numerator)).ok()?;
    let quotient = product.checked_div(Uint128::from(denominator)).ok()?;

    match rounding {
        Rounding::Floor => Some(quotient),
        Rounding::Ceil => {
            if product % Uint128::from(denominator) != Uint128::zero() {
                quotient.checked_add(Uint128::one()).ok()
            } else {
                Some(quotient)
            }
        }
    }
}

/// Compute collateral-aware health factor in basis points.
///
/// Returns `u32::MAX` when `total_utilized == 0` (no outstanding debt).
/// Returns `0` when `effective_collateral == 0` and `total_utilized > 0`.
/// Otherwise evaluates `(effective_collateral * 10_000) / total_utilized` with 256-bit
/// precision and saturates at `u32::MAX`.
pub fn compute_health_factor_bps(effective_collateral: Uint128, total_utilized: Uint128) -> u32 {
    if total_utilized.is_zero() {
        u32::MAX
    } else if effective_collateral.is_zero() {
        0u32
    } else {
        let col_256 = Uint256::from(effective_collateral);
        let bps_256 = Uint256::from(10_000u32);
        let ut_256 = Uint256::from(total_utilized);
        let numerator = col_256 * bps_256;
        let quotient = numerator / ut_256;
        Uint128::try_from(quotient)
            .ok()
            .and_then(|v| u32::try_from(v.u128()).ok())
            .unwrap_or(u32::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mul_div_basic() {
        assert_eq!(
            mul_div(Uint128::new(1_000), 300, 10_000, Rounding::Floor),
            Some(Uint128::new(30))
        );
    }

    #[test]
    fn mul_div_truncates_toward_zero() {
        // 7 * 1 / 3 = 2.33… → 2
        assert_eq!(
            mul_div(Uint128::new(7), 1, 3, Rounding::Floor),
            Some(Uint128::new(2))
        );
    }

    #[test]
    fn mul_div_identity_denominator() {
        assert_eq!(
            mul_div(Uint128::new(42), 1, 1, Rounding::Floor),
            Some(Uint128::new(42))
        );
    }

    #[test]
    fn mul_div_exact_floor() {
        // 1 000 × 3 / 10 = 300 exactly
        assert_eq!(
            mul_div(Uint128::new(1_000), 3, 10, Rounding::Floor),
            Some(Uint128::new(300))
        );
    }

    #[test]
    fn mul_div_exact_ceil() {
        // 1 000 × 3 / 10 = 300 exactly — ceil should not add 1
        assert_eq!(
            mul_div(Uint128::new(1_000), 3, 10, Rounding::Ceil),
            Some(Uint128::new(300))
        );
    }

    #[test]
    fn mul_div_remainder_floor() {
        // 1 001 × 3 / 10 = 300.3 → floor → 300
        assert_eq!(
            mul_div(Uint128::new(1_001), 3, 10, Rounding::Floor),
            Some(Uint128::new(300))
        );
    }

    #[test]
    fn mul_div_remainder_ceil() {
        // 1 001 × 3 / 10 = 300.3 → ceil → 301
        assert_eq!(
            mul_div(Uint128::new(1_001), 3, 10, Rounding::Ceil),
            Some(Uint128::new(301))
        );
    }

    #[test]
    fn mul_div_zero_numerator() {
        assert_eq!(
            mul_div(Uint128::new(1_000_000), 0, 10_000, Rounding::Floor),
            Some(Uint128::zero())
        );
        assert_eq!(
            mul_div(Uint128::new(1_000_000), 0, 10_000, Rounding::Ceil),
            Some(Uint128::zero())
        );
    }

    #[test]
    fn mul_div_zero_a() {
        assert_eq!(
            mul_div(Uint128::zero(), 300, 10_000, Rounding::Floor),
            Some(Uint128::zero())
        );
        assert_eq!(
            mul_div(Uint128::zero(), 300, 10_000, Rounding::Ceil),
            Some(Uint128::zero())
        );
    }

    #[test]
    fn mul_div_denominator_equals_numerator() {
        // a × n / n = a
        assert_eq!(
            mul_div(Uint128::new(42), 7, 7, Rounding::Floor),
            Some(Uint128::new(42))
        );
        assert_eq!(
            mul_div(Uint128::new(42), 7, 7, Rounding::Ceil),
            Some(Uint128::new(42))
        );
    }

    #[test]
    fn mul_div_large_values_floor() {
        // u128::MAX / 2 × 2 / 2 = u128::MAX / 2
        let half = Uint128::from(u128::MAX / 2);
        assert_eq!(mul_div(half, 2, 2, Rounding::Floor), Some(half));
    }

    #[test]
    fn mul_div_one_bps_of_small_amount_floor() {
        // 1 token × 1 bps / 10_000 = 0.0001 → floor → 0
        assert_eq!(
            mul_div(Uint128::new(1), 1, 10_000, Rounding::Floor),
            Some(Uint128::zero())
        );
    }

    #[test]
    fn mul_div_one_bps_of_small_amount_ceil() {
        // 1 token × 1 bps / 10_000 = 0.0001 → ceil → 1
        assert_eq!(
            mul_div(Uint128::new(1), 1, 10_000, Rounding::Ceil),
            Some(Uint128::new(1))
        );
    }

    #[test]
    fn mul_div_zero_denominator_returns_none() {
        assert_eq!(mul_div(Uint128::new(100), 1, 0, Rounding::Floor), None);
    }

    #[test]
    fn mul_div_overflow_returns_none() {
        assert_eq!(mul_div(Uint128::MAX, 2, 1, Rounding::Floor), None);
    }

    #[test]
    fn health_factor_zero_utilized_returns_u32_max() {
        assert_eq!(
            compute_health_factor_bps(Uint128::new(1000), Uint128::zero()),
            u32::MAX
        );
        assert_eq!(
            compute_health_factor_bps(Uint128::zero(), Uint128::zero()),
            u32::MAX
        );
    }

    #[test]
    fn health_factor_zero_collateral_with_debt_returns_zero() {
        assert_eq!(
            compute_health_factor_bps(Uint128::zero(), Uint128::new(500)),
            0
        );
    }

    #[test]
    fn health_factor_par_collateralization() {
        assert_eq!(
            compute_health_factor_bps(Uint128::new(1000), Uint128::new(1000)),
            10_000
        );
    }

    #[test]
    fn health_factor_over_and_under_collateralized() {
        assert_eq!(
            compute_health_factor_bps(Uint128::new(2000), Uint128::new(1000)),
            20_000
        );
        assert_eq!(
            compute_health_factor_bps(Uint128::new(500), Uint128::new(1000)),
            5_000
        );
    }

    #[test]
    fn health_factor_large_collateral_saturates_without_overflow() {
        assert_eq!(
            compute_health_factor_bps(Uint128::MAX, Uint128::new(1)),
            u32::MAX
        );
    }
}
