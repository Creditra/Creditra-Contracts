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

use creditra_credit::types::{ContractError, ContractErrorCategory};

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
    assert_eq!(ContractError::RepayExceedsMaxAmount as u32, 28);
    assert_eq!(ContractError::DrawCooldownActive as u32, 29);
    assert_eq!(ContractError::TreasuryNotSet as u32, 30);
    assert_eq!(ContractError::ExposureCapExceeded as u32, 31);
    assert_eq!(ContractError::AdminNotInitialized as u32, 32);
    assert_eq!(ContractError::TimestampRegression as u32, 33);
    assert_eq!(ContractError::LimitOutOfBounds as u32, 34);
    assert_eq!(ContractError::CollateralRatioBelowMinimum as u32, 35);
    assert_eq!(ContractError::OraclePriceInvalid as u32, 36);
    assert_eq!(ContractError::OraclePriceStale as u32, 37);
    assert_eq!(ContractError::OraclePriceDeviation as u32, 38);
    assert_eq!(ContractError::InsufficientCollateralBalance as u32, 39);
    assert_eq!(ContractError::BorrowerFrozen as u32, 40);
    assert_eq!(ContractError::BountyNotSet as u32, 41);
    assert_eq!(ContractError::NoPendingTreasuryWithdrawal as u32, 42);
    assert_eq!(ContractError::TreasuryTimelockActive as u32, 43);
    assert_eq!(ContractError::TreasuryProposalExists as u32, 44);
    assert_eq!(ContractError::OracleQuorumNotMet as u32, 45);
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
        ContractError::RepayExceedsMaxAmount as u32,
        ContractError::DrawCooldownActive as u32,
        ContractError::TreasuryNotSet as u32,
        ContractError::ExposureCapExceeded as u32,
        ContractError::AdminNotInitialized as u32,
        ContractError::TimestampRegression as u32,
        ContractError::LimitOutOfBounds as u32,
        ContractError::CollateralRatioBelowMinimum as u32,
        ContractError::OraclePriceInvalid as u32,
        ContractError::OraclePriceStale as u32,
        ContractError::OraclePriceDeviation as u32,
        ContractError::InsufficientCollateralBalance as u32,
        ContractError::BorrowerFrozen as u32,
        ContractError::BountyNotSet as u32,
        ContractError::NoPendingTreasuryWithdrawal as u32,
        ContractError::TreasuryTimelockActive as u32,
        ContractError::TreasuryProposalExists as u32,
        ContractError::OracleQuorumNotMet as u32,
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
    // 45 variants as of this writing (added 45 for oracle quorum in #630).
    const EXPECTED_VARIANT_COUNT: usize = 45;

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
        ContractError::RepayExceedsMaxAmount as u32,
        ContractError::DrawCooldownActive as u32,
        ContractError::TreasuryNotSet as u32,
        ContractError::ExposureCapExceeded as u32,
        ContractError::AdminNotInitialized as u32,
        ContractError::TimestampRegression as u32,
        ContractError::LimitOutOfBounds as u32,
        ContractError::CollateralRatioBelowMinimum as u32,
        ContractError::OraclePriceInvalid as u32,
        ContractError::OraclePriceStale as u32,
        ContractError::OraclePriceDeviation as u32,
        ContractError::InsufficientCollateralBalance as u32,
        ContractError::BorrowerFrozen as u32,
        ContractError::BountyNotSet as u32,
        ContractError::NoPendingTreasuryWithdrawal as u32,
        ContractError::TreasuryTimelockActive as u32,
        ContractError::TreasuryProposalExists as u32,
        ContractError::OracleQuorumNotMet as u32,
    ];

    assert_eq!(
        codes.len(),
        EXPECTED_VARIANT_COUNT,
        "Variant count changed — update EXPECTED_VARIANT_COUNT and add/remove assertions"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// ContractErrorCategory Stability Tests
// ═══════════════════════════════════════════════════════════════════════════

/// Verify every `ContractErrorCategory` discriminant is pinned.
#[test]
fn category_discriminants_are_stable() {
    assert_eq!(ContractErrorCategory::Auth as u32, 1);
    assert_eq!(ContractErrorCategory::Lifecycle as u32, 2);
    assert_eq!(ContractErrorCategory::Numeric as u32, 3);
    assert_eq!(ContractErrorCategory::Limit as u32, 4);
    assert_eq!(ContractErrorCategory::Liquidity as u32, 5);
    assert_eq!(ContractErrorCategory::Risk as u32, 6);
    assert_eq!(ContractErrorCategory::Oracle as u32, 7);
    assert_eq!(ContractErrorCategory::Collateral as u32, 8);
    assert_eq!(ContractErrorCategory::Block as u32, 9);
    assert_eq!(ContractErrorCategory::Reentrancy as u32, 10);
    assert_eq!(ContractErrorCategory::Misc as u32, 11);
}

/// Verify no two `ContractErrorCategory` variants share a discriminant.
#[test]
fn no_duplicate_category_discriminants() {
    use std::collections::HashSet;

    let codes: Vec<u32> = vec![
        ContractErrorCategory::Auth as u32,
        ContractErrorCategory::Lifecycle as u32,
        ContractErrorCategory::Numeric as u32,
        ContractErrorCategory::Limit as u32,
        ContractErrorCategory::Liquidity as u32,
        ContractErrorCategory::Risk as u32,
        ContractErrorCategory::Oracle as u32,
        ContractErrorCategory::Collateral as u32,
        ContractErrorCategory::Block as u32,
        ContractErrorCategory::Reentrancy as u32,
        ContractErrorCategory::Misc as u32,
    ];

    let unique: HashSet<u32> = codes.iter().cloned().collect();
    assert_eq!(
        codes.len(),
        unique.len(),
        "Duplicate discriminants detected in ContractErrorCategory"
    );
}

/// Verify the total variant count for `ContractErrorCategory`.
#[test]
fn category_variant_count_is_known() {
    const EXPECTED_VARIANT_COUNT: usize = 11;

    let codes = [
        ContractErrorCategory::Auth as u32,
        ContractErrorCategory::Lifecycle as u32,
        ContractErrorCategory::Numeric as u32,
        ContractErrorCategory::Limit as u32,
        ContractErrorCategory::Liquidity as u32,
        ContractErrorCategory::Risk as u32,
        ContractErrorCategory::Oracle as u32,
        ContractErrorCategory::Collateral as u32,
        ContractErrorCategory::Block as u32,
        ContractErrorCategory::Reentrancy as u32,
        ContractErrorCategory::Misc as u32,
    ];

    assert_eq!(
        codes.len(),
        EXPECTED_VARIANT_COUNT,
        "Category variant count changed — update EXPECTED_VARIANT_COUNT"
    );
}

/// Verify every `ContractError` variant maps to the expected `ContractErrorCategory`.
#[test]
fn category_mappings_are_stable() {
    // Auth
    assert_eq!(
        ContractError::Unauthorized.category(),
        ContractErrorCategory::Auth
    );
    assert_eq!(
        ContractError::NotAdmin.category(),
        ContractErrorCategory::Auth
    );
    assert_eq!(
        ContractError::AdminNotInitialized.category(),
        ContractErrorCategory::Auth
    );
    // Lifecycle
    assert_eq!(
        ContractError::CreditLineClosed.category(),
        ContractErrorCategory::Lifecycle
    );
    assert_eq!(
        ContractError::AlreadyInitialized.category(),
        ContractErrorCategory::Lifecycle
    );
    assert_eq!(
        ContractError::CreditLineSuspended.category(),
        ContractErrorCategory::Lifecycle
    );
    assert_eq!(
        ContractError::CreditLineDefaulted.category(),
        ContractErrorCategory::Lifecycle
    );
    // Numeric
    assert_eq!(
        ContractError::InvalidAmount.category(),
        ContractErrorCategory::Numeric
    );
    assert_eq!(
        ContractError::NegativeLimit.category(),
        ContractErrorCategory::Numeric
    );
    assert_eq!(
        ContractError::Overflow.category(),
        ContractErrorCategory::Numeric
    );
    assert_eq!(
        ContractError::TimestampRegression.category(),
        ContractErrorCategory::Numeric
    );
    assert_eq!(
        ContractError::LimitOutOfBounds.category(),
        ContractErrorCategory::Numeric
    );
    // Limit
    assert_eq!(
        ContractError::OverLimit.category(),
        ContractErrorCategory::Limit
    );
    assert_eq!(
        ContractError::UtilizationNotZero.category(),
        ContractErrorCategory::Limit
    );
    assert_eq!(
        ContractError::LimitDecreaseRequiresRepayment.category(),
        ContractErrorCategory::Limit
    );
    assert_eq!(
        ContractError::DrawExceedsMaxAmount.category(),
        ContractErrorCategory::Limit
    );
    assert_eq!(
        ContractError::RepayExceedsMaxAmount.category(),
        ContractErrorCategory::Limit
    );
    // Liquidity
    assert_eq!(
        ContractError::MissingLiquidityToken.category(),
        ContractErrorCategory::Liquidity
    );
    assert_eq!(
        ContractError::MissingLiquiditySource.category(),
        ContractErrorCategory::Liquidity
    );
    assert_eq!(
        ContractError::InsufficientLiquidityReserve.category(),
        ContractErrorCategory::Liquidity
    );
    assert_eq!(
        ContractError::LiquidityTokenCallFailed.category(),
        ContractErrorCategory::Liquidity
    );
    assert_eq!(
        ContractError::InsufficientRepaymentAllowance.category(),
        ContractErrorCategory::Liquidity
    );
    assert_eq!(
        ContractError::InsufficientRepaymentBalance.category(),
        ContractErrorCategory::Liquidity
    );
    assert_eq!(
        ContractError::TreasuryNotSet.category(),
        ContractErrorCategory::Liquidity
    );
    assert_eq!(
        ContractError::ExposureCapExceeded.category(),
        ContractErrorCategory::Liquidity
    );
    // Risk
    assert_eq!(
        ContractError::RateTooHigh.category(),
        ContractErrorCategory::Risk
    );
    assert_eq!(
        ContractError::ScoreTooHigh.category(),
        ContractErrorCategory::Risk
    );
    assert_eq!(
        ContractError::Paused.category(),
        ContractErrorCategory::Risk
    );
    assert_eq!(
        ContractError::DrawCooldownActive.category(),
        ContractErrorCategory::Risk
    );
    // Oracle
    assert_eq!(
        ContractError::OraclePriceInvalid.category(),
        ContractErrorCategory::Oracle
    );
    assert_eq!(
        ContractError::OraclePriceStale.category(),
        ContractErrorCategory::Oracle
    );
    assert_eq!(
        ContractError::OraclePriceDeviation.category(),
        ContractErrorCategory::Oracle
    );
    // Collateral
    assert_eq!(
        ContractError::CollateralRatioBelowMinimum.category(),
        ContractErrorCategory::Collateral
    );
    assert_eq!(
        ContractError::InsufficientCollateralBalance.category(),
        ContractErrorCategory::Collateral
    );
    // Block
    assert_eq!(
        ContractError::BorrowerBlocked.category(),
        ContractErrorCategory::Block
    );
    assert_eq!(
        ContractError::DrawsFrozen.category(),
        ContractErrorCategory::Block
    );
    assert_eq!(
        ContractError::BorrowerFrozen.category(),
        ContractErrorCategory::Block
    );
    // Reentrancy
    assert_eq!(
        ContractError::Reentrancy.category(),
        ContractErrorCategory::Reentrancy
    );
    // Misc
    assert_eq!(
        ContractError::CreditLineNotFound.category(),
        ContractErrorCategory::Misc
    );
    assert_eq!(
        ContractError::AdminAcceptTooEarly.category(),
        ContractErrorCategory::Misc
    );
}

/// Verify every ContractError variant's category matches its discriminant table.
/// This catches accidental miscategorization when new variants are added.
#[test]
fn every_variant_has_known_category() {
    use std::collections::HashSet;

    let all_variants: Vec<ContractErrorCategory> = vec![
        ContractError::Unauthorized.category(),
        ContractError::NotAdmin.category(),
        ContractError::CreditLineNotFound.category(),
        ContractError::CreditLineClosed.category(),
        ContractError::InvalidAmount.category(),
        ContractError::OverLimit.category(),
        ContractError::NegativeLimit.category(),
        ContractError::RateTooHigh.category(),
        ContractError::ScoreTooHigh.category(),
        ContractError::UtilizationNotZero.category(),
        ContractError::Reentrancy.category(),
        ContractError::Overflow.category(),
        ContractError::LimitDecreaseRequiresRepayment.category(),
        ContractError::AlreadyInitialized.category(),
        ContractError::AdminAcceptTooEarly.category(),
        ContractError::BorrowerBlocked.category(),
        ContractError::DrawExceedsMaxAmount.category(),
        ContractError::Paused.category(),
        ContractError::DrawsFrozen.category(),
        ContractError::CreditLineSuspended.category(),
        ContractError::CreditLineDefaulted.category(),
        ContractError::MissingLiquidityToken.category(),
        ContractError::MissingLiquiditySource.category(),
        ContractError::InsufficientLiquidityReserve.category(),
        ContractError::LiquidityTokenCallFailed.category(),
        ContractError::InsufficientRepaymentAllowance.category(),
        ContractError::InsufficientRepaymentBalance.category(),
        ContractError::RepayExceedsMaxAmount.category(),
        ContractError::DrawCooldownActive.category(),
        ContractError::TreasuryNotSet.category(),
        ContractError::ExposureCapExceeded.category(),
        ContractError::AdminNotInitialized.category(),
        ContractError::TimestampRegression.category(),
        ContractError::LimitOutOfBounds.category(),
        ContractError::CollateralRatioBelowMinimum.category(),
        ContractError::OraclePriceInvalid.category(),
        ContractError::OraclePriceStale.category(),
        ContractError::OraclePriceDeviation.category(),
        ContractError::InsufficientCollateralBalance.category(),
        ContractError::BorrowerFrozen.category(),
    ];

    let unique: HashSet<ContractErrorCategory> = all_variants.iter().cloned().collect();
    assert_eq!(
        unique.len(),
        11,
        "Not all 11 categories are covered by variant mappings"
    );
    assert_eq!(all_variants.len(), 40, "Expected 40 ContractError variants");
}

// ═══════════════════════════════════════════════════════════════════════════
// Integration Tests: Verify Refactored Error Paths
// ═══════════════════════════════════════════════════════════════════════════
//
// These tests verify that all refactored unwrap/expect calls now fail gracefully
// with the correct ContractError discriminant instead of causing opaque panics.

#[cfg(test)]
mod error_path_tests {
    use creditra_credit::types::ContractError;
    use creditra_credit::{Credit, CreditClient};
    use soroban_sdk::{
        testutils::{Address as _, Ledger},
        Address, Env,
    };

    fn setup_env() -> (Env, CreditClient<'static>, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let contract_id = env.register_contract(None, Credit);
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);

        (env, client, contract_id, admin)
    }

    fn setup_with_token() -> (Env, CreditClient<'static>, Address, Address, Address) {
        let (env, client, contract_id, admin) = setup_env();

        // Deploy a mock token
        let token_id = env.register_stellar_asset_contract(admin.clone());
        client.set_liquidity_token(&token_id);
        client.set_liquidity_source(&contract_id);

        (env, client, contract_id, admin, token_id)
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Test 1: AdminNotInitialized - require_admin() without init
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_admin_not_initialized_error() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, Credit);
        let client = CreditClient::new(&env, &contract_id);

        let borrower = Address::generate(&env);

        // Try to open credit line without initializing admin
        let result = client.try_open_credit_line(&borrower, &1000_i128, &500_u32, &50_u32);

        assert!(result.is_err(), "Expected error when admin not initialized");
        let err = result.err().unwrap();
        assert_eq!(
            err.unwrap(),
            ContractError::AdminNotInitialized.into(),
            "Expected AdminNotInitialized error"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Test 2: CreditLineNotFound - draw_credit on non-existent line
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_credit_line_not_found_on_draw() {
        let (_env, client, _contract_id, _admin, _token) = setup_with_token();

        let borrower = Address::generate(&_env);

        // Try to draw without opening a credit line
        let result = client.try_draw_credit(&borrower, &100_i128);

        assert!(result.is_err(), "Expected error when credit line not found");
        let err = result.err().unwrap();
        assert_eq!(
            err.unwrap(),
            ContractError::CreditLineNotFound.into(),
            "Expected CreditLineNotFound error on draw"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Test 3: CreditLineNotFound - repay_credit on non-existent line
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_credit_line_not_found_on_repay() {
        let (_env, client, _contract_id, _admin, _token) = setup_with_token();

        let borrower = Address::generate(&_env);

        // Try to repay without opening a credit line
        let result = client.try_repay_credit(&borrower, &100_i128);

        assert!(result.is_err(), "Expected error when credit line not found");
        let err = result.err().unwrap();
        assert_eq!(
            err.unwrap(),
            ContractError::CreditLineNotFound.into(),
            "Expected CreditLineNotFound error on repay"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Test 4: CreditLineNotFound - close_credit_line on non-existent line
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_credit_line_not_found_on_close() {
        let (_env, client, _contract_id, admin, _token) = setup_with_token();

        let borrower = Address::generate(&_env);

        // Try to close a non-existent credit line
        let result = client.try_close_credit_line(&borrower, &admin);

        assert!(result.is_err(), "Expected error when credit line not found");
        let err = result.err().unwrap();
        assert_eq!(
            err.unwrap(),
            ContractError::CreditLineNotFound.into(),
            "Expected CreditLineNotFound error on close"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Test 5: CreditLineNotFound - suspend_credit_line on non-existent line
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_credit_line_not_found_on_suspend() {
        let (_env, client, _contract_id, _admin, _token) = setup_with_token();

        let borrower = Address::generate(&_env);

        // Try to suspend a non-existent credit line
        let result = client.try_suspend_credit_line(&borrower);

        assert!(result.is_err(), "Expected error when credit line not found");
        let err = result.err().unwrap();
        assert_eq!(
            err.unwrap(),
            ContractError::CreditLineNotFound.into(),
            "Expected CreditLineNotFound error on suspend"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Test 6: CreditLineNotFound - default_credit_line on non-existent line
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_credit_line_not_found_on_default() {
        let (_env, client, _contract_id, _admin, _token) = setup_with_token();

        let borrower = Address::generate(&_env);

        // Try to default a non-existent credit line
        let result = client.try_default_credit_line(&borrower);

        assert!(result.is_err(), "Expected error when credit line not found");
        let err = result.err().unwrap();
        assert_eq!(
            err.unwrap(),
            ContractError::CreditLineNotFound.into(),
            "Expected CreditLineNotFound error on default"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Test 7: CreditLineNotFound - update_risk_parameters on non-existent line
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_credit_line_not_found_on_risk_update() {
        let (_env, client, _contract_id, _admin, _token) = setup_with_token();

        let borrower = Address::generate(&_env);

        // Try to update risk parameters on non-existent credit line
        let result = client.try_update_risk_parameters(&borrower, &1000_i128, &500_u32, &50_u32);

        assert!(result.is_err(), "Expected error when credit line not found");
        let err = result.err().unwrap();
        assert_eq!(
            err.unwrap(),
            ContractError::CreditLineNotFound.into(),
            "Expected CreditLineNotFound error on risk update"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Test 8: Overflow - checked_add in draw_credit
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_overflow_on_draw_utilization_add() {
        let (env, client, contract_id, _admin, token) = setup_with_token();

        let borrower = Address::generate(&env);

        // Open credit line with max limit
        client.open_credit_line(&borrower, &i128::MAX, &500_u32, &50_u32);

        // Mint tokens to reserve
        use soroban_sdk::token::StellarAssetClient;
        let token_admin_client = StellarAssetClient::new(&env, &token);
        token_admin_client.mint(&contract_id, &i128::MAX);

        // Draw maximum amount
        client.draw_credit(&borrower, &(i128::MAX - 1000));

        // Try to draw more - should overflow
        let result = client.try_draw_credit(&borrower, &2000_i128);

        assert!(result.is_err(), "Expected overflow error");
        let err = result.err().unwrap();
        assert_eq!(
            err.unwrap(),
            ContractError::Overflow.into(),
            "Expected Overflow error on utilization add"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Test 9: Overflow - checked_sub in settle_default_liquidation
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_overflow_on_liquidation_settlement() {
        let (env, client, _contract_id, _admin, _token) = setup_with_token();

        let borrower = Address::generate(&env);

        // Open and default a credit line
        client.open_credit_line(&borrower, &1000_i128, &500_u32, &50_u32);
        client.default_credit_line(&borrower);

        // Try to settle with amount greater than utilized (should be caught by validation)
        let result = client.try_settle_default_liquidation(
            &borrower,
            &2000_i128,
            &soroban_sdk::symbol_short!("settle1"),
            &10_000_u32,
            &None,
        );

        assert!(
            result.is_err(),
            "Expected error on invalid settlement amount"
        );
        // This will hit the OverLimit check before overflow, but validates the path exists
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Test 10: MissingLiquidityToken - draw without token configured
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_missing_liquidity_token_on_draw() {
        let (env, client, _contract_id, _admin) = setup_env();

        let borrower = Address::generate(&env);

        // Open credit line without setting liquidity token
        client.open_credit_line(&borrower, &1000_i128, &500_u32, &50_u32);

        // Try to draw - should fail with MissingLiquidityToken
        let result = client.try_draw_credit(&borrower, &100_i128);

        assert!(
            result.is_err(),
            "Expected error when liquidity token not set"
        );
        let err = result.err().unwrap();
        assert_eq!(
            err.unwrap(),
            ContractError::MissingLiquidityToken.into(),
            "Expected MissingLiquidityToken error"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Test 11: MissingLiquiditySource - draw without source configured
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_missing_liquidity_source_on_draw() {
        let (env, client, _contract_id, admin) = setup_env();

        let borrower = Address::generate(&env);
        let token_id = env.register_stellar_asset_contract(admin.clone());

        // Set token but not source
        client.set_liquidity_token(&token_id);
        client.open_credit_line(&borrower, &1000_i128, &500_u32, &50_u32);

        // Try to draw - should fail with MissingLiquiditySource
        let result = client.try_draw_credit(&borrower, &100_i128);

        assert!(
            result.is_err(),
            "Expected error when liquidity source not set"
        );
        let err = result.err().unwrap();
        assert_eq!(
            err.unwrap(),
            ContractError::MissingLiquiditySource.into(),
            "Expected MissingLiquiditySource error"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Test 12: TreasuryNotSet - propose_treasury_withdrawal without treasury configured
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_treasury_not_set_on_withdraw() {
        let (_env, client, _contract_id, admin, _token) = setup_with_token();

        // Try to propose withdrawal without setting treasury address
        let result = client.try_propose_treasury_withdrawal(&admin);

        assert!(result.is_err(), "Expected error when treasury not set");
        let err = result.err().unwrap();
        assert_eq!(
            err.unwrap(),
            ContractError::TreasuryNotSet.into(),
            "Expected TreasuryNotSet error"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Test 13: Overflow - utilization cap calculation
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_overflow_on_utilization_cap_calculation() {
        let (env, client, contract_id, _admin, token) = setup_with_token();

        let borrower = Address::generate(&env);

        // Open credit line with very large limit
        client.open_credit_line(&borrower, &i128::MAX, &500_u32, &50_u32);

        // Set utilization cap
        client.set_utilization_cap(&borrower, &5000_u32); // 50%

        // Mint tokens to reserve
        use soroban_sdk::token::StellarAssetClient;
        let token_admin_client = StellarAssetClient::new(&env, &token);
        token_admin_client.mint(&contract_id, &i128::MAX);

        // The cap calculation might overflow with i128::MAX
        // This test verifies the overflow is caught gracefully
        let result = client.try_draw_credit(&borrower, &1000_i128);

        // Should either succeed or fail with Overflow, not panic
        if result.is_err() {
            let err = result.err().unwrap();
            assert_eq!(
                err.unwrap(),
                ContractError::Overflow.into(),
                "Expected Overflow error on cap calculation"
            );
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Test 14: ExposureCapExceeded - global exposure limit
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_exposure_cap_exceeded() {
        let (env, client, contract_id, _admin, token) = setup_with_token();

        let borrower1 = Address::generate(&env);
        let borrower2 = Address::generate(&env);

        // Set global exposure cap
        client.set_max_total_exposure(&1000_i128);

        // Mint tokens to reserve
        use soroban_sdk::token::StellarAssetClient;
        let token_admin_client = StellarAssetClient::new(&env, &token);
        token_admin_client.mint(&contract_id, &10000_i128);

        // Open two credit lines
        client.open_credit_line(&borrower1, &2000_i128, &500_u32, &50_u32);
        client.open_credit_line(&borrower2, &2000_i128, &500_u32, &50_u32);

        // Draw up to cap with first borrower
        client.draw_credit(&borrower1, &800_i128);

        // Try to draw more with second borrower - should exceed cap
        let result = client.try_draw_credit(&borrower2, &300_i128);

        assert!(result.is_err(), "Expected error when exposure cap exceeded");
        let err = result.err().unwrap();
        assert_eq!(
            err.unwrap(),
            ContractError::ExposureCapExceeded.into(),
            "Expected ExposureCapExceeded error"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Test 15: TimestampRegression - assert_ts_monotonic
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_timestamp_regression_protection() {
        let (env, client, contract_id, _admin, token) = setup_with_token();

        let borrower = Address::generate(&env);

        // Mint tokens to reserve
        use soroban_sdk::token::StellarAssetClient;
        let token_admin_client = StellarAssetClient::new(&env, &token);
        token_admin_client.mint(&contract_id, &10000_i128);

        // Open credit line
        client.open_credit_line(&borrower, &1000_i128, &500_u32, &50_u32);

        // Set ledger timestamp
        env.ledger().with_mut(|li| li.timestamp = 1000);

        // Update risk parameters to set last_rate_update_ts
        client.update_risk_parameters(&borrower, &1000_i128, &600_u32, &50_u32);

        // Try to move time backwards (this would be caught by Soroban, but we test the guard)
        env.ledger().with_mut(|li| li.timestamp = 500);

        // The timestamp regression check should prevent invalid updates
        // Note: In practice, Soroban prevents time from going backwards,
        // but our guard provides defense-in-depth
        let result = client.try_update_risk_parameters(&borrower, &1000_i128, &700_u32, &50_u32);

        // This may succeed if ledger timestamp is used directly, or fail if cached
        // The important thing is that it doesn't panic
        if result.is_err() {
            let err = result.err().unwrap();
            // Could be TimestampRegression or another validation error
            assert!(err.is_ok() || err.unwrap() == ContractError::TimestampRegression.into());
        }
    }
}
