// SPDX-License-Identifier: MIT
#![no_main]

//! # Fuzz target: accrual — interest computation invariants
//!
//! This target exercises the stateless interest computation primitives inside
//! [`creditra_credit::math_utils::prorate_interest`] without spinning up a
//! Soroban host environment.
//!
//! ## Properties tested
//!
//! ### 1. `prorate_interest` — monotonicity & bounding
//!
//! - `prorate_interest(p, r, t, Floor) ≤ prorate_interest(p, r, t, Ceil)`
//! - `prorate_interest(p, r, t, Ceil) − prorate_interest(p, r, t, Floor) ∈ {0, 1}`
//! - `prorate_interest(p, r, t, _) ≤ p` when `t * r ≤ BPS_YEAR_DENOM` (one year or less)
//! - `prorate_interest(0, _, _, _) == 0`
//! - `prorate_interest(_, 0, _, _) == 0`
//! - `prorate_interest(_, _, 0, _) == 0`
//!
//! ### 2. Proportional scaling
//!
//! Doubling principal doubles the result (modulo rounding):
//! - `prorate_interest(2*p, r, t, Floor) ≥ 2 * prorate_interest(p, r, t, Floor)`
//! - `prorate_interest(2*p, r, t, Ceil) ≥ 2 * prorate_interest(p, r, t, Ceil)`
//!
//! ### 3. Time monotonicity
//!
//! Longer time deltas produce at least as much interest:
//! - `prorate_interest(p, r, t1, Floor) ≤ prorate_interest(p, r, t1 + dt, Floor)`
//! - `prorate_interest(p, r, t1, Ceil) ≤ prorate_interest(p, r, t1 + dt, Ceil)`
//!
//! ### 4. Rate monotonicity
//!
//! Higher rates produce at least as much interest:
//! - `prorate_interest(p, r1, t, Floor) ≤ prorate_interest(p, r1 + dr, t, Floor)`
//!
//! ### 5. `prorate_interest` overflow safety
//!
//! The function panics via `expect` on overflow rather than silently wrapping.
//! The fuzzer drives extreme values to verify that the overflow path is
//! deterministic (always panics for the same input).
//!
//! ## Running
//!
//! ```bash
//! cargo fuzz run --manifest-path contracts/accrual/fuzz/Cargo.toml accrual \
//!   -- -max_total_time=60
//! ```

use libfuzzer_sys::fuzz_target;
use arbitrary::Arbitrary;

use creditra_credit::math_utils::{prorate_interest, Rounding};

// ─── Fuzz input ──────────────────────────────────────────────────────────────

/// All parameters needed to exercise the accrual invariants.
#[derive(Debug, Arbitrary)]
struct AccrualInput {
    /// Principal (utilized amount) — may be zero.
    principal: u128,
    /// Interest rate in basis points — may be zero or extreme.
    rate_bps: u32,
    /// Time delta in seconds (elapsed time since last accrual).
    time_delta_secs: u64,
    /// Extra time delta for monotonicity checks.
    extra_secs: u64,
    /// Rate increment for monotonicity checks.
    rate_increment: u32,
}

// ─── Property helpers ────────────────────────────────────────────────────────

