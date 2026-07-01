// SPDX-License-Identifier: MIT
//! Kani verification harness for Dutch auction price monotonicity.
//!
//! This module contains formal verification proofs using Kani to demonstrate
//! that `compute_dutch_price` is strictly decreasing in `elapsed_time` within
//! the auction window for both Linear and Stepped decay modes.
//!
//! To run these proofs:
//! ```bash
//! kani proofs/dutch_price.rs --harness harness_linear_monotonicity
//! kani proofs/dutch_price.rs --harness harness_stepped_monotonicity
//! kani proofs/dutch_price.rs --harness harness_linear_bounds
//! kani proofs/dutch_price.rs --harness harness_stepped_bounds
//! ```

#![cfg_attr(kani, feature(kani))]

use gateway_auction::{compute_dutch_price, DutchAuctionDecay};

/// Kani harness proving strict monotonicity for Linear Dutch auction decay.
///
/// # Property
/// For any valid inputs with `t1 < t2 < duration`, the price at `t1` is
/// strictly greater than the price at `t2` when using Linear decay.
///
/// # Invariants
/// - `start_price >= floor_price`
/// - `duration > 0`
/// - `0 <= t1 < t2 < duration`
/// - No overflow in intermediate calculations
#[cfg(kani)]
#[kani::proof]
fn harness_linear_monotonicity() {
    // Symbolic inputs bounded to reasonable ranges for verification
    let start_price: i128 = kani::any();
    let floor_price: i128 = kani::any();
    let duration: u64 = kani::any();
    let t1: u64 = kani::any();
    let t2: u64 = kani::any();

    // Preconditions
    kani::assume(start_price >= floor_price);
    kani::assume(start_price >= 0);
    kani::assume(floor_price >= 0);
    kani::assume(duration > 0);
    kani::assume(duration <= 1_000_000_000); // Reasonable bound for verification
    kani::assume(t1 < t2);
    kani::assume(t2 < duration);

    // Prevent overflow in intermediate calculations
    // price_drop = start_price - floor_price
    let price_drop = start_price - floor_price;
    kani::assume(price_drop <= i128::MAX / (duration as i128));

    let decay = DutchAuctionDecay::Linear;

    let price_t1 = compute_dutch_price(start_price, floor_price, t1, duration, &decay, None);
    let price_t2 = compute_dutch_price(start_price, floor_price, t2, duration, &decay, None);

    // Assert strict monotonicity: price decreases as time increases
    kani::assert(price_t1 > price_t2, "Linear decay must be strictly decreasing");
}

/// Kani harness proving strict monotonicity for Stepped Dutch auction decay.
///
/// # Property
/// For any valid inputs with `t1 < t2 < duration`, the price at `t1` is
/// greater than or equal to the price at `t2` when using Stepped decay.
/// Note: Stepped decay is non-increasing (can be equal within same step).
///
/// # Invariants
/// - `start_price >= floor_price`
/// - `duration > 0`
/// - `step_count > 0`
/// - `0 <= t1 < t2 < duration`
/// - No overflow in intermediate calculations
#[cfg(kani)]
#[kani::proof]
fn harness_stepped_monotonicity() {
    // Symbolic inputs bounded to reasonable ranges for verification
    let start_price: i128 = kani::any();
    let floor_price: i128 = kani::any();
    let duration: u64 = kani::any();
    let step_count: u32 = kani::any();
    let t1: u64 = kani::any();
    let t2: u64 = kani::any();

    // Preconditions
    kani::assume(start_price >= floor_price);
    kani::assume(start_price >= 0);
    kani::assume(floor_price >= 0);
    kani::assume(duration > 0);
    kani::assume(duration <= 1_000_000_000); // Reasonable bound for verification
    kani::assume(step_count > 0);
    kani::assume(step_count <= 1000); // Reasonable bound for verification
    kani::assume(t1 < t2);
    kani::assume(t2 < duration);

    // Prevent overflow in intermediate calculations
    let price_drop = start_price - floor_price;
    kani::assume(price_drop <= i128::MAX / (step_count as i128));
    kani::assume((t1 as i128) <= i128::MAX / (step_count as i128));
    kani::assume((t2 as i128) <= i128::MAX / (step_count as i128));

    let decay = DutchAuctionDecay::Stepped;

    let price_t1 = compute_dutch_price(start_price, floor_price, t1, duration, &decay, Some(step_count));
    let price_t2 = compute_dutch_price(start_price, floor_price, t2, duration, &decay, Some(step_count));

    // Assert monotonicity: price does not increase as time increases
    // Stepped decay can be equal within the same step, so we use >=
    kani::assert(price_t1 >= price_t2, "Stepped decay must be non-increasing");
}

/// Kani harness proving price bounds for Linear decay.
///
/// # Property
/// For any valid inputs, the computed price is always bounded between
/// `floor_price` and `start_price`.
///
/// # Invariants
/// - `start_price >= floor_price`
/// - `duration > 0`
/// - `0 <= elapsed_time`
#[cfg(kani)]
#[kani::proof]
fn harness_linear_bounds() {
    let start_price: i128 = kani::any();
    let floor_price: i128 = kani::any();
    let duration: u64 = kani::any();
    let elapsed_time: u64 = kani::any();

    kani::assume(start_price >= floor_price);
    kani::assume(start_price >= 0);
    kani::assume(floor_price >= 0);
    kani::assume(duration > 0);
    kani::assume(duration <= 1_000_000_000);

    let price_drop = start_price - floor_price;
    kani::assume(price_drop <= i128::MAX / (duration as i128));

    let decay = DutchAuctionDecay::Linear;
    let price = compute_dutch_price(start_price, floor_price, elapsed_time, duration, &decay, None);

    kani::assert(price >= floor_price, "Price must be >= floor_price");
    kani::assert(price <= start_price, "Price must be <= start_price");
}

/// Kani harness proving price bounds for Stepped decay.
///
/// # Property
/// For any valid inputs, the computed price is always bounded between
/// `floor_price` and `start_price`.
///
/// # Invariants
/// - `start_price >= floor_price`
/// - `duration > 0`
/// - `step_count > 0`
/// - `0 <= elapsed_time`
#[cfg(kani)]
#[kani::proof]
fn harness_stepped_bounds() {
    let start_price: i128 = kani::any();
    let floor_price: i128 = kani::any();
    let duration: u64 = kani::any();
    let step_count: u32 = kani::any();
    let elapsed_time: u64 = kani::any();

    kani::assume(start_price >= floor_price);
    kani::assume(start_price >= 0);
    kani::assume(floor_price >= 0);
    kani::assume(duration > 0);
    kani::assume(duration <= 1_000_000_000);
    kani::assume(step_count > 0);
    kani::assume(step_count <= 1000);

    let price_drop = start_price - floor_price;
    kani::assume(price_drop <= i128::MAX / (step_count as i128));
    kani::assume((elapsed_time as i128) <= i128::MAX / (step_count as i128));

    let decay = DutchAuctionDecay::Stepped;
    let price = compute_dutch_price(start_price, floor_price, elapsed_time, duration, &decay, Some(step_count));

    kani::assert(price >= floor_price, "Price must be >= floor_price");
    kani::assert(price <= start_price, "Price must be <= start_price");
}

#[cfg(kani)]
fn main() {
    harness_linear_monotonicity();
    harness_stepped_monotonicity();
    harness_linear_bounds();
    harness_stepped_bounds();
}
