// SPDX-License-Identifier: MIT

//! Property test: interest accrual is **monotonic** — accrued interest never
//! decreases.
//!
//! # Invariant
//!
//! The core protocol principle is *"accrued interest never decreases"*. The
//! accrual function [`creditra_credit::accrual::accrued_interest`] computes
//!
//! ```text
//! interest = principal · rate_bps · elapsed_seconds / (10_000 · SECONDS_PER_YEAR)
//! ```
//!
//! (floored). This test verifies, over randomized inputs, that the result is
//! non-decreasing in each argument and that the headline time-monotonicity
//! holds: for any `t1 ≤ t2`, accrued interest at `t2` is at least the amount
//! at `t1`.
//!
//! The properties checked are:
//!
//! 1. **Monotone in elapsed time** — the "never decreases" invariant: holding
//!    principal and rate fixed, more elapsed time yields `≥` interest.
//! 2. **Monotone in principal** — a larger balance yields `≥` interest.
//! 3. **Monotone in rate** — a higher rate yields `≥` interest.
//! 4. **Zero boundary** — a zero principal, rate, or elapsed time yields
//!    exactly zero interest.
//! 5. **Total / panic-free** — for arbitrary `u128`/`u32`/`u64` inputs the
//!    function returns `Ok` or `Err(Overflow)` but never panics or wraps.
//! 6. **Overflow monotonicity** — if a smaller input overflows, every larger
//!    input on the same axis also overflows (the error region is upward-closed).
//!
//! # References
//!
//! - `contracts/creditra-credit/src/accrual.rs` — `accrued_interest`
//! - `contracts/credit/src/math_utils.rs` — Soroban `prorate_interest` (mirror)
//! - Issue #756

use cosmwasm_std::Uint128;
use creditra_credit::accrual::accrued_interest;
use creditra_credit::error::ContractError;
use proptest::prelude::*;
use proptest::test_runner::Config as ProptestConfig;

/// Principal range wide enough to exercise real values while leaving head-room
/// so that, combined with the rate/time ranges below, most cases compute a
/// finite result (overflow paths are covered separately).
fn principal() -> impl Strategy<Value = u128> {
    0u128..=1_000_000_000_000_000_000_000_000u128 // up to 1e24
}

/// Annualised interest rate in basis points, `0..=10_000` (0%..=100%).
fn rate_bps() -> impl Strategy<Value = u32> {
    0u32..=10_000u32
}

