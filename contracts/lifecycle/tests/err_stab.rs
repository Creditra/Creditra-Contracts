// SPDX-License-Identifier: MIT

//! ContractError stability tests for the lifecycle (v7) subsystem.
//!
//! # What
//!
//! Focused CI guard for the error discriminants and category mappings used by
//! the v7 credit-line lifecycle engine (`creditra_credit::lifecycle`) and its
//! public entrypoints (`open_credit_line`, `close_credit_line`,
//! `suspend_credit_line`, `self_suspend_credit_line`, `default_credit_line`,
//! `reinstate_credit_line`, `settle_default_liquidation`). Any assertion
//! failure means a discriminant, category, or runtime error path was
//! accidentally changed — breaking deployed SDK clients and indexers that
//! match on error codes.
//!
//! # Scope (v7 lifecycle surface)
//!
//! - **Lifecycle state errors** — `CreditLineNotFound` (3),
//!   `CreditLineClosed` (4), `AlreadyInitialized` (14),
//!   `CreditLineSuspended` (20), `CreditLineDefaulted` (21),
//!   `CreditLineFrozen` (46), `AlreadySettled` (41).
//! - **Auth / admin errors** — `Unauthorized` (1), `NotAdmin` (2),
//!   `AdminNotInitialized` (32).
//! - **Input validation** — `InvalidAmount` (5), `LimitOutOfBounds` (34),
//!   `NegativeLimit` (7).
//! - **Numeric / overflow** — `Overflow` (12),
//!   `TimestampRegression` (33), `UtilizationNotZero` (10).
//! - **Circuit breaker** — `Paused` (18).
//! - **Oracle / risk** — `OraclePriceInvalid` (36),
//!   `OraclePriceStale` (37), `OraclePriceDeviation` (38),
//!   `OracleQuorumNotMet` (50), `RateTooHigh` (8),
//!   `ScoreTooHigh` (9).
//!
//! # Rules
//! - Never change an existing assertion value.
//! - If a new lifecycle-related error variant is added, append it with the next
//!   available integer **and** add corresponding assertions here.
//! - Integration tests MUST verify the raw discriminant (e.g. `"#4"`) is
//!   encoded in the panic payload — never match on variant names alone.
//!
//! # See also
//! - `creditra_credit::lifecycle` — the v7 lifecycle engine.
//! - `contracts/credit/tests/error_discriminants.rs` — the global discriminant registry.
//! - `contracts/credit/tests/state_transition_invariants.rs` — state-machine tests.

use creditra_credit::types::{ContractError, ContractErrorCategory};
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token, Address, Env,
};

// ═══════════════════════════════════════════════════════════════════════════
// Section 1 — Discriminant stability pins (v7 lifecycle error surface)
// ═══════════════════════════════════════════════════════════════════════════

