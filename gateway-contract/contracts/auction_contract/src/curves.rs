// contracts/gateway-auction/src/curves.rs
//
//! Standalone price-decay curve primitives for Dutch auctions.
//!
//! # Relationship with `compute_dutch_price`
//!
//! This module provides low-level curve implementations parameterised by
//! explicit `rate` / `step_size` / `factor` values (all `u128`).
//!
//! The higher-level [`super::compute_dutch_price`] function in the crate
//! root serves the same purpose but derives those parameters from
//! `start_price`, `floor_price`, and `duration` (using `i128` arithmetic
//! to match the Soroban ABI). The two APIs are intentionally distinct:
//!
//! * [`DecayCurve`] — suitable for off-chain estimation and standalone
//!   previews where callers already know the per-tick rates.
//! * [`super::compute_dutch_price`] — used inside the auction
//!   entrypoints; fits the `DutchAuctionDecay` contract-type ABI.
//!
//! Both must maintain identical mathematical semantics for the same
//! underlying curve shape. If a bug is found in one, verify that the
//! other is not also affected.

/// Represents the decay curve used in the Dutch auction
#[derive(Clone, Debug)]
pub enum DecayCurve {
    /// Linear decay: price = start_price - rate * time
    Linear { rate: u128 },

    /// Stepped decay: price reduces every interval
    /// price = start_price - (steps * step_size)
    Stepped { step_size: u128, interval: u64 },

    /// Exponential decay using fixed-point factor
    /// price = start_price * factor^time (scaled)
    Exponential { factor: u128, scale: u128 },
}

/// Errors for curve calculations
#[derive(Debug)]
pub enum CurveError {
    Overflow,
    InvalidInput,
}

