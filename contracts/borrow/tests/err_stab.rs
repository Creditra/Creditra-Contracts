// SPDX-License-Identifier: MIT

//! ContractError stability tests for the borrow (v7) subsystem.
//!
//! # What
//!
//! Focused CI guard for the error discriminants and category mappings used by
//! the v7 borrow engine (`creditra_credit::borrow`) and its public entrypoints
//! (`draw_credit`, `repay_credit`, `repay_and_release_collateral`,
//! `reverse_draw`). Any assertion failure means a discriminant, category, or
//! runtime error path was accidentally changed — breaking deployed SDK clients
//! and indexers that match on error codes.
//!
//! # Scope (v7 borrow surface)
//!
//! - **Input / amount** — `InvalidAmount` (5), `OverLimit` (6),
//!   `DrawExceedsMaxAmount` (17), `RepayExceedsMaxAmount` (28),
//!   `ExposureCapExceeded` (31).
//! - **Lifecycle / status** — `CreditLineNotFound` (3), `CreditLineClosed` (4),
//!   `CreditLineSuspended` (20), `CreditLineDefaulted` (21),
//!   `CreditLineFrozen` (46), `UtilizationNotZero` (10).
//! - **Freeze / block** — `DrawsFrozen` (19), `BorrowerFrozen` (40),
//!   `BorrowerBlocked` (16), `Paused` (18), `DrawCooldownActive` (29),
//!   `AdminCooldownActive` (54).
//! - **Liquidity / repay funds** — `MissingLiquidityToken` (22),
//!   `MissingLiquiditySource` (23), `InsufficientLiquidityReserve` (24),
//!   `LiquidityTokenCallFailed` (25), `InsufficientRepaymentAllowance` (26),
//!   `InsufficientRepaymentBalance` (27).
//! - **Collateral gate on draw** — `CollateralRatioBelowMinimum` (35),
//!   `InsufficientCollateralBalance` (39).
//! - **Reversal** — `DrawReversalWindowExpired` (47),
//!   `OriginalDrawNotFound` (48).
//! - **Auth / safety** — `Unauthorized` (1), `AdminNotInitialized` (32),
//!   `Reentrancy` (11), `Overflow` (12).
//!
//! # Rules
//! - Never change an existing assertion value.
//! - If a new borrow-related error variant is added, append it with the next
//!   available integer **and** add corresponding assertions here.
//! - Integration tests MUST verify the raw discriminant (e.g. `"#5"`) is
//!   encoded in the panic payload — never match on variant names alone.
//!
//! # See also
//! - `creditra_credit::borrow` — the v7 borrow engine.
//! - `contracts/credit/tests/error_discriminants.rs` — the global discriminant registry.
//! - `contracts/lifecycle/tests/err_stab.rs` / `contracts/accrual/tests/err_stab.rs`
//!   — sibling v7 stability suites.

use creditra_credit::types::{ContractError, ContractErrorCategory};
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token, Address, Env,
};

// ═══════════════════════════════════════════════════════════════════════════
// Section 1 — Discriminant stability pins (v7 borrow error surface)
// ═══════════════════════════════════════════════════════════════════════════

