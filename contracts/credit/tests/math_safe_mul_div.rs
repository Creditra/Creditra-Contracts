//! Property test for `math_utils::mul_div` (issue #471).
//!
//! Asserts the production `u128` [`mul_div`] agrees with a
//! [`primitive_types::U256`] reference for 4096 deterministic pseudo-random
//! `(a, numerator, denominator)` triples per [`Rounding`] variant.
//!
//! The subtlety: `mul_div` forms the intermediate product in `u128` and
//! panics (`"math_utils: mul overflow"`) when `a * numerator > u128::MAX`,
//! even when the mathematically-true `a * numerator / denominator` would fit.
//! The U256 reference never overflows there, so the oracle is:
//!
//! * if the true product exceeds `u128::MAX` → `mul_div` MUST panic;
//! * otherwise → `mul_div` MUST equal the exact U256 floor/ceil result.
//!
//! Seeds are fixed, so the stream — and therefore any failure — is
//! reproducible. Run with `cargo test -p creditra-credit --test math_safe_mul_div`.

use std::panic::catch_unwind;

use creditra_credit::math_utils::{mul_div, Rounding};
use primitive_types::U256;

/// Random triples exercised per rounding mode.
const CASES_PER_VARIANT: usize = 4096;

/// Widen a `u128` to `U256` via two 64-bit limbs (no reliance on a
/// `From<u128>` impl).
fn to_u256(x: u128) -> U256 {
    (U256::from((x >> 64) as u64) << 64) | U256::from(x as u64)
}

/// Narrow a `U256` back to `u128`. Caller guarantees `x <= u128::MAX`, i.e. the
/// two high limbs are zero.
fn to_u128(x: U256) -> u128 {
    ((x.0[1] as u128) << 64) | (x.0[0] as u128)
}

/// Deterministic xorshift64* PRNG — a fixed non-zero seed yields a reproducible
/// stream.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A full-width `u128` masked to a pseudo-random bit width in `0..=128`, so
    /// the stream straddles both the in-range and the overflowing-product
    /// regimes of `mul_div` (uniform full-width draws would almost always
    /// overflow).
    fn next_u128_varwidth(&mut self) -> u128 {
        let raw = ((self.next_u64() as u128) << 64) | (self.next_u64() as u128);
        let bits = (self.next_u64() % 129) as u32; // 0..=128
        if bits == 0 {
            0
        } else if bits >= 128 {
            raw
        } else {
            raw & ((1u128 << bits) - 1)
        }
    }
}

/// Silence only the *expected* overflow panics emitted by `mul_div`, so the
/// thousands of intentional overflow cases don't flood stderr while genuine
/// failures still surface.
fn quiet_expected_overflow_panics() {
    std::panic::set_hook(Box::new(|info| {
        let msg = info.to_string();
        if !msg.contains("math_utils: mul overflow") {
            eprintln!("{msg}");
        }
    }));
}

fn check_variant(seed: u64, rounding: Rounding) {
    quiet_expected_overflow_panics();

    let mut rng = Rng(seed);
    let max = to_u256(u128::MAX);

    for _ in 0..CASES_PER_VARIANT {
        let a = rng.next_u128_varwidth();
        let numerator = rng.next_u128_varwidth();
        // mul_div asserts `denominator != 0`; keep it strictly positive.
        let denominator = rng.next_u128_varwidth().max(1);

        let product = to_u256(a) * to_u256(numerator);

        if product > max {
            // True product exceeds u128::MAX -> mul_div must overflow-panic in
            // its checked_mul, identically for both rounding modes.
            let result = catch_unwind(move || mul_div(a, numerator, denominator, rounding));
            assert!(
                result.is_err(),
                "mul_div({a}, {numerator}, {denominator}, {rounding:?}) must panic when a*num > u128::MAX",
            );
            continue;
        }

        let denom = to_u256(denominator);
        let floor = product / denom;
        let expected = match rounding {
            Rounding::Floor => floor,
            Rounding::Ceil => {
                if product % denom != U256::zero() {
                    floor + U256::one()
                } else {
                    floor
                }
            }
        };

        // In the in-range regime the reference always fits in u128: floor <=
        // product <= u128::MAX, and ceil only adds 1 when there is a remainder,
        // which requires denom > 1 and hence floor < u128::MAX.
        assert!(
            expected <= max,
            "reference result unexpectedly exceeded u128::MAX for ({a}, {numerator}, {denominator})",
        );

        let got = mul_div(a, numerator, denominator, rounding);
        assert_eq!(
            got,
            to_u128(expected),
            "mul_div({a}, {numerator}, {denominator}, {rounding:?}) disagreed with U256 reference",
        );
    }
}

#[test]
fn mul_div_floor_matches_u256_reference() {
    check_variant(0x0000_0000_C0FF_EE01, Rounding::Floor);
}

#[test]
fn mul_div_ceil_matches_u256_reference() {
    check_variant(0x0000_0000_C0FF_EE02, Rounding::Ceil);
}

/// A few hand-picked boundary triples, independent of the PRNG, pinning the
/// floor/ceil semantics and the exact overflow boundary.
#[test]
fn mul_div_known_edge_cases() {
    // Exact division: floor == ceil.
    assert_eq!(mul_div(1_000, 3, 10, Rounding::Floor), 300);
    assert_eq!(mul_div(1_001, 3, 10, Rounding::Floor), 300);
    assert_eq!(mul_div(1_001, 3, 10, Rounding::Ceil), 301);

    // Largest non-overflowing product: u128::MAX * 1 fits exactly.
    assert_eq!(mul_div(u128::MAX, 1, 1, Rounding::Floor), u128::MAX);
    assert_eq!(mul_div(u128::MAX, 1, 1, Rounding::Ceil), u128::MAX);

    // Smallest overflowing product: u128::MAX * 2 must panic in both modes.
    quiet_expected_overflow_panics();
    for rounding in [Rounding::Floor, Rounding::Ceil] {
        let r = catch_unwind(move || mul_div(u128::MAX, 2, 1, rounding));
        assert!(
            r.is_err(),
            "u128::MAX * 2 must overflow-panic ({rounding:?})"
        );
    }
}
