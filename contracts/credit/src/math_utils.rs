// SPDX-License-Identifier: MIT

//! # Fixed-Point Interest Math Utilities
//!
//! Deterministic, integer-only arithmetic helpers for computing interest
//! accruals inside the Creditra credit contract.  No `f32` or `f64` is used.
//!
//! ## Scaling Factor
//!
//! All intermediate products are scaled by `SCALE = 10^18` before division so
//! that the final result retains sub-unit precision up to 18 decimal places.
//! The caller chooses whether the remainder is discarded (floor) or rounded up
//! (ceiling) via the [`Rounding`] enum.
//!
//! ## Basis Points
//!
//! Interest rates are expressed in **basis points** (bps), where
//! `1 bps = 0.01% = 1 / 10_000`.  The annual rate in bps is therefore divided
//! by `BPS_DENOMINATOR = 10_000` when computing the fractional rate.
//!
//! ## Annual Seconds
//!
//! Time is measured in ledger seconds.  One Julian year is defined as
//! `SECONDS_PER_YEAR = 31_557_600` (365.25 × 86 400), matching the convention
//! used by most on-chain interest protocols.
//!
//! ## Overflow Safety
//!
//! The prorate helper promotes all operands to `u128` before multiplying.
//! The worst-case intermediate product is:
//!
//! ```text
//! principal  ≤ i128::MAX  ≈ 1.7 × 10^38
//! rate_bps   ≤ 10_000
//! time_delta ≤ u64::MAX   ≈ 1.8 × 10^19
//! ```
//!
//! `principal × rate_bps × time_delta` can reach ~3 × 10^61, which overflows
//! `u128` (max ~3.4 × 10^38).  To prevent this the multiplication is split
//! into two checked steps:
//!
//! 1. `a = principal × rate_bps`  — fits in u128 for any realistic principal
//!    (≤ 10^28 × 10^4 = 10^32 < 10^38).
//! 2. `b = a × time_delta`        — checked; panics on overflow.
//!
//! The denominator `BPS_DENOMINATOR × SECONDS_PER_YEAR` is pre-computed as a
//! `u128` constant so the final division is a single operation.

#![allow(dead_code)]

/// Scaling factor used for fixed-point intermediate arithmetic (10^18).
pub const SCALE: u128 = 1_000_000_000_000_000_000_u128;

/// Number of basis points in 100 % (10 000 bps = 100 %).
pub const BPS_DENOMINATOR: u128 = 10_000;

/// Seconds in one Julian year (365.25 days × 86 400 s/day).
pub const SECONDS_PER_YEAR: u128 = 31_557_600;

/// Combined denominator: `BPS_DENOMINATOR × SECONDS_PER_YEAR`.
///
/// Dividing by this value converts `(amount × rate_bps × seconds)` into the
/// annualised interest amount expressed in the same unit as `amount`.
pub const BPS_YEAR_DENOM: u128 = BPS_DENOMINATOR * SECONDS_PER_YEAR; // 315_576_000_000

// ─── Rounding direction ──────────────────────────────────────────────────────

/// Rounding direction for fixed-point division.
///
/// - [`Rounding::Floor`] — truncate toward zero (default, favours the protocol).
/// - [`Rounding::Ceil`]  — round up away from zero (favours the borrower when
///   computing minimum repayment amounts).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Rounding {
    /// Truncate the fractional part (round toward zero).
    Floor,
    /// Add one if there is any non-zero remainder (round away from zero).
    Ceil,
}

// ─── Core fixed-point helpers ─────────────────────────────────────────────────

/// Multiply `a` by `b` expressed as a fraction `(numerator / denominator)`,
/// returning the result rounded according to `rounding`.
///
/// # Formula
///
/// ```text
/// result = (a × numerator) / denominator   [± 1 ulp depending on Rounding]
/// ```
///
/// # Panics
///
/// Panics on overflow if `a × numerator` exceeds `u128::MAX`.
///
/// # Examples
///
/// ```rust
/// use creditra_credit::math_utils::{mul_div, Rounding};
///
/// // 1 000 × (3 / 10) = 300 (floor)
/// assert_eq!(mul_div(1_000, 3, 10, Rounding::Floor), 300);
///
/// // 1 001 × (3 / 10) = 300.3 → ceil → 301
/// assert_eq!(mul_div(1_001, 3, 10, Rounding::Ceil), 301);
/// ```
pub fn mul_div(a: u128, numerator: u128, denominator: u128, rounding: Rounding) -> u128 {
    assert!(denominator != 0, "math_utils: division by zero");
    let product = a.checked_mul(numerator).expect("math_utils: mul overflow");
    let quotient = product / denominator;
    match rounding {
        Rounding::Floor => quotient,
        Rounding::Ceil => {
            if product % denominator != 0 {
                quotient.checked_add(1).expect("math_utils: ceil overflow")
            } else {
                quotient
            }
        }
    }
}

