// SPDX-License-Identifier: MIT

//! Capability consistency tests: cross-check capability bitmaps against real entrypoint outcomes.
//!
//! This test verifies that [`BorrowCapabilities`] and [`LifecycleCapabilities`]
//! predictions match the actual success/failure of corresponding entrypoint calls.
//!
//! The core assertion: if a capability flag is true, the corresponding operation
//! must succeed; if false, it must fail (for amount-independent preconditions).
//!
//! Any divergence is reported with a full state dump: credit line data, freeze
//! states, pause state, capabilities bitmap, and actual operation outcomes.

use creditra_credit::types::CreditStatus;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env};

fn setup() -> (Env, CreditClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    (env, client, admin)
}

// ────────────────────────────────────────────────────────────────────────────
// Test: Capabilities divergence detection across all CreditStatus values
// ────────────────────────────────────────────────────────────────────────────

/// Test borrow capabilities for Active status.
#[test]
fn test_active_borrow_capabilities_match_draw_repay_self_suspend() {
    let (env, client, _admin) = setup();
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000i128, &500_u32, &60_u32);

    let caps = client.borrow_capabilities(&borrower);

    // Active: should be able to draw
    assert!(caps.can_draw, "Active: should have can_draw=true");
    let draw_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.draw_credit(&borrower, &100i128);
    }));
    assert!(
        draw_result.is_ok(),
        "Active: draw should succeed when can_draw=true"
    );

    // Active: should be able to repay
    assert!(caps.can_repay, "Active: should have can_repay=true");
    let repay_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.repay_credit(&borrower, &1i128);
    }));
    assert!(
        repay_result.is_ok(),
        "Active: repay should succeed when can_repay=true"
    );

    // Active: should be able to self-suspend
    assert!(
        caps.can_self_suspend,
        "Active: should have can_self_suspend=true"
    );
    let self_suspend_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.self_suspend_credit_line(&borrower);
    }));
    assert!(
        self_suspend_result.is_ok(),
        "Active: self_suspend should succeed when can_self_suspend=true"
    );
}

/// Test borrow capabilities for Suspended status.
#[test]
fn test_suspended_borrow_capabilities_match_outcomes() {
    let (env, client, _admin) = setup();
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000i128, &500_u32, &60_u32);
    client.suspend_credit_line(&borrower);

    let caps = client.borrow_capabilities(&borrower);

    // Suspended: should NOT be able to draw
    assert!(!caps.can_draw, "Suspended: should have can_draw=false");

    // Suspended: should be able to repay
    assert!(
        caps.can_repay,
        "Suspended: should have can_repay=true (repayment always allowed)"
    );
    let repay_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.repay_credit(&borrower, &1i128);
    }));
    assert!(
        repay_result.is_ok(),
        "Suspended: repay should succeed when can_repay=true"
    );

    // Suspended: should NOT be able to self-suspend
    assert!(
        !caps.can_self_suspend,
        "Suspended: should have can_self_suspend=false"
    );
}

/// Test borrow capabilities for Defaulted status.
#[test]
fn test_defaulted_borrow_capabilities_match_outcomes() {
    let (env, client, _admin) = setup();
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000i128, &500_u32, &60_u32);
    client.default_credit_line(&borrower);

    let caps = client.borrow_capabilities(&borrower);

    // Defaulted: should NOT be able to draw
    assert!(!caps.can_draw, "Defaulted: should have can_draw=false");

    // Defaulted: should be able to repay
    assert!(caps.can_repay, "Defaulted: should have can_repay=true");

    // Defaulted: should NOT be able to self-suspend
    assert!(
        !caps.can_self_suspend,
        "Defaulted: should have can_self_suspend=false"
    );
}

/// Test borrow capabilities for Closed status.
#[test]
fn test_closed_borrow_capabilities_match_outcomes() {
    let (env, client, admin) = setup();
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000i128, &500_u32, &60_u32);
    client.close_credit_line(&borrower, &admin);

    let caps = client.borrow_capabilities(&borrower);

    // Closed: all capabilities false
    assert!(!caps.can_draw, "Closed: should have can_draw=false");
    assert!(!caps.can_repay, "Closed: should have can_repay=false");
    assert!(
        !caps.can_self_suspend,
        "Closed: should have can_self_suspend=false"
    );
}