/// Elapsed time in seconds, `0..=100` years.
fn elapsed_seconds() -> impl Strategy<Value = u64> {
    0u64..=(31_536_000u64 * 100)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, .. ProptestConfig::default() })]

    /// **Never decreases in time.** For `t1 ≤ t2`, accrued interest at `t2`
    /// is `≥` the amount at `t1` (whenever both are computable).
    #[test]
    fn accrual_is_monotone_in_time(
        p in principal(),
        r in rate_bps(),
        t1 in elapsed_seconds(),
        t2 in elapsed_seconds(),
    ) {
        let (lo, hi) = if t1 <= t2 { (t1, t2) } else { (t2, t1) };
        let principal = Uint128::new(p);

        let i_lo = accrued_interest(principal, r, lo);
        let i_hi = accrued_interest(principal, r, hi);

        if let (Ok(a), Ok(b)) = (i_lo, i_hi) {
            prop_assert!(
                b >= a,
                "time monotonicity violated: accrued({lo}s)={a} > accrued({hi}s)={b} \
                 (principal={p}, rate={r})",
            );
        }
    }

    /// **Never decreases in principal.** A larger balance accrues `≥` interest.
    #[test]
    fn accrual_is_monotone_in_principal(
        p1 in principal(),
        p2 in principal(),
        r in rate_bps(),
        t in elapsed_seconds(),
    ) {
        let (lo, hi) = if p1 <= p2 { (p1, p2) } else { (p2, p1) };

        let i_lo = accrued_interest(Uint128::new(lo), r, t);
        let i_hi = accrued_interest(Uint128::new(hi), r, t);

        if let (Ok(a), Ok(b)) = (i_lo, i_hi) {
            prop_assert!(b >= a, "principal monotonicity violated: {a} > {b}");
        }
    }

    /// **Never decreases in rate.** A higher rate accrues `≥` interest.
    #[test]
    fn accrual_is_monotone_in_rate(
        p in principal(),
        r1 in rate_bps(),
        r2 in rate_bps(),
        t in elapsed_seconds(),
    ) {
        let (lo, hi) = if r1 <= r2 { (r1, r2) } else { (r2, r1) };
        let principal = Uint128::new(p);

        let i_lo = accrued_interest(principal, lo, t);
        let i_hi = accrued_interest(principal, hi, t);

        if let (Ok(a), Ok(b)) = (i_lo, i_hi) {
            prop_assert!(b >= a, "rate monotonicity violated: {a} > {b}");
        }
    }

    /// **Zero boundary.** Any zero input produces exactly zero interest.
    #[test]
    fn zero_input_accrues_nothing(
        p in principal(),
        r in rate_bps(),
        t in elapsed_seconds(),
    ) {
        prop_assert_eq!(accrued_interest(Uint128::zero(), r, t).unwrap(), Uint128::zero());
        prop_assert_eq!(accrued_interest(Uint128::new(p), 0, t).unwrap(), Uint128::zero());
        prop_assert_eq!(accrued_interest(Uint128::new(p), r, 0).unwrap(), Uint128::zero());
    }

    /// **Total & panic-free.** Over the full unrestricted input domain the
    /// function returns `Ok(_)` or `Err(Overflow)` — never panics or wraps.
    #[test]
    fn accrual_never_panics(
        p in any::<u128>(),
        r in any::<u32>(),
        t in any::<u64>(),
    ) {
        match accrued_interest(Uint128::new(p), r, t) {
            Ok(_) => {}
            Err(e) => prop_assert_eq!(e, ContractError::Overflow),
        }
    }

    /// **Overflow region is upward-closed in time.** If a smaller elapsed time
    /// overflows, so does every larger one — the failure never "heals" into a
    /// smaller (i.e. decreased) result.
    #[test]
    fn overflow_is_upward_closed_in_time(
        p in any::<u128>(),
        r in 1u32..=10_000u32,
        t1 in any::<u64>(),
        t2 in any::<u64>(),
    ) {
        let (lo, hi) = if t1 <= t2 { (t1, t2) } else { (t2, t1) };
        let principal = Uint128::new(p);

        if accrued_interest(principal, r, lo).is_err() && lo > 0 {
            prop_assert!(
                accrued_interest(principal, r, hi).is_err(),
                "overflow at {lo}s but not at {hi}s (principal={p}, rate={r})",
            );
        }
    }
}

// ── Deterministic regression cases ─────────────────────────────────────────

#[cfg(test)]
mod deterministic {
    use super::*;
    use creditra_credit::accrual::SECONDS_PER_YEAR;

    /// A concrete increasing time series must produce a non-decreasing
    /// sequence of accrued amounts.
    #[test]
    fn increasing_time_series_is_non_decreasing() {
        let principal = Uint128::new(1_000_000);
        let rate = 500u32; // 5%
        let mut prev = Uint128::zero();
        for days in 0u64..=365 {
            let secs = days * 86_400;
            let cur = accrued_interest(principal, rate, secs).unwrap();
            assert!(cur >= prev, "day {days}: {cur} < previous {prev}");
            prev = cur;
        }
        // One full year at 5% on 1_000_000 = 50_000.
        assert_eq!(
            accrued_interest(principal, rate, SECONDS_PER_YEAR).unwrap(),
            Uint128::new(50_000)
        );
    }

    /// Interest is strictly positive once enough time passes to clear the
    /// floor, confirming monotonicity is not vacuously satisfied by zeros.
    #[test]
    fn interest_becomes_positive_and_grows() {
        let principal = Uint128::new(1_000_000);
        let rate = 500u32;
        let a = accrued_interest(principal, rate, 86_400).unwrap(); // 1 day
        let b = accrued_interest(principal, rate, 86_400 * 2).unwrap(); // 2 days
        assert!(a > Uint128::zero());
        assert!(b > a);
    }
}