/// Calculates price based on selected decay curve
///
/// # Arguments
/// - `start_price`: initial auction price
/// - `elapsed`: time since auction start
/// - `curve`: decay model
///
/// # Returns
/// Result<u128, CurveError>
pub fn calculate_price(
    start_price: u128,
    elapsed: u64,
    curve: &DecayCurve,
) -> Result<u128, CurveError> {
    match curve {
        DecayCurve::Linear { rate } => {
            let decay = rate
                .checked_mul(elapsed as u128)
                .ok_or(CurveError::Overflow)?;

            Ok(start_price.saturating_sub(decay))
        }

        DecayCurve::Stepped {
            step_size,
            interval,
        } => {
            if *interval == 0 {
                return Err(CurveError::InvalidInput);
            }

            let steps = elapsed / interval;

            let decay = step_size
                .checked_mul(steps as u128)
                .ok_or(CurveError::Overflow)?;

            Ok(start_price.saturating_sub(decay))
        }

        DecayCurve::Exponential { factor, scale } => {
            if *scale == 0 {
                return Err(CurveError::InvalidInput);
            }

            // fixed-point exponential decay
            let mut price = start_price;

            for _ in 0..elapsed {
                price = price.checked_mul(*factor).ok_or(CurveError::Overflow)? / *scale;
            }

            Ok(price)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // ── Linear decay tests ──

    #[test]
    fn linear_zero_elapsed_returns_start_price() {
        let curve = DecayCurve::Linear { rate: 10 };
        let price = calculate_price(1000, 0, &curve).unwrap();
        assert_eq!(price, 1000);
    }

    #[test]
    fn linear_basic_decay() {
        let curve = DecayCurve::Linear { rate: 10 };
        let price = calculate_price(1000, 5, &curve).unwrap();
        assert_eq!(price, 950);
    }

    #[test]
    fn linear_saturates_at_zero() {
        let curve = DecayCurve::Linear { rate: 100 };
        let price = calculate_price(100, 10, &curve).unwrap();
        assert_eq!(price, 0);
    }

    #[test]
    fn linear_no_underflow_below_zero() {
        let curve = DecayCurve::Linear { rate: 10 };
        let price = calculate_price(50, 10, &curve).unwrap();
        assert_eq!(price, 0);
    }

    #[test]
    fn linear_price_never_increases() {
        let curve = DecayCurve::Linear { rate: 5 };
        let mut prev = calculate_price(1000, 0, &curve).unwrap();
        for t in 1..=100 {
            let curr = calculate_price(1000, t, &curve).unwrap();
            assert!(
                curr <= prev,
                "price increased at t={}: {} > {}",
                t,
                curr,
                prev
            );
            prev = curr;
        }
    }

    #[test]
    fn linear_large_rate() {
        let curve = DecayCurve::Linear {
            rate: u128::MAX / 2,
        };
        let price = calculate_price(u128::MAX, 1, &curve).unwrap();
        assert!(price < u128::MAX);
    }

    // ── Stepped decay tests ──

    #[test]
    fn stepped_zero_interval_returns_error() {
        let curve = DecayCurve::Stepped {
            step_size: 100,
            interval: 0,
        };
        let result = calculate_price(1000, 10, &curve);
        assert!(result.is_err());
        match result {
            Err(CurveError::InvalidInput) => {}
            _ => panic!("expected InvalidInput error"),
        }
    }

    #[test]
    fn stepped_basic_price_drop() {
        let curve = DecayCurve::Stepped {
            step_size: 100,
            interval: 10,
        };
        // At t=0, no steps completed
        let p0 = calculate_price(1000, 0, &curve).unwrap();
        assert_eq!(p0, 1000);
        // At t=9, still in first step
        let p1 = calculate_price(1000, 9, &curve).unwrap();
        assert_eq!(p1, 1000);
        // At t=10, one step completed
        let p2 = calculate_price(1000, 10, &curve).unwrap();
        assert_eq!(p2, 900);
        // At t=20, two steps completed
        let p3 = calculate_price(1000, 20, &curve).unwrap();
        assert_eq!(p3, 800);
    }

    #[test]
    fn stepped_holds_price_within_interval() {
        let curve = DecayCurve::Stepped {
            step_size: 50,
            interval: 30,
        };
        // All times within the same interval should have the same price
        let p_at_0 = calculate_price(500, 0, &curve).unwrap();
        let p_at_15 = calculate_price(500, 15, &curve).unwrap();
        let p_at_29 = calculate_price(500, 29, &curve).unwrap();
        assert_eq!(p_at_0, p_at_15);
        assert_eq!(p_at_0, p_at_29);
        // But at the next interval boundary, price drops
        let p_at_30 = calculate_price(500, 30, &curve).unwrap();
        assert!(p_at_30 < p_at_0);
        assert_eq!(p_at_30, 450);
    }

    #[test]
    fn stepped_saturates_at_zero() {
        let curve = DecayCurve::Stepped {
            step_size: 500,
            interval: 1,
        };
        let price = calculate_price(100, 10, &curve).unwrap();
        assert_eq!(price, 0);
    }

    #[test]
    fn stepped_monotonic_non_increasing() {
        let curve = DecayCurve::Stepped {
            step_size: 7,
            interval: 5,
        };
        let mut prev = calculate_price(1000, 0, &curve).unwrap();
        for t in 1..=100 {
            let curr = calculate_price(1000, t, &curve).unwrap();
            assert!(
                curr <= prev,
                "stepped price increased at t={}: {} > {}",
                t,
                curr,
                prev
            );
            prev = curr;
        }
    }

    #[test]
    fn stepped_large_elapsed() {
        let curve = DecayCurve::Stepped {
            step_size: 1,
            interval: 1,
        };
        let price = calculate_price(10, u64::MAX, &curve).unwrap();
        assert_eq!(price, 0);
    }

    #[test]
    fn stepped_overflow_protection() {
        let curve = DecayCurve::Stepped {
            step_size: u128::MAX,
            interval: 1,
        };
        let result = calculate_price(u128::MAX, 2, &curve);
        assert!(result.is_err());
        match result {
            Err(CurveError::Overflow) => {}
            _ => panic!("expected Overflow error"),
        }
    }

    // ── Exponential decay tests ──

    #[test]
    fn exponential_zero_scale_returns_error() {
        let curve = DecayCurve::Exponential {
            factor: 9900,
            scale: 0,
        };
        let result = calculate_price(1000, 10, &curve);
        assert!(result.is_err());
        match result {
            Err(CurveError::InvalidInput) => {}
            _ => panic!("expected InvalidInput error"),
        }
    }

    #[test]
    fn exponential_zero_elapsed_returns_start_price() {
        let curve = DecayCurve::Exponential {
            factor: 9900,
            scale: 10000,
        };
        let price = calculate_price(1000, 0, &curve).unwrap();
        assert_eq!(price, 1000);
    }

    #[test]
    fn exponential_basic_decay() {
        let curve = DecayCurve::Exponential {
            factor: 9900,
            scale: 10000,
        };
        let p0 = calculate_price(1000, 0, &curve).unwrap();
        let p1 = calculate_price(1000, 1, &curve).unwrap();
        assert_eq!(p1, 990); // 1000 * 9900 / 10000
        assert!(p1 < p0);
    }

    #[test]
    fn exponential_monotonic_non_increasing() {
        let curve = DecayCurve::Exponential {
            factor: 9900,
            scale: 10000,
        };
        let mut prev = calculate_price(10000, 0, &curve).unwrap();
        for t in 1..=50 {
            let curr = calculate_price(10000, t, &curve).unwrap();
            assert!(
                curr <= prev,
                "exponential price increased at t={}: {} > {}",
                t,
                curr,
                prev
            );
            prev = curr;
        }
    }

    #[test]
    fn exponential_approaches_zero() {
        let curve = DecayCurve::Exponential {
            factor: 5000,
            scale: 10000,
        };
        let price = calculate_price(1000, 100, &curve).unwrap();
        assert!(price < 10); // With 50% decay per tick, after 100 ticks it's near zero
    }

    // ── Cross-curve comparative tests ──

    #[test]
    fn all_curves_same_start_at_zero_elapsed() {
        let curves = vec![
            DecayCurve::Linear { rate: 10 },
            DecayCurve::Stepped {
                step_size: 100,
                interval: 10,
            },
            DecayCurve::Exponential {
                factor: 9900,
                scale: 10000,
            },
        ];
        for curve in &curves {
            let price = calculate_price(500, 0, curve).unwrap();
            assert_eq!(price, 500);
        }
    }

    #[test]
    fn all_curves_non_increasing_over_short_window() {
        let curves = vec![
            DecayCurve::Linear { rate: 10 },
            DecayCurve::Stepped {
                step_size: 10,
                interval: 1,
            },
            DecayCurve::Exponential {
                factor: 9900,
                scale: 10000,
            },
        ];
        for curve in &curves {
            let p0 = calculate_price(1000, 0, curve).unwrap();
            let p5 = calculate_price(1000, 5, curve).unwrap();
            assert!(p5 <= p0, "curve {:?} increased", curve);
        }
    }

    #[test]
    fn stepped_price_never_increases() {
        let curve = DecayCurve::Stepped {
            step_size: 7,
            interval: 5,
        };
        let mut prev = calculate_price(1000, 0, &curve).unwrap();
        for t in 1..=100 {
            let curr = calculate_price(1000, t, &curve).unwrap();
            assert!(
                curr <= prev,
                "stepped price increased at t={}: {} > {}",
                t,
                curr,
                prev
            );
            prev = curr;
        }
    }

    #[test]
    fn stepped_price_never_increases_various_intervals() {
        let intervals = [1, 3, 5, 10, 20, 50];
        for &interval in &intervals {
            let curve = DecayCurve::Stepped {
                step_size: 5,
                interval,
            };
            let mut prev = calculate_price(500, 0, &curve).unwrap();
            for t in 1..=100 {
                let curr = calculate_price(500, t, &curve).unwrap();
                assert!(
                    curr <= prev,
                    "stepped price increased at t={} with interval={}: {} > {}",
                    t,
                    interval,
                    curr,
                    prev
                );
                prev = curr;
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]
        #[test]
        fn stepped_monotonic_proptest(
            start_price in 1000u128..1_000_000,
            step_size in 1u128..10_000,
            interval in 1u64..50,
            t in 0u64..100
        ) {
            let curve = DecayCurve::Stepped { step_size, interval };
            let p1 = calculate_price(start_price, t, &curve).unwrap();
            let p2 = calculate_price(start_price, t + 1, &curve).unwrap();
            prop_assert!(
                p2 <= p1,
                "stepped price increased: p1={} at t={}, p2={} at t+1={}",
                p1, t, p2, t + 1
            );
        }
    }
}
