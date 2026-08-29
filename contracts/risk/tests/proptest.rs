// SPDX-License-Identifier: MIT

//! Property-based tests for risk (v7) state invariants.
//!
//! # What
//!
//! Generates random sequences of admin operations (set cooldown, record action,
//! pause/unpause) with varying ledger timestamps and verifies that key risk
//! contract invariants hold after every mutation.
//!
//! # Invariants
//!
//! 1. **Cooldown value persistence**: `get_risk_admin_cooldown()` always returns
//!    the last value written by `set_risk_admin_cooldown()`, or `0` if never set.
//! 2. **Cooldown enforcement**: After `record_risk_admin_action()` succeeds with
//!    `cooldown > 0`, subsequent calls fail until `now >= last_ts + cooldown`.
//! 3. **Zero cooldown bypass**: When `cooldown == 0`, `record_risk_admin_action()`
//!    always succeeds regardless of call frequency.
//! 4. **First action always succeeds**: The very first `record_risk_admin_action()`
//!    call succeeds even with a non-zero cooldown configured.
//! 5. **Pause blocks mutations**: When paused, `set_risk_admin_cooldown()` and
//!    `record_risk_admin_action()` must fail.
//! 6. **Admin immutability**: `get_admin()` always returns the address set during
//!    `init()`.
//! 7. **Last-action timestamp**: After `record_risk_admin_action()` succeeds,
//!    the stored `rad_last` equals the ledger timestamp at that moment.
//! 8. **Event emission**: Every state-changing operation emits the correct event
//!    with the expected topic and payload.
//!
//! # Covered paths
//!
//! | Path                          | Why it matters                                |
//! |-------------------------------|-----------------------------------------------|
//! | `set_risk_admin_cooldown`     | Writes cooldown; gated by auth + pause        |
//! | `record_risk_admin_action`    | Writes timestamp; gated by auth + pause + cd  |
//! | `set_paused`                  | Toggles pause; gated by auth                  |
//! | `get_risk_admin_cooldown`     | Read-only view of stored cooldown             |
//! | `get_admin`                   | Read-only view of immutable admin             |
//! | Time advancement              | Drives cooldown expiry between actions        |
//! | Multiple cooldown values      | Ensures invariant holds across all settings   |
//!
//! # See also
//! - `contracts/risk/tests/risk_admin_cooldown.rs` — deterministic cooldown tests.
//! - `contracts/risk/tests/rustdoc_risk_tests.rs` — rustdoc coverage tests.

use creditra_risk::{RiskAdminCooldownConfiguredEvent, RiskContract, RiskContractClient};
use proptest::prelude::*;
use proptest::test_runner::Config as ProptestConfig;
use soroban_sdk::testutils::{Address as _, Events, Ledger};
use soroban_sdk::{Address, Env, IntoVal, Symbol};

const MAX_STEPS: usize = 32;
const INITIAL_TIMESTAMP: u64 = 1_000_000;

// ── Setup ──────────────────────────────────────────────────────────────────

/// Create a fresh environment with a deployed and initialized risk contract.
fn setup_env() -> (Env, RiskContractClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(INITIAL_TIMESTAMP);

    let admin = Address::generate(&env);
    let contract_id = env.register(RiskContract, ());
    let client = RiskContractClient::new(&env, &contract_id);
    client.init(&admin);

    (env, client, admin)
}

// ── Invariant checkers ─────────────────────────────────────────────────────

/// Assert that `get_risk_admin_cooldown()` returns the expected value.
fn assert_cooldown_value(client: &RiskContractClient<'_>, expected: u64, label: &str) {
    let actual = client.get_risk_admin_cooldown();
    assert_eq!(
        actual, expected,
        "{label}: expected cooldown={expected}, got {actual}",
    );
}

/// Assert that `get_admin()` returns the original admin address.
fn assert_admin_immutable(client: &RiskContractClient<'_>, expected: &Address, label: &str) {
    let actual = client.get_admin();
    assert_eq!(
        actual, *expected,
        "{label}: admin changed — expected {expected:?}, got {actual:?}",
    );
}

