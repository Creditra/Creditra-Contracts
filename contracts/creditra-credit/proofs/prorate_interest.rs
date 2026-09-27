// SPDX-License-Identifier: MIT

//! # Kani proof harnesses for [`crate::math_utils::prorate_interest`]
//!
//! These harnesses use the [Kani](https://model-checking.github.io/kani/)
//! model checker to prove — exhaustively over a bounded symbolic input
//! domain — that [`prorate_interest`] is **monotonic** in each numeric
//! argument and **overflow-safe** (never panics) within the protocol's
//! realistic operating envelope.
//!
//! They compile only under `cfg(kani)` and are invisible to the normal
//! `cargo build` / `cargo test` pipeline. Run them with:
//!
//! ```text
//! cargo kani -p creditra-credit
//! ```
//!
//! ## Why these properties
//!
//! `prorate_interest` computes
//! `round((principal × rate_bps × time_delta) / (BPS_DENOMINATOR × SECONDS_PER_YEAR))`
//! using two `checked_mul` steps. The accrual layer relies on two
//! behavioural guarantees that were previously only asserted by
//! example-based unit tests:
//!
//! 1. **Monotonicity.** Interest must never *decrease* when principal,
//!    rate, or elapsed time increases. A violation would let a borrower
//!    reduce owed interest by accruing over a longer window, or let a
//!    larger debt accrue less than a smaller one — accounting-integrity
//!    bugs.
//! 2. **Overflow-safety.** Within the realistic input envelope the function
//!    must never trigger the `checked_mul` panic. *Outside* the envelope the
//!    panic is the intended behaviour (the caller maps it to
//!    [`crate::types::ContractError::Overflow`]), so these proofs bound the
//!    domain to the safe envelope rather than the full type range.
//!
//! ## The safe envelope
//!
//! The worst-case product is `principal × rate_bps × time_delta`. With the
//! bounds below it is at most `10^24 × 10^4 × 10^9 = 10^37`, comfortably
//! under `u128::MAX ≈ 3.4 × 10^38` (~300× margin), so neither `checked_mul`
//! can overflow:
//!
//! | Input        | Bound            | Justification                                              |
//! |--------------|------------------|-----------------------------------------------------------|
//! | `principal`  | `≤ 10^24`        | Far above any realistic utilized balance (7–18 decimals)  |
//! | `rate_bps`   | `≤ 10_000`       | Enforced cap `risk::MAX_INTEREST_RATE_BPS` (100 % APR)     |
//! | `time_delta` | `≤ 10^9` (≈31.7y)| Generous bound on a single accrual window                 |

#![cfg(kani)]

use crate::math_utils::{prorate_interest, Rounding};

/// Upper bound on `principal` for the overflow-safe envelope (`10^24`).
const PRINCIPAL_MAX: u128 = 1_000_000_000_000_000_000_000_000;

/// Upper bound on `rate_bps`: the protocol's enforced interest-rate cap
/// (`risk::MAX_INTEREST_RATE_BPS = 10_000`, i.e. 100 % APR).
const RATE_MAX: u32 = 10_000;

/// Upper bound on `time_delta` (`10^9` seconds ≈ 31.7 years).
const TIME_MAX: u64 = 1_000_000_000;

/// Draw a symbolic input triple constrained to the overflow-safe envelope.
fn bounded() -> (u128, u32, u64) {
    let principal: u128 = kani::any();
    let rate_bps: u32 = kani::any();
    let time_delta: u64 = kani::any();
    kani::assume(principal <= PRINCIPAL_MAX);
    kani::assume(rate_bps <= RATE_MAX);
    kani::assume(time_delta <= TIME_MAX);
    (principal, rate_bps, time_delta)
}

/// Within the safe envelope, `prorate_interest` never panics (no
/// `checked_mul` overflow) for either rounding direction.
#[kani::proof]
fn prorate_interest_overflow_safe() {
    let (principal, rate_bps, time_delta) = bounded();
    let _ = prorate_interest(principal, rate_bps, time_delta, Rounding::Floor);
    let _ = prorate_interest(principal, rate_bps, time_delta, Rounding::Ceil);
}