/// Scale `amount` up by [`SCALE`] (multiply by 10^18).
///
/// Used to convert a raw integer into a fixed-point representation before
/// performing division so that fractional precision is preserved.
///
/// # Panics
///
/// Panics if the result would overflow `u128`.
pub fn scale_up(amount: u128) -> u128 {
    amount.checked_mul(SCALE).expect("math_utils: scale_up overflow")
}

/// Scale `amount` down by [`SCALE`] (divide by 10^18), applying `rounding`.
///
/// Used to convert a fixed-point intermediate value back to a raw integer
/// after division.
pub fn scale_down(amount: u128, rounding: Rounding) -> u128 {
    let quotient = amount / SCALE;
    match rounding {
        Rounding::Floor => quotient,
        Rounding::Ceil => {
            if amount % SCALE != 0 {
                quotient.checked_add(1).expect("math_utils: scale_down ceil overflow")
            } else {
                quotient
            }
        }
    }
}

// ─── Basis-point helpers ──────────────────────────────────────────────────────

/// Apply a basis-point rate to an amount.
///
/// Computes `amount × rate_bps / BPS_DENOMINATOR`, rounded per `rounding`.
///
/// # Parameters
///
/// - `amount`   — principal in the contract's native token unit.
/// - `rate_bps` — rate in basis points (0 ..= 10 000 for 0 %–100 %).
/// - `rounding` — [`Rounding::Floor`] or [`Rounding::Ceil`].
///
/// # Panics
///
/// Panics on overflow if `amount × rate_bps > u128::MAX`.
///
/// # Examples
///
/// ```rust
/// use creditra_credit::math_utils::{apply_bps, Rounding};
///
/// // 10 000 tokens at 300 bps (3 %) = 300 tokens
/// assert_eq!(apply_bps(10_000, 300, Rounding::Floor), 300);
///
/// // 1 token at 1 bps = 0.0001 → floor → 0
/// assert_eq!(apply_bps(1, 1, Rounding::Floor), 0);
///
/// // 1 token at 1 bps = 0.0001 → ceil → 1
/// assert_eq!(apply_bps(1, 1, Rounding::Ceil), 1);
/// ```
pub fn apply_bps(amount: u128, rate_bps: u32, rounding: Rounding) -> u128 {
    mul_div(amount, rate_bps as u128, BPS_DENOMINATOR, rounding)
}

// ─── Time-prorating helper ────────────────────────────────────────────────────

/// Compute the interest accrued on `principal` over `time_delta` seconds at an
/// annual rate of `rate_bps` basis points.
///
/// # Formula
///
/// ```text
/// interest = (principal × rate_bps × time_delta) / (BPS_DENOMINATOR × SECONDS_PER_YEAR)
/// ```
///
/// Intermediate arithmetic is performed in `u128` with checked multiplication
/// to detect overflow early.  The final division uses [`Rounding`] to control
/// whether the fractional remainder is discarded or rounded up.
///
/// # Parameters
///
/// - `principal`  — outstanding balance in the contract's native token unit.
/// - `rate_bps`   — annual interest rate in basis points (0 ..= 10 000).
/// - `time_delta` — elapsed seconds since the last accrual.
/// - `rounding`   — [`Rounding::Floor`] (protocol-favourable) or
///   [`Rounding::Ceil`] (borrower-favourable minimum repayment).
///
/// # Returns
///
/// The interest amount in the same unit as `principal`.  Returns `0` when
/// `principal`, `rate_bps`, or `time_delta` is zero.
///
/// # Panics
///
/// Panics if the intermediate product overflows `u128`.
///
/// # Examples
///
/// ```rust
/// use creditra_credit::math_utils::{prorate_interest, Rounding, SECONDS_PER_YEAR};
///
/// // 10 000 tokens at 300 bps (3 %) for exactly one year → 300 tokens
/// assert_eq!(
///     prorate_interest(10_000, 300, SECONDS_PER_YEAR as u64, Rounding::Floor),
///     300
/// );
///
/// // Zero principal → zero interest
/// assert_eq!(prorate_interest(0, 300, 86_400, Rounding::Floor), 0);
/// ```
pub fn prorate_interest(
    principal: u128,
    rate_bps: u32,
    time_delta: u64,
    rounding: Rounding,
) -> u128 {
    if principal == 0 || rate_bps == 0 || time_delta == 0 {
        return 0;
    }

    let step1 = principal
        .checked_mul(rate_bps as u128)
        .expect("math_utils: prorate overflow (step1)");

    let step2 = step1
        .checked_mul(time_delta as u128)
        .expect("math_utils: prorate overflow (step2)");

    let quotient = step2 / BPS_YEAR_DENOM;
    match rounding {
        Rounding::Floor => quotient,
        Rounding::Ceil => {
            if step2 % BPS_YEAR_DENOM != 0 {
                quotient.checked_add(1).expect("math_utils: prorate ceil overflow")
            } else {
                quotient
            }
        }
    }
}