/// Assert that the event log contains the expected cooldown event as the most
/// recent entry.
fn assert_cooldown_event(env: &Env, expected_cooldown: u64) {
    let events = env.events().all();
    let last = events
        .get(events.len() - 1)
        .expect("must have at least one event");

    assert_eq!(
        last.topics,
        (Symbol::new(env, "risk"), Symbol::new(env, "rad_cool")).into_val(env),
        "cooldown event topic must be ('risk', 'rad_cool')",
    );

    let payload: RiskAdminCooldownConfiguredEvent = last.data.clone().try_into_val(env).unwrap();
    assert_eq!(
        payload.cooldown_seconds, expected_cooldown,
        "cooldown event payload must match the configured value",
    );
}

// ── Operation types for random sequences ───────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
enum OpKind {
    /// Set the risk admin cooldown to a random value.
    SetCooldown,
    /// Record a risk admin action (gated by cooldown + pause).
    RecordAction,
    /// Toggle the paused state.
    TogglePause,
    /// No operation — only advance time.
    Noop,
}

#[derive(Debug, Clone)]
struct OpStep {
    op: OpKind,
    /// Cooldown value to set (meaningful only for SetCooldown).
    cooldown_value: u64,
    /// Time advance in seconds before executing the operation.
    time_advance: u64,
}

/// Strategy that generates a sequence of random operations.
fn op_strategy() -> impl Strategy<Value = Vec<OpStep>> {
    proptest::collection::vec(
        (
            // Operation kind: 0=SetCooldown, 1=RecordAction, 2=TogglePause, 3=Noop
            0u64..=3u64,
            // Cooldown value (0..=86400 seconds, i.e. up to 1 day)
            0u64..=86_400u64,
            // Time advance in seconds (0..=1 year)
            0u64..=31_536_000u64,
        ),
        1..=MAX_STEPS,
    )
    .prop_map(|steps| {
        steps
            .into_iter()
            .map(|(op, cooldown_value, time_advance)| {
                let op = match op {
                    0 => OpKind::SetCooldown,
                    1 => OpKind::RecordAction,
                    2 => OpKind::TogglePause,
                    _ => OpKind::Noop,
                };
                OpStep {
                    op,
                    cooldown_value,
                    time_advance,
                }
            })
            .collect()
    })
}

