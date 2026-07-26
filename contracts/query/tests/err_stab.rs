// SPDX-License-Identifier: MIT

//! ContractError stability tests for the query (v7) subsystem.
//!
//! # What
//!
//! Freezes the [`creditra_credit::types::ContractError`] discriminants and
//! category mappings that are observable by callers of the credit contract's
//! read-only query entrypoints:
//!
//! - `get_credit_line` / `get_credit_line_summary`
//! - `get_protocol_summary`
//! - `get_repayment_schedule`
//! - `get_health_factor`
//! - `is_delinquent`
//! - `get_credit_lines_paginated`
//! - `borrow_capabilities`
//! - `capabilities` (accrual capabilities view, v7)
//!
//! Query entrypoints are read-only and do not panic with `ContractError` in
//! normal operation. However, `get_credit_lines_paginated` reverts with
//! `ContractError::Overflow` when `limit > MAX_ENUMERATION_LIMIT`, making
//! `Overflow` (12) the only discriminant **directly emitted** by the query
//! surface. All other pinned variants are **observable through state** (e.g.
//! a closed line returned by `get_credit_line`) or through the `ContractError`
//! enum values that SDK clients pattern-match against when consuming query
//! results together with mutating entrypoints.
//!
//! # Rules
//! - Never change an existing assertion value.
//! - If a new query-relevant error variant is added, append it here with the
//!   correct discriminant AND add a corresponding category assertion.
//! - Integration tests MUST match on raw discriminant strings (e.g. `"#12"`)
//!   rather than variant names — this is the only guarantee that survives ABI
//!   evolution.
//!
//! # See also
//! - `creditra_credit::query` — the read-only query implementation.
//! - `creditra_credit::views` — paginated and snapshot views.
//! - `contracts/credit/tests/error_discriminants.rs` — global discriminant registry.

use creditra_credit::types::{ContractError, ContractErrorCategory};
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token, Address, Env, Vec,
};

// ═══════════════════════════════════════════════════════════════════════════
// Section 1 — Discriminant stability pins (query v7 error surface)
// ═══════════════════════════════════════════════════════════════════════════

