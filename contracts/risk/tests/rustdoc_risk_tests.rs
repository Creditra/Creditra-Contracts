// SPDX-License-Identifier: MIT

//! Focused rustdoc coverage tests for `creditra-risk` public entrypoints.
//!
//! Validates that all public APIs documented in `lib.rs` behave as their
//! rustdoc specifies, with particular emphasis on storage side-effects,
//! event emission, and error paths.

use creditra_risk::{
    ContractError, ContractErrorCategory, RiskAdminCooldownConfiguredEvent, RiskContract,
    RiskContractClient,
};
use soroban_sdk::{
    testutils::{Address as _, Events, Ledger},
    symbol_short, Address, Env, IntoVal, Symbol, TryFromVal, TryIntoVal,
};

// ── Helpers ───────────────────────────────────────────────────────────────

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(RiskContract, ());
    let client = RiskContractClient::new(&env, &contract_id);
    client.init(&admin);
    (env, admin, contract_id)
}

// ── Test: ContractError::category() ──────────────────────────────────────

/// Validate that every [`ContractError`] variant maps to the correct
/// [`ContractErrorCategory`] as documented.
#[test]
fn test_contract_error_category() {
    assert_eq!(
        ContractError::Unauthorized.category(),
        ContractErrorCategory::Auth,
        "Unauthorized must map to Auth"
    );
    assert_eq!(
        ContractError::NotAdmin.category(),
        ContractErrorCategory::Auth,
        "NotAdmin must map to Auth"
    );
    assert_eq!(
        ContractError::Paused.category(),
        ContractErrorCategory::Risk,
        "Paused must map to Risk"
    );
    assert_eq!(
        ContractError::RiskAdminCooldownActive.category(),
        ContractErrorCategory::Risk,
        "RiskAdminCooldownActive must map to Risk"
    );
}

// ── Test: init() storage side-effect ─────────────────────────────────────

/// Validate that [`RiskContract::init`] stores the admin address under
/// the documented `"admin"` key in instance storage.
#[test]
fn test_init_stores_admin() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(RiskContract, ());
    let client = RiskContractClient::new(&env, &contract_id);

    client.init(&admin);

    let retrieved: Address = env
        .as_contract(&contract_id, || {
            env.storage()
                .instance()
                .get::<_, Address>(&Symbol::new(&env, "admin"))
        })
        .expect("admin key must be set after init");

    assert_eq!(retrieved, admin, "stored admin must match initialized value");
}

// ── Test: set_risk_admin_cooldown() event emission ───────────────────────

/// Validate that [`RiskContract::set_risk_admin_cooldown`] publishes a
/// [`RiskAdminCooldownConfiguredEvent`] with the correct topic and payload.
///
/// Soroban events are tuples `(contract_id, topics: Vec<Val>, data: Val)`.
#[test]
fn test_set_risk_admin_cooldown_publishes_event() {
    let (env, _admin, contract_id) = setup();
    let client = RiskContractClient::new(&env, &contract_id);

    client.set_risk_admin_cooldown(&3_600);

    let events = env.events().all();
    assert_eq!(events.len(), 1, "exactly one event must be published");

    let event = events.get(0).unwrap();
    // event is (contract_id: Address, topics: Vec<Val>, data: Val)
    let topics = &event.1;
    let data = event.2.clone();
    assert_eq!(
        Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap(),
        symbol_short!("risk"),
        "first topic must be 'risk'"
    );
    assert_eq!(
        Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
        symbol_short!("rad_cool"),
        "second topic must be 'rad_cool'"
    );

    let payload: RiskAdminCooldownConfiguredEvent = data.try_into_val(&env).unwrap();
    assert_eq!(
        payload.cooldown_seconds, 3_600,
        "event payload must match the configured cooldown"
    );
}

// ── Test: get_risk_admin_cooldown() default value ────────────────────────

/// Validate that [`RiskContract::get_risk_admin_cooldown`] returns `0`
/// (disabled) when no cooldown has been configured, as documented.
#[test]
fn test_get_risk_admin_cooldown_default_is_zero() {
    let (env, _admin, contract_id) = setup();
    let client = RiskContractClient::new(&env, &contract_id);

    let cooldown = client.get_risk_admin_cooldown();
    assert_eq!(
        cooldown, 0,
        "get_risk_admin_cooldown must return 0 when unconfigured"
    );
}

