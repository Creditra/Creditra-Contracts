// SPDX-License-Identifier: MIT

//! # Unit tests for `borrow` module pure-logic properties
//!
//! These tests cover the same invariants as the cargo-fuzz harness in
//! `contracts/borrow/fuzz/targets/main.rs` but are structured as ordinary
//! `#[test]` functions so they run under `cargo test`.
//!
//! ## Coverage targets
//!
//! | Property | Function | Coverage |
//! |---|---|---|
//! | `draw_status_error` enum dispatch | `test_draw_status_error_*` | all 5 variants |
//! | `effective_repay` capping | `test_effective_repay_*` | overpay, partial, exact, zero |
//! | `interest_repaid` capping | `test_interest_repaid_*` | sufficient, insufficient, zero |
//! | `mul_div` overflow safety | `test_mul_div_*` | edge values |
//! | `safe_mul_div` properties | `test_safe_mul_div_*` | None cases, floor/ceil |
//! | `apply_bps` invariants | `test_apply_bps_*` | 0 bps, 10k bps, typical |

use creditra_credit::borrow::draw_status_error;
use creditra_credit::math_utils::{apply_bps, mul_div, safe_mul_div, Rounding};
use creditra_credit::types::{ContractError, ContractErrorCategory, CreditStatus};

// ─── draw_status_error ───────────────────────────────────────────────────────

/// Active status must allow draws (return None).
#[test]
fn test_draw_status_error_active_is_none() {
    assert_eq!(draw_status_error(CreditStatus::Active), None);
}

/// Restricted status must allow draws (return None) — borrower repays to cure.
#[test]
fn test_draw_status_error_restricted_is_none() {
    assert_eq!(draw_status_error(CreditStatus::Restricted), None);
}

/// Suspended status must block draws with `CreditLineSuspended`.
#[test]
fn test_draw_status_error_suspended() {
    let result = draw_status_error(CreditStatus::Suspended);
    assert_eq!(result, Some(ContractError::CreditLineSuspended));
    assert_eq!(result.unwrap().category(), ContractErrorCategory::Lifecycle);
}

/// Defaulted status must block draws with `CreditLineDefaulted`.
#[test]
fn test_draw_status_error_defaulted() {
    let result = draw_status_error(CreditStatus::Defaulted);
    assert_eq!(result, Some(ContractError::CreditLineDefaulted));
    assert_eq!(result.unwrap().category(), ContractErrorCategory::Lifecycle);
}

/// Closed status must block draws with `CreditLineClosed`.
#[test]
fn test_draw_status_error_closed() {
    let result = draw_status_error(CreditStatus::Closed);
    assert_eq!(result, Some(ContractError::CreditLineClosed));
    assert_eq!(result.unwrap().category(), ContractErrorCategory::Lifecycle);
}

/// All five status variants are tested and produce a Lifecycle error or None.
#[test]
fn test_draw_status_error_all_variants_lifecycle_or_none() {
    let variants = [
        CreditStatus::Active,
        CreditStatus::Restricted,
        CreditStatus::Suspended,
        CreditStatus::Defaulted,
        CreditStatus::Closed,
    ];
    for status in variants {
        let result = draw_status_error(status);
        if let Some(err) = result {
            assert_eq!(
                err.category(),
                ContractErrorCategory::Lifecycle,
                "unexpected non-Lifecycle error for status {:?}: {:?}",
                status,
                err
            );
        }
    }
}

// ─── effective_repay capping (mirrors repay_credit / repay_and_release) ──────

/// Helper that reproduces the production capping logic:
///
/// ```rust
/// let effective_repay = if amount > credit_line.utilized_amount {
///     credit_line.utilized_amount
/// } else { amount };
/// ```
fn compute_effective_repay(utilized: i128, amount: i128) -> i128 {
    if amount > utilized { utilized } else { amount }
}

/// Overpayment: amount > utilized → clamp to utilized.
#[test]
fn test_effective_repay_overpayment_clamps() {
    let utilized = 1_000_i128;
    let amount = 5_000_i128;
    let eff = compute_effective_repay(utilized, amount);
    assert_eq!(eff, utilized, "overpayment must clamp to utilized");
}

/// Partial repayment: amount < utilized → pass-through.
#[test]
fn test_effective_repay_partial_passthrough() {
    let utilized = 1_000_i128;
    let amount = 300_i128;
    let eff = compute_effective_repay(utilized, amount);
    assert_eq!(eff, amount, "partial repay must equal amount");
}

/// Exact repayment: amount == utilized → no clamping.
#[test]
fn test_effective_repay_exact_equals_utilized() {
    let utilized = 500_i128;
    let eff = compute_effective_repay(utilized, utilized);
    assert_eq!(eff, utilized);
}

/// Zero utilized → effective repay is always 0.
#[test]
fn test_effective_repay_zero_utilized() {
    let eff = compute_effective_repay(0, 9_999);
    assert_eq!(eff, 0);
}

