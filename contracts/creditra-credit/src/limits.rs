// SPDX-License-Identifier: MIT

//! # Per-borrower interest-rate ceilings
//!
//! Pure, side-effect-free logic for enforcing a cap on the interest rate that
//! may be charged to any single borrower. Interest rates are expressed in
//! **basis points** (bps), where `10_000 bps == 100%`.
//!
//! ## Model
//!
//! The protocol maintains two layers of ceiling:
//!
//! 1. A protocol-wide **default** ceiling that applies to every borrower who
//!    has no explicit override.
//! 2. An optional **per-borrower override** that replaces the default for a
//!    single borrower.
//!
//! The [`effective_ceiling_bps`] function resolves these two layers: the
//! per-borrower override always wins when present, otherwise the default
//! applies. Every configurable ceiling is bounded above by the absolute
//! protocol maximum [`MAX_RATE_BPS`], which no default or override may exceed.
//!
//! ## Security properties
//!
//! - All arithmetic is overflow-safe: bounds are checked with `u32`/`Uint128`
//!   checked operations, and there are no `unwrap()`/`expect()`/`panic!` calls
//!   in production paths.
//! - Ceiling configuration is validated at the boundary via
//!   [`validate_ceiling_bps`], so out-of-range values can never be persisted.
//! - Rate enforcement is total: [`check_rate_within_ceiling`] returns a typed
//!   [`ContractError`] rather than silently clamping, so callers cannot
//!   accidentally over-charge a borrower.

use cosmwasm_std::Uint128;

use crate::error::ContractError;

/// Basis-point denominator: `10_000 bps == 100%`.
pub const BPS_DENOMINATOR: u32 = 10_000;

/// Absolute protocol-wide maximum interest rate, in basis points.
///
/// No configured ceiling — neither the default nor any per-borrower override —
/// may exceed this bound. `10_000 bps == 100%` is chosen as a conservative
/// hard cap: a per-borrower interest rate above the principal itself is treated
/// as a configuration error rather than a valid ceiling.
pub const MAX_RATE_BPS: u32 = 10_000;

/// Return `true` if `bps` is a valid ceiling value (within [`MAX_RATE_BPS`]).
///
/// A ceiling of `0` is permitted and means "no interest may be charged".
pub const fn is_valid_ceiling_bps(bps: u32) -> bool {
    bps <= MAX_RATE_BPS
}

/// Validate a ceiling value, returning it unchanged when in range.
///
/// # Errors
///
/// Returns [`ContractError::InvalidAmount`] when `bps` exceeds
/// [`MAX_RATE_BPS`].
pub fn validate_ceiling_bps(bps: u32) -> Result<u32, ContractError> {
    if is_valid_ceiling_bps(bps) {
        Ok(bps)
    } else {
        Err(ContractError::InvalidAmount)
    }
}

/// Resolve the effective ceiling for a borrower.
///
/// The per-borrower `borrower_override` always takes precedence; when it is
/// `None`, the protocol-wide `default_bps` applies.
///
/// # Examples
///
/// ```
/// use creditra_credit::limits::effective_ceiling_bps;
///
/// // No override → default applies.
/// assert_eq!(effective_ceiling_bps(1_500, None), 1_500);
/// // Override present → override wins, even when higher or lower.
/// assert_eq!(effective_ceiling_bps(1_500, Some(800)), 800);
/// assert_eq!(effective_ceiling_bps(1_500, Some(2_000)), 2_000);
/// ```
pub const fn effective_ceiling_bps(default_bps: u32, borrower_override: Option<u32>) -> u32 {
    match borrower_override {
        Some(bps) => bps,
        None => default_bps,
    }
}

/// Assert that a proposed interest rate does not exceed a ceiling.
///
/// # Errors
///
/// Returns [`ContractError::RateCeilingExceeded`] when
/// `rate_bps > ceiling_bps`.
pub fn check_rate_within_ceiling(rate_bps: u32, ceiling_bps: u32) -> Result<(), ContractError> {
    if rate_bps > ceiling_bps {
        return Err(ContractError::RateCeilingExceeded);
    }
    Ok(())
}

/// Clamp a proposed interest rate down to the ceiling.
///
/// Unlike [`check_rate_within_ceiling`], this never errors — it returns the
/// smaller of `rate_bps` and `ceiling_bps`. Use it when the protocol prefers
/// to silently honour the cap rather than reject the request.
pub const fn clamp_rate_to_ceiling(rate_bps: u32, ceiling_bps: u32) -> u32 {
    if rate_bps < ceiling_bps {
        rate_bps
    } else {
        ceiling_bps
    }
}