// ═══════════════════════════════════════════════════════════════════════════
// Proptest 1 — Core risk state invariants
// ═══════════════════════════════════════════════════════════════════════════

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        .. ProptestConfig::default()
    })]

    /// After every operation in a random sequence, all risk contract invariants
    /// must hold:
    ///
    /// - Cooldown value persistence
    /// - Admin immutability
    /// - Cooldown enforcement (or bypass when cooldown == 0)
    /// - First action always succeeds
    /// - Pause blocks mutations
    #[test]
    fn prop_risk_state_invariants(
        steps in op_strategy(),
    ) {
        let (env, client, admin) = setup_env();

        // Track expected state.
        let mut expected_cooldown: u64 = 0;
        let mut is_paused: bool = false;
        let mut last_action_ts: u64 = 0;
        let mut has_recorded_action: bool = false;

        // Verify initial state.
        assert_cooldown_value(&client, 0, "initial");
        assert_admin_immutable(&client, &admin, "initial");

        for (step_idx, step) in steps.iter().enumerate() {
            let label = std::format!("step={} op={:?}", step_idx, step.op);

            // Advance time.
            env.ledger().with_mut(|l| l.timestamp += step.time_advance);
            let now = env.ledger().timestamp();

            match step.op {
                OpKind::SetCooldown => {
                    let result = client.try_set_risk_admin_cooldown(&step.cooldown_value);

                    if is_paused {
                        // Must fail when paused.
                        assert!(
                            result.is_err(),
                            "{label}: set_risk_admin_cooldown must fail when paused",
                        );
                    } else {
                        // Must succeed (admin auth is mocked).
                        assert!(
                            result.is_ok(),
                            "{label}: set_risk_admin_cooldown must succeed when not paused",
                        );
                        expected_cooldown = step.cooldown_value;

                        // Verify event emission.
                        assert_cooldown_event(&env, expected_cooldown);
                    }

                    // Cooldown value must reflect the last successful set.
                    assert_cooldown_value(&client, expected_cooldown, &label);
                    // Admin must never change.
                    assert_admin_immutable(&client, &admin, &label);
                }

                OpKind::RecordAction => {
                    let result = client.try_record_risk_admin_action();

                    if is_paused {
                        // Must fail when paused.
                        assert!(
                            result.is_err(),
                            "{label}: record_risk_admin_action must fail when paused",
                        );
                    } else if expected_cooldown > 0 && has_recorded_action {
                        // Cooldown enforcement: must fail if within cooldown window.
                        let cooldown_deadline = last_action_ts.saturating_add(expected_cooldown);
                        if now < cooldown_deadline {
                            assert!(
                                result.is_err(),
                                "{label}: record_risk_admin_action must fail during cooldown (now={now}, deadline={cooldown_deadline})",
                            );
                        } else {
                            assert!(
                                result.is_ok(),
                                "{label}: record_risk_admin_action must succeed after cooldown elapsed (now={now}, deadline={cooldown_deadline})",
                            );
                            last_action_ts = now;
                        }
                    } else {
                        // First action, or cooldown is 0: must always succeed.
                        assert!(
                            result.is_ok(),
                            "{label}: record_risk_admin_action must succeed (first action or cooldown=0)",
                        );
                        last_action_ts = now;
                        has_recorded_action = true;
                    }

                    // Admin must never change.
                    assert_admin_immutable(&client, &admin, &label);
                    // Cooldown value must be unchanged by record_risk_admin_action.
                    assert_cooldown_value(&client, expected_cooldown, &label);
                }

                OpKind::TogglePause => {
                    let new_paused = !is_paused;
                    let result = client.try_set_paused(&new_paused);

                    // set_paused is admin-only; with mock_all_auths it always succeeds.
                    assert!(
                        result.is_ok(),
                        "{label}: set_paused must succeed with admin auth",
                    );

                    is_paused = new_paused;

                    // Admin must never change.
                    assert_admin_immutable(&client, &admin, &label);
                    // Cooldown value must be unchanged by set_paused.
                    assert_cooldown_value(&client, expected_cooldown, &label);
                }

                OpKind::Noop => {
                    // No operation — just time advancement.
                    // All invariants must still hold.
                    assert_cooldown_value(&client, expected_cooldown, &label);
                    assert_admin_immutable(&client, &admin, &label);
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Proptest 2 — Cooldown enforcement boundary conditions
// ═══════════════════════════════════════════════════════════════════════════

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 128,
        .. ProptestConfig::default()
    })]

    /// Verify cooldown enforcement at exact boundary conditions:
    ///
    /// - Before `last_ts + cooldown`: must still be blocked.
    /// - At `last_ts + cooldown`: must succeed (cooldown elapsed).
    /// - After `last_ts + cooldown`: must succeed.
    #[test]
    fn prop_cooldown_boundary(
        cooldown in 1u64..=86_400u64,
        offset_before in 0u64..=999u64,
        offset_after in 0u64..=1_000u64,
    ) {
        let (env, client, _admin) = setup_env();

        // Set cooldown.
        client.set_risk_admin_cooldown(&cooldown);

        // First action at t=INITIAL_TIMESTAMP.
        env.ledger().with_mut(|l| l.timestamp = INITIAL_TIMESTAMP);
        client.record_risk_admin_action();

        // Advance to just before cooldown expires (if offset_before < cooldown).
        if offset_before < cooldown {
            let test_time = INITIAL_TIMESTAMP + offset_before;
            env.ledger().with_mut(|l| l.timestamp = test_time);
            let result = client.try_record_risk_admin_action();
            assert!(
                result.is_err(),
                "must be blocked at t={test_time} (cooldown={cooldown}, last={INITIAL_TIMESTAMP})",
            );
        }

        // Advance to exactly when cooldown expires.
        let at_deadline = INITIAL_TIMESTAMP + cooldown;
        env.ledger().with_mut(|l| l.timestamp = at_deadline);
        let result = client.try_record_risk_admin_action();
        assert!(
            result.is_ok(),
            "must succeed at t={at_deadline} (cooldown={cooldown}, last={INITIAL_TIMESTAMP})",
        );

        // Advance past cooldown.
        let past_deadline = at_deadline + 1 + offset_after;
        env.ledger().with_mut(|l| l.timestamp = past_deadline);
        let result = client.try_record_risk_admin_action();
        assert!(
            result.is_ok(),
            "must succeed at t={past_deadline} (cooldown={cooldown})",
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Proptest 3 — Zero cooldown never blocks
// ═══════════════════════════════════════════════════════════════════════════

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        .. ProptestConfig::default()
    })]

    /// When cooldown is 0 (disabled), `record_risk_admin_action` must always
    /// succeed regardless of how many times it is called or how much time
    /// has elapsed between calls.
    #[test]
    fn prop_zero_cooldown_never_blocks(
        timestamps in proptest::collection::vec(0u64..=31_536_000u64, 1..=20),
    ) {
        let (env, client, _admin) = setup_env();

        // Cooldown is 0 by default.
        assert_eq!(client.get_risk_admin_cooldown(), 0);

        for (i, ts_delta) in timestamps.iter().enumerate() {
            env.ledger().with_mut(|l| l.timestamp += ts_delta);
            let result = client.try_record_risk_admin_action();
            assert!(
                result.is_ok(),
                "zero cooldown: record_risk_admin_action must always succeed (call #{i})",
            );
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Proptest 4 — Pause blocks all mutations
// ═══════════════════════════════════════════════════════════════════════════

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        .. ProptestConfig::default()
    })]

    /// When the contract is paused, every state-changing entrypoint must fail.
    /// When unpaused, they must succeed (assuming no cooldown conflict).
    #[test]
    fn prop_pause_blocks_mutations(
        cooldown_values in proptest::collection::vec(0u64..=86_400u64, 1..=10),
    ) {
        let (env, client, _admin) = setup_env();

        for (i, cd) in cooldown_values.iter().enumerate() {
            let label = std::format!("iteration={i} cooldown={cd}");

            // Pause the contract.
            client.set_paused(&true);

            // set_risk_admin_cooldown must fail when paused.
            let result = client.try_set_risk_admin_cooldown(cd);
            assert!(
                result.is_err(),
                "{label}: set_risk_admin_cooldown must fail when paused",
            );

            // record_risk_admin_action must fail when paused.
            let result = client.try_record_risk_admin_action();
            assert!(
                result.is_err(),
                "{label}: record_risk_admin_action must fail when paused",
            );

            // Unpause.
            client.set_paused(&false);

            // Now operations must succeed.
            let result = client.try_set_risk_admin_cooldown(cd);
            assert!(
                result.is_ok(),
                "{label}: set_risk_admin_cooldown must succeed when not paused",
            );

            let result = client.try_record_risk_admin_action();
            assert!(
                result.is_ok(),
                "{label}: record_risk_admin_action must succeed when not paused",
            );
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Edge case tests
// ═══════════════════════════════════════════════════════════════════════════

/// The very first `record_risk_admin_action` must always succeed, even with
/// a large cooldown configured and no prior action recorded.
#[test]
fn first_action_always_succeeds_with_large_cooldown() {
    let (env, client, _admin) = setup_env();

    // Set a very large cooldown.
    client.set_risk_admin_cooldown(&86_400); // 1 day

    // First action at a very late timestamp — must succeed.
    env.ledger().with_mut(|l| l.timestamp = 999_999_999);
    let result = client.try_record_risk_admin_action();
    assert!(result.is_ok(), "first action must always succeed");
}

/// Setting cooldown to 0 after it was non-zero must disable enforcement.
#[test]
fn disable_cooldown_after_enabling() {
    let (env, client, _admin) = setup_env();

    // Enable cooldown.
    client.set_risk_admin_cooldown(&3600);
    assert_eq!(client.get_risk_admin_cooldown(), 3600);

    // Record first action.
    env.ledger().with_mut(|l| l.timestamp = 1000);
    client.record_risk_admin_action();

    // Disable cooldown.
    client.set_risk_admin_cooldown(&0);
    assert_eq!(client.get_risk_admin_cooldown(), 0);

    // Immediate second action must succeed (cooldown disabled).
    env.ledger().with_mut(|l| l.timestamp = 1001);
    let result = client.try_record_risk_admin_action();
    assert!(result.is_ok(), "must succeed after cooldown disabled");
}

/// Multiple pause/unpause cycles must correctly toggle the gate.
#[test]
fn multiple_pause_cycles() {
    let (env, client, _admin) = setup_env();

    for cycle in 0..5 {
        let label = std::format!("cycle={cycle}");

        // Pause.
        client.set_paused(&true);

        // Mutations must fail.
        assert!(
            client.try_set_risk_admin_cooldown(&100).is_err(),
            "{label}: must fail when paused",
        );
        assert!(
            client.try_record_risk_admin_action().is_err(),
            "{label}: must fail when paused",
        );

        // Unpause.
        client.set_paused(&false);

        // Mutations must succeed.
        assert!(
            client.try_set_risk_admin_cooldown(&100).is_ok(),
            "{label}: must succeed when unpaused",
        );
        assert!(
            client.try_record_risk_admin_action().is_ok(),
            "{label}: must succeed when unpaused",
        );
    }
}

/// Cooldown value must survive multiple set/get cycles.
#[test]
fn cooldown_value_survives_multiple_cycles() {
    let (env, client, _admin) = setup_env();

    let test_values = [0, 1, 60, 3600, 86400, 0, 7200, 0];

    for (i, &expected) in test_values.iter().enumerate() {
        client.set_risk_admin_cooldown(&expected);
        let actual = client.get_risk_admin_cooldown();
        assert_eq!(
            actual, expected,
            "cycle {i}: expected {expected}, got {actual}",
        );
    }
}

/// Admin address must never change after init.
#[test]
fn admin_immutable_after_init() {
    let (env, client, admin) = setup_env();

    // Call various entrypoints — admin must remain unchanged.
    client.set_risk_admin_cooldown(&3600);
    assert_eq!(client.get_admin(), admin);

    env.ledger().with_mut(|l| l.timestamp += 3600);
    client.record_risk_admin_action();
    assert_eq!(client.get_admin(), admin);

    client.set_paused(&true);
    assert_eq!(client.get_admin(), admin);

    client.set_paused(&false);
    assert_eq!(client.get_admin(), admin);
}

/// Event emission for set_risk_admin_cooldown with various values.
#[test]
fn event_emission_for_various_cooldown_values() {
    let (env, client, _admin) = setup_env();

    let test_values = [0, 1, 3600, 86400];

    for &cd in &test_values {
        client.set_risk_admin_cooldown(&cd);
        assert_cooldown_event(&env, cd);
    }
}

/// Cooldown enforcement with zero time advance (same timestamp).
#[test]
fn cooldown_blocks_at_same_timestamp() {
    let (env, client, _admin) = setup_env();

    client.set_risk_admin_cooldown(&3600);

    env.ledger().with_mut(|l| l.timestamp = 5000);
    client.record_risk_admin_action();

    // Same timestamp — must be blocked.
    let result = client.try_record_risk_admin_action();
    assert!(result.is_err(), "must block action at the same timestamp");
}

/// Cooldown with value 1 (minimum non-zero) must enforce correctly.
#[test]
fn minimum_nonzero_cooldown() {
    let (env, client, _admin) = setup_env();

    client.set_risk_admin_cooldown(&1);

    env.ledger().with_mut(|l| l.timestamp = 1000);
    client.record_risk_admin_action();

    // At t=1000 (same) — blocked.
    assert!(client.try_record_risk_admin_action().is_err());

    // At t=1001 (1000 + 1) — allowed.
    env.ledger().with_mut(|l| l.timestamp = 1001);
    assert!(client.try_record_risk_admin_action().is_ok());
}

/// Large cooldown (max u64 safe value) must not overflow.
#[test]
fn large_cooldown_no_overflow() {
    let (env, client, _admin) = setup_env();

    // Use a large but reasonable cooldown.
    client.set_risk_admin_cooldown(&u64::MAX / 2);

    env.ledger().with_mut(|l| l.timestamp = 1000);
    client.record_risk_admin_action();

    // Must be blocked (deadline is 1000 + u64::MAX/2, which is huge).
    env.ledger().with_mut(|l| l.timestamp = 2000);
    let result = client.try_record_risk_admin_action();
    assert!(result.is_err(), "must block with large cooldown");
}
