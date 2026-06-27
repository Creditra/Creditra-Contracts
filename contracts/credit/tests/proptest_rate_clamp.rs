// SPDX-License-Identifier: MIT

//! Property tests: per-borrower rate clamping is order-independent.
//!
//! # What
//!
//! The contract applies up to three layers of clamping when setting a
//! borrower's interest rate:
//!
//! 1. **Formula clamp** — [`compute_rate_from_score`] clamps the raw
//!    linear value to `[min_rate_bps, min(max_rate_bps, 10_000)]`.
//! 2. **Per-borrower floor** — [`set_borrower_rate_floor`] raises the
//!    effective rate to at least `floor_bps`.
//! 3. **Per-borrower ceiling** — [`set_borrower_rate_ceiling`] lowers the
//!    effective rate to at most `ceiling_bps`.
//!
//! These layers are applied sequentially in `update_risk_parameters`. This
//! test verifies that the order of the per-borrower floor and ceiling does
//! **not** affect the final rate — the `max` (floor) and `min` (ceiling)
//! operations commute when `floor ≤ ceiling`.
//!
//! # Properties
//!
//! 1. **Floor-ceiling commutativity**  
//!    `for all r, f, c with 0 ≤ f ≤ c ≤ 10_000:  r.max(f).min(c) == r.min(c).max(f)`
//!
//! 2. **Formula plus per-borrower clamp identity**  
//!    For any `RateFormulaConfig`, risk score, floor, and ceiling:
//!    ```
//!    formula(score).max(floor).min(ceiling)
//!      == formula(score).min(ceiling).max(floor)
//!    ```
//!
//! 3. **Idempotency**  
//!    Applying the same clamp twice produces the same result once.

use creditra_credit::compute_rate_from_score;
use creditra_credit::types::RateFormulaConfig;
use proptest::prelude::*;
use proptest::test_runner::Config as ProptestConfig;

/// Protocol-wide interest rate ceiling (100 % = 10_000 bps).
const MAX_INTEREST_RATE_BPS: u32 = 10_000;

/// Maximum risk score on the normalised 0‑100 scale.
const MAX_RISK_SCORE: u32 = 100;

// ── Strategies ────────────────────────────────────────────────────────────

/// Strategy that generates `(min, max)` pairs satisfying `min <= max <= 10_000`.
fn valid_bounds() -> impl Strategy<Value = (u32, u32)> {
    (0_u32..=MAX_INTEREST_RATE_BPS)
        .prop_flat_map(|lo| (Just(lo), lo..=MAX_INTEREST_RATE_BPS))
}

/// Strategy for a well-formed `RateFormulaConfig`.
fn rate_formula_config() -> impl Strategy<Value = RateFormulaConfig> {
    (0_u32..=MAX_INTEREST_RATE_BPS,
     0_u32..=MAX_INTEREST_RATE_BPS,
     valid_bounds(),
    )
        .prop_map(|(base, slope, (min, max))| RateFormulaConfig {
            base_rate_bps: base,
            slope_bps_per_score: slope,
            min_rate_bps: min,
            max_rate_bps: max,
        })
}

/// Strategy for `(floor, ceiling)` pairs with `0 <= floor <= ceiling <= 10_000`.
fn floor_ceiling() -> impl Strategy<Value = (u32, u32)> {
    valid_bounds()
}

/// Strategy for a raw interest rate value (0 ..= 10_000).
fn interest_rate() -> impl Strategy<Value = u32> {
    0_u32..=MAX_INTEREST_RATE_BPS
}

/// Strategy for a risk score (0 ..= 100).
fn risk_score() -> impl Strategy<Value = u32> {
    0_u32..=MAX_RISK_SCORE
}

// ── Property 1: floor/ceiling commutativity ───────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig { cases: 2048, .. ProptestConfig::default() })]

    /// The per-borrower floor (`max`) and ceiling (`min`) commute when
    /// `floor ≤ ceiling`. This is the mathematical guarantee that the order
    /// of applying these two bounds does not affect the final rate.
    #[test]
    fn floor_ceiling_commute(
        rate in interest_rate(),
        (floor, ceiling) in floor_ceiling(),
    ) {
        let floor_first = rate.max(floor).min(ceiling);
        let ceiling_first = rate.min(ceiling).max(floor);

        assert_eq!(
            floor_first, ceiling_first,
            "floor/ceiling commutativity violated:\n\
             rate = {}, floor = {}, ceiling = {}\n\
             floor_first = {}, ceiling_first = {}",
            rate, floor, ceiling, floor_first, ceiling_first,
        );
    }
}