/// Compute the maximum interest chargeable on `principal` at `ceiling_bps`.
///
/// `interest = principal * ceiling_bps / 10_000`, computed with checked
/// `Uint128` arithmetic so that a large principal can never overflow. The
/// division by the non-zero constant [`BPS_DENOMINATOR`] truncates toward
/// zero, matching integer-interest accrual conventions.
///
/// # Errors
///
/// Returns [`ContractError::InvalidAmount`] if the intermediate multiplication
/// `principal * ceiling_bps` overflows `Uint128`.
pub fn max_interest_for_principal(
    principal: Uint128,
    ceiling_bps: u32,
) -> Result<Uint128, ContractError> {
    let scaled = principal
        .checked_mul(Uint128::from(ceiling_bps))
        .map_err(|_| ContractError::InvalidAmount)?;
    // BPS_DENOMINATOR is a non-zero constant, so division cannot fail; the
    // checked form is used to keep the code free of production unwraps.
    scaled
        .checked_div(Uint128::from(BPS_DENOMINATOR))
        .map_err(|_| ContractError::InvalidAmount)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_valid_ceiling_bps / validate_ceiling_bps ─────────────────────────

    #[test]
    fn zero_ceiling_is_valid() {
        assert!(is_valid_ceiling_bps(0));
        assert_eq!(validate_ceiling_bps(0), Ok(0));
    }

    #[test]
    fn max_ceiling_is_valid() {
        assert!(is_valid_ceiling_bps(MAX_RATE_BPS));
        assert_eq!(validate_ceiling_bps(MAX_RATE_BPS), Ok(MAX_RATE_BPS));
    }

    #[test]
    fn mid_range_ceiling_is_valid() {
        assert!(is_valid_ceiling_bps(1_500));
        assert_eq!(validate_ceiling_bps(1_500), Ok(1_500));
    }

    #[test]
    fn just_over_max_is_invalid() {
        assert!(!is_valid_ceiling_bps(MAX_RATE_BPS + 1));
        assert_eq!(
            validate_ceiling_bps(MAX_RATE_BPS + 1),
            Err(ContractError::InvalidAmount)
        );
    }

    #[test]
    fn far_over_max_is_invalid() {
        assert!(!is_valid_ceiling_bps(u32::MAX));
        assert_eq!(
            validate_ceiling_bps(u32::MAX),
            Err(ContractError::InvalidAmount)
        );
    }

    // ── effective_ceiling_bps ───────────────────────────────────────────────

    #[test]
    fn effective_falls_back_to_default_when_no_override() {
        assert_eq!(effective_ceiling_bps(1_500, None), 1_500);
        assert_eq!(effective_ceiling_bps(0, None), 0);
    }

    #[test]
    fn effective_override_takes_precedence() {
        assert_eq!(effective_ceiling_bps(1_500, Some(800)), 800);
    }

    #[test]
    fn effective_override_can_exceed_default() {
        // An override is authoritative; it may raise as well as lower the cap
        // (still bounded elsewhere by validate_ceiling_bps at write time).
        assert_eq!(effective_ceiling_bps(1_000, Some(2_500)), 2_500);
    }

    #[test]
    fn effective_zero_override_disables_interest() {
        assert_eq!(effective_ceiling_bps(1_500, Some(0)), 0);
    }

    // ── check_rate_within_ceiling ───────────────────────────────────────────

    #[test]
    fn rate_below_ceiling_ok() {
        assert_eq!(check_rate_within_ceiling(900, 1_000), Ok(()));
    }

    #[test]
    fn rate_equal_to_ceiling_ok() {
        assert_eq!(check_rate_within_ceiling(1_000, 1_000), Ok(()));
    }

    #[test]
    fn rate_above_ceiling_rejected() {
        assert_eq!(
            check_rate_within_ceiling(1_001, 1_000),
            Err(ContractError::RateCeilingExceeded)
        );
    }

    #[test]
    fn zero_ceiling_rejects_any_positive_rate() {
        assert_eq!(check_rate_within_ceiling(0, 0), Ok(()));
        assert_eq!(
            check_rate_within_ceiling(1, 0),
            Err(ContractError::RateCeilingExceeded)
        );
    }

    // ── clamp_rate_to_ceiling ───────────────────────────────────────────────

    #[test]
    fn clamp_leaves_rate_below_ceiling_unchanged() {
        assert_eq!(clamp_rate_to_ceiling(900, 1_000), 900);
    }

    #[test]
    fn clamp_caps_rate_above_ceiling() {
        assert_eq!(clamp_rate_to_ceiling(5_000, 1_000), 1_000);
    }

    #[test]
    fn clamp_at_exact_boundary() {
        assert_eq!(clamp_rate_to_ceiling(1_000, 1_000), 1_000);
    }

    #[test]
    fn clamp_to_zero_ceiling() {
        assert_eq!(clamp_rate_to_ceiling(5_000, 0), 0);
    }

    // ── max_interest_for_principal ──────────────────────────────────────────

    #[test]
    fn interest_at_full_rate_equals_principal() {
        // 100% of 1_000 == 1_000
        assert_eq!(
            max_interest_for_principal(Uint128::new(1_000), 10_000),
            Ok(Uint128::new(1_000))
        );
    }

    #[test]
    fn interest_at_fifteen_percent() {
        // 1_500 bps of 1_000 == 150
        assert_eq!(
            max_interest_for_principal(Uint128::new(1_000), 1_500),
            Ok(Uint128::new(150))
        );
    }

    #[test]
    fn interest_zero_rate_is_zero() {
        assert_eq!(
            max_interest_for_principal(Uint128::new(1_000), 0),
            Ok(Uint128::zero())
        );
    }

    #[test]
    fn interest_zero_principal_is_zero() {
        assert_eq!(
            max_interest_for_principal(Uint128::zero(), 5_000),
            Ok(Uint128::zero())
        );
    }

    #[test]
    fn interest_truncates_toward_zero() {
        // 1 bps of 100 == 100 * 1 / 10_000 == 0 (integer truncation)
        assert_eq!(
            max_interest_for_principal(Uint128::new(100), 1),
            Ok(Uint128::zero())
        );
    }

    #[test]
    fn interest_on_large_principal_does_not_overflow() {
        // principal * ceiling_bps fits in Uint128 for max principal * 10_000.
        let principal = Uint128::MAX
            .checked_div(Uint128::from(MAX_RATE_BPS))
            .unwrap();
        let result = max_interest_for_principal(principal, MAX_RATE_BPS);
        assert!(result.is_ok());
    }

    #[test]
    fn interest_overflow_returns_error() {
        // Uint128::MAX * 10_000 overflows the 128-bit intermediate.
        assert_eq!(
            max_interest_for_principal(Uint128::MAX, MAX_RATE_BPS),
            Err(ContractError::InvalidAmount)
        );
    }
}