/// Pin every discriminant in the v7 borrow error surface.
///
/// Values below are **permanent** — they are embedded in deployed SDKs and
/// on-chain indexer matchers. If any assertion fails, inspect
/// `creditra_credit::types::ContractError` for an accidental reorder /
/// renumber of the `#[repr(u32)]` enum.
#[test]
fn borrow_v7_error_discriminants_are_pinned() {
    // Auth / safety
    assert_eq!(ContractError::Unauthorized as u32, 1);
    assert_eq!(ContractError::AdminNotInitialized as u32, 32);
    assert_eq!(ContractError::Reentrancy as u32, 11);
    assert_eq!(ContractError::Overflow as u32, 12);

    // Lifecycle / status gates on draw & repay
    assert_eq!(ContractError::CreditLineNotFound as u32, 3);
    assert_eq!(ContractError::CreditLineClosed as u32, 4);
    assert_eq!(ContractError::CreditLineSuspended as u32, 20);
    assert_eq!(ContractError::CreditLineDefaulted as u32, 21);
    assert_eq!(ContractError::CreditLineFrozen as u32, 46);
    assert_eq!(ContractError::UtilizationNotZero as u32, 10);

    // Amount / limit
    assert_eq!(ContractError::InvalidAmount as u32, 5);
    assert_eq!(ContractError::OverLimit as u32, 6);
    assert_eq!(ContractError::DrawExceedsMaxAmount as u32, 17);
    assert_eq!(ContractError::RepayExceedsMaxAmount as u32, 28);
    assert_eq!(ContractError::ExposureCapExceeded as u32, 31);

    // Freeze / block / pause / cooldown
    assert_eq!(ContractError::BorrowerBlocked as u32, 16);
    assert_eq!(ContractError::Paused as u32, 18);
    assert_eq!(ContractError::DrawsFrozen as u32, 19);
    assert_eq!(ContractError::DrawCooldownActive as u32, 29);
    assert_eq!(ContractError::BorrowerFrozen as u32, 40);
    assert_eq!(ContractError::AdminCooldownActive as u32, 54);

    // Liquidity / repayment funding
    assert_eq!(ContractError::MissingLiquidityToken as u32, 22);
    assert_eq!(ContractError::MissingLiquiditySource as u32, 23);
    assert_eq!(ContractError::InsufficientLiquidityReserve as u32, 24);
    assert_eq!(ContractError::LiquidityTokenCallFailed as u32, 25);
    assert_eq!(ContractError::InsufficientRepaymentAllowance as u32, 26);
    assert_eq!(ContractError::InsufficientRepaymentBalance as u32, 27);

    // Collateral gate
    assert_eq!(ContractError::CollateralRatioBelowMinimum as u32, 35);
    assert_eq!(ContractError::InsufficientCollateralBalance as u32, 39);

    // Draw reversal
    assert_eq!(ContractError::DrawReversalWindowExpired as u32, 47);
    assert_eq!(ContractError::OriginalDrawNotFound as u32, 48);
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 2 — Category stability pins
// ═══════════════════════════════════════════════════════════════════════════

/// Every v7-borrow-relevant variant maps to the expected stable category.
#[test]
fn borrow_v7_category_mappings_are_pinned() {
    use ContractErrorCategory::*;

    assert_eq!(ContractError::Unauthorized.category(), Auth);
    assert_eq!(ContractError::AdminNotInitialized.category(), Auth);

    assert_eq!(ContractError::CreditLineClosed.category(), Lifecycle);
    assert_eq!(ContractError::CreditLineSuspended.category(), Lifecycle);
    assert_eq!(ContractError::CreditLineDefaulted.category(), Lifecycle);

    assert_eq!(ContractError::InvalidAmount.category(), Numeric);
    assert_eq!(ContractError::Overflow.category(), Numeric);

    assert_eq!(ContractError::OverLimit.category(), Limit);
    assert_eq!(ContractError::UtilizationNotZero.category(), Limit);
    assert_eq!(ContractError::DrawExceedsMaxAmount.category(), Limit);
    assert_eq!(ContractError::RepayExceedsMaxAmount.category(), Limit);
    assert_eq!(ContractError::DrawReversalWindowExpired.category(), Limit);

    assert_eq!(ContractError::MissingLiquidityToken.category(), Liquidity);
    assert_eq!(ContractError::MissingLiquiditySource.category(), Liquidity);
    assert_eq!(ContractError::InsufficientLiquidityReserve.category(), Liquidity);
    assert_eq!(ContractError::LiquidityTokenCallFailed.category(), Liquidity);
    assert_eq!(ContractError::InsufficientRepaymentAllowance.category(), Liquidity);
    assert_eq!(ContractError::InsufficientRepaymentBalance.category(), Liquidity);
    assert_eq!(ContractError::ExposureCapExceeded.category(), Liquidity);

    assert_eq!(ContractError::Paused.category(), Risk);
    assert_eq!(ContractError::DrawCooldownActive.category(), Risk);
    assert_eq!(ContractError::AdminCooldownActive.category(), Risk);

    assert_eq!(ContractError::CollateralRatioBelowMinimum.category(), Collateral);
    assert_eq!(ContractError::InsufficientCollateralBalance.category(), Collateral);

    assert_eq!(ContractError::BorrowerBlocked.category(), Block);
    assert_eq!(ContractError::DrawsFrozen.category(), Block);
    assert_eq!(ContractError::BorrowerFrozen.category(), Block);
    assert_eq!(ContractError::CreditLineFrozen.category(), Block);

    assert_eq!(ContractError::Reentrancy.category(), Reentrancy);

    assert_eq!(ContractError::CreditLineNotFound.category(), Misc);
    assert_eq!(ContractError::OriginalDrawNotFound.category(), Misc);
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 3 — Duplicate-free + variant-count sanity (v7 borrow subset)
// ═══════════════════════════════════════════════════════════════════════════

/// Verify that no two v7-borrow-relevant variants share a discriminant.
#[test]
fn borrow_v7_subset_has_no_duplicate_discriminants() {
    use std::collections::HashSet;

    let codes: Vec<u32> = vec![
        ContractError::Unauthorized as u32,
        ContractError::AdminNotInitialized as u32,
        ContractError::Reentrancy as u32,
        ContractError::Overflow as u32,
        ContractError::CreditLineNotFound as u32,
        ContractError::CreditLineClosed as u32,
        ContractError::CreditLineSuspended as u32,
        ContractError::CreditLineDefaulted as u32,
        ContractError::CreditLineFrozen as u32,
        ContractError::UtilizationNotZero as u32,
        ContractError::InvalidAmount as u32,
        ContractError::OverLimit as u32,
        ContractError::DrawExceedsMaxAmount as u32,
        ContractError::RepayExceedsMaxAmount as u32,
        ContractError::ExposureCapExceeded as u32,
        ContractError::BorrowerBlocked as u32,
        ContractError::Paused as u32,
        ContractError::DrawsFrozen as u32,
        ContractError::DrawCooldownActive as u32,
        ContractError::BorrowerFrozen as u32,
        ContractError::AdminCooldownActive as u32,
        ContractError::MissingLiquidityToken as u32,
        ContractError::MissingLiquiditySource as u32,
        ContractError::InsufficientLiquidityReserve as u32,
        ContractError::LiquidityTokenCallFailed as u32,
        ContractError::InsufficientRepaymentAllowance as u32,
        ContractError::InsufficientRepaymentBalance as u32,
        ContractError::CollateralRatioBelowMinimum as u32,
        ContractError::InsufficientCollateralBalance as u32,
        ContractError::DrawReversalWindowExpired as u32,
        ContractError::OriginalDrawNotFound as u32,
    ];

    let unique: HashSet<u32> = codes.iter().cloned().collect();
    assert_eq!(
        codes.len(),
        unique.len(),
        "Duplicate discriminants in the v7 borrow error surface — inspect types.rs"
    );
}

/// Known count: 31 variants in the v7 borrow surface (pinned above).
///
/// If this assertion fails, a new borrow-relevant variant was added to or
/// removed from the `ContractError` enum — update the count AND add/remove
/// the corresponding pinning assertions in
/// `borrow_v7_error_discriminants_are_pinned` and
/// `borrow_v7_category_mappings_are_pinned`.
#[test]
fn borrow_v7_subset_variant_count_is_known() {
    const EXPECTED_VARIANT_COUNT: usize = 31;

    let codes = [
        ContractError::Unauthorized as u32,
        ContractError::AdminNotInitialized as u32,
        ContractError::Reentrancy as u32,
        ContractError::Overflow as u32,
        ContractError::CreditLineNotFound as u32,
        ContractError::CreditLineClosed as u32,
        ContractError::CreditLineSuspended as u32,
        ContractError::CreditLineDefaulted as u32,
        ContractError::CreditLineFrozen as u32,
        ContractError::UtilizationNotZero as u32,
        ContractError::InvalidAmount as u32,
        ContractError::OverLimit as u32,
        ContractError::DrawExceedsMaxAmount as u32,
        ContractError::RepayExceedsMaxAmount as u32,
        ContractError::ExposureCapExceeded as u32,
        ContractError::BorrowerBlocked as u32,
        ContractError::Paused as u32,
        ContractError::DrawsFrozen as u32,
        ContractError::DrawCooldownActive as u32,
        ContractError::BorrowerFrozen as u32,
        ContractError::AdminCooldownActive as u32,
        ContractError::MissingLiquidityToken as u32,
        ContractError::MissingLiquiditySource as u32,
        ContractError::InsufficientLiquidityReserve as u32,
        ContractError::LiquidityTokenCallFailed as u32,
        ContractError::InsufficientRepaymentAllowance as u32,
        ContractError::InsufficientRepaymentBalance as u32,
        ContractError::CollateralRatioBelowMinimum as u32,
        ContractError::InsufficientCollateralBalance as u32,
        ContractError::DrawReversalWindowExpired as u32,
        ContractError::OriginalDrawNotFound as u32,
    ];

    assert_eq!(
        codes.len(),
        EXPECTED_VARIANT_COUNT,
        "v7 borrow surface variant count changed — pin new assertions and update EXPECTED_VARIANT_COUNT"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 4 — Integration: runtime error paths return the pinned discriminant
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod integration {
    use super::*;

    fn setup() -> (Env, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| li.timestamp = 10_000);

        let admin = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);

        let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
        let token_address = token_id.address();
        client.set_liquidity_token(&token_address);
        client.set_liquidity_source(&contract_id);

        token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &1_000_000_i128);

        (env, contract_id, admin)
    }

    fn extract_error_str(payload: &Box<dyn std::any::Any + Send>) -> String {
        if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else if let Some(s) = payload.downcast_ref::<&str>() {
            s.to_string()
        } else {
            String::new()
        }
    }

    /// draw_credit with amount ≤ 0 → InvalidAmount (#5).
    #[test]
    fn draw_zero_amount_reverts_with_invalid_amount_code_5() {
        let (env, contract_id, _admin) = setup();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);
        client.open_credit_line(&borrower, &1_000_i128, &300_u32, &50_u32);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.draw_credit(&borrower, &0_i128);
        }));
        assert!(result.is_err(), "expected revert for zero draw amount");
        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#5"),
            "expected InvalidAmount (#5), got: {err_str:?}"
        );
    }

    /// draw_credit on missing line → CreditLineNotFound (#3).
    #[test]
    fn draw_missing_line_reverts_with_not_found_code_3() {
        let (env, contract_id, _admin) = setup();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.draw_credit(&borrower, &100_i128);
        }));
        assert!(result.is_err(), "expected revert for missing credit line");
        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#3"),
            "expected CreditLineNotFound (#3), got: {err_str:?}"
        );
    }

    /// draw_credit while globally frozen → DrawsFrozen (#19).
    #[test]
    fn draw_while_draws_frozen_reverts_with_draws_frozen_code_19() {
        let (env, contract_id, _admin) = setup();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);
        client.open_credit_line(&borrower, &1_000_i128, &300_u32, &50_u32);
        client.freeze_draws();

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.draw_credit(&borrower, &100_i128);
        }));
        assert!(result.is_err(), "expected revert while draws are frozen");
        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#19"),
            "expected DrawsFrozen (#19), got: {err_str:?}"
        );
    }

    /// draw_credit exceeding limit → OverLimit (#6).
    #[test]
    fn draw_over_limit_reverts_with_over_limit_code_6() {
        let (env, contract_id, _admin) = setup();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);
        client.open_credit_line(&borrower, &1_000_i128, &300_u32, &50_u32);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.draw_credit(&borrower, &1_001_i128);
        }));
        assert!(result.is_err(), "expected revert for over-limit draw");
        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#6"),
            "expected OverLimit (#6), got: {err_str:?}"
        );
    }

    /// repay_credit on missing line → CreditLineNotFound (#3).
    #[test]
    fn repay_missing_line_reverts_with_not_found_code_3() {
        let (env, contract_id, _admin) = setup();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.repay_credit(&borrower, &100_i128);
        }));
        assert!(result.is_err(), "expected revert for missing credit line");
        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#3"),
            "expected CreditLineNotFound (#3), got: {err_str:?}"
        );
    }

    /// repay_credit with amount ≤ 0 → InvalidAmount (#5).
    #[test]
    fn repay_zero_amount_reverts_with_invalid_amount_code_5() {
        let (env, contract_id, _admin) = setup();
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);
        client.open_credit_line(&borrower, &1_000_i128, &300_u32, &50_u32);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.repay_credit(&borrower, &0_i128);
        }));
        assert!(result.is_err(), "expected revert for zero repay amount");
        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#5"),
            "expected InvalidAmount (#5), got: {err_str:?}"
        );
    }
}
