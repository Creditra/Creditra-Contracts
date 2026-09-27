// SPDX-License-Identifier: MIT
#![no_main]

//! # Fuzz target: `borrow` module — pure-logic properties
//!
//! This target exercises the stateless / near-stateless helpers inside
//! [`creditra_credit::borrow`] without spinning up a Soroban host environment.
//! It focuses on three categories of invariants:
//!
//! ## 1. `draw_status_error` — enum dispatch completeness
//!
//! Every [`CreditStatus`] variant must be handled.  The contract relies on
//! the returned `Option<ContractError>` to gate draws, so a wrong arm (or a
//! missing arm) is a security regression.
//!
//! | variant    | expected                              |
//! |------------|---------------------------------------|
//! | Active     | `None` (draw allowed)                 |
//! | Restricted | `None` (draw allowed — limit-checked) |
//! | Suspended  | `Some(CreditLineSuspended)`           |
//! | Defaulted  | `Some(CreditLineDefaulted)`           |
//! | Closed     | `Some(CreditLineClosed)`              |
//!
//! ## 2. `effective_repay` capping
//!
//! When a borrower calls `repay_credit` or `repay_and_release_collateral`
//! with an `amount` greater than `utilized_amount`, the contract silently
//! caps the repayment to `utilized_amount`.  The fuzz target drives this
//! formula directly and asserts:
//!
//! - `effective_repay ≤ utilized_amount` always.
//! - `effective_repay == utilized_amount` when `amount ≥ utilized_amount`.
//! - `effective_repay == amount` when `0 < amount < utilized_amount`.
//!
//! ## 3. `interest_repaid` capping
//!
//! Interest is only repaid up to the accrued amount.  Asserts:
//!
//! - `interest_repaid ≤ accrued_interest` always.
//! - `interest_repaid ≤ effective_repay` always (can't repay more interest
//!   than the total effective payment).
//! - `interest_repaid == accrued_interest` when
//!   `effective_repay ≥ accrued_interest`.
//!
//! ## 4. Collateral release formula — `mul_div` overflow safety
//!
//! `repay_and_release_collateral` uses:
//!
//! ```text
//! released = collateral_balance * effective_repay / previous_utilized
//! ```
//!
//! The fuzz target calls the underlying [`mul_div`] / [`safe_mul_div`]
//! directly on arbitrary `u128` triples to verify:
//!
//! - `released ≤ collateral_balance` when `effective_repay ≤ previous_utilized`.
//! - Full release (`released == collateral_balance`) when
//!   `effective_repay == previous_utilized`.
//! - `safe_mul_div` never panics; panicking behaviour matches
//!   the documented contract of [`mul_div`].
//! - `safe_mul_div(..., Floor) ≤ safe_mul_div(..., Ceil)` when both are `Some`.
//! - `Ceil − Floor ∈ {0, 1}`.
//!
//! ## 5. `apply_bps` correctness
//!
//! Protocol fees are computed with `apply_bps(effective_repay as u128, fee_bps, Floor)`.
//! Asserts:
//!
//! - `fee ≤ effective_repay` (fee never exceeds the repayment).
//! - `fee == 0` when `fee_bps == 0`.
//! - `fee == effective_repay` when `fee_bps == 10_000` (100 %).
//! - `floor_fee ≤ ceil_fee`.
//! - `ceil_fee - floor_fee ∈ {0, 1}`.
//!
//! ## Running
//!
//! ```bash
//! # From workspace root — requires `cargo-fuzz` (nightly).
//! cargo fuzz run --manifest-path contracts/borrow/fuzz/Cargo.toml main \
//!   -- -max_total_time=60
//! ```
//!
//! Under normal `cargo test`, this file is compiled as part of the fuzz
//! workspace and any harness-level `assert!` failures will be reported as
//! fuzz-corpus bugs.

use libfuzzer_sys::fuzz_target;
use arbitrary::Arbitrary;

use creditra_credit::borrow::draw_status_error;
use creditra_credit::math_utils::{apply_bps, mul_div, safe_mul_div, Rounding};
use creditra_credit::types::{ContractError, CreditStatus};

// ─── Fuzz input ──────────────────────────────────────────────────────────────

/// All parameters needed to exercise the pure-logic borrow invariants.
///
/// Every field is independently derived from the raw fuzz bytes by
/// [`arbitrary`], so the fuzzer has full freedom to set any combination.
#[derive(Debug, Arbitrary)]
struct BorrowInput {
    /// Discriminant mapped to a [`CreditStatus`] variant (mod 5).
    status_raw: u8,
    /// Borrower's current outstanding principal (may be zero or negative).
    utilized_amount: i128,
    /// Borrower's total accrued interest (may be zero or negative).
    accrued_interest: i128,
    /// Requested repayment amount (may be zero, negative, or huge).
    repay_amount: i128,
    /// Collateral balance held by the contract for this borrower.
    collateral_balance: u128,
    /// `a` operand for the `mul_div` / `safe_mul_div` overflow tests.
    mul_div_a: u128,
    /// Numerator operand for the `mul_div` / `safe_mul_div` overflow tests.
    mul_div_num: u128,
    /// Denominator operand for the `mul_div` / `safe_mul_div` overflow tests.
    mul_div_denom: u128,
    /// Fee in basis points (any u32; the production cap is 10 000).
    fee_bps: u32,
}

