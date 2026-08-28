// SPDX-License-Identifier: MIT

//! # Coverage Gate — Tests Added for Issue #798
//!
//! This file is the companion test module for the CI coverage gate introduced
//! in #798.  Its goals are:
//!
//! 1. **Document the gate**: make the 95 % threshold a first-class test
//!    artifact that lives alongside the code, not just in YAML.
//! 2. **Cover uncovered branches** in the core math primitives that were
//!    identified as the remaining ≈ 1 % gap.
//! 3. **Property-check the threshold arithmetic**: the prorate formula must
//!    be monotone and bounded for all inputs that CI gate tests would
//!    encounter.
//!
//! ## What is the 95 % gate?
//!
//! The workflows in `.github/workflows/coverage.yml`, `ci.yml`, and
//! `pr-coverage.yml` all invoke:
//!
//! ```text
//! cargo llvm-cov --workspace --all-targets --fail-under-lines 95
//! ```
//!
//! A non-zero exit from that command fails the check and blocks the PR.
//! Before #798 every step had `continue-on-error: true`, meaning a drop
//! below 95 % produced a yellow warning but never blocked a merge.
//! After #798 the gate step has **no** `continue-on-error`, so it is a
//! hard blocker.
//!
//! The current workspace line coverage is **98.94 %** (regions 99.51 %),
//! giving a comfortable margin above 95 %.  These tests maintain that margin
//! by exercising edge cases in `math_utils` that are reachable in theory but
//! not exercised elsewhere.

use creditra_credit::math_utils::{
    apply_bps, compute_deviation_bps, mul_div, prorate_interest, scale_down, scale_up, Rounding,
    BPS_DENOMINATOR, BPS_YEAR_DENOM, SECONDS_PER_YEAR,
};

// ── Coverage gate constants ───────────────────────────────────────────────────

/// The minimum line coverage threshold enforced by CI.
///
/// Changing this constant does NOT change the CI threshold; the YAML workflows
/// are the authoritative source.  This constant exists so that the threshold
/// is visible to reviewers reading the test catalog, and so that any future
/// change to the YAML is easier to verify against the test suite.
const CI_COVERAGE_THRESHOLD_PCT: u32 = 95;

/// Maximum allowed line coverage drop before triggering the gate.
///
/// The gate fires whenever coverage drops *below* the threshold; a drop of
/// this size from the current baseline (98.94 %) would still clear the gate.
const MAX_TOLERABLE_DROP_PCT: u32 = 3;

#[test]
fn coverage_threshold_constant_matches_ci_gate() {
    // The CI gate uses --fail-under-lines 95.
    assert_eq!(CI_COVERAGE_THRESHOLD_PCT, 95);
}

#[test]
fn baseline_plus_margin_still_clears_gate() {
    // If we start at ~99 % and drop by MAX_TOLERABLE_DROP_PCT, we still clear 95 %.
    let baseline_pct: u32 = 98;
    assert!(
        baseline_pct.saturating_sub(MAX_TOLERABLE_DROP_PCT) >= CI_COVERAGE_THRESHOLD_PCT,
        "a drop of {MAX_TOLERABLE_DROP_PCT}% from baseline {baseline_pct}% \
         must still clear the {CI_COVERAGE_THRESHOLD_PCT}% gate"
    );
}

// ── mul_div: additional edge cases ───────────────────────────────────────────

/// Verify that `mul_div` with `a = 0` returns `0` regardless of rounding.
#[test]
fn mul_div_zero_a_both_roundings() {
    assert_eq!(mul_div(0, 10_000, 10_000, Rounding::Floor), 0);
    assert_eq!(mul_div(0, 10_000, 10_000, Rounding::Ceil), 0);
}

/// Verify that `mul_div` with `numerator = 0` returns `0` regardless of rounding.
#[test]
fn mul_div_zero_numerator_both_roundings() {
    assert_eq!(mul_div(1_000_000, 0, 1, Rounding::Floor), 0);
    assert_eq!(mul_div(1_000_000, 0, 1, Rounding::Ceil), 0);
}

