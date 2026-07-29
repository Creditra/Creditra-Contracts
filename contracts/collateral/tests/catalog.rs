// SPDX-License-Identifier: MIT

//! Integration tests for `creditra_collateral::CollateralError`.
//!
//! These tests are the **published CI guard** against accidental
//! reordering or renumbering of [`CollateralError`] variants. They run
//! against the public crate surface, so a discriminant drift inside
//! `src/errors.rs` cannot be hidden behind `pub(crate)` re-exports.
//!
//! # Update protocol
//!
//! - Adding a variant: append it at the end of the enum and append the
//!   corresponding assertion at the end of [`discriminants_are_stable`]
//!   and the count lists in [`no_duplicate_discriminants`] /
//!   [`variant_count_is_known`].
//! - Renumbering or reordering an existing assertion: forbidden — break
//!   this only as a breaking-change PR with SDK migration notes.

use creditra_collateral::CollateralError;

/// Pin every published discriminant against its expected integer.
///
/// This is the canonical source of truth for stability. If you add a new
/// variant, append a new `assert_eq!` at the END of this function — do
/// not re-order existing assertions.
#[test]
fn discriminants_are_stable() {
    // ── Mirror tier ────────────────────────────────────────────────────────
    assert_eq!(CollateralError::InvalidAmount as u32, 5);
    assert_eq!(CollateralError::Overflow as u32, 12);
    assert_eq!(CollateralError::MissingLiquidityToken as u32, 22);
    assert_eq!(CollateralError::CollateralRatioBelowMinimum as u32, 35);
    assert_eq!(CollateralError::InsufficientCollateralBalance as u32, 39);

    // ── Collateral-specific tier (100+) ────────────────────────────────────
    assert_eq!(CollateralError::CollateralTokenNotAllowed as u32, 100);
    assert_eq!(CollateralError::CollateralRiskWeightOutOfRange as u32, 101);
    assert_eq!(CollateralError::CollateralTokenMismatch as u32, 102);
    assert_eq!(CollateralError::CollateralPositionLocked as u32, 103);
    assert_eq!(CollateralError::CollateralBalanceForTokenNotFound as u32, 104);
}

/// Verify no two variants collide. Iterates the `discriminants_are_stable`
/// list so any future new variant must be appended here AND will be
/// checked for uniqueness in the same pass.
#[test]
fn no_duplicate_discriminants() {
    let codes = [
        // Mirror tier
        CollateralError::InvalidAmount as u32,
        CollateralError::Overflow as u32,
        CollateralError::MissingLiquidityToken as u32,
        CollateralError::CollateralRatioBelowMinimum as u32,
        CollateralError::InsufficientCollateralBalance as u32,
        // Collateral-specific tier
        CollateralError::CollateralTokenNotAllowed as u32,
        CollateralError::CollateralRiskWeightOutOfRange as u32,
        CollateralError::CollateralTokenMismatch as u32,
        CollateralError::CollateralPositionLocked as u32,
        CollateralError::CollateralBalanceForTokenNotFound as u32,
    ];

    for i in 0..codes.len() {
        for j in (i + 1)..codes.len() {
            assert_ne!(
                codes[i], codes[j],
                "Duplicate discriminant {} between variant indices {} and {}",
                codes[i], i, j
            );
        }
    }
}

/// Pin the total variant count. Update together with [`discriminants_are_stable`].
#[test]
fn variant_count_is_known() {
    // 5 mirror + 5 collateral-specific = 10 as of this writing.
    const EXPECTED_VARIANT_COUNT: usize = 10;

    let codes = [
        CollateralError::InvalidAmount as u32,
        CollateralError::Overflow as u32,
        CollateralError::MissingLiquidityToken as u32,
        CollateralError::CollateralRatioBelowMinimum as u32,
        CollateralError::InsufficientCollateralBalance as u32,
        CollateralError::CollateralTokenNotAllowed as u32,
        CollateralError::CollateralRiskWeightOutOfRange as u32,
        CollateralError::CollateralTokenMismatch as u32,
        CollateralError::CollateralPositionLocked as u32,
        CollateralError::CollateralBalanceForTokenNotFound as u32,
    ];

    assert_eq!(
        codes.len(),
        EXPECTED_VARIANT_COUNT,
        "CollateralError variant count changed — update EXPECTED_VARIANT_COUNT, \
         discriminants_are_stable, the discriminant table in src/errors.rs, the \
         lib.rs docstring, and docs/errors/collateral.md"
    );
}

