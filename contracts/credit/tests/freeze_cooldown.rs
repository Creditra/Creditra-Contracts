// SPDX-License-Identifier: MIT

//! Admin freeze cooldown tests for the Credit contract.
//!
//! # Coverage
//! - Cooldown is disabled by default (no `set_freeze_cooldown` call)
//! - Setting a cooldown and verifying it blocks rapid freeze actions
//! - Cooldown applies to all freeze/unfreeze entrypoints:
//!   - freeze_draws
//!   - unfreeze_draws
//!   - freeze_credit_line
//!   - unfreeze_credit_line
//!   - freeze_borrower_until
//!   - unfreeze_borrower
//! - Setting cooldown to 0 disables it
//! - Cooldown resets after the configured interval elapses
//! - Cooldown is not retroactive (first action always succeeds)
//! - get_freeze_cooldown returns None when not configured
//! - Cooldown is shared across all freeze action types

use creditra_credit::{Credit, CreditClient, FreezeReason};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env};

// ── helpers ──────────────────────────────────────────────────────────────────

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    (env, admin, contract_id)
}

fn setup_with_token() -> (Env, Address, Address, Address) {
    let (env, admin, contract_id) = setup();
    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();
    let client = CreditClient::new(&env, &contract_id);
    client.set_liquidity_token(&token_address);
    (env, admin, contract_id, token_address)
}

// ── Cooldown defaults ────────────────────────────────────────────────────────

#[test]
fn freeze_cooldown_disabled_by_default() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    let cooldown = client.get_freeze_cooldown();
    assert!(cooldown.is_none(), "cooldown should be None by default");
}

#[test]
fn freeze_actions_succeed_when_cooldown_disabled() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    // Both actions should succeed without any cooldown
    client.freeze_draws(&FreezeReason::LiquidityReserve);
    client.freeze_draws(&FreezeReason::Compliance);
    // No panic = success
}

// ── Cooldown enforcement ─────────────────────────────────────────────────────

#[test]
fn freeze_draws_blocked_within_cooldown() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_freeze_cooldown(&3600); // 1 hour cooldown

    // First freeze succeeds
    client.freeze_draws(&FreezeReason::LiquidityReserve);

    // Second freeze within cooldown fails
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.freeze_draws(&FreezeReason::Compliance);
    }));
    assert!(
        result.is_err(),
        "second freeze_draws must fail within cooldown"
    );
}

#[test]
fn unfreeze_draws_blocked_within_cooldown() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_freeze_cooldown(&3600);

    // First action succeeds
    client.freeze_draws(&FreezeReason::LiquidityReserve);

    // Unfreeze within cooldown fails
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.unfreeze_draws();
    }));
    assert!(
        result.is_err(),
        "unfreeze_draws must fail within cooldown"
    );
}

#[test]
fn freeze_credit_line_blocked_within_cooldown() {
    let (env, _admin, contract_id, _token_address) = setup_with_token();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);
    let borrower2 = Address::generate(&env);

    client.open_credit_line(&borrower, &1_000, &300, &50);
    client.open_credit_line(&borrower2, &1_000, &300, &50);

    client.set_freeze_cooldown(&3600);

    // First freeze succeeds
    client.freeze_credit_line(&borrower, &FreezeReason::Compliance);

    // Second freeze within cooldown fails
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.freeze_credit_line(&borrower2, &FreezeReason::RiskInvestigation);
    }));
    assert!(
        result.is_err(),
        "second freeze_credit_line must fail within cooldown"
    );
}

#[test]
fn unfreeze_credit_line_blocked_within_cooldown() {
    let (env, _admin, contract_id, _token_address) = setup_with_token();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &1_000, &300, &50);
    client.freeze_credit_line(&borrower, &FreezeReason::Compliance);

    client.set_freeze_cooldown(&3600);

    // First unfreeze succeeds
    client.unfreeze_credit_line(&borrower);

    // Second unfreeze within cooldown fails
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.unfreeze_credit_line(&borrower);
    }));
    assert!(
        result.is_err(),
        "second unfreeze_credit_line must fail within cooldown"
    );
}

#[test]
fn freeze_borrower_until_blocked_within_cooldown() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let admin = _admin.clone();
    let borrower = Address::generate(&env);
    let borrower2 = Address::generate(&env);

    client.set_freeze_cooldown(&3600);

    let now = env.ledger().timestamp();
    let future = now + 86_400; // 24 hours from now

    // First freeze succeeds
    client.freeze_borrower_until(&admin, &borrower, &future);

    // Second freeze within cooldown fails
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.freeze_borrower_until(&admin, &borrower2, &future);
    }));
    assert!(
        result.is_err(),
        "second freeze_borrower_until must fail within cooldown"
    );
}

