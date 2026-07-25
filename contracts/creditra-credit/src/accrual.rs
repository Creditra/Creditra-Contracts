// SPDX-License-Identifier: MIT

//! # Simple-interest accrual
//!
//! Pure, side-effect-free computation of the interest accrued on a utilized
//! balance over an elapsed period. The formula mirrors the Soroban sibling
//! contract's `prorate_interest` / `compute_interest` so the two runtimes
//! accrue identically:
//!
//! ```text
//! interest = principal · rate_bps · elapsed_seconds / (10_000 · SECONDS_PER_YEAR)
//! ```
//!
//! rounded **down** (floor). Interest is zero whenever any of `principal`,
//! `rate_bps`, or `elapsed_seconds` is zero.
//!
//! ## Monotonicity
//!
//! The accrued amount is **non-decreasing** in each of its three inputs — most
//! importantly in `elapsed_seconds`: as ledger time advances, accrued interest
//! can only grow or stay flat, never shrink. This "accrued interest never
//! decreases" invariant is exercised by the property test in
//! `tests/proptest_monotonic.rs`.
//!
//! ## Overflow safety
//!
//! Every multiplication uses `Uint128::checked_mul`. If the intermediate
//! product `principal · rate_bps · elapsed_seconds` would exceed
//! `Uint128::MAX`, the function returns [`ContractError::Overflow`] rather than
//! panicking or wrapping, so callers revert deterministically. There are no
//! `unwrap()`/`expect()`/`panic!` calls on any production path.

use cosmwasm_std::Uint128;

use crate::error::ContractError;

/// Seconds in a 365-day year — the accrual time base.
pub const SECONDS_PER_YEAR: u64 = 31_536_000;

/// Basis-point denominator: `10_000 bps == 100%`.
pub const BPS_DENOMINATOR: u64 = 10_000;

/// Combined divisor `10_000 · SECONDS_PER_YEAR`, applied once after the
/// numerator is accumulated. Non-zero by construction, so division is total.
const BPS_YEAR_DENOM: Uint128 =
    Uint128::new((BPS_DENOMINATOR as u128) * (SECONDS_PER_YEAR as u128));

/// Compute the simple interest accrued on `principal` at `rate_bps`
/// (annualised basis points) over `elapsed_seconds`.
///
/// Returns `interest = principal · rate_bps · elapsed_seconds / (10_000 ·
/// SECONDS_PER_YEAR)`, rounded down. The result is `0` when any input is zero.
///
/// # Errors
///
/// Returns [`ContractError::Overflow`] if the intermediate product
/// `principal · rate_bps · elapsed_seconds` overflows `Uint128`.
///
/// # Examples
///
/// ```
/// use creditra_credit::accrual::{accrued_interest, SECONDS_PER_YEAR};
/// use cosmwasm_std::Uint128;
///
/// // 10_000 at 300 bps (3%) for exactly one year → 300.
/// assert_eq!(
///     accrued_interest(Uint128::new(10_000), 300, SECONDS_PER_YEAR).unwrap(),
///     Uint128::new(300)
/// );
///
/// // Zero elapsed time → zero interest.
/// assert_eq!(
///     accrued_interest(Uint128::new(10_000), 300, 0).unwrap(),
///     Uint128::zero()
/// );
/// ```
pub fn accrued_interest(
    principal: Uint128,
    rate_bps: u32,
    elapsed_seconds: u64,
) -> Result<Uint128, ContractError> {
    if principal.is_zero() || rate_bps == 0 || elapsed_seconds == 0 {
        return Ok(Uint128::zero());
    }

    let numerator = principal
        .checked_mul(Uint128::from(rate_bps))
        .and_then(|v| v.checked_mul(Uint128::from(elapsed_seconds)))
        .map_err(|_| ContractError::Overflow)?;

    // BPS_YEAR_DENOM is a non-zero constant; the checked form keeps the path
    // free of production unwraps.
    numerator
        .checked_div(BPS_YEAR_DENOM)
        .map_err(|_| ContractError::Overflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_full_year_at_three_percent() {
        // 10_000 · 300 · SECONDS_PER_YEAR / (10_000 · SECONDS_PER_YEAR) = 300
        assert_eq!(
            accrued_interest(Uint128::new(10_000), 300, SECONDS_PER_YEAR).unwrap(),
            Uint128::new(300)
        );
    }

    #[test]
    fn one_day_at_five_percent() {
        // 1_000_000 · 500 · 86_400 / 315_360_000_000 = 43_200_000_000_000
        // / 315_360_000_000 = 136.98… → 136 (floored)
        assert_eq!(
            accrued_interest(Uint128::new(1_000_000), 500, 86_400).unwrap(),
            Uint128::new(136)
        );
    }

    #[test]
    fn zero_principal_is_zero() {
        assert_eq!(
            accrued_interest(Uint128::zero(), 500, 86_400).unwrap(),
            Uint128::zero()
        );
    }

    #[test]
    fn zero_rate_is_zero() {
        assert_eq!(
            accrued_interest(Uint128::new(1_000_000), 0, 86_400).unwrap(),
            Uint128::zero()
        );
    }

    #[test]
    fn zero_time_is_zero() {
        assert_eq!(
            accrued_interest(Uint128::new(1_000_000), 500, 0).unwrap(),
            Uint128::zero()
        );
    }

    #[test]
    fn sub_unit_interest_floors_to_zero() {
        // 1 · 1 · 1 / 315_360_000_000 = 0 (floored)
        assert_eq!(
            accrued_interest(Uint128::new(1), 1, 1).unwrap(),
            Uint128::zero()
        );
    }

    #[test]
    fn half_year_is_half_the_annual_interest() {
        let full = accrued_interest(Uint128::new(1_000_000), 400, SECONDS_PER_YEAR).unwrap();
        let half = accrued_interest(Uint128::new(1_000_000), 400, SECONDS_PER_YEAR / 2).unwrap();
        assert_eq!(full, Uint128::new(40_000));
        assert_eq!(half, Uint128::new(20_000));
    }

    #[test]
    fn overflow_returns_error() {
        // Uint128::MAX · rate · seconds overflows the 128-bit numerator.
        assert_eq!(
            accrued_interest(Uint128::MAX, 10_000, SECONDS_PER_YEAR),
            Err(ContractError::Overflow)
        );
    }

    #[test]
    fn large_but_representable_principal_ok() {
        // Choose principal so principal · 10_000 · SECONDS_PER_YEAR ≤ Uint128::MAX.
        let principal = Uint128::MAX.checked_div(BPS_YEAR_DENOM).unwrap();
        let result = accrued_interest(principal, 10_000, SECONDS_PER_YEAR);
        assert!(result.is_ok());
    }
}
