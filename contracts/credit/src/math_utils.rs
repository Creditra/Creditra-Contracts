// SPDX-License-Identifier: MIT

//! # Fixed-Point Interest Math Utilities
//!
//! Deterministic, integer-only arithmetic helpers used by the interest
//! accrual path in [`crate::accrual`] and by oracle deviation checks in
//! [`crate::lib::settle_default_liquidation`].
//!
//! ## What
//!
//! - [`mul_div`] — checked `a * num / denom` with explicit [`Rounding`].
//! - [`apply_bps`] — apply a `bps` rate to a `u128` principal.
//! - [`prorate_interest`] — the live accrual primitive:
//!   `floor((u * r * Δt) / (10_000 * 31_557_600))`.
//! - [`compute_deviation_bps`] — oracle deviation in bps between two
//!   prices; `None` when the prior price is non-positive; saturates to
//!   `u32::MAX` on absurd new prices to ensure the circuit breaker still
//!   trips.
//! - [`scale_up`] / [`scale_down`] — 10^18 fixed-point helpers.
//!
//! ## How
//!
//! All intermediate products are promoted to `u128` and multiplied with
//! checked primitives; overflow panics (which in `apply_accrual`'s caller
//! is translated into `ContractError::Overflow = 12`). The caller chooses
//! whether the division remainder is discarded (floor) or rounded up
//! (ceiling) via [`Rounding`].
//!
//! Interest rates are in **basis points** (`1 bps = 1 / 10_000`). Time is in
//! ledger seconds; one **Julian year** is defined as
//! [`SECONDS_PER_YEAR`] `= 31_557_600` (365.25 × 86 400), matching the
//! convention used by most on-chain interest protocols. The pre-computed
//! constant [`BPS_YEAR_DENOM`] `= BPS_DENOMINATOR * SECONDS_PER_YEAR` lets
//! the final division be a single `u128` operation.
//!
//! ## Why (overflow safety)
//!
//! The worst-case intermediate product is:
//!
//! ```text
//! principal  ≤ i128::MAX  ≈ 1.7 × 10^38
//! rate_bps   ≤ 10_000
//! time_delta ≤ u64::MAX   ≈ 1.8 × 10^19
//! SCALE      = 10^18
//! ```
//!
//! `principal * rate_bps * time_delta` can reach ~3 × 10^61, which overflows
//! `u128` (max ~3.4 × 10^38). The multiplication is therefore split into two
//! checked steps:
//!
//! 1. `a = principal * rate_bps` — fits in u128 for any realistic principal
//!    (≤ 10^28 × 10^4 = 10^32 < 10^38).
//! 2. `b = a * time_delta` — checked; panics on overflow.
//!
//! The combination of `checked_mul` here and `overflow-checks = true` in
//! the release profile (see workspace `Cargo.toml`) is what makes the
//! accounting layer formally overflow-safe.
//!
//! ## Rounding direction
//!
//! For accrual the caller passes [`Rounding::Floor`], so every realized
//! `ΔI` rounds **down**. This biases the rounding error against protocol
//! revenue and never against the borrower's balance — a deliberate safety
//! property documented in [`docs/RISK_PRICING.md`](../../../docs/RISK_PRICING.md).

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
    amount
        .checked_mul(SCALE)
        .expect("math_utils: scale_up overflow")
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
                quotient
                    .checked_add(1)
                    .expect("math_utils: scale_down ceil overflow")
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
///   Must be non-negative; pass `utilized_amount as u128` after a sign check.
/// - `rate_bps`   — annual interest rate in basis points (0 ..= 10 000).
/// - `time_delta` — elapsed seconds since the last accrual (`current_ts - last_accrual_ts`).
/// - `rounding`   — [`Rounding::Floor`] (default, protocol-favourable) or
///   [`Rounding::Ceil`] (borrower-favourable minimum repayment).
///
/// # Returns
///
/// The interest amount in the same unit as `principal`.  Returns `0` when
/// `principal`, `rate_bps`, or `time_delta` is zero.
///
/// # Panics
///
/// Panics if the intermediate product `principal × rate_bps × time_delta`
/// overflows `u128`.  For realistic credit-line values (principal ≤ 10^28,
/// rate ≤ 10 000, time ≤ ~584 years in seconds) this will not occur.
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
///
/// // Zero rate → zero interest
/// assert_eq!(prorate_interest(10_000, 0, 86_400, Rounding::Floor), 0);
///
/// // Zero time → zero interest
/// assert_eq!(prorate_interest(10_000, 300, 0, Rounding::Floor), 0);
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

    // Step 1: principal × rate_bps  (fits in u128 for principal ≤ ~3.4 × 10^34)
    let step1 = principal
        .checked_mul(rate_bps as u128)
        .expect("math_utils: prorate overflow (step1)");

    // Step 2: step1 × time_delta
    let step2 = step1
        .checked_mul(time_delta as u128)
        .expect("math_utils: prorate overflow (step2)");

    // Step 3: divide by (BPS_DENOMINATOR × SECONDS_PER_YEAR) with rounding
    let quotient = step2 / BPS_YEAR_DENOM;
    match rounding {
        Rounding::Floor => quotient,
        Rounding::Ceil => {
            if step2 % BPS_YEAR_DENOM != 0 {
                quotient
                    .checked_add(1)
                    .expect("math_utils: prorate ceil overflow")
            } else {
                quotient
            }
        }
    }
}