/// effective_repay is always ≤ utilized_amount for any i128 pair.
#[test]
fn test_effective_repay_never_exceeds_utilized() {
    let cases: &[(i128, i128)] = &[
        (0, 0),
        (1, 1_000_000),
        (i128::MAX, i128::MAX),
        (i128::MAX, 1),
        (100, -1),
    ];
    for &(utilized, amount) in cases {
        if utilized < 0 || amount < 0 {
            continue; // production guards amount > 0 before capping
        }
        let eff = compute_effective_repay(utilized, amount);
        assert!(
            eff <= utilized,
            "eff ({eff}) must be ≤ utilized ({utilized}), amount={amount}"
        );
        assert!(eff >= 0, "eff must be non-negative");
    }
}

// ─── interest_repaid capping ─────────────────────────────────────────────────

/// Helper mirroring the production formula:
///
/// ```rust
/// let interest_repaid = effective_repay.min(credit_line.accrued_interest);
/// ```
fn compute_interest_repaid(effective_repay: i128, accrued_interest: i128) -> i128 {
    effective_repay.min(accrued_interest.max(0))
}

/// When effective_repay ≥ accrued_interest, all interest is repaid.
#[test]
fn test_interest_repaid_full_when_sufficient() {
    let accrued = 200_i128;
    let effective = 1_000_i128;
    assert_eq!(compute_interest_repaid(effective, accrued), accrued);
}

/// When effective_repay < accrued_interest, interest_repaid is capped.
#[test]
fn test_interest_repaid_capped_when_insufficient() {
    let accrued = 1_000_i128;
    let effective = 300_i128;
    assert_eq!(compute_interest_repaid(effective, accrued), effective);
}

/// Zero accrued interest → zero interest repaid regardless of payment.
#[test]
fn test_interest_repaid_zero_accrued() {
    assert_eq!(compute_interest_repaid(500, 0), 0);
}

/// Negative accrued interest (shouldn't happen in production, but must be safe).
#[test]
fn test_interest_repaid_negative_accrued_clamped_to_zero() {
    assert_eq!(compute_interest_repaid(500, -100), 0);
}

/// interest_repaid is always ≤ accrued_interest and ≤ effective_repay.
#[test]
fn test_interest_repaid_bounds_invariant() {
    let cases: &[(i128, i128, i128)] = &[
        (1_000, 200, 300),
        (1_000, 2_000, 500),
        (0, 0, 0),
        (i128::MAX, i128::MAX / 2, i128::MAX / 4),
    ];
    for &(utilized, accrued, amount) in cases {
        if utilized <= 0 || amount <= 0 {
            continue;
        }
        let eff = compute_effective_repay(utilized, amount);
        let int_rep = compute_interest_repaid(eff, accrued);
        assert!(int_rep <= accrued.max(0), "interest_repaid must be ≤ accrued");
        assert!(int_rep <= eff, "interest_repaid must be ≤ effective_repay");
        assert!(int_rep >= 0, "interest_repaid must be non-negative");
    }
}

// ─── safe_mul_div properties ─────────────────────────────────────────────────

/// Division by zero must return None.
#[test]
fn test_safe_mul_div_zero_denom_is_none() {
    assert_eq!(safe_mul_div(100, 3, 0, Rounding::Floor), None);
    assert_eq!(safe_mul_div(100, 3, 0, Rounding::Ceil), None);
}

/// Overflow returns None.
#[test]
fn test_safe_mul_div_overflow_is_none() {
    assert_eq!(safe_mul_div(u128::MAX, 2, 1, Rounding::Floor), None);
}

/// floor ≤ ceil.
#[test]
fn test_safe_mul_div_floor_le_ceil() {
    let floor = safe_mul_div(1001, 3, 10, Rounding::Floor).unwrap();
    let ceil = safe_mul_div(1001, 3, 10, Rounding::Ceil).unwrap();
    assert!(floor <= ceil);
    assert!(ceil - floor <= 1);
}

/// Exact division: floor == ceil.
#[test]
fn test_safe_mul_div_exact_division() {
    let floor = safe_mul_div(1000, 3, 10, Rounding::Floor).unwrap();
    let ceil = safe_mul_div(1000, 3, 10, Rounding::Ceil).unwrap();
    assert_eq!(floor, 300);
    assert_eq!(ceil, 300);
    assert_eq!(floor, ceil);
}

/// zero numerator always yields 0.
#[test]
fn test_safe_mul_div_zero_num() {
    assert_eq!(safe_mul_div(1000, 0, 7, Rounding::Floor), Some(0));
    assert_eq!(safe_mul_div(1000, 0, 7, Rounding::Ceil), Some(0));
}

/// zero a always yields 0.
#[test]
fn test_safe_mul_div_zero_a() {
    assert_eq!(safe_mul_div(0, 100, 7, Rounding::Floor), Some(0));
}

// ─── mul_div (panicking variant) ─────────────────────────────────────────────

/// Standard floor division.
#[test]
fn test_mul_div_floor() {
    assert_eq!(mul_div(1_000, 3, 10, Rounding::Floor), 300);
}