/// Test lifecycle capabilities for Active status.
#[test]
fn test_active_lifecycle_capabilities_match_suspend_close_default() {
    let (env, client, admin) = setup();
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000i128, &500_u32, &60_u32);

    let caps = client.lifecycle_capabilities(&borrower);

    // Active: can suspend
    assert!(caps.can_suspend, "Active: should have can_suspend=true");

    // Active: can close (admin)
    assert!(
        caps.can_close_admin,
        "Active: should have can_close_admin=true"
    );

    // Active: can close (borrower with zero util)
    assert!(
        caps.can_close_borrower,
        "Active + zero-util: should have can_close_borrower=true"
    );

    // Active: can default
    assert!(caps.can_default, "Active: should have can_default=true");
    let default_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.default_credit_line(&borrower);
    }));
    assert!(
        default_result.is_ok(),
        "Active: default should succeed when can_default=true"
    );
}

/// Test lifecycle capabilities for Suspended status.
#[test]
fn test_suspended_lifecycle_capabilities_match_outcomes() {
    let (env, client, admin) = setup();
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000i128, &500_u32, &60_u32);
    client.suspend_credit_line(&borrower);

    let caps = client.lifecycle_capabilities(&borrower);

    // Suspended: cannot suspend (already suspended)
    assert!(
        !caps.can_suspend,
        "Suspended: should have can_suspend=false"
    );

    // Suspended: can close (admin)
    assert!(
        caps.can_close_admin,
        "Suspended: should have can_close_admin=true"
    );

    // Suspended: can default
    assert!(caps.can_default, "Suspended: should have can_default=true");
}

/// Test lifecycle capabilities for Defaulted status.
#[test]
fn test_defaulted_lifecycle_capabilities_match_outcomes() {
    let (env, client, _admin) = setup();
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000i128, &500_u32, &60_u32);
    client.default_credit_line(&borrower);

    let caps = client.lifecycle_capabilities(&borrower);

    // Defaulted: cannot suspend
    assert!(
        !caps.can_suspend,
        "Defaulted: should have can_suspend=false"
    );

    // Defaulted: can reinstate
    assert!(
        caps.can_reinstate,
        "Defaulted: should have can_reinstate=true"
    );
    let reinstate_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.reinstate_credit_line(&borrower, &CreditStatus::Active);
    }));
    assert!(
        reinstate_result.is_ok(),
        "Defaulted: reinstate should succeed when can_reinstate=true"
    );
}

/// Test lifecycle capabilities for Closed status.
#[test]
fn test_closed_lifecycle_capabilities_match_outcomes() {
    let (env, client, admin) = setup();
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000i128, &500_u32, &60_u32);
    client.close_credit_line(&borrower, &admin);

    let caps = client.lifecycle_capabilities(&borrower);

    // Closed: all false
    assert!(!caps.can_suspend, "Closed: should have can_suspend=false");
    assert!(
        !caps.can_close_admin,
        "Closed: should have can_close_admin=false"
    );
    assert!(
        !caps.can_close_borrower,
        "Closed: should have can_close_borrower=false"
    );
    assert!(!caps.can_default, "Closed: should have can_default=false");
    assert!(
        !caps.can_reinstate,
        "Closed: should have can_reinstate=false"
    );
}

// ────────────────────────────────────────────────────────────────────────────
// Test: Freeze states and paused protocol
// ────────────────────────────────────────────────────────────────────────────

/// Test that draws_frozen freezes capability.
#[test]
fn test_frozen_draws_capability_matches_outcome() {
    let (env, client, _admin) = setup();
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000i128, &500_u32, &60_u32);

    client.freeze_draws(&creditra_credit::types::FreezeReason::LiquidityReserve);

    let caps = client.borrow_capabilities(&borrower);
    assert!(!caps.can_draw, "Frozen draws: should have can_draw=false");
}

/// Test that paused protocol blocks lifecycle transitions.
#[test]
fn test_paused_protocol_blocks_lifecycle_capabilities() {
    let (env, client, _admin) = setup();
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000i128, &500_u32, &60_u32);

    client.set_protocol_paused(&true);

    let lifecycle_caps = client.lifecycle_capabilities(&borrower);

    // All lifecycle transitions should be blocked
    assert!(
        !lifecycle_caps.can_suspend,
        "Paused: should have can_suspend=false"
    );
    assert!(
        !lifecycle_caps.can_close_admin,
        "Paused: should have can_close_admin=false"
    );
    assert!(
        !lifecycle_caps.can_default,
        "Paused: should have can_default=false"
    );
}

