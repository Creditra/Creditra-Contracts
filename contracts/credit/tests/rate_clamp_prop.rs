// SPDX-License-Identifier: MIT

//! Property test: clamp ordering on the piecewise-linear rate formula.
//!
//! # What
//!
//! Verifies that two sequential `clamp` operations compose correctly into a
//! single clamp with combined bounds.  This is the key mathematical invariant
//! that guarantees the per-borrower rate floor and the protocol-wide 10 000 bps
//! ceiling are safe regardless of the order in which they are applied.
//!
//! # Property
//!
//! For any valid `RateFormulaConfig` (min ≤ max), any risk score `s ∈ [0, 100]`,
//! and any external bounds `0 ≤ floor ≤ ceiling ≤ 10_000`:
//!
//! ```text
//! clamp( clamp(raw, min, min(max, 10_000)), floor, ceiling )
//!   == clamp( raw, max(floor, min), min(ceiling, max, 10_000) )
//! ```
//!
//! where `raw = base + s · slope` (saturating arithmetic).
//!
//! # Why
//!
//! The contract applies bounds in multiple layers:
//!   1. `compute_rate_from_score` clamps to `[min_rate_bps, min(max_rate_bps, 10_000)]`.
//!   2. `update_risk_parameters` may further floor the result via a per-borrower
//!      rate floor.
//!
//! This test ensures that the order of these bounds does not affect the final
//! result — the "clamp ordering" property of min/max.
//!
//! # References
//!
//! - [`crate::risk::compute_rate_from_score`]
//! - [`crate::types::RateFormulaConfig`]
//! - Issue #486

use creditra_credit::compute_rate_from_score;
use creditra_credit::types::RateFormulaConfig;
use proptest::prelude::*;
use proptest::test_runner::Config as ProptestConfig;

/// Protocol-wide interest rate ceiling (100 % = 10_000 bps).
const MAX_INTEREST_RATE_BPS: u32 = 10_000;

/// Maximum risk score on the normalised 0‑100 scale.
const MAX_RISK_SCORE: u32 = 100;

// ── Strategy: well-formed RateFormulaConfig ───────────────────────────────

/// Strategy that generates `(min, max)` pairs satisfying `min <= max <= 10_000`.
fn min_max() -> impl Strategy<Value = (u32, u32)> {
    (0_u32..=MAX_INTEREST_RATE_BPS).prop_flat_map(|min| (Just(min), min..=MAX_INTEREST_RATE_BPS))
}

/// Strategy for a complete `RateFormulaConfig` where `min <= max <= 10_000`.
fn rate_formula_config() -> impl Strategy<Value = RateFormulaConfig> {
    (
        0_u32..=MAX_INTEREST_RATE_BPS,
        0_u32..=MAX_INTEREST_RATE_BPS,
        min_max(),
    )
        .prop_map(|(base, slope, (min, max))| RateFormulaConfig {
            base_rate_bps: base,
            slope_bps_per_score: slope,
            min_rate_bps: min,
            max_rate_bps: max,
        })
}

/// Strategy for `(floor, ceiling)` pairs satisfying `0 <= floor <= ceiling <= 10_000`.
fn floor_ceiling() -> impl Strategy<Value = (u32, u32)> {
    (0_u32..=MAX_INTEREST_RATE_BPS)
        .prop_flat_map(|floor| (Just(floor), floor..=MAX_INTEREST_RATE_BPS))
}

// ── Property test ─────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig { cases: 1024, .. ProptestConfig::default() })]
    /// Tests the clamp-ordering identity on 1024 random configurations.
    ///
    /// # Shrinking
    /// On failure the test shrinks to a minimal 4‑tuple
    /// `(base, slope, min_max, floor_ceiling)` plus the provoking risk score.
    #[test]
    fn clamp_ordering_identity(
        (base, slope, (min_rate, max_rate), (floor, ceiling), score) in (
            rate_formula_config(),
            floor_ceiling(),
            0_u32..=MAX_RISK_SCORE,
        ).prop_map(|(cfg, (f, c), s)| {
            (cfg.base_rate_bps,
             cfg.slope_bps_per_score,
             (cfg.min_rate_bps, cfg.max_rate_bps),
             (f, c),
             s)
        })
    ) {
        let cfg = RateFormulaConfig {
            base_rate_bps: base,
            slope_bps_per_score: slope,
            min_rate_bps: min_rate,
            max_rate_bps: max_rate,
        };

        // ── Step 1: raw linear value (saturating arithmetic matches contract) ──
        let raw = base.saturating_add(score.saturating_mul(slope));

        // ── Step 2: formula result via the contract's clamp ────────────────────
        let formula_result = compute_rate_from_score(&cfg, score);

        // ── Step 3: external double-clamp (formula clamp + floor/ceiling) ───────
        let double_clamped = formula_result.max(floor).min(ceiling);

        // ── Step 4: single combined clamp using the universal nested-clamp
        //    identity: clamp(inner_min, outer_min, outer_max) and
        //              clamp(inner_max, outer_min, outer_max)
        //    This holds even when the bounds are inverted/disjoint.
        let inner_upper = max_rate.min(MAX_INTEREST_RATE_BPS);
        let true_lower = min_rate.max(floor).min(ceiling);
        let true_upper = inner_upper.max(floor).min(ceiling);
        let single_clamped = raw.clamp(true_lower, true_upper);

        // ── Assertion: double-clamp == single-clamp ────────────────────────────
        assert_eq!(
            double_clamped, single_clamped,
            "clamp-ordering identity violated:\n\
             cfg = (base={}, slope={}, min={}, max={})\n\
             score = {}, raw = {}\n\
             floor = {}, ceiling = {}\n\
             formula_result = {}\n\
             double_clamped = {}, single_clamped = {}\n\
             combined bound = [{}, {}]",
            base, slope, min_rate, max_rate,
            score, raw,
            floor, ceiling,
            formula_result,
            double_clamped, single_clamped,
            true_lower, true_upper,
        );
    }
}