#[test]
fn unfreeze_borrower_blocked_within_cooldown() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let admin = _admin.clone();
    let borrower = Address::generate(&env);

    let now = env.ledger().timestamp();
    let future = now + 86_400;
    client.freeze_borrower_until(&admin, &borrower, &future);

    client.set_freeze_cooldown(&3600);

    // First unfreeze succeeds
    client.unfreeze_borrower(&admin, &borrower);

    // Second unfreeze within cooldown fails
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.unfreeze_borrower(&admin, &borrower);
    }));
    assert!(
        result.is_err(),
        "second unfreeze_borrower must fail within cooldown"
    );
}

// ── Cross-entrypoint cooldown sharing ────────────────────────────────────────

#[test]
fn cooldown_is_shared_across_freeze_types() {
    let (env, _admin, contract_id, _token_address) = setup_with_token();
    let client = CreditClient::new(&env, &contract_id);
    let admin = _admin.clone();
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &1_000, &300, &50);

    client.set_freeze_cooldown(&3600);

    // Freeze draws (global)
    client.freeze_draws(&FreezeReason::LiquidityReserve);

    // Try to freeze credit line within cooldown - must fail
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.freeze_credit_line(&borrower, &FreezeReason::Compliance);
    }));
    assert!(
        result.is_err(),
        "freeze_credit_line must fail within cooldown after freeze_draws"
    );

    // Try to freeze borrower within cooldown - must fail
    let now = env.ledger().timestamp();
    let future = now + 86_400;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.freeze_borrower_until(&admin, &borrower, &future);
    }));
    assert!(
        result.is_err(),
        "freeze_borrower_until must fail within cooldown after freeze_draws"
    );
}

// ── Cooldown expiry ──────────────────────────────────────────────────────────

#[test]
fn freeze_action_succeeds_after_cooldown_expires() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_freeze_cooldown(&60); // 60 second cooldown

    // First freeze
    client.freeze_draws(&FreezeReason::LiquidityReserve);

    // Advance time past the cooldown
    let now = env.ledger().timestamp();
    env.ledger().set_timestamp(now + 61);

    // Second freeze should succeed
    client.freeze_draws(&FreezeReason::Compliance);
    // No panic = success
}

#[test]
fn freeze_action_succeeds_at_exact_cooldown_boundary() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_freeze_cooldown(&60);

    // First freeze
    client.freeze_draws(&FreezeReason::LiquidityReserve);

    // Advance time exactly to the cooldown boundary
    let now = env.ledger().timestamp();
    env.ledger().set_timestamp(now + 60);

    // Should succeed at exact boundary (now >= last_ts + cooldown)
    client.freeze_draws(&FreezeReason::Compliance);
}

// ── Disabling cooldown ───────────────────────────────────────────────────────

#[test]
fn setting_cooldown_to_zero_disables_it() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_freeze_cooldown(&3600);

    // First freeze with cooldown active
    client.freeze_draws(&FreezeReason::LiquidityReserve);

    // Disable cooldown
    client.set_freeze_cooldown(&0);
    assert!(client.get_freeze_cooldown().is_none());

    // Second freeze should succeed immediately
    client.freeze_draws(&FreezeReason::Compliance);
}

// ── Error discriminant ───────────────────────────────────────────────────────

#[test]
#[should_panic(expected = "Error(Contract, #54)")]
fn freeze_cooldown_active_error_has_correct_discriminant() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_freeze_cooldown(&3600);
    client.freeze_draws(&FreezeReason::LiquidityReserve);

    // This should panic with #54 (FreezeCooldownActive)
    client.freeze_draws(&FreezeReason::Compliance);
}

// ── First action always succeeds ─────────────────────────────────────────────

#[test]
fn first_freeze_action_always_succeeds_even_with_cooldown() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_freeze_cooldown(&3600);

    // Very first action after setting cooldown should succeed
    client.freeze_draws(&FreezeReason::LiquidityReserve);
    // No panic = success
}

// ── get_freeze_cooldown ──────────────────────────────────────────────────────

#[test]
fn get_freeze_cooldown_returns_configured_value() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    assert!(client.get_freeze_cooldown().is_none());

    client.set_freeze_cooldown(&7200);
    assert_eq!(client.get_freeze_cooldown(), Some(7200));

    client.set_freeze_cooldown(&300);
    assert_eq!(client.get_freeze_cooldown(), Some(300));
}

// ── set_freeze_cooldown requires admin auth ──────────────────────────────────

#[test]
#[should_panic]
fn set_freeze_cooldown_requires_admin_auth() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    client.set_freeze_cooldown(&3600);
}