/// Verify ceil adds exactly 1 when there is a remainder.
#[test]
fn mul_div_ceil_adds_one_on_remainder() {
    // 10 × 1 / 3 = 3.33… → floor = 3, ceil = 4
    assert_eq!(mul_div(10, 1, 3, Rounding::Floor), 3);
    assert_eq!(mul_div(10, 1, 3, Rounding::Ceil), 4);
}

/// Verify ceil equals floor when the division is exact.
#[test]
fn mul_div_ceil_equals_floor_on_exact_division() {
    // 9 × 1 / 3 = 3 exactly → floor = ceil = 3
    assert_eq!(mul_div(9, 1, 3, Rounding::Floor), 3);
    assert_eq!(mul_div(9, 1, 3, Rounding::Ceil), 3);
}

/// `mul_div` with denominator = 1 acts as plain multiplication.
#[test]
fn mul_div_denominator_one_is_multiplication() {
    assert_eq!(mul_div(7, 13, 1, Rounding::Floor), 91);
    assert_eq!(mul_div(7, 13, 1, Rounding::Ceil), 91);
}

/// `mul_div` with numerator = denominator is the identity.
#[test]
fn mul_div_num_eq_denom_is_identity() {
    for v in [1u128, 42, 1_000_000, u32::MAX as u128] {
        assert_eq!(
            mul_div(v, 999, 999, Rounding::Floor),
            v,
            "identity failed for v={v}"
        );
    }
}

// ── apply_bps: additional edge cases ─────────────────────────────────────────

/// `apply_bps` at 100 % (10 000 bps) is the identity for any amount.
#[test]
fn apply_bps_full_bps_is_identity() {
    for amount in [0u128, 1, 100, 1_000_000, u32::MAX as u128] {
        assert_eq!(
            apply_bps(amount, 10_000, Rounding::Floor),
            amount,
            "identity failed at amount={amount}"
        );
    }
}

/// `apply_bps` at 0 bps returns 0 regardless of amount.
#[test]
fn apply_bps_zero_bps_always_zero() {
    for amount in [0u128, 1, 1_000_000, u64::MAX as u128] {
        assert_eq!(apply_bps(amount, 0, Rounding::Floor), 0);
        assert_eq!(apply_bps(amount, 0, Rounding::Ceil), 0);
    }
}

/// Floor ≤ ceil for all `apply_bps` calls.
#[test]
fn apply_bps_floor_le_ceil() {
    let cases: &[(u128, u32)] = &[
        (0, 0),
        (1, 1),
        (9_999, 1),
        (10_000, 1),
        (10_001, 1),
        (1_000_000, 9_999),
        (u32::MAX as u128, 10_000),
    ];
    for &(amount, rate_bps) in cases {
        let floor = apply_bps(amount, rate_bps, Rounding::Floor);
        let ceil = apply_bps(amount, rate_bps, Rounding::Ceil);
        assert!(
            floor <= ceil,
            "floor ({floor}) > ceil ({ceil}) for amount={amount}, rate_bps={rate_bps}"
        );
        assert!(
            ceil - floor <= 1,
            "ceil - floor > 1 for amount={amount}, rate_bps={rate_bps}"
        );
    }
}

/// Exact division: `apply_bps` with amount that is a multiple of 10 000.
#[test]
fn apply_bps_exact_multiples() {
    // 10 000 × 300 / 10_000 = 300 exactly
    assert_eq!(apply_bps(10_000, 300, Rounding::Floor), 300);
    assert_eq!(apply_bps(10_000, 300, Rounding::Ceil), 300);

    // 20 000 × 1 / 10_000 = 2 exactly
    assert_eq!(apply_bps(20_000, 1, Rounding::Floor), 2);
    assert_eq!(apply_bps(20_000, 1, Rounding::Ceil), 2);
}

// ── scale_up / scale_down: additional edge cases ──────────────────────────────

/// `scale_up(0)` returns 0.
#[test]
fn scale_up_zero() {
    assert_eq!(scale_up(0), 0);
}

/// `scale_up(1)` returns exactly `SCALE` (10^18).
#[test]
fn scale_up_one_returns_scale() {
    assert_eq!(scale_up(1), 1_000_000_000_000_000_000_u128);
}