/// Ceiling division with remainder.
#[test]
fn test_mul_div_ceil_with_remainder() {
    assert_eq!(mul_div(1_001, 3, 10, Rounding::Ceil), 301);
}

/// mul_div result is consistent with safe_mul_div when no overflow.
#[test]
fn test_mul_div_matches_safe_mul_div() {
    let a = 99_999_u128;
    let num = 7_u128;
    let denom = 13_u128;
    let expected = safe_mul_div(a, num, denom, Rounding::Floor).unwrap();
    assert_eq!(mul_div(a, num, denom, Rounding::Floor), expected);
}

// ─── apply_bps fee invariants ─────────────────────────────────────────────────

/// Zero bps → zero fee.
#[test]
fn test_apply_bps_zero_rate() {
    assert_eq!(apply_bps(1_000_000, 0, Rounding::Floor), 0);
    assert_eq!(apply_bps(1_000_000, 0, Rounding::Ceil), 0);
}

/// 10 000 bps = 100 % → fee equals amount.
#[test]
fn test_apply_bps_full_rate() {
    let amount = 500_u128;
    assert_eq!(apply_bps(amount, 10_000, Rounding::Floor), amount);
}

/// 300 bps = 3 % on 10 000 tokens = 300 tokens.
#[test]
fn test_apply_bps_typical_rate() {
    assert_eq!(apply_bps(10_000, 300, Rounding::Floor), 300);
}

/// floor ≤ ceil and ceil − floor ∈ {0, 1}.
#[test]
fn test_apply_bps_floor_le_ceil() {
    let amounts = [1_u128, 99, 1_000, 9_999, 100_000, u128::MAX / 10_001];
    let bps_rates = [1_u32, 50, 300, 999, 5_000, 9_999, 10_000];
    for &amount in &amounts {
        for &bps in &bps_rates {
            let floor = apply_bps(amount, bps, Rounding::Floor);
            let ceil = apply_bps(amount, bps, Rounding::Ceil);
            assert!(
                floor <= ceil,
                "floor ({floor}) > ceil ({ceil}) for amount={amount}, bps={bps}"
            );
            assert!(
                ceil - floor <= 1,
                "ceil−floor = {} (expected ≤ 1) for amount={amount}, bps={bps}",
                ceil - floor
            );
        }
    }
}

/// fee ≤ repayment for any bps ≤ 10_000.
#[test]
fn test_apply_bps_fee_never_exceeds_amount() {
    let amounts = [1_u128, 1_000, 1_000_000, i128::MAX as u128 / 10_001];
    for &amount in &amounts {
        for bps in 0_u32..=10_000 {
            let fee = apply_bps(amount, bps, Rounding::Floor);
            assert!(fee <= amount, "fee ({fee}) > amount ({amount}) at bps={bps}");
        }
    }
}

// ─── Collateral release formula ───────────────────────────────────────────────

/// Helper that mirrors the production collateral release formula.
fn compute_released(collateral: u128, effective_repay: u128, previous_utilized: u128) -> u128 {
    if effective_repay >= previous_utilized {
        collateral
    } else {
        safe_mul_div(collateral, effective_repay, previous_utilized, Rounding::Floor)
            .unwrap_or(collateral)
    }
}

/// Full repay releases all collateral.
#[test]
fn test_collateral_release_full_repay() {
    let col = 5_000_u128;
    let utilized = 1_000_u128;
    let released = compute_released(col, utilized, utilized); // exact = full
    assert_eq!(released, col);
}

/// Overpayment also releases all collateral.
#[test]
fn test_collateral_release_overpay() {
    let col = 5_000_u128;
    let utilized = 1_000_u128;
    let released = compute_released(col, utilized + 500, utilized);
    assert_eq!(released, col);
}

/// Partial repay releases proportional collateral ≤ total.
#[test]
fn test_collateral_release_partial_proportional() {
    let col = 10_000_u128;
    let utilized = 1_000_u128;
    let effective = 250_u128; // 25%
    let released = compute_released(col, effective, utilized);
    assert_eq!(released, 2_500); // 25% of 10_000
    assert!(released <= col);
}

/// Zero effective_repay releases no collateral.
#[test]
fn test_collateral_release_zero_effective() {
    let col = 5_000_u128;
    let released = compute_released(col, 0, 1_000);
    assert_eq!(released, 0);
}

/// released is always ≤ collateral for any (col, eff, prev) tuple.
#[test]
fn test_collateral_release_never_exceeds_balance() {
    let cases: &[(u128, u128, u128)] = &[
        (10_000, 500, 1_000),
        (10_000, 1_000, 1_000),
        (10_000, 2_000, 1_000),
        (u128::MAX / 2, u128::MAX / 4, u128::MAX / 3),
        (0, 100, 200),
    ];
    for &(col, eff, prev) in cases {
        if prev == 0 {
            continue;
        }
        let released = compute_released(col, eff, prev);
        assert!(
            released <= col,
            "released ({released}) > collateral ({col}): eff={eff}, prev={prev}"
        );
    }
}
