use creditra_credit::math_utils::compute_deviation_bps;
use insta::assert_debug_snapshot;
use proptest::prelude::*;
use std::fmt;

#[derive(Debug, Clone)]
struct DeviationCase {
    new_price: i128,
    last_price: i128,
    result: Option<u32>,
}

impl fmt::Display for DeviationCase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "new={}, last={} => {:?}",
            self.new_price, self.last_price, self.result
        )
    }
}

fn deviation_case(new_price: i128, last_price: i128) -> DeviationCase {
    DeviationCase {
        new_price,
        last_price,
        result: compute_deviation_bps(new_price, last_price),
    }
}

#[test]
fn deviation_boundary_snapshots() {
    let cases = vec![
        deviation_case(1_050, 1_000),
        deviation_case(950, 1_000),
        deviation_case(1_000, 1_000),
        deviation_case(10_001, 10_000),
        deviation_case(2_000, 1_000),
        deviation_case(100, 0),
        deviation_case(100, -1),
        deviation_case(0, 1_000),
        deviation_case(-500, 1_000),
        deviation_case(i128::MAX, 1),
        deviation_case(i128::MAX, i128::MAX / 2),
        deviation_case(1, 1),
        deviation_case(i128::MAX, 1_000),
        deviation_case(i128::MIN, 1_000),
        deviation_case(0, 0),
        deviation_case(-1, -1),
        deviation_case(i128::MAX, i128::MAX),
        deviation_case(i128::MIN, i128::MIN),
        deviation_case(1_000_000, 1_000_001),
        deviation_case(1_000_000, 999_999),
        deviation_case(10_000, 1),
        deviation_case(1, 10_000),
        deviation_case(5_000, 10_000),
        deviation_case(15_000, 10_000),
    ];

    assert_debug_snapshot!("deviation_boundary_cases", cases);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn fuzz_deviation_no_panic(
        new_price in i128::MIN..=i128::MAX,
        last_price in i128::MIN..=i128::MAX,
    ) {
        let _ = compute_deviation_bps(new_price, last_price);
    }

    #[test]
    fn fuzz_deviation_positive_last_returns_some(
        last_price in 1i128..=i128::MAX,
        drop in 0i128..i128::MAX,
    ) {
        let new_price = last_price.saturating_add(drop);
        let result = compute_deviation_bps(new_price, last_price);
        assert!(result.is_some(), "positive last_price must return Some");
    }

    #[test]
    fn fuzz_deviation_zero_last_returns_none(
        new_price in i128::MIN..=i128::MAX,
    ) {
        assert_eq!(compute_deviation_bps(new_price, 0), None);
        assert_eq!(compute_deviation_bps(new_price, -1), None);
    }

    #[test]
    fn fuzz_deviation_bounds(
        new_price in 0i128..1_000_000_000_000i128,
        last_price in 1i128..1_000_000_000_000i128,
    ) {
        let result = compute_deviation_bps(new_price, last_price);
        if let Some(bps) = result {
            assert!(bps <= 10_000 || bps == u32::MAX,
                "deviation {} outside expected range", bps);
        }
    }

    #[test]
    fn fuzz_deviation_symmetric(
        last_price in 1i128..1_000_000_000i128,
        delta in 0i128..1_000_000i128,
    ) {
        let up = compute_deviation_bps(last_price + delta, last_price);
        let down = compute_deviation_bps(last_price - delta, last_price);
        assert_eq!(up, down, "deviation should be symmetric around last_price");
    }
}