// ─── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mul_div_exact_floor() {
        assert_eq!(mul_div(1_000, 3, 10, Rounding::Floor), 300);
    }

    #[test]
    fn mul_div_exact_ceil() {
        assert_eq!(mul_div(1_000, 3, 10, Rounding::Ceil), 300);
    }

    #[test]
    fn mul_div_remainder_floor() {
        assert_eq!(mul_div(1_001, 3, 10, Rounding::Floor), 300);
    }

    #[test]
    fn mul_div_remainder_ceil() {
        assert_eq!(mul_div(1_001, 3, 10, Rounding::Ceil), 301);
    }

    #[test]
    fn mul_div_zero_numerator() {
        assert_eq!(mul_div(1_000_000, 0, 10_000, Rounding::Floor), 0);
        assert_eq!(mul_div(1_000_000, 0, 10_000, Rounding::Ceil), 0);
    }

    #[test]
    fn mul_div_zero_a() {
        assert_eq!(mul_div(0, 300, 10_000, Rounding::Floor), 0);
        assert_eq!(mul_div(0, 300, 10_000, Rounding::Ceil), 0);
    }

    #[test]
    fn mul_div_one_bps_of_small_amount_floor() {
        assert_eq!(mul_div(1, 1, 10_000, Rounding::Floor), 0);
    }

    #[test]
    fn mul_div_one_bps_of_small_amount_ceil() {
        assert_eq!(mul_div(1, 1, 10_000, Rounding::Ceil), 1);
    }

    #[test]
    #[should_panic(expected = "division by zero")]
    fn mul_div_zero_denominator_panics() {
        mul_div(100, 1, 0, Rounding::Floor);
    }

    #[test]
    fn scale_up_and_down_roundtrip_floor() {
        let v = 12_345_678_u128;
        assert_eq!(scale_down(scale_up(v), Rounding::Floor), v);
    }

    #[test]
    fn scale_down_ceil_adds_one_for_remainder() {
        assert_eq!(scale_down(SCALE + 1, Rounding::Ceil), 2);
    }

    #[test]
    fn scale_down_floor_truncates_remainder() {
        assert_eq!(scale_down(SCALE + 1, Rounding::Floor), 1);
    }

    #[test]
    fn apply_bps_three_percent() {
        assert_eq!(apply_bps(10_000, 300, Rounding::Floor), 300);
    }

    #[test]
    fn apply_bps_one_bps_small_amount_floor() {
        assert_eq!(apply_bps(1, 1, Rounding::Floor), 0);
    }

    #[test]
    fn apply_bps_one_bps_small_amount_ceil() {
        assert_eq!(apply_bps(1, 1, Rounding::Ceil), 1);
    }

    #[test]
    fn prorate_interest_one_full_year_floor() {
        let interest = prorate_interest(10_000, 300, SECONDS_PER_YEAR as u64, Rounding::Floor);
        assert_eq!(interest, 300);
    }

    #[test]
    fn prorate_interest_half_year() {
        let half_year = (SECONDS_PER_YEAR / 2) as u64;
        let interest = prorate_interest(10_000, 300, half_year, Rounding::Floor);
        assert_eq!(interest, 150);
    }

    #[test]
    fn prorate_interest_zero_principal() {
        assert_eq!(prorate_interest(0, 300, 86_400, Rounding::Floor), 0);
    }

    #[test]
    fn prorate_interest_zero_rate() {
        assert_eq!(prorate_interest(10_000, 0, 86_400, Rounding::Floor), 0);
    }

    #[test]
    fn prorate_interest_zero_time() {
        assert_eq!(prorate_interest(10_000, 300, 0, Rounding::Floor), 0);
    }

    #[test]
    fn prorate_interest_one_bps_small_principal_floor() {
        let interest = prorate_interest(1, 1, SECONDS_PER_YEAR as u64, Rounding::Floor);
        assert_eq!(interest, 0);
    }

    #[test]
    fn prorate_interest_one_bps_small_principal_ceil() {
        let interest = prorate_interest(1, 1, SECONDS_PER_YEAR as u64, Rounding::Ceil);
        assert_eq!(interest, 1);
    }

    #[test]
    fn prorate_interest_floor_le_ceil() {
        let cases: &[(u128, u32, u64)] = &[
            (1, 1, 1),
            (10_000, 300, 86_400),
            (1_000_000, 9_999, SECONDS_PER_YEAR as u64),
        ];
        for &(p, r, t) in cases {
            let floor = prorate_interest(p, r, t, Rounding::Floor);
            let ceil = prorate_interest(p, r, t, Rounding::Ceil);
            assert!(floor <= ceil, "floor > ceil for p={p}, r={r}, t={t}");
            assert!(ceil - floor <= 1, "ceil - floor > 1 for p={p}, r={r}, t={t}");
        }
    }

    #[test]
    fn prorate_interest_max_u32_stress() {
        let p = u32::MAX as u128;
        let r = 10_000_u32;
        let t = u32::MAX as u64;
        let _ = prorate_interest(p, r, t, Rounding::Floor);
        let _ = prorate_interest(p, r, t, Rounding::Ceil);
    }
}
