// SPDX-License-Identifier: MIT

//! Property-based test for rate formula clamping invariants.
//!
//! This test validates that `compute_rate_from_score` always produces
//! output within the expected bounds, even with extreme or malformed
//! configuration values.
//!
//! # Test Strategy
//!
//! Uses `proptest` to generate 1024 random test cases covering:
//! - Random `base_rate_bps`, `slope_bps_per_score`, `min_rate_bps`, `max_rate_bps`
//! - Random `risk_score` (0–100)
//!
//! # Invariants Tested
//!
//! For all valid configs (where `min_rate_bps <= max_rate_bps <= 10_000`):
//! ```text
//! output ∈ [min_rate_bps, min(max_rate_bps, 10_000)]
//! ```
//!
//! # Shrinking
//!
//! On failure, proptest automatically shrinks the input to a minimal
//! counterexample (typically a 4-tuple of the most relevant parameters).

use creditra_credit::risk::{compute_rate_from_score, MAX_INTEREST_RATE_BPS, MAX_RISK_SCORE};
use creditra_credit::types::RateFormulaConfig;
use proptest::prelude::*;

/// Generate a valid `RateFormulaConfig` where `min_rate_bps <= max_rate_bps <= 10_000`.
///
/// This strategy ensures the config respects the documented invariants
/// before we even test the formula computation.
fn valid_rate_formula_config() -> impl Strategy<Value = RateFormulaConfig> {
    (
        0u32..=MAX_INTEREST_RATE_BPS,
        0u32..=u32::MAX,
        0u32..=MAX_INTEREST_RATE_BPS,
        0u32..=MAX_INTEREST_RATE_BPS,
    )
        .prop_map(|(base, slope, min, max)| {
            // Ensure min <= max by swapping if necessary
            let (min_val, max_val) = if min <= max { (min, max) } else { (max, min) };
            RateFormulaConfig {
                base_rate_bps: base,
                slope_bps_per_score: slope,
                min_rate_bps: min_val,
                max_rate_bps: max_val,
            }
        })
}

/// Generate a risk score in the valid range [0, 100].
fn risk_score() -> impl Strategy<Value = u32> {
    0u32..=MAX_RISK_SCORE
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 1024, // Run 1024 test cases as specified
        failure_persistence: None, // Don't persist failures between runs
        ..ProptestConfig::default()
    })]

    /// Property: rate formula output respects all clamping bounds.
    ///
    /// Given a valid config and risk score, the computed rate must lie within:
    /// - Lower bound: `min_rate_bps`
    /// - Upper bound: `min(max_rate_bps, 10_000)`
    ///
    /// This test covers:
    /// - Normal configurations
    /// - Edge cases (min == max, extreme slopes)
    /// - Saturating multiplication boundaries (when slope * score overflows)
    #[test]
    fn rate_formula_clamp_invariant(
        cfg in valid_rate_formula_config(),
        score in risk_score()
    ) {
        // Compute the raw formula output
        let raw_output = compute_rate_from_score(&cfg, score);

        // The expected bounds from the config
        let lower_bound = cfg.min_rate_bps;
        let upper_bound = cfg.max_rate_bps.min(MAX_INTEREST_RATE_BPS);

        // Assert the output is within bounds
        prop_assert!(
            raw_output >= lower_bound,
            "Rate {} below lower bound {} (config: base={}, slope={}, min={}, max={}, score={})",
            raw_output, lower_bound, cfg.base_rate_bps, cfg.slope_bps_per_score,
            cfg.min_rate_bps, cfg.max_rate_bps, score
        );

        prop_assert!(
            raw_output <= upper_bound,
            "Rate {} exceeds upper bound {} (config: base={}, slope={}, min={}, max={}, score={})",
            raw_output, upper_bound, cfg.base_rate_bps, cfg.slope_bps_per_score,
            cfg.min_rate_bps, cfg.max_rate_bps, score
        );
    }

    /// Property: saturating multiplication boundary cases.
    ///
    /// This test specifically targets the saturating multiplication
    /// `risk_score * slope_bps_per_score` to ensure it never overflows
    /// and produces values that are correctly clamped.
    #[test]
    fn saturating_mul_boundary(
        slope in 0u32..=u32::MAX,
        score in risk_score()
    ) {
        // Use a config with base=0 to isolate the multiplication behavior
        let cfg = RateFormulaConfig {
            base_rate_bps: 0,
            slope_bps_per_score: slope,
            min_rate_bps: 0,
            max_rate_bps: MAX_INTEREST_RATE_BPS,
        };

        let output = compute_rate_from_score(&cfg, score);

        // The output must be <= 10_000 even if slope * score would overflow
        prop_assert!(
            output <= MAX_INTEREST_RATE_BPS,
            "Saturating mul produced rate {} exceeding MAX_INTEREST_RATE_BPS {} (slope={}, score={})",
            output, MAX_INTEREST_RATE_BPS, slope, score
        );
    }

    /// Property: min == max produces constant output.
    ///
    /// When min_rate_bps == max_rate_bps, the formula should always
    /// return that value regardless of score (after clamping).
    #[test]
    fn min_equals_max_constant_output(
        base in 0u32..=MAX_INTEREST_RATE_BPS,
        slope in 0u32..=MAX_INTEREST_RATE_BPS,
        fixed_rate in 0u32..=MAX_INTEREST_RATE_BPS,
        score in risk_score()
    ) {
        let cfg = RateFormulaConfig {
            base_rate_bps: base,
            slope_bps_per_score: slope,
            min_rate_bps: fixed_rate,
            max_rate_bps: fixed_rate,
        };

        let output = compute_rate_from_score(&cfg, score);

        // When min == max, output should be exactly that value
        prop_assert_eq!(
            output,
            fixed_rate,
            "Min==max should produce constant output {} but got {} (base={}, slope={}, score={})",
            fixed_rate, output, base, slope, score
        );
    }

    /// Property: zero slope produces base rate (clamped).
    ///
    /// When slope_bps_per_score == 0, the formula should return
    /// base_rate_bps clamped to [min_rate_bps, max_rate_bps].
    #[test]
    fn zero_slope_base_rate(
        base in 0u32..=MAX_INTEREST_RATE_BPS,
        min in 0u32..=MAX_INTEREST_RATE_BPS,
        max in 0u32..=MAX_INTEREST_RATE_BPS
    ) {
        let (min_val, max_val) = if min <= max { (min, max) } else { (max, min) };
        let cfg = RateFormulaConfig {
            base_rate_bps: base,
            slope_bps_per_score: 0,
            min_rate_bps: min_val,
            max_rate_bps: max_val,
        };

        let output = compute_rate_from_score(&cfg, 0);
        let expected = base.clamp(min_val, max_val.min(MAX_INTEREST_RATE_BPS));

        prop_assert_eq!(
            output,
            expected,
            "Zero slope should return clamped base {} but got {} (base={}, min={}, max={})",
            expected, output, base, min_val, max_val
        );
    }
}