// ─── Oracle deviation helper ──────────────────────────────────────────────────

/// Compute the absolute deviation between `new_price` and `last_price` in basis points.
///
/// # Formula
/// ```text
/// deviation_bps = |new_price - last_price| * 10_000 / last_price
/// ```
///
/// Returns `None` if `last_price` is zero (undefined).
///
/// # Overflow safety
/// Intermediate arithmetic is performed in `u128`. For realistic price values
/// (≤ i128::MAX ≈ 1.7 × 10^38) the product `diff * 10_000` fits in u128.
///
/// # Examples
/// ```rust
/// use creditra_credit::math_utils::compute_deviation_bps;
///
/// // 5% deviation: last=1000, new=1050 → 500 bps
/// assert_eq!(compute_deviation_bps(1050, 1000), Some(500));
///
/// // 5% deviation downward: last=1000, new=950 → 500 bps
/// assert_eq!(compute_deviation_bps(950, 1000), Some(500));
///
/// // Zero last price → None
/// assert_eq!(compute_deviation_bps(100, 0), None);
/// ```
pub fn compute_deviation_bps(new_price: i128, last_price: i128) -> Option<u32> {
    if last_price <= 0 {
        return None;
    }
    let diff = (new_price - last_price).unsigned_abs();
    // diff * 10_000 / last_price — both operands are u128
    let numerator = diff.checked_mul(BPS_DENOMINATOR)?;
    let deviation = numerator / (last_price as u128);
    // Cap at u32::MAX to avoid truncation; any value > 10_000 already exceeds any threshold
    Some(deviation.min(u32::MAX as u128) as u32)
}

