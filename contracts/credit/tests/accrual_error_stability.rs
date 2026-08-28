// SPDX-License-Identifier: MIT

//! ContractError stability tests for the accrual (v7) subsystem.
//!
//! # What
//!
//! Focused CI guard for the error discriminants and category mappings used by
//! the v7 interest-accrual engine ([`crate::accrual`]) and its public
//! entrypoint [`CreditClient::accrue_batch`]. Any assertion failure means a
//! discriminant, category, or runtime error path was accidentally changed —
//! breaking deployed SDK clients and indexers that match on error codes.
//!
//! # Scope (v7 accrual surface)
//!
//! - **Numeric** — [`ContractError::Overflow`] (12) emitted by
//!   `apply_accrual` when `utilized_amount.checked_add(accrued_i)` or
//!   `accrued_interest.checked_add(accrued_i)` overflows, when the
//!   straddle-period `in_window.checked_add(post_window)` overflows, or when
//!   the `u128 → i128` conversion fails.
//! - **Input validation** — [`ContractError::InvalidAmount`] (5) emitted by
//!   `accrue_batch` when `borrowers.len() > ACCRUE_BATCH_MAX` (50).
//! - **Circuit breaker** — [`ContractError::Paused`] (18) emitted by
//!   `accrue_batch` via `assert_not_paused` when the protocol is paused by
//!   the emergency circuit breaker.
//! - **Lifecycle (apply_accrual callers)** — [`ContractError::CreditLineClosed`]
//!   (4), [`ContractError::CreditLineSuspended`] (20),
//!   [`ContractError::CreditLineDefaulted`] (21),
//!   [`ContractError::CreditLineNotFound`] (3),
//!   [`ContractError::CreditLineFrozen`] (46) encountered when accrual is
//!   materialized as the head of draw/repay/risk-update flows.
//! - **Oracle quorum** — [`ContractError::OracleQuorumNotMet`] (50) may gate
//!   accrual on chains that require an oracle price push before interest
//!   capitalization.
//!
//! # Rules
//! - Never change an existing assertion value.
//! - If a new accrual-related error variant is added, append it with the next
//!   available integer **and** add corresponding assertions here.
//! - Integration tests MUST verify the raw discriminant (e.g. `"#12"`) is
//!   encoded in the panic payload — never match on variant names alone.
//!
//! # See also
//! - [`crate::accrual::apply_accrual`] — the v7 accrual chokepoint.
//! - [`crate::accrual::accrue_batch`] — the batched public entrypoint.
//! - `tests/error_discriminants.rs` — the global discriminant registry.
//! - `tests/accrual_overflow_audit.rs` — overflow-determinism tests.

use creditra_credit::types::{ContractError, ContractErrorCategory};
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token, Address, Env, Vec,
};

// ═══════════════════════════════════════════════════════════════════════════
// Section 1 — Discriminant stability pins (v7 accrual error surface)
// ═══════════════════════════════════════════════════════════════════════════

