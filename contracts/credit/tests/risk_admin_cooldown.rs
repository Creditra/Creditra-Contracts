// SPDX-License-Identifier: MIT

//! Risk admin cooldown tests for the Credit contract.
//!
//! # Coverage
//! - Admin can set and get the cooldown duration
//! - Cooldown of 0 disables enforcement (backward compatible)
//! - Cooldown blocks rapid successive `update_risk_parameters` calls
//! - Cooldown elapses correctly after the configured interval
//! - Non-admin cannot set the cooldown
//! - Cooldown is respected after protocol pause/unpause
//! - Event is emitted on cooldown configuration
//! - Cooldown does NOT block non-risk-admin operations (e.g. pause)

use creditra_credit::types::ContractError;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Events, Ledger};
use soroban_sdk::{symbol_short, Address, Env, Symbol, TryFromVal};

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

fn setup_with_borrower() -> (Env, Address, Address, Address) {
    let (env, admin, contract_id) = setup();
    let borrower = Address::generate(&env);
    let client = CreditClient::new(&env, &contract_id);
    client.open_credit_line(&borrower, &10_000_i128, &500_u32, &70_u32);
    (env, admin, contract_id, borrower)
}

// ── set/get cooldown ─────────────────────────────────────────────────────────

#[test]
fn admin_can_set_and_get_cooldown() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

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
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

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
    let (env, _admin, contract_id) = setup();
    env.mock_all_auths_allowing_non_root_auth();
    let non_admin = Address::generate(&env);
    let client = CreditClient::new(&env, &contract_id);

    non_admin.require_auth();
    client.set_risk_admin_cooldown(&3600);
}

// ── cooldown enforcement ─────────────────────────────────────────────────────

#[test]
fn cooldown_zero_does_not_block_risk_update() {
    let (env, _admin, contract_id, borrower) = setup_with_borrower();
    let client = CreditClient::new(&env, &contract_id);

    // Cooldown is 0 (disabled by default) — should allow immediate successive updates.
    env.ledger().with_mut(|li| li.timestamp = 1000);
    client.update_risk_parameters(&borrower, &10_000_i128, &600_u32, &80_u32);

    env.ledger().with_mut(|li| li.timestamp = 1001);
    client.update_risk_parameters(&borrower, &10_000_i128, &700_u32, &90_u32);
}

#[test]
fn cooldown_blocks_immediate_successive_update() {
    let (env, _admin, contract_id, borrower) = setup_with_borrower();
    let client = CreditClient::new(&env, &contract_id);

    // Set 1-hour cooldown.
    client.set_risk_admin_cooldown(&3600);

    // First update at t=1000 succeeds.
    env.ledger().with_mut(|li| li.timestamp = 1000);
    client.update_risk_parameters(&borrower, &10_000_i128, &600_u32, &80_u32);

    // Second update at t=1001 (< 1 hour since last) should fail.
    env.ledger().with_mut(|li| li.timestamp = 1001);
    let result = client.try_update_risk_parameters(&borrower, &10_000_i128, &700_u32, &90_u32);
    assert!(result.is_err(), "should fail during cooldown");
    let err = result.err().unwrap();
    assert_eq!(
        err.unwrap(),
        ContractError::RiskAdminCooldownActive.into(),
        "expected RiskAdminCooldownActive error"
    );
}

#[test]
fn cooldown_elapses_correctly() {
    let (env, _admin, contract_id, borrower) = setup_with_borrower();
    let client = CreditClient::new(&env, &contract_id);

    // Set 1-hour (3600s) cooldown.
    client.set_risk_admin_cooldown(&3600);

    // First update at t=1000.
    env.ledger().with_mut(|li| li.timestamp = 1000);
    client.update_risk_parameters(&borrower, &10_000_i128, &600_u32, &80_u32);

    // Still within cooldown at t=3000 (< 1000 + 3600 = 4600).
    env.ledger().with_mut(|li| li.timestamp = 3000);
    let result = client.try_update_risk_parameters(&borrower, &10_000_i128, &700_u32, &90_u32);
    assert!(result.is_err(), "should still be in cooldown at t=3000");

    // Cooldown elapsed at t=4600 (1000 + 3600).
    env.ledger().with_mut(|li| li.timestamp = 4600);
    client.update_risk_parameters(&borrower, &10_000_i128, &800_u32, &95_u32);
}