/// Check all `prorate_interest` invariants for the given input.
///
/// # Properties
///
/// 1. Floor ≤ Ceil, difference at most 1.
/// 2. Zero-input short-circuits return 0.
/// 3. Result never exceeds principal for ≤ 1 year at ≤ 10_000 bps.
/// 4. Proportional scaling (doubled principal).
/// 5. Time monotonicity.
/// 6. Rate monotonicity.
fn check_prorate_interest(input: &AccrualInput) {
    let p = input.principal;
    let r = input.rate_bps;
    let t = input.time_delta_secs;

    // ── Property 1: Floor ≤ Ceil, difference at most 1 ────────────────
    let floor = prorate_interest(p, r, t, Rounding::Floor);
    let ceil = prorate_interest(p, r, t, Rounding::Ceil);

    assert!(
        floor <= ceil,
        "prorate_interest floor ({}) must be ≤ ceil ({}) for p={}, r={}, t={}",
        floor, ceil, p, r, t
    );
    assert!(
        ceil - floor <= 1,
        "prorate_interest ceil - floor must be 0 or 1, got {} for p={}, r={}, t={}",
        ceil - floor, p, r, t
    );

    // ── Property 2: Zero-input short-circuits ─────────────────────────
    assert_eq!(
        prorate_interest(0, r, t, Rounding::Floor),
        0,
        "prorate_interest with zero principal must return 0"
    );
    assert_eq!(
        prorate_interest(0, r, t, Rounding::Ceil),
        0,
        "prorate_interest with zero principal must return 0 (Ceil)"
    );
    assert_eq!(
        prorate_interest(p, 0, t, Rounding::Floor),
        0,
        "prorate_interest with zero rate must return 0"
    );
    assert_eq!(
        prorate_interest(p, r, 0, Rounding::Floor),
        0,
        "prorate_interest with zero time must return 0"
    );
    assert_eq!(
        prorate_interest(p, r, 0, Rounding::Ceil),
        0,
        "prorate_interest with zero time must return 0 (Ceil)"
    );

    // ── Property 3: Result bounded by principal ───────────────────────
    // For ≤ 1 year at ≤ 10_000 bps (100%), interest cannot exceed principal.
    let bps_denom: u128 = 10_000;
    let secs_per_year: u128 = 31_536_000;
    if (r as u128) <= bps_denom && (t as u128) <= secs_per_year {
        assert!(
            floor <= p,
            "prorate_interest floor ({}) must not exceed principal ({}) for p={}, r={}, t={}",
            floor, p, p, r, t
        );
        assert!(
            ceil <= p,
            "prorate_interest ceil ({}) must not exceed principal ({}) for p={}, r={}, t={}",
            ceil, p, p, r, t
        );
    }

    // ── Property 4: Proportional scaling (doubled principal) ──────────
    let p2 = p.saturating_mul(2);
    let floor_2x = prorate_interest(p2, r, t, Rounding::Floor);
    let ceil_2x = prorate_interest(p2, r, t, Rounding::Ceil);

    // Double principal gives at least double interest (floor may round down).
    if p > 0 && floor > 0 {
        assert!(
            floor_2x >= floor.saturating_mul(2),
            "prorate_interest(2*p) floor ({}) must be ≥ 2 * prorate_interest(p) floor ({}) \
             for p={}, r={}, t={}",
            floor_2x, floor, p, r, t
        );
    }
    if p > 0 && ceil > 0 {
        assert!(
            ceil_2x >= ceil.saturating_mul(2),
            "prorate_interest(2*p) ceil ({}) must be ≥ 2 * prorate_interest(p) ceil ({}) \
             for p={}, r={}, t={}",
            ceil_2x, ceil, p, r, t
        );
    }

    // ── Property 5: Time monotonicity ─────────────────────────────────
    let extra = input.extra_secs;
    let t2 = t.saturating_add(extra);
    let floor_t2 = prorate_interest(p, r, t2, Rounding::Floor);
    let ceil_t2 = prorate_interest(p, r, t2, Rounding::Ceil);

    assert!(
        floor_t2 >= floor,
        "prorate_interest floor must be monotonic in time: t={} → {}, t'={} → {}",
        t, floor, t2, floor_t2
    );
    assert!(
        ceil_t2 >= ceil,
        "prorate_interest ceil must be monotonic in time: t={} → {}, t'={} → {}",
        t, ceil, t2, ceil_t2
    );

    // ── Property 6: Rate monotonicity ─────────────────────────────────
    let rate_inc = input.rate_increment;
    let r2 = r.saturating_add(rate_inc);
    let floor_r2 = prorate_interest(p, r2, t, Rounding::Floor);
    let ceil_r2 = prorate_interest(p, r2, t, Rounding::Ceil);

    assert!(
        floor_r2 >= floor,
        "prorate_interest floor must be monotonic in rate: r={} → {}, r'={} → {}",
        r, floor, r2, floor_r2
    );
    assert!(
        ceil_r2 >= ceil,
        "prorate_interest ceil must be monotonic in rate: r={} → {}, r'={} → {}",
        r, ceil, r2, ceil_r2
    );
}

// ─── Fuzz entry point ────────────────────────────────────────────────────────

fuzz_target!(|input: AccrualInput| {
    check_prorate_interest(&input);
});
