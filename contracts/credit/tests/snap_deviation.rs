// SPDX-License-Identifier: MIT

//! Snapshot-fuzz coverage for `math_utils::compute_deviation_bps`.
//!
//! The suite combines realistic price pairs with a property test over positive
//! prices to ensure the helper remains deterministic and overflow-safe for the
//! oracle circuit-breaker path.

use creditra_credit::math_utils::compute_deviation_bps;
use proptest::prelude::*;

#[derive(Debug, Clone)]
struct DeviationTestCase {
    new_price: i128,
    last_price: i128,
    deviation_bps: Option<u32>,
}

impl DeviationTestCase {
    fn from_prices(new_price: i128, last_price: i128) -> Self {
        Self {
            new_price,
            last_price,
            deviation_bps: compute_deviation_bps(new_price, last_price),
        }
    }
}

fn realistic_price_pairs() -> Vec<DeviationTestCase> {
    vec![
        DeviationTestCase::from_prices(1_000, 1_000),
        DeviationTestCase::from_prices(1_050, 1_000),
        DeviationTestCase::from_prices(950, 1_000),
        DeviationTestCase::from_prices(10_000, 10_000),
        DeviationTestCase::from_prices(10_500, 10_000),
        DeviationTestCase::from_prices(9_500, 10_000),
        DeviationTestCase::from_prices(10_000, 9_500),
        DeviationTestCase::from_prices(12_500, 10_000),
        DeviationTestCase::from_prices(8_000, 10_000),
        DeviationTestCase::from_prices(1_234_567, 1_200_000),
        DeviationTestCase::from_prices(1_188_000, 1_200_000),
        DeviationTestCase::from_prices(5_000_000, 5_000_000),
        DeviationTestCase::from_prices(5_250_000, 5_000_000),
        DeviationTestCase::from_prices(4_750_000, 5_000_000),
        DeviationTestCase::from_prices(20_000_000, 20_000_000),
        DeviationTestCase::from_prices(20_500_000, 20_000_000),
        DeviationTestCase::from_prices(19_500_000, 20_000_000),
        DeviationTestCase::from_prices(100_000_000, 100_000_000),
        DeviationTestCase::from_prices(101_000_000, 100_000_000),
        DeviationTestCase::from_prices(99_000_000, 100_000_000),
        DeviationTestCase::from_prices(1_000_000_000, 1_000_000_000),
        DeviationTestCase::from_prices(1_010_000_000, 1_000_000_000),
        DeviationTestCase::from_prices(990_000_000, 1_000_000_000),
        DeviationTestCase::from_prices(100, 0),
        DeviationTestCase::from_prices(0, 100),
        DeviationTestCase::from_prices(-100, 100),
        DeviationTestCase::from_prices(100, -100),
    ]
}

#[test]
fn compute_deviation_bps_realistic_pairs_snapshot() {
    let cases = realistic_price_pairs();
    insta::assert_debug_snapshot!("compute_deviation_bps_realistic_pairs", cases);
}

#[test]
fn compute_deviation_bps_returns_none_for_non_positive_last_price() {
    assert_eq!(compute_deviation_bps(100, 0), None);
    assert_eq!(compute_deviation_bps(100, -1), None);
}

proptest! {
    #[test]
    fn compute_deviation_bps_matches_formula_for_positive_prices(
        new_price in 1i128..=100_000_000i128,
        last_price in 1i128..=100_000_000i128,
    ) {
        let expected = ((new_price - last_price).unsigned_abs() as u128 * 10_000u128
            / last_price as u128)
        .min(u32::MAX as u128) as u32;

        prop_assert_eq!(compute_deviation_bps(new_price, last_price), Some(expected));
    }
}
