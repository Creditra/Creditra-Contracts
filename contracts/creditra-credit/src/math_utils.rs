// SPDX-License-Identifier: MIT

//! # Math Utilities
//!
//! Overflow-safe arithmetic helpers for the CosmWasm credit contract.
//! These mirror the Soroban `math_utils` module to ensure consistent behavior
//! across both runtimes.

use cosmwasm_std::Uint128;

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
        // Uint128::MAX × 2 overflows
        assert_eq!(mul_div(Uint128::MAX, 2, 1, Rounding::Floor), None);
    }
}