// ─── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── mul_div ──────────────────────────────────────────────────────────────

    #[test]
    fn mul_div_basic() {
        assert_eq!(mul_div(1_000, 300, 10_000, Rounding::Floor), 30);
    }

    #[test]
    fn mul_div_truncates_toward_zero() {
        // 7 * 1 / 3 = 2.33… → 2
        assert_eq!(mul_div(7, 1, 3, Rounding::Floor), 2);
    }

    #[test]
    fn mul_div_identity_denominator() {
        assert_eq!(mul_div(42, 1, 1, Rounding::Floor), 42);
    }

    // ── apply_bps ────────────────────────────────────────────────────────────

    #[test]
    fn apply_bps_half_percent_truncates() {
        assert_eq!(apply_bps(200, 50, Rounding::Floor), 1);
    }

    #[test]
    fn apply_bps_sub_unit_truncates_to_zero() {
        assert_eq!(apply_bps(50, 1, Rounding::Floor), 0);
    }

    // ── mul_div ───────────────────────────────────────────────────────────────

    #[test]
    fn mul_div_exact_floor() {
        // 1 000 × 3 / 10 = 300 exactly
        assert_eq!(mul_div(1_000, 3, 10, Rounding::Floor), 300);
    }

    #[test]
    fn mul_div_exact_ceil() {
        // 1 000 × 3 / 10 = 300 exactly — ceil should not add 1
        assert_eq!(mul_div(1_000, 3, 10, Rounding::Ceil), 300);
    }

    #[test]
    fn mul_div_remainder_floor() {
        // 1 001 × 3 / 10 = 300.3 → floor → 300
        assert_eq!(mul_div(1_001, 3, 10, Rounding::Floor), 300);
    }

    #[test]
    fn mul_div_remainder_ceil() {
        // 1 001 × 3 / 10 = 300.3 → ceil → 301
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
    fn mul_div_denominator_equals_numerator() {
        // a × n / n = a
        assert_eq!(mul_div(42, 7, 7, Rounding::Floor), 42);
        assert_eq!(mul_div(42, 7, 7, Rounding::Ceil), 42);
    }

    #[test]
    fn mul_div_large_values_floor() {
        // u128::MAX / 2 × 2 / 2 = u128::MAX / 2
        let half = u128::MAX / 2;
        assert_eq!(mul_div(half, 2, 2, Rounding::Floor), half);
    }

    #[test]
    fn mul_div_one_bps_of_small_amount_floor() {
        // 1 token × 1 bps / 10_000 = 0.0001 → floor → 0
        assert_eq!(mul_div(1, 1, 10_000, Rounding::Floor), 0);
    }

    #[test]
    fn mul_div_one_bps_of_small_amount_ceil() {
        // 1 token × 1 bps / 10_000 = 0.0001 → ceil → 1
        assert_eq!(mul_div(1, 1, 10_000, Rounding::Ceil), 1);
    }

    #[test]
    #[should_panic(expected = "division by zero")]
    fn mul_div_zero_denominator_panics() {
        mul_div(100, 1, 0, Rounding::Floor);
    }

    // ── scale_up / scale_down ─────────────────────────────────────────────────

    #[test]
    fn scale_up_and_down_roundtrip_floor() {
        let v = 12_345_678_u128;
        assert_eq!(scale_down(scale_up(v), Rounding::Floor), v);
    }

    #[test]
    fn scale_up_and_down_roundtrip_ceil_exact() {
        let v = 99_u128;
        // scale_up then scale_down with ceil on an exact multiple → same value
        assert_eq!(scale_down(scale_up(v), Rounding::Ceil), v);
    }

    #[test]
    fn scale_down_ceil_adds_one_for_remainder() {
        // SCALE + 1 → quotient 1, remainder 1 → ceil → 2
        assert_eq!(scale_down(SCALE + 1, Rounding::Ceil), 2);
    }

    #[test]
    fn scale_down_floor_truncates_remainder() {
        // SCALE + 1 → quotient 1, remainder 1 → floor → 1
        assert_eq!(scale_down(SCALE + 1, Rounding::Floor), 1);
    }

    #[test]
    fn scale_down_zero() {
        assert_eq!(scale_down(0, Rounding::Floor), 0);
        assert_eq!(scale_down(0, Rounding::Ceil), 0);
    }

    // ── apply_bps ─────────────────────────────────────────────────────────────

    #[test]
    fn apply_bps_full_rate() {
        assert_eq!(apply_bps(500, 10_000, Rounding::Floor), 500);
        // 10 000 tokens × 10 000 bps (100 %) = 10 000 tokens
        assert_eq!(apply_bps(10_000, 10_000, Rounding::Floor), 10_000);
    }

    #[test]
    fn apply_bps_zero_rate() {
        assert_eq!(apply_bps(1_000_000, 0, Rounding::Floor), 0);
    }

    // ── prorate_interest ─────────────────────────────────────────────────────

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
    }

    #[test]
    fn apply_bps_one_bps_small_amount_ceil() {
        // 1 token × 1 bps = 0.0001 → ceil → 1
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
    fn prorate_interest_one_day() {
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
        // 10% on 100_000 for exactly 1 year = 10_000
        assert_eq!(
            prorate_interest(100_000, 1_000, 31_536_000, Rounding::Floor),
            10_000
        );
    }

    #[test]
    fn prorate_interest_one_hour() {
        // 5% on 1_000_000 for 3_600 s ≈ 5
        assert_eq!(prorate_interest(1_000_000, 500, 3_600, Rounding::Floor), 5);
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

    // ── compute_deviation_bps ─────────────────────────────────────────────────

    #[test]
    fn deviation_five_percent_up() {
        // 1050 vs 1000 → 50/1000 * 10_000 = 500 bps
        assert_eq!(compute_deviation_bps(1_050, 1_000), Some(500));
    }

    #[test]
    fn deviation_five_percent_down() {
        // 950 vs 1000 → 50/1000 * 10_000 = 500 bps
        assert_eq!(compute_deviation_bps(950, 1_000), Some(500));
    }

    #[test]
    fn deviation_zero_change() {
        assert_eq!(compute_deviation_bps(1_000, 1_000), Some(0));
    }

    #[test]
    fn deviation_one_bps() {
        // 10_001 vs 10_000 → 1/10_000 * 10_000 = 1 bps
        assert_eq!(compute_deviation_bps(10_001, 10_000), Some(1));
    }

    #[test]
    fn deviation_hundred_percent() {
        // 2000 vs 1000 → 1000/1000 * 10_000 = 10_000 bps
        assert_eq!(compute_deviation_bps(2_000, 1_000), Some(10_000));
    }

    #[test]
    fn deviation_zero_last_price_returns_none() {
        assert_eq!(compute_deviation_bps(100, 0), None);
    }

    #[test]
    fn deviation_negative_last_price_returns_none() {
        assert_eq!(compute_deviation_bps(100, -1), None);
    }
    // ── prorate_interest: monotonicity & overflow-safety properties ──────────
    //
    // These mirror the `cfg(kani)` harnesses in `proofs/prorate_interest.rs`
    // so the same guarantees run under the normal `cargo test` suite (Kani
    // runs only in CI). Inputs use the same overflow-safe envelope.
    mod prorate_interest_properties {
        use crate::math_utils::{prorate_interest, Rounding};
        use proptest::prelude::*;

        const PRINCIPAL_MAX: u128 = 1_000_000_000_000_000_000_000_000; // 10^24
        const RATE_MAX: u32 = 10_000;
        const TIME_MAX: u64 = 1_000_000_000; // 10^9 (~31.7 years)

        #[test]
        fn overflow_safe_at_envelope_corners() {
            let _ = prorate_interest(PRINCIPAL_MAX, RATE_MAX, TIME_MAX, Rounding::Floor);
            let _ = prorate_interest(PRINCIPAL_MAX, RATE_MAX, TIME_MAX, Rounding::Ceil);
        }

        #[test]
        fn ceil_within_one_of_floor() {
            for &(p, r, t) in &[
                (10_000u128, 300u32, 86_400u64),
                (1, 1, 1),
                (PRINCIPAL_MAX, RATE_MAX, TIME_MAX),
                (123_456_789, 777, 1_000_000),
            ] {
                let f = prorate_interest(p, r, t, Rounding::Floor);
                let c = prorate_interest(p, r, t, Rounding::Ceil);
                assert!(c >= f, "ceil < floor for ({p}, {r}, {t})");
                assert!(c - f <= 1, "ceil - floor > 1 for ({p}, {r}, {t})");
            }
        }

        proptest! {
            #[test]
            fn monotonic_in_principal(
                principal in 0u128..PRINCIPAL_MAX,
                rate_bps in 0u32..=RATE_MAX,
                time_delta in 0u64..=TIME_MAX,
            ) {
                prop_assert!(
                    prorate_interest(principal + 1, rate_bps, time_delta, Rounding::Floor)
                        >= prorate_interest(principal, rate_bps, time_delta, Rounding::Floor)
                );
                prop_assert!(
                    prorate_interest(principal + 1, rate_bps, time_delta, Rounding::Ceil)
                        >= prorate_interest(principal, rate_bps, time_delta, Rounding::Ceil)
                );
            }

            #[test]
            fn monotonic_in_rate(
                principal in 0u128..=PRINCIPAL_MAX,
                rate_bps in 0u32..RATE_MAX,
                time_delta in 0u64..=TIME_MAX,
            ) {
                prop_assert!(
                    prorate_interest(principal, rate_bps + 1, time_delta, Rounding::Floor)
                        >= prorate_interest(principal, rate_bps, time_delta, Rounding::Floor)
                );
            }

            #[test]
            fn monotonic_in_time(
                principal in 0u128..=PRINCIPAL_MAX,
                rate_bps in 0u32..=RATE_MAX,
                time_delta in 0u64..TIME_MAX,
            ) {
                prop_assert!(
                    prorate_interest(principal, rate_bps, time_delta + 1, Rounding::Floor)
                        >= prorate_interest(principal, rate_bps, time_delta, Rounding::Floor)
                );
            }

            #[test]
            fn overflow_safe_envelope(
                principal in 0u128..=PRINCIPAL_MAX,
                rate_bps in 0u32..=RATE_MAX,
                time_delta in 0u64..=TIME_MAX,
            ) {
                let _ = prorate_interest(principal, rate_bps, time_delta, Rounding::Floor);
                let _ = prorate_interest(principal, rate_bps, time_delta, Rounding::Ceil);
            }
        }
    }
}
