// SPDX-License-Identifier: MIT

//! Pure integer arithmetic helpers used across the credit contract.

#![warn(missing_docs)]

//! # Fixed-Point Interest Math Utilities
//!
//! This module provides deterministic, integer-only arithmetic helpers for
//! computing interest accruals inside the Creditra credit contract.
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
//! SCALE      = 10^18
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

/// Number of basis points in 100%.
pub const BPS_DENOMINATOR: u128 = 10_000;

/// Seconds in a 365-day year.
pub const SECONDS_PER_YEAR: u128 = 31_536_000;

const BPS_YEAR_DENOMINATOR: u128 = BPS_DENOMINATOR * SECONDS_PER_YEAR;

/// Rounding mode for integer division helpers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Rounding {
    /// Truncate the remainder.
    Floor,
    /// Round up when a non-zero remainder exists.
    Ceil,
}

/// Multiply `value` by `numerator` and divide by `denominator`.
///
/// The result is rounded according to `rounding`.
pub fn mul_div(value: u128, numerator: u128, denominator: u128, rounding: Rounding) -> u128 {
    assert!(denominator != 0, "math_utils: division by zero");

    let product = value
        .checked_mul(numerator)
        .expect("math_utils: multiplication overflow");
    let quotient = product / denominator;

    match rounding {
        Rounding::Floor => quotient,
        Rounding::Ceil => {
            if !product.is_multiple_of(denominator) {
                quotient.checked_add(1).expect("math_utils: ceil overflow")
            } else {
                quotient
            } else {
                quotient.checked_add(1).expect("math_utils: ceil overflow")
            }
        }
    }
}

/// Multiply `amount` by `SCALE`.
pub fn scale_up(amount: u128) -> u128 {
    amount
        .checked_mul(SCALE)
        .expect("math_utils: scale_up overflow")
}

/// Divide `amount` by `SCALE` using the requested rounding mode.
pub fn scale_down(amount: u128, rounding: Rounding) -> u128 {
    let quotient = amount / SCALE;

    match rounding {
        Rounding::Floor => quotient,
        Rounding::Ceil => {
            if !amount.is_multiple_of(SCALE) {
                quotient
                    .checked_add(1)
                    .expect("math_utils: scale_down ceil overflow")
            } else {
                quotient
                    .checked_add(1)
                    .expect("math_utils: scale_down ceil overflow")
            }
        }
    }
}

/// Apply a basis-point rate to an amount.
pub fn apply_bps(amount: u128, rate_bps: u32, rounding: Rounding) -> u128 {
    mul_div(amount, rate_bps as u128, BPS_DENOMINATOR, rounding)
}