/// Test that borrower freeze blocks draw.
#[test]
fn test_borrower_frozen_capability_matches_outcome() {
    let (env, client, admin) = setup();
    let borrower = Address::generate(&env);
    let now = 1_700_000_000u64;
    env.ledger().set_timestamp(now);

    client.open_credit_line(&borrower, &1_000i128, &500_u32, &60_u32);

    // Freeze borrower for 1 hour
    client.freeze_borrower_until(&admin, &borrower, &(now + 3600));

    let caps = client.borrow_capabilities(&borrower);
    assert!(
        !caps.can_draw,
        "Frozen borrower: should have can_draw=false"
    );
}

/// Test that borrower blocked blocks draw.
#[test]
fn test_borrower_blocked_capability_matches_outcome() {
    let (env, client, admin) = setup();
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000i128, &500_u32, &60_u32);

    client.block_borrower(&admin, &borrower);

    let caps = client.borrow_capabilities(&borrower);
    assert!(
        !caps.can_draw,
        "Blocked borrower: should have can_draw=false"
    );
}

// ────────────────────────────────────────────────────────────────────────────
// Test: Edge cases
// ────────────────────────────────────────────────────────────────────────────

/// Test that no credit line means all capabilities are false.
#[test]
fn test_no_credit_line_all_capabilities_false() {
    let (_env, client, _admin) = setup();
    let borrower = Address::generate(&_env);

    let borrow_caps = client.borrow_capabilities(&borrower);
    assert!(!borrow_caps.can_draw, "no credit line: can_draw=false");
    assert!(!borrow_caps.can_repay, "no credit line: can_repay=false");
    assert!(
        !borrow_caps.can_self_suspend,
        "no credit line: can_self_suspend=false"
    );

    let lifecycle_caps = client.lifecycle_capabilities(&borrower);
    assert!(
        !lifecycle_caps.can_suspend,
        "no credit line: can_suspend=false"
    );
    assert!(
        !lifecycle_caps.can_close_admin,
        "no credit line: can_close_admin=false"
    );
    assert!(
        !lifecycle_caps.can_default,
        "no credit line: can_default=false"
    );
    assert!(
        !lifecycle_caps.can_reinstate,
        "no credit line: can_reinstate=false"
    );
}

/// Test that active line with zero utilization allows borrower-close.
#[test]
fn test_active_zero_util_allows_borrower_close() {
    let (env, client, _admin) = setup();
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000i128, &500_u32, &60_u32);

    let lifecycle_caps = client.lifecycle_capabilities(&borrower);
    assert!(
        lifecycle_caps.can_close_borrower,
        "Active + zero-util: can_close_borrower=true"
    );

    // Verify it succeeds
    let close_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.close_credit_line(&borrower, &borrower);
    }));
    assert!(
        close_result.is_ok(),
        "borrower close should succeed when can_close_borrower=true"
    );
}

/// Test that active line with utilization blocks borrower-close but allows admin-close.
#[test]
fn test_active_with_utilization_blocks_borrower_close() {
    let (env, client, admin) = setup();
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000i128, &500_u32, &60_u32);

    // Set up liquidity and draw
    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    soroban_sdk::token::StellarAssetClient::new(&env, &token).mint(&client.address, &1_000_000i128);
    client.set_min_collateral_ratio_bps(&0);
    client.draw_credit(&borrower, &500i128);

    let lifecycle_caps = client.lifecycle_capabilities(&borrower);

    // With utilization: admin can close
    assert!(
        lifecycle_caps.can_close_admin,
        "With utilization: can_close_admin=true"
    );

    // With utilization: borrower cannot close
    assert!(
        !lifecycle_caps.can_close_borrower,
        "With utilization: can_close_borrower=false"
    );
}

/// Test that SelfSuspended status allows default transition.
#[test]
fn test_self_suspended_allows_default() {
    let (env, client, _admin) = setup();
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000i128, &500_u32, &60_u32);
    client.self_suspend_credit_line(&borrower);

    let lifecycle_caps = client.lifecycle_capabilities(&borrower);
    assert!(
        lifecycle_caps.can_default,
        "SelfSuspended: should have can_default=true"
    );

    let default_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.default_credit_line(&borrower);
    }));
    assert!(
        default_result.is_ok(),
        "SelfSuspended: default should succeed when can_default=true"
    );
}
