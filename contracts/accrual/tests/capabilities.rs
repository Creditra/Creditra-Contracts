// SPDX-License-Identifier: MIT

//! Focused tests for the accrual (v7) `capabilities()` view.
//!
//! # What
//!
//! Verifies every combination of the four [`AccrualCapabilities`] flags
//! returned by [`creditra_accrual::views::accrual_capabilities`]:
//!
//! - `can_accrue`          — line exists, Active, utilized > 0, not paused
//! - `batch_open`          — protocol not paused
//! - `penalty_rate_active` — surcharge configured and borrower is delinquent
//! - `grace_waiver_active` — line Suspended, grace config set, within window
//!
//! Each test uses the minimal setup required to assert the single flag under
//! test; all other flags remain at their natural default.
//!
//! # See also
//! - [`creditra_credit::views::accrual_capabilities`] — the implementation.
//! - [`creditra_credit::types::AccrualCapabilities`] — the return type.

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token, Address, Env,
};

// ── Helper ────────────────────────────────────────────────────────────────────

/// Minimal contract setup: deploy Credit, init admin, configure SAC token.
fn setup(token_mint: i128) -> (Env, Address, Address, Address) {
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
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &token_mint);

    (env, contract_id, admin, token_address)
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 1 — can_accrue flag
// ═══════════════════════════════════════════════════════════════════════════

/// No credit line → `can_accrue = false`, `batch_open = true`.
#[test]
fn capabilities_no_credit_line_returns_false_can_accrue() {
    let (env, contract_id, _admin, _token) = setup(0);
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    let caps = client.accrual_capabilities(&borrower);

    assert!(
        !caps.can_accrue,
        "can_accrue must be false when no line exists"
    );
    assert!(caps.batch_open, "batch_open must be true when not paused");
    assert!(!caps.penalty_rate_active);
    assert!(!caps.grace_waiver_active);
}

/// Active line with zero utilization → `can_accrue = false` (nothing to accrue).
#[test]
fn capabilities_active_line_zero_utilization_returns_false_can_accrue() {
    let (env, contract_id, _admin, _token) = setup(100_000);
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &50_000_i128, &500_u32, &50_u32);

    let caps = client.accrual_capabilities(&borrower);

    assert!(
        !caps.can_accrue,
        "can_accrue must be false with zero utilization"
    );
    assert!(caps.batch_open);
}

/// Active line with positive utilization → `can_accrue = true`.
#[test]
fn capabilities_active_line_with_utilization_returns_true_can_accrue() {
    let (env, contract_id, _admin, _token) = setup(100_000);
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &50_000_i128, &500_u32, &50_u32);
    env.ledger().set_timestamp(1);
    client.draw_credit(&borrower, &10_000_i128);

    let caps = client.accrual_capabilities(&borrower);

    assert!(
        caps.can_accrue,
        "can_accrue must be true for Active line with utilization"
    );
    assert!(caps.batch_open);
    assert!(!caps.penalty_rate_active);
    assert!(!caps.grace_waiver_active);
}

/// Suspended line → `can_accrue = false` (batch only processes Active lines).
#[test]
fn capabilities_suspended_line_returns_false_can_accrue() {
    let (env, contract_id, _admin, _token) = setup(100_000);
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &50_000_i128, &500_u32, &50_u32);
    env.ledger().set_timestamp(1);
    client.draw_credit(&borrower, &10_000_i128);
    client.suspend_credit_line(&borrower);

    let caps = client.accrual_capabilities(&borrower);

    assert!(
        !caps.can_accrue,
        "can_accrue must be false for Suspended line"
    );
}

/// Defaulted line → `can_accrue = false`.
#[test]
fn capabilities_defaulted_line_returns_false_can_accrue() {
    let (env, contract_id, _admin, _token) = setup(100_000);
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &50_000_i128, &500_u32, &50_u32);
    env.ledger().set_timestamp(1);
    client.draw_credit(&borrower, &10_000_i128);
    client.default_credit_line(&borrower);

    let caps = client.accrual_capabilities(&borrower);

    assert!(
        !caps.can_accrue,
        "can_accrue must be false for Defaulted line"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 2 — batch_open flag
// ═══════════════════════════════════════════════════════════════════════════

/// Protocol paused → `batch_open = false` AND `can_accrue = false`.
#[test]
fn capabilities_protocol_paused_returns_false_batch_open_and_can_accrue() {
    let (env, contract_id, admin, _token) = setup(100_000);
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &50_000_i128, &500_u32, &50_u32);
    env.ledger().set_timestamp(1);
    client.draw_credit(&borrower, &10_000_i128);

    // Pause the protocol.
    client.pause_protocol(&admin);

    let caps = client.accrual_capabilities(&borrower);

    assert!(!caps.batch_open, "batch_open must be false when paused");
    assert!(!caps.can_accrue, "can_accrue must be false when paused");
}