/// Compute prorated interest for an elapsed time interval.
pub fn prorate_interest(
    principal: u128,
    rate_bps: u32,
    elapsed_secs: u64,
    rounding: Rounding,
) -> u128 {
    if principal == 0 || rate_bps == 0 || elapsed_secs == 0 {
        return 0;
    }

    let step1 = principal
        .checked_mul(rate_bps as u128)
        .expect("math_utils: prorate step1 overflow");
    let step2 = step1
        .checked_mul(elapsed_secs as u128)
        .expect("math_utils: prorate step2 overflow");

    let quotient = step2 / BPS_YEAR_DENOMINATOR;
    match rounding {
        Rounding::Floor => quotient,
        Rounding::Ceil => {
            if !step2.is_multiple_of(BPS_YEAR_DENOM) {
                quotient
                    .checked_add(1)
                    .expect("math_utils: prorate ceil overflow")
            } else {
                quotient
                    .checked_add(1)
                    .expect("math_utils: prorate ceil overflow")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── mul_div ───────────────────────────────────────────────────────────────

    #[test]
    fn mul_div_exact_floor() {
        // 1 000 × 3 / 10 = 300 exactly
        assert_eq!(mul_div(1_000, 3, 10, Rounding::Floor), 300);
        assert_eq!(mul_div(1_001, 3, 10, Rounding::Floor), 300);
        assert_eq!(mul_div(1_001, 3, 10, Rounding::Ceil), 301);
    }

    #[test]
    fn apply_bps_matches_basic_basis_point_math() {
        assert_eq!(apply_bps(10_000, 300, Rounding::Floor), 300);
    }

    #[test]
    fn apply_bps_full_rate() {
        // 10 000 tokens × 10 000 bps (100 %) = 10 000 tokens
        assert_eq!(apply_bps(10_000, 10_000, Rounding::Floor), 10_000);
    }

    #[test]
    fn apply_bps_zero_rate() {
        assert_eq!(apply_bps(1_000_000, 0, Rounding::Floor), 0);
    }

    // ── prorate_interest ─────────────────────────────────────────────────────

    #[test]
    fn prorate_interest_one_day_old() {
        // 5% annual on 1_000_000 for 1 day (using 365.25-day year)
        assert_eq!(prorate_interest(1_000_000, 500, 86_400, Rounding::Floor), 136);
    }

    #[test]
    fn prorate_interest_zero_elapsed() {
        assert_eq!(prorate_interest(1_000_000, 500, 0, Rounding::Floor), 0);
        assert_eq!(apply_bps(1_000_000, 0, Rounding::Floor), 0);
        assert_eq!(apply_bps(1_000_000, 0, Rounding::Ceil), 0);
    }

    #[test]
    fn apply_bps_zero_amount() {
        assert_eq!(apply_bps(0, 300, Rounding::Floor), 0);
        assert_eq!(apply_bps(0, 300, Rounding::Ceil), 0);
    }

    #[test]
    fn apply_bps_one_bps_small_amount_floor() {
        // 1 token × 1 bps = 0.0001 → floor → 0
        assert_eq!(apply_bps(1, 1, Rounding::Floor), 0);
        assert_eq!(apply_bps(1, 1, Rounding::Ceil), 1);
    }

    #[test]
    fn apply_bps_one_bps_threshold_floor() {
        // 10 000 tokens × 1 bps = 1 token exactly
        assert_eq!(apply_bps(10_000, 1, Rounding::Floor), 1);
    }

    #[test]
    fn apply_bps_large_amount() {
        // i128::MAX as u128 × 1 bps / 10_000
        let large: u128 = i128::MAX as u128;
        let expected = large / 10_000;
        assert_eq!(apply_bps(large, 1, Rounding::Floor), expected);
    }

    // ── prorate_interest ──────────────────────────────────────────────────────

    #[test]
    fn prorate_interest_one_full_year_floor() {
        // 10 000 tokens at 300 bps for exactly one year → 300 tokens
        let interest = prorate_interest(10_000, 300, SECONDS_PER_YEAR as u64, Rounding::Floor);
        assert_eq!(interest, 300);
    }

    #[test]
    fn prorate_interest_one_full_year_ceil() {
        // Exact result → ceil should equal floor
        let interest = prorate_interest(10_000, 300, SECONDS_PER_YEAR as u64, Rounding::Ceil);
        assert_eq!(interest, 300);
    }

    #[test]
    fn prorate_interest_half_year() {
        // 10 000 tokens at 300 bps for half a year → 150 tokens
        let half_year = (SECONDS_PER_YEAR / 2) as u64;
        let interest = prorate_interest(10_000, 300, half_year, Rounding::Floor);
        assert_eq!(interest, 150);
    }

    #[test]
    fn prorate_interest_small_principal_one_day_floor() {
        // 10 000 tokens at 300 bps for one day
        // = 10_000 × 300 × 86_400 / 315_576_000_000
        // = 259_200_000 / 315_576_000_000 ≈ 0.000821 → floor → 0
        let interest = prorate_interest(10_000, 300, 86_400, Rounding::Floor);
        assert_eq!(interest, 0);
    }

    #[test]
    fn prorate_interest_one_day_ceil() {
        // Same as above but ceil → 1
        let interest = prorate_interest(10_000, 300, 86_400, Rounding::Ceil);
        assert_eq!(interest, 1);
    }

    #[test]
    fn prorate_interest_zero_principal() {
        assert_eq!(prorate_interest(0, 500, 86_400, Rounding::Floor), 0);
    }

    #[test]
    fn prorate_interest_full_year() {
        // 10% on 100_000 for exactly SECONDS_PER_YEAR (365.25 days) = 10_000
        assert_eq!(
            prorate_interest(100_000, 1_000, SECONDS_PER_YEAR as u64, Rounding::Floor),
            10_000
        );
    }

    #[test]
    fn prorate_interest_one_hour() {
        // 5% on 1_000_000 for 3_600 s ≈ 5
        assert_eq!(prorate_interest(1_000_000, 500, 3_600, Rounding::Floor), 5);
        assert_eq!(prorate_interest(0, 300, 86_400, Rounding::Floor), 0);
        assert_eq!(prorate_interest(10_000, 0, 86_400, Rounding::Floor), 0);
        assert_eq!(prorate_interest(10_000, 300, 0, Rounding::Floor), 0);
    }

    #[test]
    fn prorate_interest_max_rate_one_year() {
        // 10 000 tokens at 10 000 bps (100 %) for one year → 10 000 tokens
        let interest = prorate_interest(10_000, 10_000, SECONDS_PER_YEAR as u64, Rounding::Floor);
        assert_eq!(interest, 10_000);
    }

    #[test]
    fn prorate_interest_one_bps_small_principal_floor() {
        // 1 token at 1 bps for one year = 1 × 1 / 10_000 = 0.0001 → floor → 0
        let interest = prorate_interest(1, 1, SECONDS_PER_YEAR as u64, Rounding::Floor);
        assert_eq!(interest, 0);
    }

    #[test]
    fn prorate_interest_one_bps_small_principal_ceil() {
        // 1 token at 1 bps for one year = 0.0001 → ceil → 1
        let interest = prorate_interest(1, 1, SECONDS_PER_YEAR as u64, Rounding::Ceil);
        assert_eq!(interest, 1);
    }

    #[test]
    fn prorate_interest_large_principal_one_year() {
        // 1_000_000_000 tokens at 500 bps for one year → 50_000_000 tokens
        let interest =
            prorate_interest(1_000_000_000, 500, SECONDS_PER_YEAR as u64, Rounding::Floor);
        assert_eq!(interest, 50_000_000);
    }

    #[test]
    fn prorate_interest_floor_less_than_or_equal_ceil() {
        // Property: floor result ≤ ceil result for any inputs
        let cases: &[(u128, u32, u64)] = &[
            (1, 1, 1),
            (10_000, 300, 86_400),
            (1_000_000, 9_999, SECONDS_PER_YEAR as u64),
            (u32::MAX as u128, 10_000, u32::MAX as u64),
        ];
        for &(p, r, t) in cases {
            let floor = prorate_interest(p, r, t, Rounding::Floor);
            let ceil = prorate_interest(p, r, t, Rounding::Ceil);
            assert!(
                floor <= ceil,
                "floor ({floor}) > ceil ({ceil}) for principal={p}, rate={r}, time={t}"
            );
        }
    }

    #[test]
    fn prorate_interest_ceil_floor_diff_at_most_one() {
        // Property: ceil - floor ∈ {0, 1}
        let cases: &[(u128, u32, u64)] = &[
            (1, 1, 1),
            (7, 3, 100),
            (10_000, 300, 86_400),
            (999_999, 1, SECONDS_PER_YEAR as u64),
        ];
        for &(p, r, t) in cases {
            let floor = prorate_interest(p, r, t, Rounding::Floor);
            let ceil = prorate_interest(p, r, t, Rounding::Ceil);
            assert!(
                ceil - floor <= 1,
                "ceil - floor > 1 for principal={p}, rate={r}, time={t}"
            );
        }
    }

    #[test]
    fn prorate_interest_monotone_in_time() {
        // More time → more (or equal) interest
        let p = 1_000_000_u128;
        let r = 300_u32;
        let t1 = 86_400_u64;
        let t2 = 86_400_u64 * 30;
        assert!(
            prorate_interest(p, r, t2, Rounding::Floor)
                >= prorate_interest(p, r, t1, Rounding::Floor)
        );
    }

    #[test]
    fn prorate_interest_monotone_in_rate() {
        // Higher rate → more (or equal) interest
        let p = 1_000_000_u128;
        let t = SECONDS_PER_YEAR as u64;
        assert!(
            prorate_interest(p, 500, t, Rounding::Floor)
                >= prorate_interest(p, 300, t, Rounding::Floor)
        );
    }

    #[test]
    fn prorate_interest_monotone_in_principal() {
        // Larger principal → more (or equal) interest
        let r = 300_u32;
        let t = SECONDS_PER_YEAR as u64;
        assert!(
            prorate_interest(2_000_000, r, t, Rounding::Floor)
                >= prorate_interest(1_000_000, r, t, Rounding::Floor)
        );
    }

    #[test]
    fn prorate_interest_max_u32_principal_and_time() {
        // Stress test with u32::MAX values — should not panic
        let p = u32::MAX as u128; // ~4.3 × 10^9
        let r = 10_000_u32;
        let t = u32::MAX as u64; // ~4.3 × 10^9 seconds ≈ 136 years
                                 // p × r × t = 4.3e9 × 10_000 × 4.3e9 ≈ 1.85 × 10^23 — fits in u128
        let _ = prorate_interest(p, r, t, Rounding::Floor);
        let _ = prorate_interest(p, r, t, Rounding::Ceil);
    }

    #[test]
    fn prorate_interest_exact_boundary_no_remainder() {
        // Construct inputs where the division is exact → floor == ceil
        // principal × rate_bps × time_delta must be divisible by BPS_YEAR_DENOM
        // Use principal = BPS_YEAR_DENOM, rate = 10_000, time = SECONDS_PER_YEAR
        // → BPS_YEAR_DENOM × 10_000 × SECONDS_PER_YEAR / BPS_YEAR_DENOM
        //   = 10_000 × SECONDS_PER_YEAR
        let p = BPS_YEAR_DENOM;
        let r = 10_000_u32;
        let t = SECONDS_PER_YEAR as u64;
        let floor = prorate_interest(p, r, t, Rounding::Floor);
        let ceil = prorate_interest(p, r, t, Rounding::Ceil);
        assert_eq!(floor, ceil, "exact division should give floor == ceil");
    }
}