/// Pin every discriminant in the v7 accrual error surface.
///
/// Values below are **permanent** — they are embedded in deployed SDKs and
/// on-chain indexer matchers. If any assertion fails, inspect `types.rs` for
/// an accidental reorder / renumber of the `#[repr(u32)]` enum.
#[test]
fn accrual_v7_error_discriminants_are_pinned() {
    // Numeric → accrual math (v7 core primitive)
    assert_eq!(ContractError::Overflow as u32, 12);
    assert_eq!(ContractError::InvalidAmount as u32, 5);
    assert_eq!(ContractError::TimestampRegression as u32, 33);
    assert_eq!(ContractError::LimitOutOfBounds as u32, 34);
    assert_eq!(ContractError::NegativeLimit as u32, 7);

    // Circuit breaker → gates `accrue_batch`
    assert_eq!(ContractError::Paused as u32, 18);

    // Lifecycle → state checks at the head of every apply_accrual caller
    assert_eq!(ContractError::CreditLineNotFound as u32, 3);
    assert_eq!(ContractError::CreditLineClosed as u32, 4);
    assert_eq!(ContractError::CreditLineSuspended as u32, 20);
    assert_eq!(ContractError::CreditLineDefaulted as u32, 21);
    assert_eq!(ContractError::CreditLineFrozen as u32, 46);

    // Draws-frozen / freeze-surface that gates materialization
    assert_eq!(ContractError::DrawsFrozen as u32, 19);
    assert_eq!(ContractError::BorrowerFrozen as u32, 40);
    assert_eq!(ContractError::BorrowerBlocked as u32, 16);

    // Oracle → price validity is a precondition of risk-driven accrual
    assert_eq!(ContractError::OraclePriceInvalid as u32, 36);
    assert_eq!(ContractError::OraclePriceStale as u32, 37);
    assert_eq!(ContractError::OraclePriceDeviation as u32, 38);
    assert_eq!(ContractError::OracleQuorumNotMet as u32, 50);

    // Liquidity → apply_accrual runs inside draw_credit / repay_credit
    assert_eq!(ContractError::MissingLiquidityToken as u32, 22);
    assert_eq!(ContractError::MissingLiquiditySource as u32, 23);
    assert_eq!(ContractError::InsufficientLiquidityReserve as u32, 24);
    assert_eq!(ContractError::InsufficientRepaymentAllowance as u32, 26);
    assert_eq!(ContractError::InsufficientRepaymentBalance as u32, 27);
    assert_eq!(ContractError::LiquidityTokenCallFailed as u32, 25);
    assert_eq!(ContractError::ExposureCapExceeded as u32, 31);

    // Limit → numeric ceilings on draw (apply_accrual runs BEFORE the check)
    assert_eq!(ContractError::OverLimit as u32, 6);
    assert_eq!(ContractError::DrawExceedsMaxAmount as u32, 17);
    assert_eq!(ContractError::RepayExceedsMaxAmount as u32, 28);
    assert_eq!(ContractError::UtilizationNotZero as u32, 10);
    //     assert_eq!(ContractError::LimitDecreaseRequiresRepayment as u32, 13);

    // Risk → rate/score clamp paths that execute with accrual head
    assert_eq!(ContractError::RateTooHigh as u32, 8);
    assert_eq!(ContractError::ScoreTooHigh as u32, 9);
    assert_eq!(ContractError::DrawCooldownActive as u32, 29);

    // Auth / reentrancy → invariants on every state-changing entrypoint
    assert_eq!(ContractError::Unauthorized as u32, 1);
    assert_eq!(ContractError::NotAdmin as u32, 2);
    assert_eq!(ContractError::AdminNotInitialized as u32, 32);
    assert_eq!(ContractError::Reentrancy as u32, 11);
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 2 — Category stability pins
// ═══════════════════════════════════════════════════════════════════════════

/// Every v7-accrual-relevant variant maps to the expected stable category.
#[test]
fn accrual_v7_category_mappings_are_pinned() {
    use ContractErrorCategory::*;

    // Numeric bucket (discriminant 3)
    assert_eq!(ContractError::Overflow.category(), Numeric);
    assert_eq!(ContractError::InvalidAmount.category(), Numeric);
    assert_eq!(ContractError::TimestampRegression.category(), Numeric);
    assert_eq!(ContractError::LimitOutOfBounds.category(), Numeric);
    assert_eq!(ContractError::NegativeLimit.category(), Numeric);

    // Risk bucket (discriminant 6)
    assert_eq!(ContractError::Paused.category(), Risk);
    assert_eq!(ContractError::RateTooHigh.category(), Risk);
    assert_eq!(ContractError::ScoreTooHigh.category(), Risk);
    assert_eq!(ContractError::DrawCooldownActive.category(), Risk);

    // Lifecycle bucket (discriminant 2)
    assert_eq!(ContractError::CreditLineClosed.category(), Lifecycle);
    assert_eq!(ContractError::CreditLineSuspended.category(), Lifecycle);
    assert_eq!(ContractError::CreditLineDefaulted.category(), Lifecycle);

    // Block bucket (discriminant 9)
    assert_eq!(ContractError::DrawsFrozen.category(), Block);
    assert_eq!(ContractError::BorrowerFrozen.category(), Block);
    assert_eq!(ContractError::BorrowerBlocked.category(), Block);
    assert_eq!(ContractError::CreditLineFrozen.category(), Block);

    // Oracle bucket (discriminant 7)
    assert_eq!(ContractError::OraclePriceInvalid.category(), Oracle);
    assert_eq!(ContractError::OraclePriceStale.category(), Oracle);
    assert_eq!(ContractError::OraclePriceDeviation.category(), Oracle);
    assert_eq!(ContractError::OracleQuorumNotMet.category(), Oracle);

    // Liquidity bucket (discriminant 5)
    assert_eq!(ContractError::MissingLiquidityToken.category(), Liquidity);
    assert_eq!(ContractError::MissingLiquiditySource.category(), Liquidity);
    assert_eq!(
        ContractError::InsufficientLiquidityReserve.category(),
        Liquidity
    );
    assert_eq!(
        ContractError::InsufficientRepaymentAllowance.category(),
        Liquidity
    );
    assert_eq!(
        ContractError::InsufficientRepaymentBalance.category(),
        Liquidity
    );
    assert_eq!(
        ContractError::LiquidityTokenCallFailed.category(),
        Liquidity
    );
    assert_eq!(ContractError::ExposureCapExceeded.category(), Liquidity);

    // Limit bucket (discriminant 4)
    assert_eq!(ContractError::OverLimit.category(), Limit);
    assert_eq!(ContractError::DrawExceedsMaxAmount.category(), Limit);
    assert_eq!(ContractError::RepayExceedsMaxAmount.category(), Limit);
    assert_eq!(ContractError::UtilizationNotZero.category(), Limit);
    assert_eq!(
        //         ContractError::LimitDecreaseRequiresRepayment.category(),
        Limit
    );

    // Auth / Reentrancy / Misc buckets
    assert_eq!(ContractError::Unauthorized.category(), Auth);
    assert_eq!(ContractError::NotAdmin.category(), Auth);
    assert_eq!(ContractError::AdminNotInitialized.category(), Auth);
    assert_eq!(ContractError::Reentrancy.category(), Reentrancy);
    assert_eq!(ContractError::CreditLineNotFound.category(), Misc);
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 3 — Duplicate-free + variant-count sanity (v7 subset)
// ═══════════════════════════════════════════════════════════════════════════

/// Verify that no two v7-accrual-relevant variants share a discriminant.
#[test]
fn accrual_v7_subset_has_no_duplicate_discriminants() {
    use std::collections::HashSet;

    let codes: Vec<u32> = vec![
        ContractError::Overflow as u32,
        ContractError::InvalidAmount as u32,
        ContractError::TimestampRegression as u32,
        ContractError::LimitOutOfBounds as u32,
        ContractError::NegativeLimit as u32,
        ContractError::Paused as u32,
        ContractError::CreditLineNotFound as u32,
        ContractError::CreditLineClosed as u32,
        ContractError::CreditLineSuspended as u32,
        ContractError::CreditLineDefaulted as u32,
        ContractError::CreditLineFrozen as u32,
        ContractError::DrawsFrozen as u32,
        ContractError::BorrowerFrozen as u32,
        ContractError::BorrowerBlocked as u32,
        ContractError::OraclePriceInvalid as u32,
        ContractError::OraclePriceStale as u32,
        ContractError::OraclePriceDeviation as u32,
        ContractError::OracleQuorumNotMet as u32,
        ContractError::MissingLiquidityToken as u32,
        ContractError::MissingLiquiditySource as u32,
        ContractError::InsufficientLiquidityReserve as u32,
        ContractError::InsufficientRepaymentAllowance as u32,
        ContractError::InsufficientRepaymentBalance as u32,
        ContractError::LiquidityTokenCallFailed as u32,
        ContractError::ExposureCapExceeded as u32,
        ContractError::OverLimit as u32,
        ContractError::DrawExceedsMaxAmount as u32,
        ContractError::RepayExceedsMaxAmount as u32,
        ContractError::UtilizationNotZero as u32,
        //         ContractError::LimitDecreaseRequiresRepayment as u32,
        ContractError::RateTooHigh as u32,
        ContractError::ScoreTooHigh as u32,
        ContractError::DrawCooldownActive as u32,
        ContractError::Unauthorized as u32,
        ContractError::NotAdmin as u32,
        ContractError::AdminNotInitialized as u32,
        ContractError::Reentrancy as u32,
    ];

    let unique: HashSet<u32> = codes.iter().cloned().collect();
    assert_eq!(
        codes.len(),
        unique.len(),
        "Duplicate discriminants in the v7 accrual error surface — inspect types.rs"
    );
}

/// Known count: 37 variants in the v7 accrual surface (pinned above).
///
/// If this assertion fails, a new accrual-relevant variant was added to or
/// removed from the `ContractError` enum — update the count AND add/remove
/// the corresponding pinning assertions in
/// [`accrual_v7_error_discriminants_are_pinned`] and
/// [`accrual_v7_category_mappings_are_pinned`].
#[test]
fn accrual_v7_subset_variant_count_is_known() {
    const EXPECTED_VARIANT_COUNT: usize = 37;

    let codes = [
        ContractError::Overflow as u32,
        ContractError::InvalidAmount as u32,
        ContractError::TimestampRegression as u32,
        ContractError::LimitOutOfBounds as u32,
        ContractError::NegativeLimit as u32,
        ContractError::Paused as u32,
        ContractError::CreditLineNotFound as u32,
        ContractError::CreditLineClosed as u32,
        ContractError::CreditLineSuspended as u32,
        ContractError::CreditLineDefaulted as u32,
        ContractError::CreditLineFrozen as u32,
        ContractError::DrawsFrozen as u32,
        ContractError::BorrowerFrozen as u32,
        ContractError::BorrowerBlocked as u32,
        ContractError::OraclePriceInvalid as u32,
        ContractError::OraclePriceStale as u32,
        ContractError::OraclePriceDeviation as u32,
        ContractError::OracleQuorumNotMet as u32,
        ContractError::MissingLiquidityToken as u32,
        ContractError::MissingLiquiditySource as u32,
        ContractError::InsufficientLiquidityReserve as u32,
        ContractError::InsufficientRepaymentAllowance as u32,
        ContractError::InsufficientRepaymentBalance as u32,
        ContractError::LiquidityTokenCallFailed as u32,
        ContractError::ExposureCapExceeded as u32,
        ContractError::OverLimit as u32,
        ContractError::DrawExceedsMaxAmount as u32,
        ContractError::RepayExceedsMaxAmount as u32,
        ContractError::UtilizationNotZero as u32,
        //         ContractError::LimitDecreaseRequiresRepayment as u32,
        ContractError::RateTooHigh as u32,
        ContractError::ScoreTooHigh as u32,
        ContractError::DrawCooldownActive as u32,
        ContractError::Unauthorized as u32,
        ContractError::NotAdmin as u32,
        ContractError::AdminNotInitialized as u32,
        ContractError::Reentrancy as u32,
    ];

    assert_eq!(
        codes.len(),
        EXPECTED_VARIANT_COUNT,
        "v7 accrual surface variant count changed — pin new assertions and update EXPECTED_VARIANT_COUNT"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 4 — Integration: runtime error paths return the pinned discriminant
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod integration {
    use super::*;

    const ACCRUE_BATCH_MAX: u32 = 50;

    /// Deploy the contract, init admin, configure a SAC token as liquidity
    /// source, and mint `reserve_amount` tokens into the reserve. Mirrors the
    /// helper in `accrual_overflow_audit.rs` to keep fixtures consistent.
    fn setup_with_token(reserve_amount: i128) -> (Env, Address, Address, Address) {
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

        token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &reserve_amount);

        (env, contract_id, admin, token_address)
    }

    /// Extract the raw Soroban error string from a caught panic payload.
    ///
    /// Soroban encodes contract errors as `"Error(Contract, #<discriminant>)"`
    /// inside the panic message. We string-match because the opaque payload
    /// does not implement `PartialEq` across Soroban versions.
    fn extract_error_str(payload: &Box<dyn std::any::Any + Send>) -> String {
        if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else if let Some(s) = payload.downcast_ref::<&str>() {
            s.to_string()
        } else {
            String::new()
        }
    }

    // ── Test 4.1 — accrue_batch > ACCRUE_BATCH_MAX → InvalidAmount (5) ──

    /// `accrue_batch` with 51 borrowers MUST revert with
    /// `ContractError::InvalidAmount` (discriminant 5) — not a bare panic,
    /// not Overflow, not any other code.
    #[test]
    fn accrue_batch_over_max_reverts_with_invalid_amount_code_5() {
        let (env, contract_id, _admin, _token) = setup_with_token(0_i128);
        let client = CreditClient::new(&env, &contract_id);

        let mut borrowers: Vec<Address> = Vec::new(&env);
        for _ in 0..=ACCRUE_BATCH_MAX {
            borrowers.push_back(Address::generate(&env));
        }
        assert_eq!(borrowers.len() as u32, ACCRUE_BATCH_MAX + 1);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.accrue_batch(&borrowers);
        }));
        assert!(result.is_err(), "expected revert for oversized batch");

        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#5"),
            "expected InvalidAmount (#5) for batch > 50, got: {:?}",
            err_str
        );
        // Sanity: must NOT be mistaken for Overflow (#12).
        assert!(!err_str.contains("#12"), "must not be Overflow");
    }

    // ── Test 4.2 — accrue_batch while paused → Paused (18) ──

    /// `accrue_batch` with the protocol paused MUST revert with
    /// `ContractError::Paused` (discriminant 18) via `assert_not_paused`.
    #[test]
    fn accrue_batch_while_paused_reverts_with_paused_code_18() {
        let (env, contract_id, admin, _token) = setup_with_token(0_i128);
        let client = CreditClient::new(&env, &contract_id);

        client.pause_protocol(&admin);

        let borrowers: Vec<Address> = Vec::new(&env);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.accrue_batch(&borrowers);
        }));
        assert!(result.is_err(), "expected revert when paused");

        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#18"),
            "expected Paused (#18) when paused, got: {:?}",
            err_str
        );
    }

    // ── Test 4.3 — apply_accrual overflow → Overflow (12) via update_risk_parameters ──

    /// When `utilized_amount.checked_add(accrued_i)` overflows inside
    /// `apply_accrual`, the calling entrypoint MUST encode
    /// `ContractError::Overflow` (discriminant 12). This is the v7 accrual
    /// engine's canonical overflow path.
    #[test]
    fn apply_accrual_utilized_overflow_emits_code_12() {
        let huge_principal: i128 = i128::MAX / 2;
        let (env, contract_id, _admin, _token) = setup_with_token(huge_principal);
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);

        client.open_credit_line(&borrower, &huge_principal, &10_000_u32, &50_u32);

        env.ledger().set_timestamp(1);
        client.draw_credit(&borrower, &huge_principal);
        env.ledger().set_timestamp(2);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.update_risk_parameters(&borrower, &i128::MAX, &10_000_u32, &50_u32);
        }));
        assert!(result.is_err(), "expected accrual overflow to revert");

        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#12"),
            "expected Overflow (#12) from accrual math, got: {:?}",
            err_str
        );
    }

    // ── Test 4.4 — draw_credit on non-existent line → CreditLineNotFound (3) ──

    /// `draw_credit` invokes `apply_accrual` AFTER loading the credit line.
    /// When no line exists the code must be `CreditLineNotFound` (3).
    #[test]
    fn draw_head_accrual_not_found_emits_code_3() {
        let (env, contract_id, _admin, _token) = setup_with_token(10_000_i128);
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.draw_credit(&borrower, &100_i128);
        }));
        assert!(result.is_err());

        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#3"),
            "expected CreditLineNotFound (#3), got: {:?}",
            err_str
        );
    }

    // ── Test 4.5 — repay_credit on closed line → CreditLineClosed (4) ──

    /// Once a line is `Closed`, any operation that invokes `apply_accrual` at
    /// its head (including `repay_credit`) MUST return
    /// `CreditLineClosed` (4).
    #[test]
    fn repay_head_accrual_closed_emits_code_4() {
        let (env, contract_id, admin, _token) = setup_with_token(10_000_i128);
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);

        client.open_credit_line(&borrower, &1000_i128, &500_u32, &50_u32);
        client.close_credit_line(&borrower, &admin);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.repay_credit(&borrower, &100_i128);
        }));
        assert!(result.is_err());

        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#4"),
            "expected CreditLineClosed (#4), got: {:?}",
            err_str
        );
    }

    // ── Test 4.6 — draw_credit on suspended line → CreditLineSuspended (20) ──

    #[test]
    fn draw_head_accrual_suspended_emits_code_20() {
        let (env, contract_id, _admin, _token) = setup_with_token(10_000_i128);
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

    // ── Test 4.7 — draw_credit on defaulted line → CreditLineDefaulted (21) ──

    #[test]
    fn draw_head_accrual_defaulted_emits_code_21() {
        let (env, contract_id, _admin, _token) = setup_with_token(10_000_i128);
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

    // ── Test 4.8 — admin_not_initialized accrual-path caller → AdminNotInitialized (32) ──

    /// Any state-changing entrypoint that internally calls `apply_accrual`
    /// must first pass the admin-initialization gate when admin gates apply.
    #[test]
    fn accrual_caller_fails_admin_not_initialized_code_32() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);

        // No `client.init` call → admin not yet set.
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

    // ── Test 4.9 — determinism: same accrual-overflow inputs → same code (12) twice ──

    /// Reproducibility guard: two independent runs with identical
    /// overflow-triggering inputs MUST both encode `#12` — no flakiness, no
    /// fallback to a different error code.
    #[test]
    fn accrual_overflow_discriminant_is_deterministic_code_12_twice() {
        let huge_principal: i128 = i128::MAX / 2;

        for run in 1..=2 {
            let (env, contract_id, _admin, _token) = setup_with_token(huge_principal);
            let client = CreditClient::new(&env, &contract_id);
            let borrower = Address::generate(&env);

            client.open_credit_line(&borrower, &huge_principal, &10_000_u32, &50_u32);
            env.ledger().set_timestamp(1);
            client.draw_credit(&borrower, &huge_principal);
            env.ledger().set_timestamp(2);

            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                client.update_risk_parameters(&borrower, &i128::MAX, &10_000_u32, &50_u32);
            }));
            assert!(result.is_err(), "run {} must revert", run);

            let err_str = extract_error_str(&result.unwrap_err());
            assert!(
                err_str.contains("#12"),
                "run {}: expected Overflow (#12), got: {:?}",
                run,
                err_str
            );
        }
    }

    // ── Test 4.10 — accrue_batch boundary (exactly 50) must succeed, (51) fails with #5 ──

    /// Exact-boundary pinning. ACCRUE_BATCH_MAX = 50 is the cap; exactly 50
    /// borrowers must succeed and 51 must fail with `InvalidAmount` (#5).
    #[test]
    fn accrue_batch_boundary_exact_50_ok_51_code_5() {
        let (env, contract_id, _admin, _token) = setup_with_token(0_i128);
        let client = CreditClient::new(&env, &contract_id);

        // Exactly 50 → OK (even with non-existent borrowers, accrue_batch skips silently).
        let mut batch_ok: Vec<Address> = Vec::new(&env);
        for _ in 0..ACCRUE_BATCH_MAX {
            batch_ok.push_back(Address::generate(&env));
        }
        assert_eq!(batch_ok.len() as u32, ACCRUE_BATCH_MAX);
        // Must not panic.
        client.accrue_batch(&batch_ok);

        // 51 → InvalidAmount (#5).
        let mut batch_bad: Vec<Address> = Vec::new(&env);
        for _ in 0..=ACCRUE_BATCH_MAX {
            batch_bad.push_back(Address::generate(&env));
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.accrue_batch(&batch_bad);
        }));
        assert!(result.is_err());
        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#5"),
            "boundary 51 must be InvalidAmount (#5), got: {:?}",
            err_str
        );
    }
}