// ── Additional edge-case tests ────────────────────────────────────────────

/// Verifies that the saturating-mul boundary (u32::MAX) does not break the
/// clamp-ordering identity.  This is a deterministic companion to the random
/// proptest above.
#[test]
fn saturating_mul_boundary_preserves_clamp_ordering() {
    let cfg = RateFormulaConfig {
        base_rate_bps: u32::MAX,
        slope_bps_per_score: u32::MAX,
        min_rate_bps: 0,
        max_rate_bps: MAX_INTEREST_RATE_BPS,
    };

    // Test across the full risk-score range
    for score in [0_u32, 1, 50, 99, 100] {
        let raw = u32::MAX.saturating_add(score.saturating_mul(u32::MAX));
        let formula_result = compute_rate_from_score(&cfg, score);

        // With floor=0, ceiling=10_000: the formula already clamps to [0, 10_000].
        let double_clamped = formula_result.max(0).min(MAX_INTEREST_RATE_BPS);
        let single_clamped = raw.max(0).min(MAX_INTEREST_RATE_BPS).max(0);

        assert_eq!(
            double_clamped, single_clamped,
            "saturating-mul boundary failed at score={}: formula={}, expected={}",
            score, formula_result, MAX_INTEREST_RATE_BPS,
        );
    }
}

/// Verifies the clamp-ordering identity on the edge of the global cap.
#[test]
fn global_cap_edge_preserves_clamp_ordering() {
    // max_rate_bps = 10_000, floor = 9_000, ceiling = 10_000
    let cfg = RateFormulaConfig {
        base_rate_bps: 5_000,
        slope_bps_per_score: 200,
        min_rate_bps: 100,
        max_rate_bps: MAX_INTEREST_RATE_BPS,
    };

    for score in [0_u32, 25, 50, 75, 100] {
        let raw = 5_000u32.saturating_add(score.saturating_mul(200));
        let formula_result = compute_rate_from_score(&cfg, score);

        let floor = 9_000;
        let ceiling = 10_000;
        let double_clamped = formula_result.max(floor).min(ceiling);

        let combined_lower = floor.max(cfg.min_rate_bps);
        let combined_upper = ceiling.min(cfg.max_rate_bps).min(MAX_INTEREST_RATE_BPS);
        let true_lower = cfg.min_rate_bps.max(floor).min(ceiling);
        let true_upper = cfg
            .max_rate_bps
            .min(MAX_INTEREST_RATE_BPS)
            .max(floor)
            .min(ceiling);
        let single_clamped = raw.clamp(true_lower, true_upper);

        assert_eq!(
            double_clamped, single_clamped,
            "global cap edge failed at score={}: double={}, single={}",
            score, double_clamped, single_clamped
        );
    }
}

/// Verifies that zero bounds (floor=ceiling=0) don't break the identity.
#[test]
fn zero_bounds_preserve_clamp_ordering() {
    let cfg = RateFormulaConfig {
        base_rate_bps: 500,
        slope_bps_per_score: 50,
        min_rate_bps: 200,
        max_rate_bps: 5_000,
    };

    for score in [0_u32, 50, 100] {
        let raw = 500u32.saturating_add(score.saturating_mul(50));
        let formula_result = compute_rate_from_score(&cfg, score);

        // floor = 0, ceiling = 0
        let double_clamped = formula_result.max(0).min(0);
        let true_lower = cfg.min_rate_bps.max(0).min(0);
        let true_upper = cfg.max_rate_bps.min(MAX_INTEREST_RATE_BPS).max(0).min(0);
        let single_clamped = raw.clamp(true_lower, true_upper);

        assert_eq!(
            double_clamped, single_clamped,
            "zero bounds failed at score={}",
            score
        );
    }
}

/// Verifies that floor == ceiling (degenerate range) preserves the identity.
#[test]
fn degenerate_range_preserves_clamp_ordering() {
    let cfg = RateFormulaConfig {
        base_rate_bps: 1_000,
        slope_bps_per_score: 100,
        min_rate_bps: 500,
        max_rate_bps: 8_000,
    };

    for score in [0_u32, 50, 100] {
        let raw = 1_000u32.saturating_add(score.saturating_mul(100));
        let formula_result = compute_rate_from_score(&cfg, score);

        // Degenerate: floor == ceiling == 3_000
        let double_clamped = formula_result.max(3_000).min(3_000);
        let true_lower = cfg.min_rate_bps.max(3_000).min(3_000);
        let true_upper = cfg
            .max_rate_bps
            .min(MAX_INTEREST_RATE_BPS)
            .max(3_000)
            .min(3_000);
        let single_clamped = raw.clamp(true_lower, true_upper);

        assert_eq!(
            double_clamped, single_clamped,
            "degenerate range failed at score={}",
            score
        );
    }
}