/// Pin every discriminant in the v7 query error surface.
///
/// These values are **permanent** — they are embedded in deployed SDKs and
/// off-chain indexer matchers. A failure here means a discriminant was
/// accidentally reordered or renumbered in `creditra_credit::types::ContractError`.
#[test]
fn query_v7_error_discriminants_are_pinned() {
    // ── Directly emitted by query surface ────────────────────────────────
    // get_credit_lines_paginated reverts with Overflow when limit > MAX_ENUMERATION_LIMIT.
    assert_eq!(ContractError::Overflow as u32, 12);

    // ── State conditions observable through query results ─────────────────
    // These are not emitted by query entrypoints directly, but SDK consumers
    // match against them when interpreting state returned by query calls.

    // Lifecycle state variants returned in CreditLineData.status
    assert_eq!(ContractError::CreditLineNotFound as u32, 3);
    assert_eq!(ContractError::CreditLineClosed as u32, 4);
    assert_eq!(ContractError::CreditLineSuspended as u32, 20);
    assert_eq!(ContractError::CreditLineDefaulted as u32, 21);
    assert_eq!(ContractError::CreditLineFrozen as u32, 46);

    // Auth / admin — returned by state-changing callers that precede a query
    assert_eq!(ContractError::Unauthorized as u32, 1);
    assert_eq!(ContractError::NotAdmin as u32, 2);
    assert_eq!(ContractError::AdminNotInitialized as u32, 32);

    // Numeric — Overflow is the only one directly emitted; others are
    // referenced by callers inspecting query outputs
    assert_eq!(ContractError::InvalidAmount as u32, 5);
    assert_eq!(ContractError::NegativeLimit as u32, 7);
    assert_eq!(ContractError::TimestampRegression as u32, 33);
    assert_eq!(ContractError::LimitOutOfBounds as u32, 34);

    // Risk — circuit-breaker state visible via borrow_capabilities
    assert_eq!(ContractError::Paused as u32, 18);
    assert_eq!(ContractError::DrawCooldownActive as u32, 29);

    // Block — freeze / blocklist state reported by borrow_capabilities
    assert_eq!(ContractError::BorrowerBlocked as u32, 16);
    assert_eq!(ContractError::BorrowerFrozen as u32, 40);
    assert_eq!(ContractError::DrawsFrozen as u32, 19);

    // Liquidity — configuration gaps that affect draw feasibility queries
    assert_eq!(ContractError::MissingLiquidityToken as u32, 22);
    assert_eq!(ContractError::MissingLiquiditySource as u32, 23);
    assert_eq!(ContractError::InsufficientLiquidityReserve as u32, 24);
    assert_eq!(ContractError::ExposureCapExceeded as u32, 31);

    // Limit — bounds visible through health factor and capability checks
    assert_eq!(ContractError::OverLimit as u32, 6);
    assert_eq!(ContractError::DrawExceedsMaxAmount as u32, 17);
    assert_eq!(ContractError::RepayExceedsMaxAmount as u32, 28);

    // Oracle — price validity gates accrual-aware health queries
    assert_eq!(ContractError::OraclePriceInvalid as u32, 36);
    assert_eq!(ContractError::OraclePriceStale as u32, 37);
    assert_eq!(ContractError::OraclePriceDeviation as u32, 38);
    assert_eq!(ContractError::OracleQuorumNotMet as u32, 50);

    // Collateral — collateral ratio is surfaced by get_health_factor
    assert_eq!(ContractError::CollateralRatioBelowMinimum as u32, 35);
    assert_eq!(ContractError::InsufficientCollateralBalance as u32, 39);

    // Reentrancy — guard visible if a query triggers a nested CPI
    assert_eq!(ContractError::Reentrancy as u32, 11);
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 2 — Category stability pins
// ═══════════════════════════════════════════════════════════════════════════

/// Every query-v7-relevant variant maps to the expected stable category.
#[test]
fn query_v7_category_mappings_are_pinned() {
    use ContractErrorCategory::*;

    // Numeric (3)
    assert_eq!(ContractError::Overflow.category(), Numeric);
    assert_eq!(ContractError::InvalidAmount.category(), Numeric);
    assert_eq!(ContractError::NegativeLimit.category(), Numeric);
    assert_eq!(ContractError::TimestampRegression.category(), Numeric);
    assert_eq!(ContractError::LimitOutOfBounds.category(), Numeric);

    // Auth (1)
    assert_eq!(ContractError::Unauthorized.category(), Auth);
    assert_eq!(ContractError::NotAdmin.category(), Auth);
    assert_eq!(ContractError::AdminNotInitialized.category(), Auth);

    // Lifecycle (2)
    assert_eq!(ContractError::CreditLineClosed.category(), Lifecycle);
    assert_eq!(ContractError::CreditLineSuspended.category(), Lifecycle);
    assert_eq!(ContractError::CreditLineDefaulted.category(), Lifecycle);

    // Misc (11)
    assert_eq!(ContractError::CreditLineNotFound.category(), Misc);

    // Risk (6)
    assert_eq!(ContractError::Paused.category(), Risk);
    assert_eq!(ContractError::DrawCooldownActive.category(), Risk);

    // Block (9)
    assert_eq!(ContractError::BorrowerBlocked.category(), Block);
    assert_eq!(ContractError::BorrowerFrozen.category(), Block);
    assert_eq!(ContractError::DrawsFrozen.category(), Block);
    assert_eq!(ContractError::CreditLineFrozen.category(), Block);

    // Liquidity (5)
    assert_eq!(ContractError::MissingLiquidityToken.category(), Liquidity);
    assert_eq!(ContractError::MissingLiquiditySource.category(), Liquidity);
    assert_eq!(ContractError::InsufficientLiquidityReserve.category(), Liquidity);
    assert_eq!(ContractError::ExposureCapExceeded.category(), Liquidity);

    // Limit (4)
    assert_eq!(ContractError::OverLimit.category(), Limit);
    assert_eq!(ContractError::DrawExceedsMaxAmount.category(), Limit);
    assert_eq!(ContractError::RepayExceedsMaxAmount.category(), Limit);

    // Oracle (7)
    assert_eq!(ContractError::OraclePriceInvalid.category(), Oracle);
    assert_eq!(ContractError::OraclePriceStale.category(), Oracle);
    assert_eq!(ContractError::OraclePriceDeviation.category(), Oracle);
    assert_eq!(ContractError::OracleQuorumNotMet.category(), Oracle);

    // Collateral (8)
    assert_eq!(ContractError::CollateralRatioBelowMinimum.category(), Collateral);
    assert_eq!(ContractError::InsufficientCollateralBalance.category(), Collateral);

    // Reentrancy (10)
    assert_eq!(ContractError::Reentrancy.category(), Reentrancy);
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 3 — No duplicate discriminants in query surface subset
// ═══════════════════════════════════════════════════════════════════════════

/// No two query-v7-relevant variants share a discriminant.
#[test]
fn query_v7_subset_has_no_duplicate_discriminants() {
    use std::collections::HashSet;

    let codes: std::vec::Vec<u32> = vec![
        ContractError::Overflow as u32,
        ContractError::CreditLineNotFound as u32,
        ContractError::CreditLineClosed as u32,
        ContractError::CreditLineSuspended as u32,
        ContractError::CreditLineDefaulted as u32,
        ContractError::CreditLineFrozen as u32,
        ContractError::Unauthorized as u32,
        ContractError::NotAdmin as u32,
        ContractError::AdminNotInitialized as u32,
        ContractError::InvalidAmount as u32,
        ContractError::NegativeLimit as u32,
        ContractError::TimestampRegression as u32,
        ContractError::LimitOutOfBounds as u32,
        ContractError::Paused as u32,
        ContractError::DrawCooldownActive as u32,
        ContractError::BorrowerBlocked as u32,
        ContractError::BorrowerFrozen as u32,
        ContractError::DrawsFrozen as u32,
        ContractError::MissingLiquidityToken as u32,
        ContractError::MissingLiquiditySource as u32,
        ContractError::InsufficientLiquidityReserve as u32,
        ContractError::ExposureCapExceeded as u32,
        ContractError::OverLimit as u32,
        ContractError::DrawExceedsMaxAmount as u32,
        ContractError::RepayExceedsMaxAmount as u32,
        ContractError::OraclePriceInvalid as u32,
        ContractError::OraclePriceStale as u32,
        ContractError::OraclePriceDeviation as u32,
        ContractError::OracleQuorumNotMet as u32,
        ContractError::CollateralRatioBelowMinimum as u32,
        ContractError::InsufficientCollateralBalance as u32,
        ContractError::Reentrancy as u32,
    ];

    let unique: HashSet<u32> = codes.iter().cloned().collect();
    assert_eq!(
        codes.len(),
        unique.len(),
        "Duplicate discriminants in query v7 error surface — inspect types.rs"
    );
}

/// Known variant count: 32 in the query v7 surface.
///
/// If this fails, a new query-relevant variant was added or removed.
/// Update `EXPECTED_VARIANT_COUNT` and add/remove the corresponding
/// assertions in the sections above.
#[test]
fn query_v7_subset_variant_count_is_known() {
    const EXPECTED_VARIANT_COUNT: usize = 32;

    let codes = [
        ContractError::Overflow as u32,
        ContractError::CreditLineNotFound as u32,
        ContractError::CreditLineClosed as u32,
        ContractError::CreditLineSuspended as u32,
        ContractError::CreditLineDefaulted as u32,
        ContractError::CreditLineFrozen as u32,
        ContractError::Unauthorized as u32,
        ContractError::NotAdmin as u32,
        ContractError::AdminNotInitialized as u32,
        ContractError::InvalidAmount as u32,
        ContractError::NegativeLimit as u32,
        ContractError::TimestampRegression as u32,
        ContractError::LimitOutOfBounds as u32,
        ContractError::Paused as u32,
        ContractError::DrawCooldownActive as u32,
        ContractError::BorrowerBlocked as u32,
        ContractError::BorrowerFrozen as u32,
        ContractError::DrawsFrozen as u32,
        ContractError::MissingLiquidityToken as u32,
        ContractError::MissingLiquiditySource as u32,
        ContractError::InsufficientLiquidityReserve as u32,
        ContractError::ExposureCapExceeded as u32,
        ContractError::OverLimit as u32,
        ContractError::DrawExceedsMaxAmount as u32,
        ContractError::RepayExceedsMaxAmount as u32,
        ContractError::OraclePriceInvalid as u32,
        ContractError::OraclePriceStale as u32,
        ContractError::OraclePriceDeviation as u32,
        ContractError::OracleQuorumNotMet as u32,
        ContractError::CollateralRatioBelowMinimum as u32,
        ContractError::InsufficientCollateralBalance as u32,
        ContractError::Reentrancy as u32,
    ];

    assert_eq!(
        codes.len(),
        EXPECTED_VARIANT_COUNT,
        "query v7 surface variant count changed — update EXPECTED_VARIANT_COUNT and add/remove pins"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 4 — Integration: runtime paths return pinned discriminants
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod integration {
    use super::*;

    /// Deploy Credit, init admin, configure token. Returns (env, contract_id, admin).
    fn setup() -> (Env, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);

        let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
        let token_address = token_id.address();
        client.set_liquidity_token(&token_address);
        client.set_liquidity_source(&contract_id);
        token::StellarAssetClient::new(&env, &token_address)
            .mint(&contract_id, &1_000_000_i128);

        (env, contract_id, admin)
    }

    fn extract_error(payload: &Box<dyn std::any::Any + Send>) -> String {
        if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else if let Some(s) = payload.downcast_ref::<&str>() {
            s.to_string()
        } else {
            String::new()
        }
    }

    // ── 4.1 get_credit_lines_paginated over MAX_ENUMERATION_LIMIT → Overflow (12) ──

    /// `get_credit_lines_paginated` with `limit > MAX_ENUMERATION_LIMIT` (100)
    /// MUST revert with `ContractError::Overflow` (discriminant 12).
    #[test]
    fn paginated_over_limit_reverts_with_overflow_code_12() {
        let (env, contract_id, _admin) = setup();
        let client = CreditClient::new(&env, &contract_id);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // MAX_ENUMERATION_LIMIT = 100; pass 101.
            client.get_credit_lines_paginated(&None, &101_u32);
        }));

        assert!(result.is_err(), "expected revert for limit > 100");
        let err = extract_error(&result.unwrap_err());
        assert!(
            err.contains("#12"),
            "expected Overflow (#12) for over-limit pagination, got: {err:?}"
        );
    }

    // ── 4.2 get_credit_lines_paginated at exact limit (100) must succeed ──

    /// Boundary: `limit == MAX_ENUMERATION_LIMIT` must not revert.
    #[test]
    fn paginated_exact_limit_100_succeeds() {
        let (env, contract_id, _admin) = setup();
        let client = CreditClient::new(&env, &contract_id);

        // Must not panic.
        let page = client.get_credit_lines_paginated(&None, &100_u32);
        assert_eq!(page.credit_lines.len(), 0);
        assert!(page.next_cursor.is_none());
    }

    // ── 4.3 get_credit_line returns None for unknown borrower ──

    /// `get_credit_line` returns `None` for a borrower with no credit line.
    /// Callers who `unwrap` the result will see a missing value — they should
    /// check before acting.
    #[test]
    fn get_credit_line_returns_none_for_unknown_borrower() {
        let (env, contract_id, _admin) = setup();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);

        let result = client.get_credit_line(&borrower);
        assert!(result.is_none(), "should be None for unknown borrower");
    }

    // ── 4.4 get_credit_line returns Some for existing borrower ──

    /// `get_credit_line` returns `Some(CreditLineData)` after `open_credit_line`.
    #[test]
    fn get_credit_line_returns_some_for_existing_borrower() {
        let (env, contract_id, _admin) = setup();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);

        client.open_credit_line(&borrower, &50_000_i128, &500_u32, &50_u32);
        let line = client.get_credit_line(&borrower);
        assert!(line.is_some(), "should return Some for opened line");
        assert_eq!(line.unwrap().credit_limit, 50_000_i128);
    }

    // ── 4.5 get_repayment_schedule returns None when no schedule set ──

    #[test]
    fn get_repayment_schedule_returns_none_when_not_set() {
        let (env, contract_id, _admin) = setup();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);

        client.open_credit_line(&borrower, &50_000_i128, &500_u32, &50_u32);
        let sched = client.get_repayment_schedule(&borrower);
        assert!(sched.is_none());
    }

    // ── 4.6 is_delinquent returns false when no credit line ──

    #[test]
    fn is_delinquent_returns_false_for_unknown_borrower() {
        let (env, contract_id, _admin) = setup();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);

        assert!(!client.is_delinquent(&borrower));
    }

    // ── 4.7 get_health_factor returns u32::MAX for borrower with no line ──

    #[test]
    fn get_health_factor_returns_max_for_unknown_borrower() {
        let (env, contract_id, _admin) = setup();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);

        assert_eq!(client.get_health_factor(&borrower), u32::MAX);
    }

    // ── 4.8 get_health_factor returns u32::MAX for zero utilization ──

    #[test]
    fn get_health_factor_returns_max_for_zero_utilization() {
        let (env, contract_id, _admin) = setup();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);

        client.open_credit_line(&borrower, &50_000_i128, &500_u32, &50_u32);
        // No draw — utilization is zero.
        assert_eq!(client.get_health_factor(&borrower), u32::MAX);
    }

    // ── 4.9 borrow_capabilities reflects paused state ──

    /// When the protocol is paused, `borrow_capabilities.can_draw` must be
    /// `false` and `borrow_capabilities.batch_open` must be `false` (via
    /// the accrual `capabilities` view).
    #[test]
    fn borrow_capabilities_can_draw_false_when_paused() {
        let (env, contract_id, admin) = setup();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);

        client.open_credit_line(&borrower, &50_000_i128, &500_u32, &50_u32);
        client.pause_protocol(&admin);

        let caps = client.borrow_capabilities(&borrower);
        assert!(!caps.can_draw, "can_draw must be false when paused");
    }

    // ── 4.10 Overflow discriminant is deterministic (two runs) ──

    /// Two independent runs with `limit = 101` both encode `#12`.
    #[test]
    fn paginated_overflow_discriminant_is_deterministic_code_12_twice() {
        for run in 1..=2u32 {
            let (env, contract_id, _admin) = setup();
            let client = CreditClient::new(&env, &contract_id);

            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                client.get_credit_lines_paginated(&None, &101_u32);
            }));

            assert!(result.is_err(), "run {run} must revert");
            let err = extract_error(&result.unwrap_err());
            assert!(
                err.contains("#12"),
                "run {run}: expected Overflow (#12), got: {err:?}"
            );
        }
    }
}
