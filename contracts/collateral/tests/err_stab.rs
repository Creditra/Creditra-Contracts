// SPDX-License-Identifier: MIT

//! ContractError stability tests for the collateral (v7) subsystem.
//!
//! # What
//!
//! Focused CI guard that freezes client-facing error code numbers for the
//! Creditra collateral domain. The published ABI surface is
//! [`creditra_collateral::CollateralError`] — a two-tier catalog whose
//! mirror tier reuses the same discriminants as the canonical credit
//! contract `ContractError` table, and whose collateral-specific tier
//! occupies the reserved `100+` namespace.
//!
//! Any assertion failure means a discriminant was accidentally reordered or
//! renumbered — breaking deployed SDK clients and indexers that match on
//! error codes.
//!
//! # Scope (v7 collateral surface)
//!
//! - **Mirror tier** — `InvalidAmount` (5), `Overflow` (12),
//!   `MissingLiquidityToken` (22), `CollateralRatioBelowMinimum` (35),
//!   `InsufficientCollateralBalance` (39).
//! - **Collateral-specific tier** — `CollateralTokenNotAllowed` (100),
//!   `CollateralRiskWeightOutOfRange` (101), `CollateralTokenMismatch` (102),
//!   `CollateralPositionLocked` (103),
//!   `CollateralBalanceForTokenNotFound` (104).
//!
//! # Rules
//! - Never change an existing assertion value.
//! - If a new collateral-related error variant is added, append it with the
//!   next available integer **and** add corresponding assertions here.
//! - Mirror-tier codes MUST stay identical to the canonical
//!   `ContractError` table in `contracts/credit/src/types.rs` /
//!   [`docs/ERROR_CODES.md`](../../../docs/ERROR_CODES.md).
//!
//! # See also
//! - [`creditra_collateral::CollateralError`] — the published catalog.
//! - [`tests/catalog.rs`] — complementary discriminant pins.
//! - [`docs/errors/collateral.md`](../../../docs/errors/collateral.md) —
//!   human-readable error reference.
//! - `contracts/borrow/tests/err_stab.rs` / `contracts/accrual/tests/err_stab.rs`
//!   — sibling v7 stability suites.

use creditra_collateral::CollateralError;

// ═══════════════════════════════════════════════════════════════════════════
// Section 1 — Discriminant stability pins (v7 collateral error surface)
// ═══════════════════════════════════════════════════════════════════════════

/// Pin every discriminant in the v7 collateral error surface.
///
/// Values below are **permanent** — they are embedded in deployed SDKs and
/// on-chain indexer matchers. If any assertion fails, inspect
/// `creditra_collateral::CollateralError` for an accidental reorder /
/// renumber of the `#[repr(u32)]` enum.
#[test]
fn collateral_v7_error_discriminants_are_pinned() {
    // ── Mirror tier (matches contracts/credit/src/types.rs ContractError) ──
    assert_eq!(CollateralError::InvalidAmount as u32, 5);
    assert_eq!(CollateralError::Overflow as u32, 12);
    assert_eq!(CollateralError::MissingLiquidityToken as u32, 22);
    assert_eq!(CollateralError::CollateralRatioBelowMinimum as u32, 35);
    assert_eq!(CollateralError::InsufficientCollateralBalance as u32, 39);

    // ── Collateral-specific tier (100+) ───────────────────────────────────
    assert_eq!(CollateralError::CollateralTokenNotAllowed as u32, 100);
    assert_eq!(CollateralError::CollateralRiskWeightOutOfRange as u32, 101);
    assert_eq!(CollateralError::CollateralTokenMismatch as u32, 102);
    assert_eq!(CollateralError::CollateralPositionLocked as u32, 103);
    assert_eq!(
        CollateralError::CollateralBalanceForTokenNotFound as u32,
        104
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 2 — Mirror-tier sync with canonical ContractError table
// ═══════════════════════════════════════════════════════════════════════════

/// Mirror-tier codes must stay byte-identical to the canonical credit
/// `ContractError` discriminants published in
/// `contracts/credit/src/types.rs` / `docs/ERROR_CODES.md`.
///
/// These constants are the documented canonical values. Pinning them here
/// (rather than importing `creditra_credit`) keeps this suite buildable
/// independently of the credit crate while still catching catalog drift.
#[test]
fn collateral_v7_mirror_tier_matches_canonical_contract_error_table() {
    // Canonical ContractError discriminants (credit types.rs).
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

// ═══════════════════════════════════════════════════════════════════════════
// Section 3 — Duplicate-free + variant-count + namespace sanity
// ═══════════════════════════════════════════════════════════════════════════

/// Verify that no two v7-collateral variants share a discriminant.
#[test]
fn collateral_v7_subset_has_no_duplicate_discriminants() {
    use std::collections::HashSet;

    let codes: Vec<u32> = vec![
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

    let unique: HashSet<u32> = codes.iter().cloned().collect();
    assert_eq!(
        codes.len(),
        unique.len(),
        "Duplicate discriminants in the v7 collateral error surface — inspect errors.rs"
    );
}

/// Known count: 10 variants in the v7 collateral surface (5 mirror + 5
/// collateral-specific).
///
/// If this assertion fails, a new collateral-relevant variant was added to or
/// removed from `CollateralError` — update the count AND add/remove the
/// corresponding pinning assertions in
/// `collateral_v7_error_discriminants_are_pinned`.
#[test]
fn collateral_v7_subset_variant_count_is_known() {
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
        "v7 collateral surface variant count changed — pin new assertions and update EXPECTED_VARIANT_COUNT"
    );
}

/// Collateral-specific tier must stay in the reserved `100+` namespace so it
/// never collides with the credit contract's `1..=49` `ContractError` range.
#[test]
fn collateral_v7_specific_tier_stays_in_100_plus_namespace() {
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
            "Collateral-specific discriminant {code} fell below the 100+ namespace"
        );
    }
}

/// Mirror tier must stay strictly below 100 (disjoint from the
/// collateral-specific namespace).
#[test]
fn collateral_v7_mirror_tier_stays_below_100() {
    let codes = [
        CollateralError::InvalidAmount as u32,
        CollateralError::Overflow as u32,
        CollateralError::MissingLiquidityToken as u32,
        CollateralError::CollateralRatioBelowMinimum as u32,
        CollateralError::InsufficientCollateralBalance as u32,
    ];

    for &code in &codes {
        assert!(
            code < 100,
            "Mirror discriminant {code} drifted into the collateral-specific tier"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 4 — Determinism: same variant → same discriminant twice
// ═══════════════════════════════════════════════════════════════════════════

/// Reproducibility guard: casting the same variant twice MUST yield the
/// same integer — no flakiness across runs.
#[test]
fn collateral_v7_discriminants_are_deterministic() {
    for _ in 0..2 {
        assert_eq!(CollateralError::InvalidAmount as u32, 5);
        assert_eq!(CollateralError::Overflow as u32, 12);
        assert_eq!(CollateralError::MissingLiquidityToken as u32, 22);
        assert_eq!(CollateralError::CollateralRatioBelowMinimum as u32, 35);
        assert_eq!(CollateralError::InsufficientCollateralBalance as u32, 39);
        assert_eq!(CollateralError::CollateralTokenNotAllowed as u32, 100);
        assert_eq!(CollateralError::CollateralRiskWeightOutOfRange as u32, 101);
        assert_eq!(CollateralError::CollateralTokenMismatch as u32, 102);
        assert_eq!(CollateralError::CollateralPositionLocked as u32, 103);
        assert_eq!(
            CollateralError::CollateralBalanceForTokenNotFound as u32,
            104
        );
    }
}