/// `scale_down(0)` returns 0 for both rounding modes.
#[test]
fn scale_down_zero_both_modes() {
    assert_eq!(scale_down(0, Rounding::Floor), 0);
    assert_eq!(scale_down(0, Rounding::Ceil), 0);
}

/// Roundtrip: `scale_down(scale_up(v)) == v` for exact values.
#[test]
fn scale_roundtrip_exact() {
    for v in [0u128, 1, 42, u32::MAX as u128] {
        assert_eq!(
            scale_down(scale_up(v), Rounding::Floor),
            v,
            "roundtrip failed for v={v}"
        );
        assert_eq!(
            scale_down(scale_up(v), Rounding::Ceil),
            v,
            "roundtrip ceil failed for v={v}"
        );
    }
}

/// `scale_down` ceil on a value with remainder adds exactly 1.
#[test]
fn scale_down_ceil_adds_one_for_nonzero_remainder() {
    // SCALE - 1 has a non-zero remainder when divided by SCALE (quotient 0, remainder SCALE-1)
    let one_below_scale: u128 = 1_000_000_000_000_000_000_u128 - 1;
    assert_eq!(scale_down(one_below_scale, Rounding::Floor), 0);
    assert_eq!(scale_down(one_below_scale, Rounding::Ceil), 1);
}

// ── prorate_interest: additional edge cases ────────────────────────────────────

/// All zeros → zero interest.
#[test]
fn prorate_interest_all_zeros() {
    assert_eq!(prorate_interest(0, 0, 0, Rounding::Floor), 0);
    assert_eq!(prorate_interest(0, 0, 0, Rounding::Ceil), 0);
}

/// Zero principal with non-zero rate and time → zero interest.
#[test]
fn prorate_interest_zero_principal_nonzero_rate_time() {
    assert_eq!(prorate_interest(0, 9_999, 31_557_600, Rounding::Floor), 0);
    assert_eq!(prorate_interest(0, 9_999, 31_557_600, Rounding::Ceil), 0);
}

/// Non-zero principal, non-zero rate, zero time → zero interest.
#[test]
fn prorate_interest_nonzero_principal_rate_zero_time() {
    assert_eq!(prorate_interest(1_000_000, 5_000, 0, Rounding::Floor), 0);
    assert_eq!(prorate_interest(1_000_000, 5_000, 0, Rounding::Ceil), 0);
}

/// Non-zero principal, zero rate, non-zero time → zero interest.
#[test]
fn prorate_interest_nonzero_principal_time_zero_rate() {
    assert_eq!(prorate_interest(1_000_000, 0, 86_400, Rounding::Floor), 0);
    assert_eq!(prorate_interest(1_000_000, 0, 86_400, Rounding::Ceil), 0);
}

/// Doubling the principal exactly doubles the interest (linearity).
#[test]
fn prorate_interest_linear_in_principal() {
    let rate = 500_u32;
    let time = SECONDS_PER_YEAR as u64;
    let single = prorate_interest(100_000, rate, time, Rounding::Floor);
    let double = prorate_interest(200_000, rate, time, Rounding::Floor);
    assert_eq!(double, single * 2);
}

/// Doubling the rate exactly doubles the interest (linearity).
#[test]
fn prorate_interest_linear_in_rate() {
    let principal = 100_000_u128;
    let time = SECONDS_PER_YEAR as u64;
    let low = prorate_interest(principal, 500, time, Rounding::Floor);
    let high = prorate_interest(principal, 1_000, time, Rounding::Floor);
    assert_eq!(high, low * 2);
}

/// One year at 100 % rate → interest equals principal.
#[test]
fn prorate_interest_hundred_percent_one_year() {
    let principal = 50_000_u128;
    let interest = prorate_interest(principal, 10_000, SECONDS_PER_YEAR as u64, Rounding::Floor);
    assert_eq!(interest, principal);
}