// ── Test: get_risk_admin_cooldown() reads stored value ───────────────────

/// Validate that [`RiskContract::get_risk_admin_cooldown`] correctly reads
/// the value written by [`RiskContract::set_risk_admin_cooldown`].
#[test]
fn test_get_risk_admin_cooldown_reads_stored_value() {
    let (env, _admin, contract_id) = setup();
    let client = RiskContractClient::new(&env, &contract_id);

    client.set_risk_admin_cooldown(&7_200);
    let cooldown = client.get_risk_admin_cooldown();

    assert_eq!(
        cooldown, 7_200,
        "get_risk_admin_cooldown must return the configured value"
    );
}

// ── Test: record_risk_admin_action() writes timestamp ────────────────────

/// Validate that [`RiskContract::record_risk_admin_action`] writes
/// `env.ledger().timestamp()` to storage under the `"rad_last"` key.
#[test]
fn test_record_risk_admin_action_writes_timestamp() {
    let (env, _admin, contract_id) = setup();
    let client = RiskContractClient::new(&env, &contract_id);

    env.ledger().with_mut(|li| li.timestamp = 10_000);
    client.record_risk_admin_action();

    let stored_ts: u64 = env
        .as_contract(&contract_id, || {
            env.storage()
                .instance()
                .get::<_, u64>(&Symbol::new(&env, "rad_last"))
        })
        .expect("rad_last key must be set after record_risk_admin_action");

    assert_eq!(
        stored_ts, 10_000,
        "stored timestamp must match ledger.timestamp"
    );
}

// ── Test: record_risk_admin_action() enforces cooldown ───────────────────

/// Validate that [`RiskContract::record_risk_admin_action`] reverts with
/// [`ContractError::RiskAdminCooldownActive`] when called during an active
/// cooldown window, as documented.
#[test]
#[should_panic]
fn test_record_risk_admin_action_enforces_cooldown() {
    let (env, _admin, contract_id) = setup();
    let client = RiskContractClient::new(&env, &contract_id);

    client.set_risk_admin_cooldown(&3_600);

    env.ledger().with_mut(|li| li.timestamp = 1_000);
    client.record_risk_admin_action();

    // Second call at t=2_000 (1_000 seconds later) is still within the
    // 3_600-second cooldown — must revert.
    env.ledger().with_mut(|li| li.timestamp = 2_000);
    client.record_risk_admin_action();
}

// ── Test: record_risk_admin_action() allows after cooldown elapsed ───────

/// Validate that [`RiskContract::record_risk_admin_action`] succeeds when
/// the cooldown interval has fully elapsed since the last action.
#[test]
fn test_record_risk_admin_action_allows_after_cooldown() {
    let (env, _admin, contract_id) = setup();
    let client = RiskContractClient::new(&env, &contract_id);

    client.set_risk_admin_cooldown(&3_600);

    env.ledger().with_mut(|li| li.timestamp = 1_000);
    client.record_risk_admin_action();

    // Advance exactly 3_600 seconds — cooldown is now elapsed.
    env.ledger().with_mut(|li| li.timestamp = 4_600);
    client.record_risk_admin_action();
}

// ── Test: get_admin() returns initialized admin ──────────────────────────

/// Validate that [`RiskContract::get_admin`] returns the admin address
/// set during [`RiskContract::init`].
#[test]
fn test_get_admin_returns_initialized_admin() {
    let (env, admin, contract_id) = setup();
    let client = RiskContractClient::new(&env, &contract_id);

    let retrieved_admin = client.get_admin();
    assert_eq!(
        retrieved_admin, admin,
        "get_admin must return the admin set during init"
    );
}

// ── Test: get_admin() panics when not initialized ────────────────────────

/// Validate that [`RiskContract::get_admin`] panics with `"admin not
/// initialized"` when called before [`RiskContract::init`], as documented.
#[test]
#[should_panic(expected = "admin not initialized")]
fn test_get_admin_panics_when_not_initialized() {
    let env = Env::default();
    let contract_id = env.register(RiskContract, ());
    let client = RiskContractClient::new(&env, &contract_id);

    // init has not been called — get_admin must panic.
    let _ = client.get_admin();
}