/// Interest is non-decreasing in `principal` (other args fixed).
#[kani::proof]
fn prorate_interest_monotonic_in_principal() {
    let principal: u128 = kani::any();
    let rate_bps: u32 = kani::any();
    let time_delta: u64 = kani::any();
    // `principal + 1` must stay in-envelope, so bound strictly below MAX.
    kani::assume(principal < PRINCIPAL_MAX);
    kani::assume(rate_bps <= RATE_MAX);
    kani::assume(time_delta <= TIME_MAX);

    let lo_floor = prorate_interest(principal, rate_bps, time_delta, Rounding::Floor);
    let hi_floor = prorate_interest(principal + 1, rate_bps, time_delta, Rounding::Floor);
    assert!(hi_floor >= lo_floor);

    let lo_ceil = prorate_interest(principal, rate_bps, time_delta, Rounding::Ceil);
    let hi_ceil = prorate_interest(principal + 1, rate_bps, time_delta, Rounding::Ceil);
    assert!(hi_ceil >= lo_ceil);
}

/// Interest is non-decreasing in `rate_bps` (other args fixed).
#[kani::proof]
fn prorate_interest_monotonic_in_rate() {
    let principal: u128 = kani::any();
    let rate_bps: u32 = kani::any();
    let time_delta: u64 = kani::any();
    kani::assume(principal <= PRINCIPAL_MAX);
    kani::assume(rate_bps < RATE_MAX);
    kani::assume(time_delta <= TIME_MAX);

    let lo_floor = prorate_interest(principal, rate_bps, time_delta, Rounding::Floor);
    let hi_floor = prorate_interest(principal, rate_bps + 1, time_delta, Rounding::Floor);
    assert!(hi_floor >= lo_floor);

    let lo_ceil = prorate_interest(principal, rate_bps, time_delta, Rounding::Ceil);
    let hi_ceil = prorate_interest(principal, rate_bps + 1, time_delta, Rounding::Ceil);
    assert!(hi_ceil >= lo_ceil);
}

/// Interest is non-decreasing in `time_delta` (other args fixed).
#[kani::proof]
fn prorate_interest_monotonic_in_time() {
    let principal: u128 = kani::any();
    let rate_bps: u32 = kani::any();
    let time_delta: u64 = kani::any();
    kani::assume(principal <= PRINCIPAL_MAX);
    kani::assume(rate_bps <= RATE_MAX);
    kani::assume(time_delta < TIME_MAX);

    let lo_floor = prorate_interest(principal, rate_bps, time_delta, Rounding::Floor);
    let hi_floor = prorate_interest(principal, rate_bps, time_delta + 1, Rounding::Floor);
    assert!(hi_floor >= lo_floor);

    let lo_ceil = prorate_interest(principal, rate_bps, time_delta, Rounding::Ceil);
    let hi_ceil = prorate_interest(principal, rate_bps, time_delta + 1, Rounding::Ceil);
    assert!(hi_ceil >= lo_ceil);
}

/// The `Ceil` result is always ≥ the `Floor` result and exceeds it by at
/// most one base unit — rounding can never move interest by more than 1.
#[kani::proof]
fn prorate_interest_rounding_bounds() {
    let (principal, rate_bps, time_delta) = bounded();
    let floor = prorate_interest(principal, rate_bps, time_delta, Rounding::Floor);
    let ceil = prorate_interest(principal, rate_bps, time_delta, Rounding::Ceil);
    assert!(ceil >= floor);
    assert!(ceil - floor <= 1);
}

/// Any zero argument yields exactly zero interest (documented short-circuit).
#[kani::proof]
fn prorate_interest_zero_short_circuit() {
    let (principal, rate_bps, time_delta) = bounded();
    assert_eq!(prorate_interest(0, rate_bps, time_delta, Rounding::Ceil), 0);
    assert_eq!(prorate_interest(principal, 0, time_delta, Rounding::Ceil), 0);
    assert_eq!(prorate_interest(principal, rate_bps, 0, Rounding::Ceil), 0);
}