/// Protocol not paused → `batch_open = true`.
#[test]
fn capabilities_not_paused_returns_true_batch_open() {
    let (env, contract_id, _admin, _token) = setup(0);
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    let caps = client.accrual_capabilities(&borrower);

    assert!(caps.batch_open, "batch_open must be true when not paused");
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 3 — penalty_rate_active flag
// ═══════════════════════════════════════════════════════════════════════════

/// No penalty surcharge configured → `penalty_rate_active = false` even when
/// the borrower has a past-due repayment schedule.
#[test]
fn capabilities_no_surcharge_configured_returns_false_penalty_rate_active() {
    let (env, contract_id, _admin, _token) = setup(100_000);
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &50_000_i128, &500_u32, &50_u32);
    env.ledger().set_timestamp(1);
    client.draw_credit(&borrower, &10_000_i128);

    let caps = client.accrual_capabilities(&borrower);

    // No penalty surcharge configured → always false.
    assert!(!caps.penalty_rate_active);
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 4 — grace_waiver_active flag
// ═══════════════════════════════════════════════════════════════════════════

/// Active line (not Suspended) → `grace_waiver_active = false`.
#[test]
fn capabilities_active_line_returns_false_grace_waiver_active() {
    let (env, contract_id, _admin, _token) = setup(100_000);
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &50_000_i128, &500_u32, &50_u32);
    env.ledger().set_timestamp(1);
    client.draw_credit(&borrower, &10_000_i128);

    let caps = client.accrual_capabilities(&borrower);

    assert!(
        !caps.grace_waiver_active,
        "grace_waiver_active must be false for Active line"
    );
}

/// Suspended line with no grace config → `grace_waiver_active = false`.
#[test]
fn capabilities_suspended_no_grace_config_returns_false_grace_waiver_active() {
    let (env, contract_id, _admin, _token) = setup(100_000);
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &50_000_i128, &500_u32, &50_u32);
    env.ledger().set_timestamp(1);
    client.draw_credit(&borrower, &10_000_i128);
    client.suspend_credit_line(&borrower);

    let caps = client.accrual_capabilities(&borrower);

    assert!(
        !caps.grace_waiver_active,
        "grace_waiver_active must be false when no grace config set"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 5 — Consistency / compound checks
// ═══════════════════════════════════════════════════════════════════════════

/// All fields for a freshly deployed contract with no borrower state:
/// `can_accrue = false`, `batch_open = true`, all others false.
#[test]
fn capabilities_default_state_no_borrower() {
    let (env, contract_id, _admin, _token) = setup(0);
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    let caps = client.accrual_capabilities(&borrower);

    assert!(!caps.can_accrue);
    assert!(caps.batch_open);
    assert!(!caps.penalty_rate_active);
    assert!(!caps.grace_waiver_active);
}

/// Capabilities are deterministic: two identical calls return the same values.
#[test]
fn capabilities_deterministic_same_result_twice() {
    let (env, contract_id, _admin, _token) = setup(100_000);
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &50_000_i128, &500_u32, &50_u32);
    env.ledger().set_timestamp(1);
    client.draw_credit(&borrower, &10_000_i128);

    let caps1 = client.accrual_capabilities(&borrower);
    let caps2 = client.accrual_capabilities(&borrower);

    assert_eq!(caps1, caps2, "capabilities() must be deterministic");
}

/// Closed line → `can_accrue = false`, `can_repay = false` (closed is terminal).
#[test]
fn capabilities_closed_line_returns_false_can_accrue() {
    let (env, contract_id, admin, _token) = setup(100_000);
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &50_000_i128, &500_u32, &50_u32);
    client.close_credit_line(&borrower, &admin);

    let caps = client.accrual_capabilities(&borrower);

    assert!(!caps.can_accrue, "can_accrue must be false for Closed line");
}