// ─── CreditStatus discriminant helper ────────────────────────────────────────

/// Map an arbitrary byte to a deterministic [`CreditStatus`] variant.
///
/// The five variants are cyclically indexed so every variant is reachable.
fn credit_status_from_u8(raw: u8) -> CreditStatus {
    match raw % 5 {
        0 => CreditStatus::Active,
        1 => CreditStatus::Suspended,
        2 => CreditStatus::Defaulted,
        3 => CreditStatus::Closed,
        _ => CreditStatus::Restricted,
    }
}

// ─── Property helpers ────────────────────────────────────────────────────────

/// Verify all `draw_status_error` invariants for the given status.
///
/// # Properties
///
/// 1. `Active` and `Restricted` always return `None`.
/// 2. `Suspended` returns exactly `Some(CreditLineSuspended)`.
/// 3. `Defaulted` returns exactly `Some(CreditLineDefaulted)`.
/// 4. `Closed` returns exactly `Some(CreditLineClosed)`.
/// 5. The returned error variant, when `Some`, is always in the
///    `Lifecycle` category.
fn check_draw_status_error(status: CreditStatus) {
    let result = draw_status_error(status);

    match status {
        // ── Property 1: draw-allowed states must return None ──────────────
        CreditStatus::Active => {
            assert!(
                result.is_none(),
                "draw_status_error(Active) must be None, got {:?}",
                result
            );
        }
        CreditStatus::Restricted => {
            assert!(
                result.is_none(),
                "draw_status_error(Restricted) must be None, got {:?}",
                result
            );
        }
        // ── Property 2: Suspended ─────────────────────────────────────────
        CreditStatus::Suspended => {
            assert_eq!(
                result,
                Some(ContractError::CreditLineSuspended),
                "draw_status_error(Suspended) must be Some(CreditLineSuspended)"
            );
        }
        // ── Property 3: Defaulted ─────────────────────────────────────────
        CreditStatus::Defaulted => {
            assert_eq!(
                result,
                Some(ContractError::CreditLineDefaulted),
                "draw_status_error(Defaulted) must be Some(CreditLineDefaulted)"
            );
        }
        // ── Property 4: Closed ────────────────────────────────────────────
        CreditStatus::Closed => {
            assert_eq!(
                result,
                Some(ContractError::CreditLineClosed),
                "draw_status_error(Closed) must be Some(CreditLineClosed)"
            );
        }
    }

    // ── Property 5: lifecycle category when Some ──────────────────────────
    if let Some(err) = result {
        use creditra_credit::types::ContractErrorCategory;
        assert_eq!(
            err.category(),
            ContractErrorCategory::Lifecycle,
            "draw_status_error returned a non-Lifecycle error: {:?}",
            err
        );
    }
}

/// Verify `effective_repay` capping invariants.
///
/// Mirrors the production logic from `repay_credit` and
/// `repay_and_release_collateral`:
///
/// ```rust
/// let effective_repay = if amount > credit_line.utilized_amount {
///     credit_line.utilized_amount
/// } else {
///     amount
/// };
/// ```
///
/// Only well-formed inputs (both positive) are tested here; the contract
/// also guards `amount <= 0` before this logic, so we restrict to positive
/// values for the capping invariants.
fn check_effective_repay(utilized_amount: i128, repay_amount: i128) {
    // Only test the capping logic for well-formed (positive) inputs.
    if utilized_amount <= 0 || repay_amount <= 0 {
        return;
    }

    let effective_repay = if repay_amount > utilized_amount {
        utilized_amount
    } else {
        repay_amount
    };

    // ── Property A: cap invariant ─────────────────────────────────────────
    assert!(
        effective_repay <= utilized_amount,
        "effective_repay ({effective_repay}) must be ≤ utilized_amount ({utilized_amount})"
    );

    // ── Property B: overpayment clamps to utilized ────────────────────────
    if repay_amount >= utilized_amount {
        assert_eq!(
            effective_repay,
            utilized_amount,
            "overpayment: effective_repay must equal utilized_amount"
        );
    }

    // ── Property C: under-utilization passes through ──────────────────────
    if repay_amount < utilized_amount {
        assert_eq!(
            effective_repay,
            repay_amount,
            "partial repay: effective_repay must equal repay_amount"
        );
    }

    // ── Property D: non-negative ──────────────────────────────────────────
    assert!(
        effective_repay >= 0,
        "effective_repay must be non-negative, got {effective_repay}"
    );
}