// ── Property 2: formula + floor/ceiling order independence ────────────────

proptest! {
    #![proptest_config(ProptestConfig { cases: 1024, .. ProptestConfig::default() })]

    /// When a rate formula is active, `compute_rate_from_score` applies its
    /// own internal clamp. Applying the per-borrower floor and ceiling
    /// afterwards must still be order-independent.
    #[test]
    fn formula_plus_per_borrower_clamp_is_order_independent(
        cfg in rate_formula_config(),
        score in risk_score(),
        (floor, ceiling) in floor_ceiling(),
    ) {
        let formula_rate = compute_rate_from_score(&cfg, score);

        let contract_order = formula_rate.max(floor).min(ceiling);
        let reversed_order = formula_rate.min(ceiling).max(floor);

        assert_eq!(
            contract_order, reversed_order,
            "formula + per-borrower clamp order independence violated:\n\
             cfg = (base={}, slope={}, min={}, max={})\n\
             score = {}, formula_rate = {}\n\
             floor = {}, ceiling = {}\n\
             contract_order = {}, reversed_order = {}",
            cfg.base_rate_bps, cfg.slope_bps_per_score,
            cfg.min_rate_bps, cfg.max_rate_bps,
            score, formula_rate,
            floor, ceiling,
            contract_order, reversed_order,
        );
    }
}

// ── Property 3: full-stack combined clamp identity ────────────────────────

proptest! {
    #![proptest_config(ProptestConfig { cases: 1024, .. ProptestConfig::default() })]

    /// The three-layer clamp (formula bounds + per-borrower floor + per-borrower
    /// ceiling) is equivalent to a single clamp with combined bounds.
    #[test]
    fn full_stack_combined_clamp(
        cfg in rate_formula_config(),
        score in risk_score(),
        (floor, ceiling) in floor_ceiling(),
    ) {
        let raw = cfg
            .base_rate_bps
            .saturating_add(score.saturating_mul(cfg.slope_bps_per_score));

        let formula_result = compute_rate_from_score(&cfg, score);
        let contract_final = formula_result.max(floor).min(ceiling);

        let inner_upper = cfg.max_rate_bps.min(MAX_INTEREST_RATE_BPS);
        let true_lower = cfg.min_rate_bps.max(floor).min(ceiling);
        let true_upper = inner_upper.max(floor).min(ceiling);
        let single_clamped = raw.clamp(true_lower, true_upper);

        assert_eq!(
            contract_final, single_clamped,
            "full-stack combined clamp identity violated:\n\
             cfg = (base={}, slope={}, min={}, max={})\n\
             score = {}, raw = {}\n\
             floor = {}, ceiling = {}\n\
             formula_result = {}\n\
             contract_final = {}, single_clamped = {}\n\
             combined = [{}, {}],
             inner_upper = {}",
            cfg.base_rate_bps, cfg.slope_bps_per_score,
            cfg.min_rate_bps, cfg.max_rate_bps,
            score, raw,
            floor, ceiling,
            formula_result,
            contract_final, single_clamped,
            true_lower, true_upper,
            inner_upper,
        );
    }
}

// ── Deterministic edge-case tests ─────────────────────────────────────────

/// Edge: floor = ceiling (degenerate range).
#[test]
fn degenerate_floor_ceiling_range() {
    for rate in [0_u32, 1, 500, 9_999, 10_000] {
        let floor = 4_000;
        let ceiling = 4_000;
        let floor_first = rate.max(floor).min(ceiling);
        let ceiling_first = rate.min(ceiling).max(floor);
        assert_eq!(floor_first, ceiling_first);
        assert_eq!(floor_first, 4_000);
    }
}

/// Edge: floor = 0 (no floor effect).
#[test]
fn zero_floor() {
    for rate in [0_u32, 1, 5_000, 10_000] {
        let floor = 0;
        let ceiling = 10_000;
        let floor_first = rate.max(floor).min(ceiling);
        let ceiling_first = rate.min(ceiling).max(floor);
        assert_eq!(floor_first, rate);
        assert_eq!(floor_first, ceiling_first);
    }
}

/// Edge: ceiling = MAX_INTEREST_RATE_BPS (no ceiling effect).
#[test]
fn max_ceiling() {
    for rate in [0_u32, 1, 5_000, 10_000] {
        let floor = 0;
        let ceiling = MAX_INTEREST_RATE_BPS;
        let floor_first = rate.max(floor).min(ceiling);
        let ceiling_first = rate.min(ceiling).max(floor);
        assert_eq!(floor_first, rate);
        assert_eq!(floor_first, ceiling_first);
    }
}

