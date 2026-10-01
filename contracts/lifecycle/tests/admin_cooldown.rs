// SPDX-License-Identifier: MIT

//! Regression tests for `AdminLifecycleCooldownActive` (v7 lifecycle admin cool-off).

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env};

const START_TS: u64 = 10_000;
const COOLDOWN_SECONDS: u64 = 120;

fn setup(start_ts: u64) -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = start_ts);

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);
    client.set_liquidity_source(&contract_id);

    (env, contract_id, admin)
}

fn set_timestamp(env: &Env, timestamp: u64) {
    env.ledger().with_mut(|li| li.timestamp = timestamp);
}

fn assert_admin_lifecycle_cooldown_active(result: std::thread::Result<()>, context: &str) {
    assert!(result.is_err(), "{context}: expected panic for active cooldown");
    let err = result.unwrap_err();
    let err_str = if let Some(s) = err.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = err.downcast_ref::<&str>() {
        s.to_string()
    } else {
        String::new()
    };
    assert!(
        err_str.contains("#56") || err_str.contains("AdminLifecycleCooldownActive"),
        "{context}: expected AdminLifecycleCooldownActive (#56), got {err_str:?}"
    );
}

#[test]
fn admin_lifecycle_cooldown_zero_disables_guard() {
    let (env, contract_id, _admin) = setup(START_TS);
    let client = CreditClient::new(&env, &contract_id);

    client.set_admin_lifecycle_cooldown_seconds(&0_u64);
    assert_eq!(client.get_admin_lifecycle_cooldown_seconds(), Some(0));

    // Zero cooldown: consecutive critical actions at the same timestamp succeed
    client.set_credit_limit_bounds(&100_i128, &1_000_000_i128);
    client.set_credit_limit_bounds(&200_i128, &2_000_000_i128);

    let (min, max) = client.get_credit_limit_bounds();
    assert_eq!(min, Some(200));
    assert_eq!(max, Some(2_000_000));
}

#[test]
fn admin_lifecycle_cooldown_rejects_before_boundary_and_allows_at_boundary() {
    let (env, contract_id, _admin) = setup(START_TS);
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);

    client.set_admin_lifecycle_cooldown_seconds(&COOLDOWN_SECONDS);
    assert_eq!(client.get_admin_lifecycle_cooldown_seconds(), Some(COOLDOWN_SECONDS));

    // First action starts the cooldown clock
    client.set_credit_limit_bounds(&100_i128, &1_000_000_i128);
    assert_eq!(
        client.get_last_admin_lifecycle_critical_action_ts(),
        Some(START_TS)
    );

    // Second action before cooldown elapsed fails
    set_timestamp(&env, START_TS + COOLDOWN_SECONDS - 1);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.set_per_borrower_liquidation_grace(&borrower, &60_u64);
    }));
    assert_admin_lifecycle_cooldown_active(
        result,
        "second action before cooldown boundary must revert"
    );

    // Second action at boundary succeeds
    set_timestamp(&env, START_TS + COOLDOWN_SECONDS);
    client.set_per_borrower_liquidation_grace(&borrower, &60_u64);
    assert_eq!(
        client.get_last_admin_lifecycle_critical_action_ts(),
        Some(START_TS + COOLDOWN_SECONDS)
    );
    assert_eq!(client.get_per_borrower_liquidation_grace(&borrower), 60);
}

#[test]
fn configuring_lifecycle_cooldown_does_not_consume_cooldown_window() {
    let (env, contract_id, _admin) = setup(START_TS);
    let client = CreditClient::new(&env, &contract_id);

    client.set_admin_lifecycle_cooldown_seconds(&COOLDOWN_SECONDS);

    // Setting/updating the cooldown configuration itself should not consume the cooldown clock
    set_timestamp(&env, START_TS + 10);
    client.set_admin_lifecycle_cooldown_seconds(&COOLDOWN_SECONDS);

    // This action at t=20 should succeed because the clock hasn't been set by configuration
    set_timestamp(&env, START_TS + 20);
    client.set_credit_limit_bounds(&100_i128, &1_000_000_i128);
    assert_eq!(
        client.get_last_admin_lifecycle_critical_action_ts(),
        Some(START_TS + 20)
    );

    // A consecutive change at t=30 should be blocked
    set_timestamp(&env, START_TS + 30);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.set_late_fee_flat(&100_i128);
    }));
    assert_admin_lifecycle_cooldown_active(result, "consecutive action within window must revert");
}

#[test]
fn different_lifecycle_critical_actions_share_single_cooldown_clock() {
    let (env, contract_id, _admin) = setup(START_TS);
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);

    client.set_admin_lifecycle_cooldown_seconds(&COOLDOWN_SECONDS);

    // Action 1: set limit bounds
    client.set_credit_limit_bounds(&100_i128, &1_000_000_i128);

    // Action 2: set late fee (should fail)
    set_timestamp(&env, START_TS + 10);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.set_late_fee_flat(&100_i128);
    }));
    assert_admin_lifecycle_cooldown_active(result, "different action must share same cooldown anchor");
}