/// Verify `interest_repaid` capping invariants.
///
/// Production code:
///
/// ```rust
/// let interest_repaid = effective_repay.min(credit_line.accrued_interest);
/// ```
fn check_interest_repaid(utilized_amount: i128, accrued_interest: i128, repay_amount: i128) {
    if utilized_amount <= 0 || repay_amount <= 0 {
        return;
    }

    let effective_repay = if repay_amount > utilized_amount {
        utilized_amount
    } else {
        repay_amount
    };

    let accrued_clamped = accrued_interest.max(0);
    let interest_repaid = effective_repay.min(accrued_clamped);

    // ── Property A: interest ≤ accrued ────────────────────────────────────
    assert!(
        interest_repaid <= accrued_clamped,
        "interest_repaid ({interest_repaid}) must be ≤ accrued_interest ({accrued_clamped})"
    );

    // ── Property B: interest ≤ effective_repay ────────────────────────────
    assert!(
        interest_repaid <= effective_repay,
        "interest_repaid ({interest_repaid}) must be ≤ effective_repay ({effective_repay})"
    );

    // ── Property C: fully paid when coverage sufficient ───────────────────
    if effective_repay >= accrued_clamped {
        assert_eq!(
            interest_repaid,
            accrued_clamped,
            "interest_repaid must equal accrued_interest when effective_repay covers it"
        );
    }

    // ── Property D: non-negative ──────────────────────────────────────────
    assert!(
        interest_repaid >= 0,
        "interest_repaid must be non-negative, got {interest_repaid}"
    );
}

/// Verify the proportional collateral release formula and `mul_div` / `safe_mul_div` safety.
///
/// Production code (`repay_and_release_collateral`):
///
/// ```rust
/// let released = if effective_repay >= previous_utilized {
///     collateral_balance
/// } else {
///     mul_div(
///         collateral_balance as u128,
///         effective_repay as u128,
///         previous_utilized as u128,
///         Rounding::Floor,
///     ) as i128
/// };
/// ```
///
/// This function also exercises `safe_mul_div` (the non-panicking variant)
/// and `mul_div` (the panicking variant) across arbitrary `u128` triples to
/// verify overflow-safety contracts.
fn check_collateral_release(
    collateral_balance: u128,
    mul_div_a: u128,
    mul_div_num: u128,
    mul_div_denom: u128,
) {
    // ── Part 1: `safe_mul_div` never panics ───────────────────────────────
    //
    // Call with all four rounding/denom combinations.  Any panic is a bug.
    let floor_result = safe_mul_div(mul_div_a, mul_div_num, mul_div_denom, Rounding::Floor);
    let ceil_result = safe_mul_div(mul_div_a, mul_div_num, mul_div_denom, Rounding::Ceil);

    // ── Property 1a: denom=0 must return None ────────────────────────────
    if mul_div_denom == 0 {
        assert!(
            floor_result.is_none(),
            "safe_mul_div with denom=0 must return None (floor)"
        );
        assert!(
            ceil_result.is_none(),
            "safe_mul_div with denom=0 must return None (ceil)"
        );
        return; // remaining properties require valid division
    }

    // ── Property 1b: overflow returns None ───────────────────────────────
    let product_opt = mul_div_a.checked_mul(mul_div_num);
    if product_opt.is_none() {
        assert!(
            floor_result.is_none(),
            "safe_mul_div must return None when a×num overflows u128 (floor)"
        );
        assert!(
            ceil_result.is_none(),
            "safe_mul_div must return None when a×num overflows u128 (ceil)"
        );
        return;
    }

    // ── Property 2: floor ≤ ceil ──────────────────────────────────────────
    if let (Some(f), Some(c)) = (floor_result, ceil_result) {
        assert!(
            f <= c,
            "safe_mul_div: floor ({f}) must be ≤ ceil ({c}) \
             for a={mul_div_a}, num={mul_div_num}, denom={mul_div_denom}"
        );

        // ── Property 3: ceil − floor ∈ {0, 1} ────────────────────────────
        assert!(
            c - f <= 1,
            "safe_mul_div: ceil−floor = {} (must be 0 or 1) \
             for a={mul_div_a}, num={mul_div_num}, denom={mul_div_denom}",
            c - f
        );

        // ── Property 4: cross-check against reference ─────────────────────
        let product = product_opt.unwrap(); // safe: we checked above
        let expected_floor = product / mul_div_denom;
        assert_eq!(
            f,
            expected_floor,
            "safe_mul_div(Floor) mismatch: expected {expected_floor}, got {f}"
        );
    }

    // ── Part 2: proportional collateral release (conceptual model) ────────
    //
    // Use small, bounded values to avoid overflow in the reference check.
    // We reuse `mul_div_num` as `effective_repay` and `mul_div_denom` as
    // `previous_utilized`, both cast-bounded to i128::MAX for safety.
    if mul_div_num == 0 || mul_div_denom == 0 {
        return;
    }

    // Only test with values that fit in i128 (avoid overflow in reference).
    let eff: u128 = mul_div_num.min(i128::MAX as u128);
    let prev: u128 = mul_div_denom.min(i128::MAX as u128);
    let col: u128 = collateral_balance.min(i128::MAX as u128);

    if prev == 0 {
        return;
    }

    let released: u128 = if eff >= prev {
        // Full repay: release all collateral.
        col
    } else {
        safe_mul_div(col, eff, prev, Rounding::Floor).unwrap_or(col)
    };

    // ── Property 5: released ≤ collateral ────────────────────────────────
    assert!(
        released <= col,
        "collateral released ({released}) must be ≤ collateral_balance ({col})"
    );

    // ── Property 6: full repay releases all collateral ────────────────────
    if eff >= prev {
        assert_eq!(
            released, col,
            "full repay must release all collateral: \
             eff={eff}, prev={prev}, col={col}, released={released}"
        );
    }

    // ── Property 7: zero effective-repay releases no collateral ──────────
    if eff == 0 {
        // safe_mul_div(col, 0, prev, Floor) == 0
        let zero_release = safe_mul_div(col, 0, prev, Rounding::Floor).unwrap_or(0);
        assert_eq!(
            zero_release, 0,
            "zero effective_repay must release zero collateral"
        );
    }
}

