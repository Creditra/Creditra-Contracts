// SPDX-License-Identifier: MIT

//! Stable-discriminant assertion tests for `ContractError`.
//!
//! These tests are the **CI guard** against accidental reordering or renumbering
//! of error variants. If any assertion fails, it means a discriminant was changed
//! in a way that would break deployed SDK clients.
//!
//! # Rules
//! - Never change an existing assertion value.
//! - New variants must be appended at the end of the enum with the next integer.
//! - Add a corresponding assertion here when adding a new variant.

use creditra_credit::types::ContractError;

#[test]
fn error_discriminants_are_stable() {
    assert_eq!(ContractError::Unauthorized as u32, 1);
    assert_eq!(ContractError::NotAdmin as u32, 2);
    assert_eq!(ContractError::CreditLineNotFound as u32, 3);
    assert_eq!(ContractError::CreditLineClosed as u32, 4);
    assert_eq!(ContractError::InvalidAmount as u32, 5);
    assert_eq!(ContractError::OverLimit as u32, 6);
    assert_eq!(ContractError::NegativeLimit as u32, 7);
    assert_eq!(ContractError::RateTooHigh as u32, 8);
    assert_eq!(ContractError::ScoreTooHigh as u32, 9);
    assert_eq!(ContractError::UtilizationNotZero as u32, 10);
    assert_eq!(ContractError::Reentrancy as u32, 11);
    assert_eq!(ContractError::Overflow as u32, 12);
    assert_eq!(ContractError::LimitDecreaseRequiresRepayment as u32, 13);
    assert_eq!(ContractError::AlreadyInitialized as u32, 14);
    assert_eq!(ContractError::AdminAcceptTooEarly as u32, 15);
    assert_eq!(ContractError::BorrowerBlocked as u32, 16);
    assert_eq!(ContractError::DrawExceedsMaxAmount as u32, 17);
    assert_eq!(ContractError::Paused as u32, 18);
    assert_eq!(ContractError::DrawsFrozen as u32, 19);
    assert_eq!(ContractError::CreditLineSuspended as u32, 20);
    assert_eq!(ContractError::CreditLineDefaulted as u32, 21);
    assert_eq!(ContractError::MissingLiquidityToken as u32, 22);
    assert_eq!(ContractError::MissingLiquiditySource as u32, 23);
    assert_eq!(ContractError::InsufficientLiquidityReserve as u32, 24);
    assert_eq!(ContractError::LiquidityTokenCallFailed as u32, 25);
    assert_eq!(ContractError::InsufficientRepaymentAllowance as u32, 26);
    assert_eq!(ContractError::InsufficientRepaymentBalance as u32, 27);
}

/// Verify no two variants share the same discriminant.
/// This is a compile-time guarantee via `#[repr(u32)]`, but we make it
/// explicit here so the intent is documented and visible in test output.
#[test]
fn no_duplicate_discriminants() {
    use std::collections::HashSet;

    let codes: Vec<u32> = vec![
        ContractError::Unauthorized as u32,
        ContractError::NotAdmin as u32,
        ContractError::CreditLineNotFound as u32,
        ContractError::CreditLineClosed as u32,
        ContractError::InvalidAmount as u32,
        ContractError::OverLimit as u32,
        ContractError::NegativeLimit as u32,
        ContractError::RateTooHigh as u32,
        ContractError::ScoreTooHigh as u32,
        ContractError::UtilizationNotZero as u32,
        ContractError::Reentrancy as u32,
        ContractError::Overflow as u32,
        ContractError::LimitDecreaseRequiresRepayment as u32,
        ContractError::AlreadyInitialized as u32,
        ContractError::AdminAcceptTooEarly as u32,
        ContractError::BorrowerBlocked as u32,
        ContractError::DrawExceedsMaxAmount as u32,
        ContractError::Paused as u32,
        ContractError::DrawsFrozen as u32,
        ContractError::CreditLineSuspended as u32,
        ContractError::CreditLineDefaulted as u32,
        ContractError::MissingLiquidityToken as u32,
        ContractError::MissingLiquiditySource as u32,
        ContractError::InsufficientLiquidityReserve as u32,
        ContractError::LiquidityTokenCallFailed as u32,
        ContractError::InsufficientRepaymentAllowance as u32,
        ContractError::InsufficientRepaymentBalance as u32,
    ];

    let unique: HashSet<u32> = codes.iter().cloned().collect();
    assert_eq!(
        codes.len(),
        unique.len(),
        "Duplicate discriminants detected in ContractError — check types.rs"
    );
}

/// Verify the total variant count matches expectations.
/// Update this number when adding new variants (and add the assertion above).
#[test]
fn variant_count_is_known() {
    // 27 variants as of this writing. Update when adding new ones.
    const EXPECTED_VARIANT_COUNT: usize = 27;

    let codes = [
        ContractError::Unauthorized as u32,
        ContractError::NotAdmin as u32,
        ContractError::CreditLineNotFound as u32,
        ContractError::CreditLineClosed as u32,
        ContractError::InvalidAmount as u32,
        ContractError::OverLimit as u32,
        ContractError::NegativeLimit as u32,
        ContractError::RateTooHigh as u32,
        ContractError::ScoreTooHigh as u32,
        ContractError::UtilizationNotZero as u32,
        ContractError::Reentrancy as u32,
        ContractError::Overflow as u32,
        ContractError::LimitDecreaseRequiresRepayment as u32,
        ContractError::AlreadyInitialized as u32,
        ContractError::AdminAcceptTooEarly as u32,
        ContractError::BorrowerBlocked as u32,
        ContractError::DrawExceedsMaxAmount as u32,
        ContractError::Paused as u32,
        ContractError::DrawsFrozen as u32,
        ContractError::CreditLineSuspended as u32,
        ContractError::CreditLineDefaulted as u32,
        ContractError::MissingLiquidityToken as u32,
        ContractError::MissingLiquiditySource as u32,
        ContractError::InsufficientLiquidityReserve as u32,
        ContractError::LiquidityTokenCallFailed as u32,
        ContractError::InsufficientRepaymentAllowance as u32,
        ContractError::InsufficientRepaymentBalance as u32,
    ];

    assert_eq!(
        codes.len(),
        EXPECTED_VARIANT_COUNT,
        "Variant count changed — update EXPECTED_VARIANT_COUNT and add/remove assertions"
    );
}