#[test]
fn cooldown_is_per_action_not_per_borrower() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let borrower1 = Address::generate(&env);
    let borrower2 = Address::generate(&env);

    client.open_credit_line(&borrower1, &10_000_i128, &500_u32, &70_u32);
    client.open_credit_line(&borrower2, &10_000_i128, &500_u32, &70_u32);

    client.set_risk_admin_cooldown(&3600);

    // Update borrower1 at t=1000.
    env.ledger().with_mut(|li| li.timestamp = 1000);
    client.update_risk_parameters(&borrower1, &10_000_i128, &600_u32, &80_u32);

    // Update borrower2 at t=1001 should also be blocked (cooldown is global).
    env.ledger().with_mut(|li| li.timestamp = 1001);
    let result = client.try_update_risk_parameters(&borrower2, &10_000_i128, &600_u32, &80_u32);
    assert!(result.is_err(), "cooldown is global, not per-borrower");
}

// ── event emission ───────────────────────────────────────────────────────────

#[test]
fn set_cooldown_emits_event() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_risk_admin_cooldown(&7200);

    let all_events = env.events().all();
    let event = all_events.last().unwrap();
    let topics = event.1;

    // Topic: ("credit", "rad_cooldown")
    assert_eq!(
        soroban_sdk::Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap(),
        symbol_short!("credit"),
    );
    assert_eq!(
        soroban_sdk::Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
        Symbol::new(&env, "rad_cooldown"),
    );

    // Payload: RiskAdminCooldownConfiguredEvent { cooldown_seconds: 7200 }
    // The payload is decoded from the 3rd topic slot in Soroban events
    let payload = &event.2;
    let decoded: creditra_credit::events::RiskAdminCooldownConfiguredEvent =
        soroban_sdk::TryFromVal::try_from_val(&env, payload).unwrap();
    assert_eq!(decoded.cooldown_seconds, 7200);
}

// ── interaction with pause ───────────────────────────────────────────────────

#[test]
fn cooldown_config_requires_unpaused() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_protocol_paused(&true);

    let result = client.try_set_risk_admin_cooldown(&3600);
    assert!(result.is_err(), "should fail when paused");
}

#[test]
fn cooldown_blocks_even_after_unpause() {
    let (env, _admin, contract_id, borrower) = setup_with_borrower();
    let client = CreditClient::new(&env, &contract_id);

    client.set_risk_admin_cooldown(&3600);

    // First update at t=1000.
    env.ledger().with_mut(|li| li.timestamp = 1000);
    client.update_risk_parameters(&borrower, &10_000_i128, &600_u32, &80_u32);

    // Pause and unpause — cooldown should still be in effect.
    client.set_protocol_paused(&true);
    client.set_protocol_paused(&false);

    env.ledger().with_mut(|li| li.timestamp = 1001);
    let result = client.try_update_risk_parameters(&borrower, &10_000_i128, &700_u32, &90_u32);
    assert!(result.is_err(), "cooldown should survive pause/unpause");
}

// ── first action always succeeds ─────────────────────────────────────────────

#[test]
fn first_risk_update_always_succeeds_even_with_cooldown() {
    let (env, _admin, contract_id, borrower) = setup_with_borrower();
    let client = CreditClient::new(&env, &contract_id);

    client.set_risk_admin_cooldown(&3600);

    // Even at a very late timestamp with no prior action recorded, it should succeed.
    env.ledger().with_mut(|li| li.timestamp = 999_999_999);
    client.update_risk_parameters(&borrower, &10_000_i128, &600_u32, &80_u32);
}