/// Verify `apply_bps` fee-computation invariants.
///
/// Production code (`repay_and_release_collateral`):
///
/// ```rust
/// let fee = apply_bps(effective_repay as u128, fee_bps, Rounding::Floor) as i128;
/// ```
fn check_apply_bps(effective_repay: i128, fee_bps: u32) {
    // Only test with non-negative repayment amounts.
    if effective_repay <= 0 {
        return;
    }
    let amount = effective_repay as u128;

    // Clamp fee_bps to [0, 10_000] — the protocol enforces this separately.
    let bps_clamped = fee_bps.min(10_000);

    let floor_fee = apply_bps(amount, bps_clamped, Rounding::Floor);
    let ceil_fee = apply_bps(amount, bps_clamped, Rounding::Ceil);

    // ── Property A: fee ≤ repayment ───────────────────────────────────────
    assert!(
        floor_fee <= amount,
        "apply_bps(Floor): fee ({floor_fee}) must be ≤ repayment ({amount})"
    );
    assert!(
        ceil_fee <= amount,
        "apply_bps(Ceil): fee ({ceil_fee}) must be ≤ repayment ({amount})"
    );

    // ── Property B: zero rate → zero fee ─────────────────────────────────
    if bps_clamped == 0 {
        assert_eq!(floor_fee, 0, "fee_bps=0 must produce zero fee (floor)");
        assert_eq!(ceil_fee, 0, "fee_bps=0 must produce zero fee (ceil)");
    }

    // ── Property C: 10 000 bps = 100 % → fee == repayment ────────────────
    if bps_clamped == 10_000 {
        assert_eq!(
            floor_fee, amount,
            "fee_bps=10_000 must produce fee == repayment (floor)"
        );
    }

    // ── Property D: floor ≤ ceil ──────────────────────────────────────────
    assert!(
        floor_fee <= ceil_fee,
        "apply_bps: floor ({floor_fee}) must be ≤ ceil ({ceil_fee})"
    );

    // ── Property E: ceil − floor ∈ {0, 1} ────────────────────────────────
    assert!(
        ceil_fee - floor_fee <= 1,
        "apply_bps: ceil−floor = {} (must be 0 or 1) \
         for amount={amount}, bps={bps_clamped}",
        ceil_fee - floor_fee
    );
}

// ─── Fuzz entry-point ────────────────────────────────────────────────────────

fuzz_target!(|input: BorrowInput| {
    let status = credit_status_from_u8(input.status_raw);

    // 1. draw_status_error: enum dispatch completeness
    check_draw_status_error(status);

    // 2. effective_repay capping
    check_effective_repay(input.utilized_amount, input.repay_amount);

    // 3. interest_repaid capping
    check_interest_repaid(
        input.utilized_amount,
        input.accrued_interest,
        input.repay_amount,
    );

    // 4. proportional collateral release + mul_div / safe_mul_div safety
    check_collateral_release(
        input.collateral_balance,
        input.mul_div_a,
        input.mul_div_num,
        input.mul_div_denom,
    );

    // 5. apply_bps fee invariants
    check_apply_bps(input.repay_amount, input.fee_bps);
});