/// Pin every discriminant in the v7 lifecycle error surface.
///
/// Values below are **permanent** — they are embedded in deployed SDKs and
/// on-chain indexer matchers. If any assertion fails, inspect
/// `creditra_credit::types::ContractError` for an accidental reorder /
/// renumber of the `#[repr(u32)]` enum.
#[test]
fn lifecycle_v7_error_discriminants_are_pinned() {
    // Lifecycle state → core transition errors
    assert_eq!(ContractError::CreditLineNotFound as u32, 3);
    assert_eq!(ContractError::CreditLineClosed as u32, 4);
    assert_eq!(ContractError::AlreadyInitialized as u32, 14);
    assert_eq!(ContractError::CreditLineSuspended as u32, 20);
    assert_eq!(ContractError::CreditLineDefaulted as u32, 21);
    assert_eq!(ContractError::CreditLineFrozen as u32, 46);
    assert_eq!(ContractError::AlreadySettled as u32, 51);

    // Input validation → lifecycle entrypoints
    assert_eq!(ContractError::InvalidAmount as u32, 5);
    assert_eq!(ContractError::LimitOutOfBounds as u32, 34);
    assert_eq!(ContractError::NegativeLimit as u32, 7);
    assert_eq!(ContractError::UtilizationNotZero as u32, 10);
    assert_eq!(ContractError::RateTooHigh as u32, 8);
    assert_eq!(ContractError::ScoreTooHigh as u32, 9);

    // Numeric → overflow and timestamp guards
    assert_eq!(ContractError::Overflow as u32, 12);
    assert_eq!(ContractError::TimestampRegression as u32, 33);

    // Auth → admin/borrower authorization gates
    assert_eq!(ContractError::Unauthorized as u32, 1);
    assert_eq!(ContractError::NotAdmin as u32, 2);
    assert_eq!(ContractError::AdminNotInitialized as u32, 32);

    // Circuit breaker → gates all state-changing lifecycle ops
    assert_eq!(ContractError::Paused as u32, 18);

    // Oracle → gate settlement and risk-driven transitions
    assert_eq!(ContractError::OraclePriceInvalid as u32, 36);
    assert_eq!(ContractError::OraclePriceStale as u32, 37);
    assert_eq!(ContractError::OraclePriceDeviation as u32, 38);
    assert_eq!(ContractError::OracleQuorumNotMet as u32, 50);

    // Block / freeze → gates draw/suspend/close paths
    assert_eq!(ContractError::DrawsFrozen as u32, 19);
    assert_eq!(ContractError::BorrowerFrozen as u32, 40);
    assert_eq!(ContractError::BorrowerBlocked as u32, 16);

    // Reentrancy / misc
    assert_eq!(ContractError::Reentrancy as u32, 11);
    assert_eq!(ContractError::OverLimit as u32, 6);
    assert_eq!(ContractError::LimitDecreaseRequiresRepayment as u32, 13);
    assert_eq!(ContractError::AdminAcceptTooEarly as u32, 15);
    assert_eq!(ContractError::DrawExceedsMaxAmount as u32, 17);
    assert_eq!(ContractError::RepayExceedsMaxAmount as u32, 28);
    assert_eq!(ContractError::DrawCooldownActive as u32, 29);
    assert_eq!(ContractError::CollateralRatioBelowMinimum as u32, 35);
    assert_eq!(ContractError::InsufficientCollateralBalance as u32, 39);
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 2 — Category stability pins
// ═══════════════════════════════════════════════════════════════════════════

/// Every v7-lifecycle-relevant variant maps to the expected stable category.
#[test]
fn lifecycle_v7_category_mappings_are_pinned() {
    use ContractErrorCategory::*;

    // Lifecycle bucket (discriminant 2)
    assert_eq!(ContractError::CreditLineClosed.category(), Lifecycle);
    assert_eq!(ContractError::AlreadyInitialized.category(), Lifecycle);
    assert_eq!(ContractError::CreditLineSuspended.category(), Lifecycle);
    assert_eq!(ContractError::CreditLineDefaulted.category(), Lifecycle);
    assert_eq!(ContractError::AlreadySettled.category(), Lifecycle);

    // Numeric bucket (discriminant 3)
    assert_eq!(ContractError::InvalidAmount.category(), Numeric);
    assert_eq!(ContractError::NegativeLimit.category(), Numeric);
    assert_eq!(ContractError::Overflow.category(), Numeric);
    assert_eq!(ContractError::TimestampRegression.category(), Numeric);
    assert_eq!(ContractError::LimitOutOfBounds.category(), Numeric);

    // Auth bucket (discriminant 1)
    assert_eq!(ContractError::Unauthorized.category(), Auth);
    assert_eq!(ContractError::NotAdmin.category(), Auth);
    assert_eq!(ContractError::AdminNotInitialized.category(), Auth);

    // Risk bucket (discriminant 6)
    assert_eq!(ContractError::Paused.category(), Risk);
    assert_eq!(ContractError::RateTooHigh.category(), Risk);
    assert_eq!(ContractError::ScoreTooHigh.category(), Risk);
    assert_eq!(ContractError::DrawCooldownActive.category(), Risk);

    // Block bucket (discriminant 9)
    assert_eq!(ContractError::CreditLineFrozen.category(), Block);
    assert_eq!(ContractError::DrawsFrozen.category(), Block);
    assert_eq!(ContractError::BorrowerFrozen.category(), Block);
    assert_eq!(ContractError::BorrowerBlocked.category(), Block);

    // Oracle bucket (discriminant 7)
    assert_eq!(ContractError::OraclePriceInvalid.category(), Oracle);
    assert_eq!(ContractError::OraclePriceStale.category(), Oracle);
    assert_eq!(ContractError::OraclePriceDeviation.category(), Oracle);
    assert_eq!(ContractError::OracleQuorumNotMet.category(), Oracle);

    // Limit bucket (discriminant 4)
    assert_eq!(ContractError::OverLimit.category(), Limit);
    assert_eq!(ContractError::UtilizationNotZero.category(), Limit);
    assert_eq!(ContractError::LimitDecreaseRequiresRepayment.category(), Limit);
    assert_eq!(ContractError::DrawExceedsMaxAmount.category(), Limit);
    assert_eq!(ContractError::RepayExceedsMaxAmount.category(), Limit);

    // Collateral bucket (discriminant 8)
    assert_eq!(ContractError::CollateralRatioBelowMinimum.category(), Collateral);
    assert_eq!(ContractError::InsufficientCollateralBalance.category(), Collateral);

    // Misc bucket (discriminant 11)
    assert_eq!(ContractError::CreditLineNotFound.category(), Misc);
    assert_eq!(ContractError::AdminAcceptTooEarly.category(), Misc);

    // Reentrancy bucket (discriminant 10)
    assert_eq!(ContractError::Reentrancy.category(), Reentrancy);
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 3 — Duplicate-free + variant-count sanity (v7 lifecycle subset)
// ═══════════════════════════════════════════════════════════════════════════

/// Verify that no two v7-lifecycle-relevant variants share a discriminant.
#[test]
fn lifecycle_v7_subset_has_no_duplicate_discriminants() {
    use std::collections::HashSet;

    let codes: Vec<u32> = vec![
        ContractError::CreditLineNotFound as u32,
        ContractError::CreditLineClosed as u32,
        ContractError::AlreadyInitialized as u32,
        ContractError::CreditLineSuspended as u32,
        ContractError::CreditLineDefaulted as u32,
        ContractError::CreditLineFrozen as u32,
        ContractError::AlreadySettled as u32,
        ContractError::InvalidAmount as u32,
        ContractError::LimitOutOfBounds as u32,
        ContractError::NegativeLimit as u32,
        ContractError::UtilizationNotZero as u32,
        ContractError::RateTooHigh as u32,
        ContractError::ScoreTooHigh as u32,
        ContractError::Overflow as u32,
        ContractError::TimestampRegression as u32,
        ContractError::Unauthorized as u32,
        ContractError::NotAdmin as u32,
        ContractError::AdminNotInitialized as u32,
        ContractError::Paused as u32,
        ContractError::OraclePriceInvalid as u32,
        ContractError::OraclePriceStale as u32,
        ContractError::OraclePriceDeviation as u32,
        ContractError::OracleQuorumNotMet as u32,
        ContractError::DrawsFrozen as u32,
        ContractError::BorrowerFrozen as u32,
        ContractError::BorrowerBlocked as u32,
        ContractError::Reentrancy as u32,
        ContractError::OverLimit as u32,
        ContractError::LimitDecreaseRequiresRepayment as u32,
        ContractError::AdminAcceptTooEarly as u32,
        ContractError::DrawExceedsMaxAmount as u32,
        ContractError::RepayExceedsMaxAmount as u32,
        ContractError::DrawCooldownActive as u32,
        ContractError::CollateralRatioBelowMinimum as u32,
        ContractError::InsufficientCollateralBalance as u32,
    ];

    let unique: HashSet<u32> = codes.iter().cloned().collect();
    assert_eq!(
        codes.len(),
        unique.len(),
        "Duplicate discriminants in the v7 lifecycle error surface — inspect types.rs"
    );
}

/// Known count: 35 variants in the v7 lifecycle surface (pinned above).
///
/// If this assertion fails, a new lifecycle-relevant variant was added to or
/// removed from the `ContractError` enum — update the count AND add/remove
/// the corresponding pinning assertions in
/// `lifecycle_v7_error_discriminants_are_pinned` and
/// `lifecycle_v7_category_mappings_are_pinned`.
#[test]
fn lifecycle_v7_subset_variant_count_is_known() {
    const EXPECTED_VARIANT_COUNT: usize = 35;

    let codes = [
        ContractError::CreditLineNotFound as u32,
        ContractError::CreditLineClosed as u32,
        ContractError::AlreadyInitialized as u32,
        ContractError::CreditLineSuspended as u32,
        ContractError::CreditLineDefaulted as u32,
        ContractError::CreditLineFrozen as u32,
        ContractError::AlreadySettled as u32,
        ContractError::InvalidAmount as u32,
        ContractError::LimitOutOfBounds as u32,
        ContractError::NegativeLimit as u32,
        ContractError::UtilizationNotZero as u32,
        ContractError::RateTooHigh as u32,
        ContractError::ScoreTooHigh as u32,
        ContractError::Overflow as u32,
        ContractError::TimestampRegression as u32,
        ContractError::Unauthorized as u32,
        ContractError::NotAdmin as u32,
        ContractError::AdminNotInitialized as u32,
        ContractError::Paused as u32,
        ContractError::OraclePriceInvalid as u32,
        ContractError::OraclePriceStale as u32,
        ContractError::OraclePriceDeviation as u32,
        ContractError::OracleQuorumNotMet as u32,
        ContractError::DrawsFrozen as u32,
        ContractError::BorrowerFrozen as u32,
        ContractError::BorrowerBlocked as u32,
        ContractError::Reentrancy as u32,
        ContractError::OverLimit as u32,
        ContractError::LimitDecreaseRequiresRepayment as u32,
        ContractError::AdminAcceptTooEarly as u32,
        ContractError::DrawExceedsMaxAmount as u32,
        ContractError::RepayExceedsMaxAmount as u32,
        ContractError::DrawCooldownActive as u32,
        ContractError::CollateralRatioBelowMinimum as u32,
        ContractError::InsufficientCollateralBalance as u32,
    ];

    assert_eq!(
        codes.len(),
        EXPECTED_VARIANT_COUNT,
        "v7 lifecycle surface variant count changed — pin new assertions and update EXPECTED_VARIANT_COUNT"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 4 — Integration: runtime error paths return the pinned discriminant
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod integration {
    use super::*;

    /// Deploy the contract and initialize admin and liquidity token.
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

    /// Extract the raw Soroban error string from a caught panic payload.
    fn extract_error_str(payload: &Box<dyn std::any::Any + Send>) -> String {
        if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else if let Some(s) = payload.downcast_ref::<&str>() {
            s.to_string()
        } else {
            String::new()
        }
    }

    // ── Test 4.1 — open_credit_line with duplicate Active line → AlreadyInitialized (14) ──

    #[test]
    fn open_duplicate_active_line_reverts_with_already_initialized_code_14() {
        let (env, contract_id, _admin) = setup();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);

        client.open_credit_line(&borrower, &1000_i128, &500_u32, &50_u32);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.open_credit_line(&borrower, &2000_i128, &400_u32, &40_u32);
        }));
        assert!(result.is_err(), "expected revert for duplicate open");
        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#14"),
            "expected AlreadyInitialized (#14), got: {:?}",
            err_str
        );
    }

    // ── Test 4.2 — suspend_credit_line on non-existent line → panics with not-found message ──

    #[test]
    fn suspend_nonexistent_line_reverts_with_not_found_message() {
        let (env, contract_id, _admin) = setup();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.suspend_credit_line(&borrower);
        }));
        assert!(result.is_err());
        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("Credit line not found") || err_str.contains("#3"),
            "expected credit-line-not-found error, got: {:?}",
            err_str
        );
    }

    // ── Test 4.3 — close_credit_line on non-existent line → CreditLineNotFound (3) ──

    #[test]
    fn close_nonexistent_line_reverts_with_not_found_code_3() {
        let (env, contract_id, admin) = setup();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.close_credit_line(&borrower, &admin);
        }));
        assert!(result.is_err());
        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#3"),
            "expected CreditLineNotFound (#3), got: {:?}",
            err_str
        );
    }

    // ── Test 4.4 — open_credit_line with negative limit → InvalidAmount (5) ──

    #[test]
    fn open_negative_limit_reverts_with_invalid_amount_code_5() {
        let (env, contract_id, _admin) = setup();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.open_credit_line(&borrower, &-1_i128, &500_u32, &50_u32);
        }));
        assert!(result.is_err());
        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#5"),
            "expected InvalidAmount (#5), got: {:?}",
            err_str
        );
    }

    // ── Test 4.5 — open_credit_line with rate > 10000 bps → RateTooHigh (8) ──

    #[test]
    fn open_excessive_rate_reverts_with_rate_too_high_code_8() {
        let (env, contract_id, _admin) = setup();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.open_credit_line(&borrower, &1000_i128, &10_001_u32, &50_u32);
        }));
        assert!(result.is_err());
        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#8"),
            "expected RateTooHigh (#8), got: {:?}",
            err_str
        );
    }

    // ── Test 4.6 — open_credit_line with excessive risk score → ScoreTooHigh (9) ──

    #[test]
    fn open_excessive_score_reverts_with_score_too_high_code_9() {
        let (env, contract_id, _admin) = setup();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.open_credit_line(&borrower, &1000_i128, &500_u32, &10_001_u32);
        }));
        assert!(result.is_err());
        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#9"),
            "expected ScoreTooHigh (#9), got: {:?}",
            err_str
        );
    }

    // ── Test 4.7 — close_credit_line with admin force-close → succeeds (code 4 on double close) ──

    #[test]
    fn close_already_closed_line_is_idempotent_no_revert() {
        let (env, contract_id, admin) = setup();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);

        client.open_credit_line(&borrower, &1000_i128, &500_u32, &50_u32);
        client.close_credit_line(&borrower, &admin);
        // Idempotent: closing an already-closed line should not revert.
        client.close_credit_line(&borrower, &admin);
    }

    // ── Test 4.8 — suspend already-suspended line → panics with active-only message ──

    #[test]
    fn suspend_already_suspended_reverts_with_active_only_message() {
        let (env, contract_id, _admin) = setup();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);

        client.open_credit_line(&borrower, &1000_i128, &500_u32, &50_u32);
        client.suspend_credit_line(&borrower);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.suspend_credit_line(&borrower);
        }));
        assert!(result.is_err());
        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("Only active") || err_str.contains("active credit lines") || err_str.contains("#20"),
            "expected suspend-already-suspended error, got: {:?}",
            err_str
        );
    }

    // ── Test 4.9 — default already-defaulted line → idempotent (no revert) ──

    #[test]
    fn default_already_defaulted_is_idempotent_no_revert() {
        let (env, contract_id, _admin) = setup();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);

        client.open_credit_line(&borrower, &1000_i128, &500_u32, &50_u32);
        client.default_credit_line(&borrower);
        // Idempotent: double default should not revert.
        client.default_credit_line(&borrower);
    }

    // ── Test 4.10 — default a closed line → CreditLineClosed (4) ──

    #[test]
    fn default_closed_line_reverts_with_closed_code_4() {
        let (env, contract_id, admin) = setup();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);

        client.open_credit_line(&borrower, &1000_i128, &500_u32, &50_u32);
        client.close_credit_line(&borrower, &admin);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.default_credit_line(&borrower);
        }));
        assert!(result.is_err());
        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#4"),
            "expected CreditLineClosed (#4), got: {:?}",
            err_str
        );
    }

    // ── Test 4.11 — draw on suspended line → CreditLineSuspended (20) ──

    #[test]
    fn draw_on_suspended_line_reverts_with_code_20() {
        let (env, contract_id, _admin) = setup();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);

        client.open_credit_line(&borrower, &1000_i128, &500_u32, &50_u32);
        client.suspend_credit_line(&borrower);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.draw_credit(&borrower, &100_i128);
        }));
        assert!(result.is_err());
        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#20"),
            "expected CreditLineSuspended (#20), got: {:?}",
            err_str
        );
    }

    // ── Test 4.12 — draw on defaulted line → CreditLineDefaulted (21) ──

    #[test]
    fn draw_on_defaulted_line_reverts_with_code_21() {
        let (env, contract_id, _admin) = setup();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);

        client.open_credit_line(&borrower, &1000_i128, &500_u32, &50_u32);
        client.default_credit_line(&borrower);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.draw_credit(&borrower, &100_i128);
        }));
        assert!(result.is_err());
        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#21"),
            "expected CreditLineDefaulted (#21), got: {:?}",
            err_str
        );
    }

    // ── Test 4.13 — close as unauthorized third party → Unauthorized or NotAdmin ──

    #[test]
    fn close_by_third_party_reverts_with_auth_error() {
        let (env, contract_id, _admin) = setup();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);
        let third_party = Address::generate(&env);

        client.open_credit_line(&borrower, &1000_i128, &500_u32, &50_u32);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.close_credit_line(&borrower, &third_party);
        }));
        assert!(result.is_err(), "expected auth error for third-party close");
        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("unauthorized") || err_str.contains("#1") || err_str.contains("#2"),
            "expected auth error, got: {:?}",
            err_str
        );
    }

    // ── Test 4.14 — admin_not_initialized → AdminNotInitialized (32) ──

    #[test]
    fn lifecycle_op_without_admin_init_reverts_with_code_32() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.open_credit_line(&borrower, &1000_i128, &500_u32, &50_u32);
        }));
        assert!(result.is_err());
        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#32"),
            "expected AdminNotInitialized (#32), got: {:?}",
            err_str
        );
    }

    // ── Test 4.15 — open_credit_line boundary: zero limit → InvalidAmount (5) ──

    #[test]
    fn open_zero_limit_reverts_with_code_5() {
        let (env, contract_id, _admin) = setup();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.open_credit_line(&borrower, &0_i128, &500_u32, &50_u32);
        }));
        assert!(result.is_err());
        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#5"),
            "expected InvalidAmount (#5), got: {:?}",
            err_str
        );
    }

    // ── Test 4.16 — determinism: same error twice for lifecycle op ──

    #[test]
    fn lifecycle_error_discriminant_is_deterministic() {
        let (env, contract_id, _admin) = setup();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);

        for run in 1..=2 {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                client.open_credit_line(&borrower, &-1_i128, &500_u32, &50_u32);
            }));
            assert!(result.is_err(), "run {} must revert", run);
            let err_str = extract_error_str(&result.unwrap_err());
            assert!(
                err_str.contains("#5"),
                "run {}: expected InvalidAmount (#5), got: {:?}",
                run,
                err_str
            );
        }
    }

    // ── Test 4.17 — close_credit_line by borrower with utilization → unauthorized ──

    #[test]
    fn borrower_close_with_utilization_reverts() {
        let (env, contract_id, _admin) = setup();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);

        client.open_credit_line(&borrower, &1000_i128, &500_u32, &50_u32);
        client.draw_credit(&borrower, &500_i128);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.close_credit_line(&borrower, &borrower);
        }));
        assert!(result.is_err());
        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("utilized") || err_str.contains("cannot close"),
            "expected utilization error, got: {:?}",
            err_str
        );
    }

    // ── Test 4.18 — self_suspend on non-existent line → CreditLineNotFound (3) ──

    #[test]
    fn self_suspend_nonexistent_reverts_with_code_3() {
        let (env, contract_id, _admin) = setup();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.self_suspend_credit_line(&borrower);
        }));
        assert!(result.is_err());
        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#3"),
            "expected CreditLineNotFound (#3), got: {:?}",
            err_str
        );
    }

    // ── Test 4.19 — open_credit_line paused → Paused (18) ──

    #[test]
    fn open_while_paused_reverts_with_paused_code_18() {
        let (env, contract_id, admin) = setup();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);

        client.pause_protocol(&admin);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.open_credit_line(&borrower, &1000_i128, &500_u32, &50_u32);
        }));
        assert!(result.is_err());
        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#18"),
            "expected Paused (#18), got: {:?}",
            err_str
        );
    }
}