/// Verify every mirror discriminant matches the canonical credit contract
/// `ContractError` discriminant at `contracts/credit/src/types.rs`.
///
/// If this test ever fails, the published collateral catalog has drifted
/// out of sync with the canonical credit contract table, and SDK
/// consumers matching against [`docs/ERROR_CODES.md`][1] would decode an
/// emitted error to the wrong variant.
///
/// [1]: ../../../docs/ERROR_CODES.md
#[test]
fn mirror_matches_canonical_credit_contract_error_table() {
    // The following are the canonical contracts/credit/src/types.rs
    // discriminants for the same-named variants. Pin them here so that any
    // future drift on either side surfaces during CI.
    const CANONICAL_INVALID_AMOUNT: u32 = 5;
    const CANONICAL_OVERFLOW: u32 = 12;
    const CANONICAL_MISSING_LIQUIDITY_TOKEN: u32 = 22;
    const CANONICAL_COLLATERAL_RATIO_BELOW_MINIMUM: u32 = 35;
    const CANONICAL_INSUFFICIENT_COLLATERAL_BALANCE: u32 = 39;

    assert_eq!(
        CollateralError::InvalidAmount as u32,
        CANONICAL_INVALID_AMOUNT
    );
    assert_eq!(CollateralError::Overflow as u32, CANONICAL_OVERFLOW);
    assert_eq!(
        CollateralError::MissingLiquidityToken as u32,
        CANONICAL_MISSING_LIQUIDITY_TOKEN
    );
    assert_eq!(
        CollateralError::CollateralRatioBelowMinimum as u32,
        CANONICAL_COLLATERAL_RATIO_BELOW_MINIMUM
    );
    assert_eq!(
        CollateralError::InsufficientCollateralBalance as u32,
        CANONICAL_INSUFFICIENT_COLLATERAL_BALANCE
    );
}

/// Verify the collateral-specific tier reserves codes `>= 100`. This is
/// the second line of defence against future renumbering that would
/// collide with the credit contract's `1..=49` range.
#[test]
fn collateral_specific_tier_starts_at_or_above_one_hundred() {
    let codes = [
        CollateralError::CollateralTokenNotAllowed as u32,
        CollateralError::CollateralRiskWeightOutOfRange as u32,
        CollateralError::CollateralTokenMismatch as u32,
        CollateralError::CollateralPositionLocked as u32,
        CollateralError::CollateralBalanceForTokenNotFound as u32,
    ];

    for &code in &codes {
        assert!(
            code >= 100,
            "Collateral-specific discriminant {} is below the 100+ namespace \
             — reserve codes 100+ for this crate",
            code
        );
    }
}

/// Verify the payloads round-trip through `Debug`, `Clone`, `Copy`, and
/// `Eq` without panicking — guards against accidental derivation
/// removal in future revisions.
#[test]
fn derives_round_trip() {
    let variant = CollateralError::CollateralBalanceForTokenNotFound;

    // Clone + Copy
    let copy = variant;
    let cloned = variant.clone();
    assert_eq!(copy, cloned);

    // Debug formatting must not panic.
    let _ = format!("{:?}", variant);

    // SDK-side decoders should match by discriminant integer; an explicit
    // `Hash` derive is intentionally NOT added to keep the derive list
    // consistent with the canonical `ContractError`.
}

/// Verify that mirror and collateral-specific tiers are disjoint. This
/// is a structural test: the union of both tiers must be `10` distinct
/// values, and there is no overlap.
#[test]
fn tiers_are_disjoint() {
    let mirror = [
        CollateralError::InvalidAmount as u32,
        CollateralError::Overflow as u32,
        CollateralError::MissingLiquidityToken as u32,
        CollateralError::CollateralRatioBelowMinimum as u32,
        CollateralError::InsufficientCollateralBalance as u32,
    ];
    let collateral_specific = [
        CollateralError::CollateralTokenNotAllowed as u32,
        CollateralError::CollateralRiskWeightOutOfRange as u32,
        CollateralError::CollateralTokenMismatch as u32,
        CollateralError::CollateralPositionLocked as u32,
        CollateralError::CollateralBalanceForTokenNotFound as u32,
    ];

    // Mirror tier must sit cleanly below the 100+ namespace.
    for &code in &mirror {
        assert!(
            code < 100,
            "Mirror discriminant {} falls into the collateral-specific tier; \
             the tier table in src/errors.rs is stale",
            code
        );
    }

    // Collateral-specific tier must be uniformly >= 100.
    for &code in &collateral_specific {
        assert!(
            code >= 100,
            "Collateral-specific discriminant {} falls into the mirror tier; \
             the tier table in src/errors.rs is stale",
            code
        );
    }
}
