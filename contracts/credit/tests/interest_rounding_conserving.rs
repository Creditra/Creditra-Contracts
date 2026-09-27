// SPDX-License-Identifier: MIT

//! Integration tests for deterministic, value-conserving interest rounding.
//!
//! These tests exercise the public [`creditra_credit::math_utils`] primitives
//! introduced to close issue #1151:
//!
//! - [`split_conserving`] — apportions an amount across weighted buckets so the
//!   parts **always sum exactly to the total** (no dust created or lost) and the
//!   allocation is fully deterministic (largest-remainder, ties by index).
//! - [`prorate_interest_conserving`] — floors prorated interest exactly once
//!   and splits it via [`split_conserving`].
//!
//! Coverage: success / happy-path splits, dust amounts, boundary values
//! (`0`, `u128::MAX`), duplicate/idempotent calls, and invalid inputs
//! (empty weight set panics).

use creditra_credit::math_utils::{prorate_interest_conserving, split_conserving, Rounding, SECONDS_PER_YEAR};

/// Helper: assert a slice sums to `total`.
fn assert_conserves(parts: &[u128], total: u128) {
    assert_eq!(parts.iter().copied().sum::<u128>(), total);
}

#[test]
fn success_two_way_even_split() {
    let parts = split_conserving(1_000, &[5_000, 5_000]);
    assert_eq!(parts, vec![500, 500]);
    assert_conserves(&parts, 1_000);
}

#[test]
fn success_three_way_weighted_split() {
    // weights 1:2:7 over 1_000 → floors 100, 200, 700, no leftover.
    let parts = split_conserving(1_000, &[1_000, 2_000, 7_000]);
    assert_eq!(parts, vec![100, 200, 700]);
    assert_conserves(&parts, 1_000);
}

#[test]
fn success_large_total_exact_sum() {
    let total = 987_654_321_987_654_321u128;
    let parts = split_conserving(total, &[1_111, 2_222, 3_333, 4_444]);
    assert_conserves(&parts, total);
    // determinism: identical output on a second call
    assert_eq!(parts, split_conserving(total, &[1_111, 2_222, 3_333, 4_444]));
}

#[test]
fn dust_single_unit_is_not_lost() {
    // 1 base unit split 50/50: must end up entirely in one bucket, sum == 1.
    let parts = split_conserving(1, &[5_000, 5_000]);
    assert_conserves(&parts, 1);
    assert!(parts == vec![1, 0] || parts == vec![0, 1]);
}

#[test]
fn dust_leftover_goes_to_largest_fractional_claim() {
    // 10 split 1/3 vs 2/3: floors 3 & 6, leftover 1 → larger fractional claim
    // (the 2/3 bucket) receives it deterministically.
    let parts = split_conserving(10, &[3_333, 6_667]);
    assert_eq!(parts, vec![3, 7]);
    assert_conserves(&parts, 10);
}

#[test]
fn dust_odd_total_three_way_still_conserved() {
    // 100 split 1:1:1 → 33/33/34 (one bucket absorbs the leftover).
    let parts = split_conserving(100, &[3_333, 3_333, 3_334]);
    assert_conserves(&parts, 100);
    let mut sorted = parts.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, vec![33, 33, 34]);
}

#[test]
fn boundary_zero_total_yields_zeros() {
    assert_eq!(split_conserving(0, &[5_000, 5_000]), vec![0, 0]);
    let parts = split_conserving(0, &[0, 0, 0]);
    assert_conserves(&parts, 0);
}

#[test]
fn boundary_max_total_is_conserved_and_does_not_overflow() {
    // Maximum-magnitude token amount must not overflow or hang and must conserve.
    let parts = split_conserving(u128::MAX, &[1, 1, 1, 1]);
    assert_conserves(&parts, u128::MAX);
    let expected = u128::MAX / 4;
    // Four equal weights → three buckets at `expected`, one at `expected + rem`.
    let rem = u128::MAX % 4;
    assert_eq!(parts.iter().copied().sum::<u128>(), u128::MAX);
    assert!(parts.iter().all(|&p| p == expected || p == expected + rem));
}

#[test]
fn boundary_single_weight_takes_everything() {
    assert_eq!(split_conserving(7_777, &[10_000]), vec![7_777]);
}

#[test]
fn boundary_all_zero_weights_collapses_to_first_bucket() {
    // No proportional signal → entire amount lands on bucket 0; still conserved.
    let parts = split_conserving(123, &[0, 0, 0]);
    assert_eq!(parts, vec![123, 0, 0]);
    assert_conserves(&parts, 123);
}

#[test]
fn boundary_one_zero_weight_receives_nothing_when_other_positive() {
    let parts = split_conserving(500, &[10_000, 0]);
    assert_eq!(parts, vec![500, 0]);
    assert_conserves(&parts, 500);
}

#[test]
fn duplicate_call_is_idempotent() {
    // Determinism: the same inputs always yield the same allocation.
    let a = split_conserving(1_234_567, &[2_500, 2_500, 5_000]);
    let b = split_conserving(1_234_567, &[2_500, 2_500, 5_000]);
    assert_eq!(a, b);
    assert_conserves(&a, 1_234_567);
}

#[test]
#[should_panic]
fn invalid_empty_weights_panics() {
    // No recipient to conserve value into → invalid configuration panics.
    let _ = split_conserving(10, &[]);
}

#[test]
fn prorate_interest_conserving_sums_to_realized_interest() {
    let realized = creditra_credit::math_utils::prorate_interest(
        10_000,
        300,
        SECONDS_PER_YEAR as u64,
        Rounding::Floor,
    );
    let shares =
        prorate_interest_conserving(10_000, 300, SECONDS_PER_YEAR as u64, &[5_000, 5_000]);
    assert_eq!(shares.iter().copied().sum::<u128>(), realized);
    assert_eq!(shares[0] + shares[1], realized);
}

#[test]
fn prorate_interest_conserving_zero_inputs() {
    assert_eq!(
        prorate_interest_conserving(0, 300, SECONDS_PER_YEAR as u64, &[3_333, 6_667]),
        vec![0, 0]
    );
    assert_eq!(
        prorate_interest_conserving(10_000, 0, SECONDS_PER_YEAR as u64, &[3_333, 6_667]),
        vec![0, 0]
    );
}