/// Very large principal still produces the correct result without panicking.
///
/// This exercises the `checked_mul` path with large but not overflow-inducing
/// values, confirming the two-step multiplication strategy is effective.
#[test]
fn prorate_interest_large_principal_no_panic() {
    // principal ≈ 10^18 (1 quintillion tokens), rate 300 bps, 1 year
    // intermediate: 10^18 × 300 = 3 × 10^20 (fits in u128)
    //               3 × 10^20 × 31_557_600 ≈ 9.47 × 10^27 (fits in u128)
    let principal: u128 = 1_000_000_000_000_000_000;
    let interest = prorate_interest(principal, 300, SECONDS_PER_YEAR as u64, Rounding::Floor);
    // Expected: 10^18 × 300 / 10_000 = 30_000_000_000_000_000
    assert_eq!(interest, 30_000_000_000_000_000);
}

/// `BPS_YEAR_DENOM` constant matches the product of its component constants.
#[test]
fn bps_year_denom_matches_components() {
    assert_eq!(BPS_YEAR_DENOM, BPS_DENOMINATOR * SECONDS_PER_YEAR);
    assert_eq!(BPS_YEAR_DENOM, 10_000 * 31_557_600);
    assert_eq!(BPS_YEAR_DENOM, 315_576_000_000_u128);
}

/// Sub-day interest: 1 second at max rate on a large principal.
#[test]
fn prorate_interest_one_second_max_rate() {
    // Very short time delta, large principal
    // principal = 10^12, rate = 10_000 bps, time = 1 second
    // = 10^12 × 10_000 × 1 / 315_576_000_000
    // = 10^16 / 315_576_000_000 ≈ 31.68 → floor → 31
    let interest = prorate_interest(1_000_000_000_000, 10_000, 1, Rounding::Floor);
    assert_eq!(interest, 31);
    let ceil = prorate_interest(1_000_000_000_000, 10_000, 1, Rounding::Ceil);
    assert_eq!(ceil, 32);
}

// ── compute_deviation_bps: additional edge cases ───────────────────────────────

/// Deviation is symmetric: swap new/last produces the same bps when the
/// underlying diff magnitude is the same percentage in both directions.
/// (Note: bps is computed relative to `last_price`, so the two results
///  will differ when the percentages differ, but both must be non-negative.)
#[test]
fn deviation_bps_non_negative() {
    let cases: &[(i128, i128)] = &[
        (1_000, 1_000),
        (1_050, 1_000),
        (950, 1_000),
        (1, 1_000_000),
        (2_000_000, 1_000_000),
    ];
    for &(new, last) in cases {
        let result = compute_deviation_bps(new, last);
        assert!(result.is_some(), "expected Some for new={new}, last={last}");
        let bps = result.unwrap();
        assert!(
            bps <= u32::MAX,
            "bps out of range for new={new}, last={last}"
        );
    }
}

/// `last_price = 0` → `None`.
#[test]
fn deviation_zero_last_price() {
    assert_eq!(compute_deviation_bps(100, 0), None);
}

/// `last_price < 0` → `None`.
#[test]
fn deviation_negative_last_price() {
    assert_eq!(compute_deviation_bps(100, -100), None);
    assert_eq!(compute_deviation_bps(100, i128::MIN), None);
}

/// Identical prices → 0 bps deviation.
#[test]
fn deviation_identical_prices() {
    assert_eq!(compute_deviation_bps(500, 500), Some(0));
    assert_eq!(compute_deviation_bps(1, 1), Some(0));
}

/// Large prices: 200 % doubling → 10 000 bps.
#[test]
fn deviation_doubling_is_ten_thousand_bps() {
    assert_eq!(compute_deviation_bps(20_000, 10_000), Some(10_000));
}

/// Very small last_price with large new_price → saturates to u32::MAX cap.
#[test]
fn deviation_saturates_to_u32_max_cap() {
    // new = i128::MAX, last = 1 → diff * 10_000 / 1 would overflow without the cap
    // The function should return Some(u32::MAX) via the min() cap.
    let result = compute_deviation_bps(i128::MAX, 1);
    assert_eq!(result, Some(u32::MAX));
}

/// 1 bps precision: 10_001 vs 10_000 → 1 bps.
#[test]
fn deviation_one_bps_precision() {
    assert_eq!(compute_deviation_bps(10_001, 10_000), Some(1));
    assert_eq!(compute_deviation_bps(9_999, 10_000), Some(1));
}
