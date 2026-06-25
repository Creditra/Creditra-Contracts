// SPDX-License-Identifier: MIT

//! # Fuzz target: `compute_deviation_bps` boundary exploration
//!
//! This target exercises [`creditra_credit::math_utils::compute_deviation_bps`]
//! across the full `(i128, i128)` input domain. The function computes the
//! absolute deviation between two oracle prices in basis points (bps), returning
//! `None` when the prior price (`last_price`) is non-positive **or** when
//! intermediate arithmetic overflows, and `Some(u32)` otherwise.
//!
//! ## Properties under test
//!
//! 1. **`None` when `last_price ≤ 0`**: The function must return `None` for any
//!    `last_price` that is zero or negative, regardless of `new_price`.
//!
//! 2. **No panics**: The function must never panic for any `(i128, i128)` input.
//!    Overflow is handled via `checked_mul` returning `None`.
//!
//! 3. **`None` only when justified**: When `last_price > 0`, a `None` result is
//!    acceptable only if the intermediate product `|new − last| × 10_000`
//!    overflows `u128`. Otherwise the function must return `Some`.
//!
//! 4. **Saturation ceiling**: Any returned `u32` value is ≤ `u32::MAX`
//!    (guaranteed by the type, but asserted for documentation).
//!
//! 5. **Zero deviation for equal prices**: When `new_price == last_price` and
//!    `last_price > 0`, the deviation must be exactly `0`.
//!
//! 6. **Cross-check**: When the reference calculation does not overflow, the
//!    returned value must match `min(|new − last| × 10_000 / last, u32::MAX)`.
//!
//! ## Coverage strategy
//!
//! `arbitrary::Unstructured` produces random `(i128, i128)` pairs, which
//! naturally covers:
//! - All four sign combinations: (+,+), (+,−), (−,+), (−,−)
//! - Zero in both positions
//! - Near-`i128::MIN` and near-`i128::MAX` values (saturation edge)
//! - Typical oracle-scale values via general fuzzing distribution

#![no_main]

use libfuzzer_sys::fuzz_target;

use creditra_credit::math_utils::compute_deviation_bps;

fuzz_target!(|data: (i128, i128)| {
    let (new_price, last_price) = data;

    let result = compute_deviation_bps(new_price, last_price);

    // ── Property 1: None when last_price <= 0 ─────────────────────────────
    if last_price <= 0 {
        assert!(
            result.is_none(),
            "expected None for non-positive last_price={last_price}, \
             but got Some({:?})",
            result
        );
        return;
    }

    // ── From here: last_price > 0 ─────────────────────────────────────────

    // Compute the reference intermediate value to determine if overflow
    // should occur. The production code does:
    //   diff = (new_price - last_price).unsigned_abs()
    //   numerator = diff.checked_mul(10_000)?   // None on overflow
    //   deviation = numerator / last_price_as_u128
    //   return Some(deviation.min(u32::MAX) as u32)
    let diff = (new_price.wrapping_sub(last_price)).unsigned_abs();
    let ref_numerator = diff.checked_mul(10_000_u128);

    match (result, ref_numerator) {
        // ── Property 3a: if the multiplication doesn't overflow, we must
        //    get Some ──────────────────────────────────────────────────────
        (None, Some(_)) => {
            panic!(
                "function returned None despite no overflow: \
                 new_price={new_price}, last_price={last_price}, diff={diff}"
            );
        }

        // ── Property 3b: if the multiplication overflows, None is correct
        (None, None) => {
            // Overflow in checked_mul ⇒ None is the expected result.
        }

        // ── Properties 4, 5, 6: we got Some(deviation) ──────────────────
        (Some(deviation), numerator_opt) => {
            // Property 4: saturation ceiling (type-enforced, but explicit).
            assert!(
                (deviation as u128) <= (u32::MAX as u128),
                "deviation {deviation} exceeds u32::MAX"
            );

            // Property 5: equal prices ⇒ zero deviation.
            if new_price == last_price {
                assert_eq!(
                    deviation, 0,
                    "equal prices (new={new_price}, last={last_price}) \
                     should yield 0 bps, got {deviation}"
                );
            }

            // Property 6: cross-check against reference when no overflow.
            if let Some(numerator) = numerator_opt {
                let expected = numerator / (last_price as u128);
                let expected_clamped = expected.min(u32::MAX as u128) as u32;
                assert_eq!(
                    deviation, expected_clamped,
                    "mismatch for new_price={new_price}, \
                     last_price={last_price}: got {deviation}, \
                     expected {expected_clamped}"
                );
            }
        }
    }
});
