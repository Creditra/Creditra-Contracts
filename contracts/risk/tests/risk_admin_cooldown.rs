// SPDX-License-Identifier: MIT
//
//! Risk admin cooldown tests for the Risk contract.
//!
//! # Coverage
//! - Admin can set and get the cooldown duration
//! - Cooldown of 0 disables enforcement (backward compatible)
//! - Cooldown blocks rapid successive risk mutations
//! - Cooldown elapses correctly after the configured interval
//! - Non-admin cannot set the cooldown
//! - First action always succeeds even with cooldown configured

use creditra_risk::{RiskContract, RiskContractClient};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env};

// ── helpers ──────────────────────────────────────────────────────────────────

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(RiskContract, ());
    let client = RiskContractClient::new(&env, &contract_id);
    client.init(&admin);
    (env, admin, contract_id)
}

// ── set/get cooldown ─────────────────────────────────────────────────────────

#[test]
fn admin_can_set_and_get_cooldown() {
    let (_env, _admin, contract_id) = setup();
    let client = RiskContractClient::new(&_env, &contract_id);

    assert_eq!(client.get_risk_admin_cooldown(), 0, "default should be 0");

    client.set_risk_admin_cooldown(&3600);
    assert_eq!(
        client.get_risk_admin_cooldown(),
        3600,
        "should return the configured value"
    );
}

#[test]
fn admin_can_disable_cooldown() {
    let (_env, _admin, contract_id) = setup();
    let client = RiskContractClient::new(&_env, &contract_id);

    client.set_risk_admin_cooldown(&3600);
    assert_eq!(client.get_risk_admin_cooldown(), 3600);

    client.set_risk_admin_cooldown(&0);
    assert_eq!(
        client.get_risk_admin_cooldown(),
        0,
        "cooldown should be disabled"
    );
}

#[test]
#[should_panic]
fn non_admin_cannot_set_cooldown() {
    let (_env, _admin, contract_id) = setup();
    _env.mock_all_auths_allowing_non_root_auth();
    let non_admin = Address::generate(&_env);
    let client = RiskContractClient::new(&_env, &contract_id);

    non_admin.require_auth();
    client.set_risk_admin_cooldown(&3600);
}

// ── cooldown enforcement ─────────────────────────────────────────────────────

#[test]
fn cooldown_zero_does_not_block_risk_update() {
    let (env, _admin, contract_id) = setup();
    let client = RiskContractClient::new(&env, &contract_id);

    // Cooldown is 0 (disabled by default) — should allow immediate successive updates.
    env.ledger().with_mut(|li| li.timestamp = 1000);
    client.record_risk_admin_action();

    env.ledger().with_mut(|li| li.timestamp = 1001);
    client.record_risk_admin_action();
}

#[test]
fn cooldown_blocks_immediate_successive_action() {
    let (env, _admin, contract_id) = setup();
    let client = RiskContractClient::new(&env, &contract_id);

    // Set 1-hour cooldown.
    client.set_risk_admin_cooldown(&3600);

    // First action at t=1000 succeeds.
    env.ledger().with_mut(|li| li.timestamp = 1000);
    client.record_risk_admin_action();

    // Second action at t=1001 (< 1 hour since last) should fail.
    env.ledger().with_mut(|li| li.timestamp = 1001);
    let result = client.try_record_risk_admin_action();
    assert!(result.is_err(), "should fail during cooldown");
}

#[test]
fn cooldown_elapses_correctly() {
    let (env, _admin, contract_id) = setup();
    let client = RiskContractClient::new(&env, &contract_id);

    // Set 1-hour (3600s) cooldown.
    client.set_risk_admin_cooldown(&3600);

    // First action at t=1000.
    env.ledger().with_mut(|li| li.timestamp = 1000);
    client.record_risk_admin_action();

    // Still within cooldown at t=3000 (< 1000 + 3600 = 4600).
    env.ledger().with_mut(|li| li.timestamp = 3000);
    let result = client.try_record_risk_admin_action();
    assert!(result.is_err(), "should still be in cooldown at t=3000");

    // Cooldown elapsed at t=4600 (1000 + 3600).
    env.ledger().with_mut(|li| li.timestamp = 4600);
    client.record_risk_admin_action();
}

// ── first action always succeeds ─────────────────────────────────────────────

#[test]
fn first_action_always_succeeds_even_with_cooldown() {
    let (env, _admin, contract_id) = setup();
    let client = RiskContractClient::new(&env, &contract_id);

    client.set_risk_admin_cooldown(&3600);

    // Even at a very late timestamp with no prior action recorded, it should succeed.
    env.ledger().with_mut(|li| li.timestamp = 999_999_999);
    client.record_risk_admin_action();
}