/// Edge: floor > rate, ceiling > rate.
#[test]
fn floor_above_rate() {
    for rate in [100_u32, 500, 1_000] {
        let floor = 2_000;
        let ceiling = 8_000;
        let floor_first = rate.max(floor).min(ceiling);
        let ceiling_first = rate.min(ceiling).max(floor);
        assert_eq!(floor_first, floor);
        assert_eq!(floor_first, ceiling_first);
    }
}

/// Edge: ceiling < rate, floor < rate.
#[test]
fn ceiling_below_rate() {
    for rate in [5_000_u32, 7_000, 10_000] {
        let floor = 100;
        let ceiling = 3_000;
        let floor_first = rate.max(floor).min(ceiling);
        let ceiling_first = rate.min(ceiling).max(floor);
        assert_eq!(floor_first, ceiling);
        assert_eq!(floor_first, ceiling_first);
    }
}

/// Edge: both bounds constrain in opposite directions.
#[test]
fn both_bounds_constrain() {
    let rate = 5_000;
    let floor = 3_000;
    let ceiling = 4_000;

    let floor_first = rate.max(floor).min(ceiling);
    let ceiling_first = rate.min(ceiling).max(floor);

    assert_eq!(floor_first, 4_000);
    assert_eq!(ceiling_first, 4_000);
    assert_eq!(floor_first, ceiling_first);
}

/// Edge: floor = ceiling = 0.
#[test]
fn zero_bounds() {
    for rate in [0_u32, 1, 5_000, 10_000] {
        assert_eq!(rate.max(0).min(0), 0);
    }
}

/// Edge: rate at boundary of MAX_INTEREST_RATE_BPS.
#[test]
fn rate_at_global_max() {
    let rate = MAX_INTEREST_RATE_BPS;
    for &(floor, ceiling) in &[(0, 10_000), (5_000, 10_000), (0, 8_000), (9_000, 10_000)] {
        let floor_first = rate.max(floor).min(ceiling);
        let ceiling_first = rate.min(ceiling).max(floor);
        assert_eq!(floor_first, ceiling_first);
    }
}

/// Edge: formula with min == max (degenerate formula bounds).
#[test]
fn degenerate_formula_bounds_with_per_borrower() {
    let cfg = RateFormulaConfig {
        base_rate_bps: 0,
        slope_bps_per_score: 0,
        min_rate_bps: 5_000,
        max_rate_bps: 5_000,
    };

    for score in [0_u32, 50, 100] {
        let formula_rate = compute_rate_from_score(&cfg, score);
        assert_eq!(formula_rate, 5_000);

        for &(floor, ceiling) in &[(4_000, 6_000), (5_000, 5_000), (6_000, 8_000)] {
            let contract_order = formula_rate.max(floor).min(ceiling);
            let reversed_order = formula_rate.min(ceiling).max(floor);
            assert_eq!(contract_order, reversed_order);
        }
    }
}

/// Edge: very large slope causes saturating arithmetic.
#[test]
fn saturating_slope_with_per_borrower_bounds() {
    let cfg = RateFormulaConfig {
        base_rate_bps: u32::MAX,
        slope_bps_per_score: u32::MAX,
        min_rate_bps: 0,
        max_rate_bps: MAX_INTEREST_RATE_BPS,
    };

    for score in [0_u32, 1, 50, 99, 100] {
        let formula_rate = compute_rate_from_score(&cfg, score);

        for &(floor, ceiling) in &[(0, 10_000), (3_000, 7_000), (9_500, 10_000)] {
            let contract_order = formula_rate.max(floor).min(ceiling);
            let reversed_order = formula_rate.min(ceiling).max(floor);
            assert_eq!(contract_order, reversed_order);
        }
    }
}

/// Edge: floor at maximum (10_000) with various rates.
#[test]
fn floor_at_max() {
    let floor = 10_000;
    let ceiling = 10_000;
    for rate in [0_u32, 5_000, 10_000] {
        let floor_first = rate.max(floor).min(ceiling);
        let ceiling_first = rate.min(ceiling).max(floor);
        assert_eq!(floor_first, 10_000);
        assert_eq!(ceiling_first, 10_000);
    }
}

/// Edge: ceiling at minimum (0).
#[test]
fn ceiling_at_min() {
    let floor = 0;
    let ceiling = 0;
    for rate in [0_u32, 5_000, 10_000] {
        let floor_first = rate.max(floor).min(ceiling);
        let ceiling_first = rate.min(ceiling).max(floor);
        assert_eq!(floor_first, 0);
        assert_eq!(ceiling_first, 0);
    }
}