// ── Test: RiskAdminCooldownConfiguredEvent round-trip ─────────────────────

/// Validate that [`RiskAdminCooldownConfiguredEvent`] can be serialized and
/// deserialized correctly. We use set_risk_admin_cooldown to trigger the real
/// event emission path and verify the payload round-trips through the event log.
#[test]
fn test_risk_admin_cooldown_configured_event_roundtrip() {
    let (env, _admin, contract_id) = setup();
    let client = RiskContractClient::new(&env, &contract_id);

    client.set_risk_admin_cooldown(&1_800);

    let all_events = env.events().all();
    assert!(!all_events.is_empty(), "at least one event must be emitted");

    // Find the rad_cool event (the one emitted by set_risk_admin_cooldown).
    let event = all_events
        .iter()
        .find(|e| {
            if let Some(v) = e.1.get(1) {
                Symbol::try_from_val(&env, &v)
                    .map(|s| s == symbol_short!("rad_cool"))
                    .unwrap_or(false)
            } else {
                false
            }
        })
        .expect("rad_cool event must be present");

    // all_events entries are (contract_id, topics, data) tuples
    let data = event.2.clone();
    let retrieved: RiskAdminCooldownConfiguredEvent = data.try_into_val(&env).unwrap();

    assert_eq!(
        retrieved.cooldown_seconds, 1_800,
        "event payload must round-trip correctly"
    );
}

// ── Test: ContractErrorCategory discriminants are stable ─────────────────

/// Validate that [`ContractErrorCategory`] discriminants are ABI-stable.
#[test]
fn test_contract_error_category_discriminants() {
    assert_eq!(
        ContractErrorCategory::Auth as u32,
        1,
        "Auth discriminant must be 1"
    );
    assert_eq!(
        ContractErrorCategory::Risk as u32,
        6,
        "Risk discriminant must be 6"
    );
}

// ── Test: ContractError discriminants are stable ─────────────────────────

/// Validate that [`ContractError`] discriminants are ABI-stable as
/// documented in the rustdoc table.
#[test]
fn test_contract_error_discriminants() {
    assert_eq!(
        ContractError::Unauthorized as u32,
        1,
        "Unauthorized discriminant must be 1"
    );
    assert_eq!(
        ContractError::NotAdmin as u32,
        2,
        "NotAdmin discriminant must be 2"
    );
    assert_eq!(
        ContractError::Paused as u32,
        3,
        "Paused discriminant must be 3"
    );
    assert_eq!(
        ContractError::RiskAdminCooldownActive as u32,
        54,
        "RiskAdminCooldownActive discriminant must be 54"
    );
}

// ── Test: set_risk_admin_cooldown storage side-effect ────────────────────

/// Validate that [`RiskContract::set_risk_admin_cooldown`] writes the
/// configured value to `Symbol("rad_cool")` in instance storage.
#[test]
fn test_set_risk_admin_cooldown_writes_storage() {
    let (env, _admin, contract_id) = setup();
    let client = RiskContractClient::new(&env, &contract_id);

    client.set_risk_admin_cooldown(&5_400);

    let stored: u64 = env
        .as_contract(&contract_id, || {
            env.storage()
                .instance()
                .get::<_, u64>(&Symbol::new(&env, "rad_cool"))
        })
        .expect("rad_cool key must be set after set_risk_admin_cooldown");

    assert_eq!(
        stored, 5_400,
        "stored cooldown must match the configured value"
    );
}

// ── Test: Cooldown disabled with seconds=0 ───────────────────────────────

/// Validate that setting `seconds=0` disables cooldown enforcement, as
/// documented in [`RiskContract::set_risk_admin_cooldown`].
#[test]
fn test_cooldown_disabled_with_zero() {
    let (env, _admin, contract_id) = setup();
    let client = RiskContractClient::new(&env, &contract_id);

    client.set_risk_admin_cooldown(&0);

    env.ledger().with_mut(|li| li.timestamp = 1_000);
    client.record_risk_admin_action();

    // Immediate second call at t=1_001 should succeed (cooldown disabled).
    env.ledger().with_mut(|li| li.timestamp = 1_001);
    client.record_risk_admin_action();
}
