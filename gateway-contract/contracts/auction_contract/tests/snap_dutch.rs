use gateway_auction::{compute_dutch_price, DutchAuctionDecay};
use insta::assert_snapshot;
use proptest::prelude::*;
use std::fmt::Write;

#[test]
fn test_snapshot_fuzz_dutch_price_boundaries() {
    let mut out = String::new();

    let starts = vec![100, 1_000_000, i128::MAX / 2];
    let floor_ratios = vec![0, 50, 100]; // 0%, 50%, 100% of start
    let durations = vec![0, 1, 100, u64::MAX / 2];
    let elapsed_ratios = vec![0, 50, 100, 150]; // 0%, 50%, 100%, 150% of duration

    for &start in &starts {
        for &f_ratio in &floor_ratios {
            let floor = (start / 100) * f_ratio;
            for &duration in &durations {
                for &e_ratio in &elapsed_ratios {
                    let elapsed = if duration == 0 {
                        e_ratio as u64
                    } else {
                        (duration as u128 * e_ratio as u128 / 100) as u64
                    };

                    let decays = vec![
                        (DutchAuctionDecay::None, None),
                        (DutchAuctionDecay::Linear, None),
                        (DutchAuctionDecay::Stepped, Some(5)),
                        (DutchAuctionDecay::Exponential, None),
                    ];

                    for (decay, step_count) in decays {
                        let price = compute_dutch_price(
                            start,
                            floor,
                            elapsed,
                            duration,
                            &decay,
                            step_count,
                        );

                        writeln!(
                            &mut out,
                            "s={:<12} f={:<12} e={:<12} d={:<12} dec={:?} steps={:?} => p={}",
                            start, floor, elapsed, duration, decay, step_count, price
                        )
                        .unwrap();
                    }
                }
            }
        }
    }

    assert_snapshot!("dutch_price_boundaries", out);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]
    #[test]
    fn fuzz_compute_dutch_price_no_panic_and_bounds(
        start in 0..1_000_000_000_000i128,
        floor_drop in 0..1_000_000_000_000i128,
        elapsed in 0..1_000_000u64,
        duration in 0..1_000_000u64,
        step_count in 1..100u32,
    ) {
        let floor = start.saturating_sub(floor_drop);

        let p_linear = compute_dutch_price(start, floor, elapsed, duration, &DutchAuctionDecay::Linear, None);
        assert!(p_linear <= start && p_linear >= floor, "linear bounds: {} not in [{}, {}]", p_linear, floor, start);

        let p_stepped = compute_dutch_price(start, floor, elapsed, duration, &DutchAuctionDecay::Stepped, Some(step_count));
        assert!(p_stepped <= start && p_stepped >= floor, "stepped bounds: {} not in [{}, {}]", p_stepped, floor, start);

        let p_exp = compute_dutch_price(start, floor, elapsed, duration, &DutchAuctionDecay::Exponential, None);
        assert!(p_exp <= start && p_exp >= floor, "exp bounds: {} not in [{}, {}]", p_exp, floor, start);
    }
}
