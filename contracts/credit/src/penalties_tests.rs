// SPDX-License-Identifier: MIT

//! Unit tests for [`crate::penalties::compute_late_fee`].
//!
//! These tests exercise both the [`crate::penalties::LateFeeConfig::Flat`]
//! (new in issue #604) and [`crate::penalties::LateFeeConfig::AprBased`]
//! (preserved existing behaviour) variants, boundary values, overflow
//! protection, and cross-mode independence.

#![cfg(test)]

use crate::penalties::{compute_late_fee, AprFeeConfig, FlatFeeConfig, LateFeeConfig};
use crate::types::ContractError;

// ── Flat surcharge mode ──────────────────────────────────────────────────────

#[test]
fn flat_single_installment() {
    let fee = compute_late_fee(LateFeeConfig::Flat(FlatFeeConfig { amount: 50 }), 1).unwrap();
    assert_eq!(fee, 50);
}

#[test]
fn flat_multiple_installments() {
    let fee = compute_late_fee(LateFeeConfig::Flat(FlatFeeConfig { amount: 50 }), 3).unwrap();
    assert_eq!(fee, 150);
}

#[test]
fn flat_zero_amount_is_noop() {
    let fee = compute_late_fee(LateFeeConfig::Flat(FlatFeeConfig { amount: 0 }), 5).unwrap();
    assert_eq!(fee, 0);
}

#[test]
fn flat_large_amount() {
    // 1_000_000 tokens × 100 installments = 100_000_000
    let fee =
        compute_late_fee(LateFeeConfig::Flat(FlatFeeConfig { amount: 1_000_000 }), 100).unwrap();
    assert_eq!(fee, 100_000_000);
}

#[test]
fn flat_zero_missed_installments_returns_zero() {
    let fee = compute_late_fee(LateFeeConfig::Flat(FlatFeeConfig { amount: 999 }), 0).unwrap();
    assert_eq!(fee, 0);
}

#[test]
fn flat_negative_amount_returns_invalid_amount() {
    let err = compute_late_fee(LateFeeConfig::Flat(FlatFeeConfig { amount: -1 }), 1).unwrap_err();
    assert_eq!(err, ContractError::InvalidAmount);
}

#[test]
fn flat_negative_amount_with_zero_missed_is_ok() {
    // Short-circuits at missed_installments == 0 before inspecting amount.
    let fee = compute_late_fee(LateFeeConfig::Flat(FlatFeeConfig { amount: -1 }), 0).unwrap();
    assert_eq!(fee, 0);
}

#[test]
fn flat_max_i128_overflow_returns_overflow() {
    // amount × count overflows i128
    let err = compute_late_fee(
        LateFeeConfig::Flat(FlatFeeConfig { amount: i128::MAX }),
        2,
    )
    .unwrap_err();
    assert_eq!(err, ContractError::Overflow);
}

#[test]
fn flat_boundary_one_token_one_installment() {
    let fee = compute_late_fee(LateFeeConfig::Flat(FlatFeeConfig { amount: 1 }), 1).unwrap();
    assert_eq!(fee, 1);
}

#[test]
fn flat_boundary_max_safe_multiplication() {
    // i128::MAX / 2 × 2 should not overflow
    let half = i128::MAX / 2;
    let fee = compute_late_fee(LateFeeConfig::Flat(FlatFeeConfig { amount: half }), 2).unwrap();
    assert_eq!(fee, half * 2);
}

// ── APR-based mode (existing behaviour preserved) ────────────────────────────

#[test]
fn apr_always_returns_zero_for_any_installments() {
    let fee =
        compute_late_fee(LateFeeConfig::AprBased(AprFeeConfig { surcharge_bps: 200 }), 5).unwrap();
    // APR surcharge is handled by crate::accrual, not here.
    assert_eq!(fee, 0);
}

#[test]
fn apr_zero_surcharge_returns_zero() {
    let fee =
        compute_late_fee(LateFeeConfig::AprBased(AprFeeConfig { surcharge_bps: 0 }), 10).unwrap();
    assert_eq!(fee, 0);
}

#[test]
fn apr_max_surcharge_returns_zero() {
    let fee = compute_late_fee(
        LateFeeConfig::AprBased(AprFeeConfig {
            surcharge_bps: 10_000,
        }),
        100,
    )
    .unwrap();
    assert_eq!(fee, 0);
}

#[test]
fn apr_zero_missed_installments_returns_zero() {
    let fee =
        compute_late_fee(LateFeeConfig::AprBased(AprFeeConfig { surcharge_bps: 500 }), 0).unwrap();
    assert_eq!(fee, 0);
}

// ── Cross-mode independence ───────────────────────────────────────────────────

#[test]
fn flat_and_apr_produce_different_results() {
    let flat_fee = compute_late_fee(LateFeeConfig::Flat(FlatFeeConfig { amount: 100 }), 3).unwrap();
    let apr_fee =
        compute_late_fee(LateFeeConfig::AprBased(AprFeeConfig { surcharge_bps: 500 }), 3).unwrap();
    assert_eq!(flat_fee, 300);
    assert_eq!(apr_fee, 0);
    assert_ne!(flat_fee, apr_fee);
}

#[test]
fn switching_config_from_apr_to_flat_does_not_carry_state() {
    // Pure functions — no state carried between calls.
    let apr = compute_late_fee(
        LateFeeConfig::AprBased(AprFeeConfig { surcharge_bps: 9_999 }),
        10,
    )
    .unwrap();
    let flat = compute_late_fee(LateFeeConfig::Flat(FlatFeeConfig { amount: 7 }), 10).unwrap();
    assert_eq!(apr, 0);
    assert_eq!(flat, 70);
}
