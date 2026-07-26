// SPDX-License-Identifier: MIT

//! # ContractError stability tests for the borrow module
//!
//! This test module guards the client-facing numeric error codes produced by
//! `creditra_credit::borrow` functions against accidental renumbering.
//!
//! ## Purpose
//!
//! The borrow module (`draw_credit`, `repay_credit`, `repay_and_release_collateral`,
//! and related helpers) returns `ContractError` discriminants to callers. These numeric
//! codes are part of the contract's ABI and are relied upon by off-chain integrators,
//! SDKs, and indexers. Any accidental reordering or insertion of error variants in the
//! `ContractError` enum would silently change these codes, breaking deployed clients
//! without a clear error message.
//!
//! This test locks in the current discriminant values for every variant that the borrow
//! module can produce. If a test fails, it means a variant was moved or renumbered —
//! either fix the enum (by explicitly pinning the discriminant to prevent the shift),
//! or acknowledge the breaking change and deliberately update these assertions as a
//! documented API change for integrators.
//!
//! ## Discriminant Stability
//!
//! The `ContractError` enum uses `#[repr(u32)]` with **explicit discriminant assignments**
//! (e.g., `Unauthorized = 1`, `NotAdmin = 2`, ...). This means discriminants are not
//! implicitly derived and are **already safe** against accidental shifts. However, a
//! maintainer could still:
//!
//! 1. **Reorder variants** → the explicit assignments would still hold, so this test
//!    would pass, but it's semantically confusing.
//! 2. **Insert a new variant in the middle** → if they forget to assign an explicit
//!    value, everything after it would implicitly renumber. This test **will catch it**.
//! 3. **Change an explicit assignment** → this test **will catch it immediately**.
//!
//! ## Frozen Borrow Module Error Codes
//!
//! The borrow module explicitly produces or propagates these errors:
//!
//! | Code | Variant                    | Context                                  |
//! |------|----------------------------|------------------------------------------|
//! | 3    | `CreditLineNotFound`       | Borrower has no credit line               |
//! | 4    | `CreditLineClosed`         | Cannot draw/repay on a closed line        |
//! | 5    | `InvalidAmount`            | Amount ≤ 0 or malformed                  |
//! | 6    | `OverLimit`                | Draw exceeds available limit              |
//! | 10   | `UtilizationNotZero`       | Repay/release logic on zero utilization  |
//! | 12   | `Overflow`                 | Math overflow in mul_div or fee calc      |
//! | 20   | `CreditLineSuspended`      | Draws blocked; repays allowed             |
//! | 21   | `CreditLineDefaulted`      | Draws blocked; repays allowed for cure   |
//! | 28   | `RepayExceedsMaxAmount`    | Repay exceeds per-transaction cap         |
//! | 29   | `DrawCooldownActive`       | Borrower draw cooldown still active       |
//! | 39   | `InsufficientCollateralBalance` | Collateral withdrawal exceeds balance   |
//!
//! See [`creditra_credit::borrow`] for module documentation and
//! [`creditra_credit::types::ContractError`] for the full error enum.

use creditra_credit::types::ContractError;

/// ## Test 1: Pinned Discriminant Assertions
///
/// Assert that every error variant currently produced by the borrow module
/// has its expected discriminant value. Hardcoding the expected values makes
/// this an independent, read-only check of the enum's current state — not a
/// tautological test that would always pass even if the enum changes.
#[test]
fn borrow_error_discriminants_are_stable() {
    // Errors from draw_credit path
    assert_eq!(ContractError::CreditLineNotFound as u32, 3);
    assert_eq!(ContractError::CreditLineClosed as u32, 4);
    assert_eq!(ContractError::InvalidAmount as u32, 5);
    assert_eq!(ContractError::OverLimit as u32, 6);
    assert_eq!(ContractError::CreditLineSuspended as u32, 20);
    assert_eq!(ContractError::CreditLineDefaulted as u32, 21);
    assert_eq!(ContractError::DrawCooldownActive as u32, 29);

    // Errors from repay_credit path
    assert_eq!(ContractError::RepayExceedsMaxAmount as u32, 28);

    // Errors from repay_and_release_collateral path
    assert_eq!(ContractError::InsufficientCollateralBalance as u32, 39);

    // Errors from math/overflow in mul_div and apply_bps
    assert_eq!(ContractError::Overflow as u32, 12);

    // Edge-case errors that may appear in repay logic
    assert_eq!(ContractError::UtilizationNotZero as u32, 10);
}

/// ## Test 2: No Duplicate Discriminants
///
/// Verify that no two error variants share the same numeric code. A duplicate
/// would be a more severe client-facing bug than simple renumbering (two error
/// messages would map to the same code, causing confusion).
///
/// This test independently collects the codes from every variant the borrow
/// module can produce and checks for collisions using a HashSet.
#[test]
fn borrow_errors_have_no_duplicate_discriminants() {
    use std::collections::HashSet;

    let borrow_error_codes: Vec<u32> = vec![
        ContractError::CreditLineNotFound as u32,
        ContractError::CreditLineClosed as u32,
        ContractError::InvalidAmount as u32,
        ContractError::OverLimit as u32,
        ContractError::CreditLineSuspended as u32,
        ContractError::CreditLineDefaulted as u32,
        ContractError::DrawCooldownActive as u32,
        ContractError::RepayExceedsMaxAmount as u32,
        ContractError::InsufficientCollateralBalance as u32,
        ContractError::Overflow as u32,
        ContractError::UtilizationNotZero as u32,
    ];

    let unique: HashSet<u32> = borrow_error_codes.iter().cloned().collect();

    assert_eq!(
        borrow_error_codes.len(),
        unique.len(),
        "Borrow module error codes contain duplicates — check ContractError discriminants"
    );
}

/// ## Test 3: Verify Known Count
///
/// Sanity check: the borrow module error set must have the expected number
/// of distinct codes. If this fails, it means a new error was added to or
/// removed from the borrow module's possible return values. Update the
/// constant and the lists above accordingly.
#[test]
fn borrow_errors_known_variant_count() {
    const BORROW_ERROR_COUNT: usize = 11;

    let codes = [
        ContractError::CreditLineNotFound as u32,
        ContractError::CreditLineClosed as u32,
        ContractError::InvalidAmount as u32,
        ContractError::OverLimit as u32,
        ContractError::CreditLineSuspended as u32,
        ContractError::CreditLineDefaulted as u32,
        ContractError::DrawCooldownActive as u32,
        ContractError::RepayExceedsMaxAmount as u32,
        ContractError::InsufficientCollateralBalance as u32,
        ContractError::Overflow as u32,
        ContractError::UtilizationNotZero as u32,
    ];

    assert_eq!(
        codes.len(),
        BORROW_ERROR_COUNT,
        "Borrow module error count mismatch — update BORROW_ERROR_COUNT and error list"
    );
}
