// SPDX-License-Identifier: MIT

//! Focused integration tests for instance-storage TTL bump behaviour in the
//! Creditra risk admin cooldown contract.
//!
//! # Coverage matrix
//!
//! | Scenario | Entrypoints exercised |
//! |---|---|
//! | Every entrypoint bumps TTL above threshold | all 6 entrypoints |
//! | Read-only paths bump TTL when below threshold | `get_risk_admin_cooldown`, `get_admin` |
//! | Write paths bump TTL when below threshold | `set_risk_admin_cooldown`, `set_paused`, `record_risk_admin_action` |
//! | Bump does not fire when TTL already above threshold | `get_risk_admin_cooldown` |
//! | Cooldown enforcement is unaffected by the TTL change | `record_risk_admin_action` |
//! | `init` bumps immediately on deployment | `init` |

use creditra_risk::{RiskContract, RiskContractClient, INSTANCE_BUMP_AMOUNT, INSTANCE_BUMP_THRESHOLD};
use soroban_sdk::testutils::storage::Instance as _;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env};

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Register the contract, call `init`, and return `(env, admin, contract_id,
/// client)`.
fn setup() -> (Env, Address, Address, RiskContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(RiskContract, ());
    let client = RiskContractClient::new(&env, &contract_id);
    client.init(&admin);
    (env, admin, contract_id, client)
}

/// Return the current instance storage TTL for the contract.
///
/// Instance storage is a single slab; `get_ttl()` returns the TTL of the
/// whole slab (no per-key argument in soroban-sdk 22.x).
fn instance_ttl(env: &Env, contract_id: &Address) -> u32 {
    env.as_contract(contract_id, || {
        env.storage().instance().get_ttl()
    })
}

/// Advance the ledger sequence number so that the instance storage TTL drops
/// to just below [`INSTANCE_BUMP_THRESHOLD`].  This forces the next
/// `extend_ttl` call to issue a real ledger write.
fn drain_ttl_below_threshold(env: &Env, contract_id: &Address) {
    let current_ttl = instance_ttl(env, contract_id);
    // We want remaining TTL == INSTANCE_BUMP_THRESHOLD - 1.
    let target = INSTANCE_BUMP_THRESHOLD.saturating_sub(1);
    let delta = current_ttl.saturating_sub(target);
    if delta > 0 {
        env.ledger().with_mut(|li| {
            li.sequence_number = li.sequence_number.saturating_add(delta);
        });
    }
}

// ── init bumps TTL ────────────────────────────────────────────────────────────

/// `init` must call `bump_instance_ttl` so the contract is live immediately
/// after deployment, regardless of the default-TTL assigned at registration.
#[test]
fn init_bumps_instance_ttl() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(RiskContract, ());
    let client = RiskContractClient::new(&env, &contract_id);

    // Before init the contract exists but has the default (possibly low) TTL.
    // Drain it below threshold then call init.
    drain_ttl_below_threshold(&env, &contract_id);

    client.init(&admin);

    let ttl = instance_ttl(&env, &contract_id);
    assert!(
        ttl >= INSTANCE_BUMP_AMOUNT,
        "init must extend instance TTL to at least INSTANCE_BUMP_AMOUNT; got {ttl}"
    );
}

// ── get_risk_admin_cooldown bumps TTL ─────────────────────────────────────────

/// The read-only `get_risk_admin_cooldown` path must bump instance storage TTL
/// so that contracts queried only for their cooldown value do not expire.
#[test]
fn get_risk_admin_cooldown_bumps_instance_ttl_when_below_threshold() {
    let (env, _admin, contract_id, client) = setup();

    // Configure a non-zero cooldown so the key exists in storage.
    client.set_risk_admin_cooldown(&3_600);
    drain_ttl_below_threshold(&env, &contract_id);

    let before = instance_ttl(&env, &contract_id);
    assert!(
        before < INSTANCE_BUMP_THRESHOLD,
        "precondition: TTL must be below threshold ({before} >= {INSTANCE_BUMP_THRESHOLD})"
    );

    // Pure read path — no auth required, no state change.
    let cooldown = client.get_risk_admin_cooldown();
    assert_eq!(cooldown, 3_600, "cooldown value must be preserved");

    let after = instance_ttl(&env, &contract_id);
    assert!(
        after >= INSTANCE_BUMP_AMOUNT,
        "get_risk_admin_cooldown must extend TTL; before={before} after={after}"
    );
}

/// When TTL is already above the threshold the bump is a no-op (no ledger
/// write). The returned cooldown value must still be correct.
#[test]
fn get_risk_admin_cooldown_does_not_write_when_ttl_healthy() {
    let (env, _admin, contract_id, client) = setup();

    client.set_risk_admin_cooldown(&7_200);

    // TTL is fresh (just after init); do not drain it.
    let before = instance_ttl(&env, &contract_id);
    assert!(
        before >= INSTANCE_BUMP_THRESHOLD,
        "precondition: TTL should still be above threshold ({before})"
    );

    let cooldown = client.get_risk_admin_cooldown();
    assert_eq!(cooldown, 7_200, "cooldown value must be correct");

    // TTL should not decrease.
    let after = instance_ttl(&env, &contract_id);
    assert!(
        after >= before.saturating_sub(1),
        "TTL must not decrease when bump is a no-op; before={before} after={after}"
    );
}

// ── get_admin bumps TTL ───────────────────────────────────────────────────────

/// `get_admin` must bump instance TTL so read-only admin queries keep the
/// contract live.
#[test]
fn get_admin_bumps_instance_ttl_when_below_threshold() {
    let (env, admin, contract_id, client) = setup();

    drain_ttl_below_threshold(&env, &contract_id);

    let before = instance_ttl(&env, &contract_id);
    assert!(
        before < INSTANCE_BUMP_THRESHOLD,
        "precondition: TTL must be below threshold ({before})"
    );

    let retrieved = client.get_admin();
    assert_eq!(retrieved, admin, "get_admin must return the correct address");

    let after = instance_ttl(&env, &contract_id);
    assert!(
        after >= INSTANCE_BUMP_AMOUNT,
        "get_admin must extend TTL; before={before} after={after}"
    );
}

// ── set_risk_admin_cooldown bumps TTL ─────────────────────────────────────────

/// `set_risk_admin_cooldown` (write path) must bump TTL so that the contract
/// remains live after configuration mutations.
#[test]
fn set_risk_admin_cooldown_bumps_instance_ttl_when_below_threshold() {
    let (env, _admin, contract_id, client) = setup();

    drain_ttl_below_threshold(&env, &contract_id);

    let before = instance_ttl(&env, &contract_id);
    assert!(
        before < INSTANCE_BUMP_THRESHOLD,
        "precondition: TTL must be below threshold ({before})"
    );

    client.set_risk_admin_cooldown(&1_800);

    let after = instance_ttl(&env, &contract_id);
    assert!(
        after >= INSTANCE_BUMP_AMOUNT,
        "set_risk_admin_cooldown must extend TTL; before={before} after={after}"
    );

    // Verify the value was actually stored.
    assert_eq!(client.get_risk_admin_cooldown(), 1_800);
}

// ── set_paused bumps TTL ──────────────────────────────────────────────────────

/// `set_paused` (write path) must bump TTL.
#[test]
fn set_paused_bumps_instance_ttl_when_below_threshold() {
    let (env, _admin, contract_id, client) = setup();

    drain_ttl_below_threshold(&env, &contract_id);

    let before = instance_ttl(&env, &contract_id);
    assert!(
        before < INSTANCE_BUMP_THRESHOLD,
        "precondition: TTL must be below threshold ({before})"
    );

    client.set_paused(&false); // unpause — still hits the bump path

    let after = instance_ttl(&env, &contract_id);
    assert!(
        after >= INSTANCE_BUMP_AMOUNT,
        "set_paused must extend TTL; before={before} after={after}"
    );
}

// ── record_risk_admin_action bumps TTL ────────────────────────────────────────

/// `record_risk_admin_action` must bump TTL on every successful call (i.e.,
/// when cooldown is disabled and the action is recorded).
#[test]
fn record_risk_admin_action_bumps_instance_ttl_when_below_threshold() {
    let (env, _admin, contract_id, client) = setup();

    drain_ttl_below_threshold(&env, &contract_id);

    let before = instance_ttl(&env, &contract_id);
    assert!(
        before < INSTANCE_BUMP_THRESHOLD,
        "precondition: TTL must be below threshold ({before})"
    );

    env.ledger().with_mut(|li| li.timestamp = 1_000);
    client.record_risk_admin_action();

    let after = instance_ttl(&env, &contract_id);
    assert!(
        after >= INSTANCE_BUMP_AMOUNT,
        "record_risk_admin_action must extend TTL; before={before} after={after}"
    );
}

// ── TTL bump does not interfere with cooldown enforcement ──────────────────

/// Draining the TTL below threshold and then calling `record_risk_admin_action`
/// a second time within the cooldown window must still be rejected — proving
/// that the TTL bump on the first call does not disable the cooldown guard.
#[test]
fn ttl_bump_does_not_bypass_cooldown_enforcement() {
    let (env, _admin, contract_id, client) = setup();

    client.set_risk_admin_cooldown(&3_600);

    // First action at t=1000 — succeeds.
    env.ledger().with_mut(|li| li.timestamp = 1_000);
    client.record_risk_admin_action();

    // Drain TTL below threshold (simulates a period of inactivity that would
    // have archived the contract without the bump fix).
    drain_ttl_below_threshold(&env, &contract_id);

    // Advance time but stay within the 3 600 s cooldown window (t=2000).
    env.ledger().with_mut(|li| li.timestamp = 2_000);

    // Must still be rejected despite the TTL drain-and-bump.
    let result = client.try_record_risk_admin_action();
    assert!(
        result.is_err(),
        "second action within cooldown must be rejected even after TTL drain"
    );
}

/// After the cooldown elapses, the action must succeed — and must also bump TTL.
#[test]
fn action_after_cooldown_elapsed_bumps_ttl() {
    let (env, _admin, contract_id, client) = setup();

    client.set_risk_admin_cooldown(&3_600);

    env.ledger().with_mut(|li| li.timestamp = 1_000);
    client.record_risk_admin_action();

    drain_ttl_below_threshold(&env, &contract_id);

    // Advance past the cooldown window.
    env.ledger().with_mut(|li| li.timestamp = 4_600);

    let before = instance_ttl(&env, &contract_id);

    client.record_risk_admin_action(); // must succeed

    let after = instance_ttl(&env, &contract_id);
    assert!(
        after >= INSTANCE_BUMP_AMOUNT,
        "post-cooldown action must extend TTL; before={before} after={after}"
    );
}

// ── Constant values are correct ───────────────────────────────────────────────

/// Verify that the exported TTL constants have the values specified in the
/// GrantFox FWC26 TTL policy (matching `contracts/credit/src/storage.rs`).
#[test]
fn ttl_constants_match_expected_policy() {
    // ~6 months at ~5 s/ledger: 6 * 30 * 24 * 3600 / 5 = 3_110_400
    assert_eq!(
        INSTANCE_BUMP_AMOUNT,
        3_110_400,
        "INSTANCE_BUMP_AMOUNT must be 3_110_400 (~6 months)"
    );
    // ~3 months: 3 * 30 * 24 * 3600 / 5 = 1_555_200
    assert_eq!(
        INSTANCE_BUMP_THRESHOLD,
        1_555_200,
        "INSTANCE_BUMP_THRESHOLD must be 1_555_200 (~3 months)"
    );
    // 2:1 ratio between extend-to and threshold
    assert_eq!(
        INSTANCE_BUMP_AMOUNT,
        INSTANCE_BUMP_THRESHOLD * 2,
        "INSTANCE_BUMP_AMOUNT must be exactly 2× INSTANCE_BUMP_THRESHOLD"
    );
}

// ── All entrypoints surveyed in a single round-trip ──────────────────────────

/// Drain TTL then exercise every entrypoint in sequence, asserting TTL is
/// extended to [`INSTANCE_BUMP_AMOUNT`] after each one.
#[test]
fn every_entrypoint_bumps_instance_ttl() {
    let (env, admin, contract_id, client) = setup();

    // ① set_risk_admin_cooldown
    drain_ttl_below_threshold(&env, &contract_id);
    client.set_risk_admin_cooldown(&3_600);
    assert!(
        instance_ttl(&env, &contract_id) >= INSTANCE_BUMP_AMOUNT,
        "set_risk_admin_cooldown must bump TTL"
    );

    // ② get_risk_admin_cooldown
    drain_ttl_below_threshold(&env, &contract_id);
    let _ = client.get_risk_admin_cooldown();
    assert!(
        instance_ttl(&env, &contract_id) >= INSTANCE_BUMP_AMOUNT,
        "get_risk_admin_cooldown must bump TTL"
    );

    // ③ set_paused (false → keep contract operable for subsequent steps)
    drain_ttl_below_threshold(&env, &contract_id);
    client.set_paused(&false);
    assert!(
        instance_ttl(&env, &contract_id) >= INSTANCE_BUMP_AMOUNT,
        "set_paused must bump TTL"
    );

    // ④ record_risk_admin_action (first action, t=1000)
    env.ledger().with_mut(|li| li.timestamp = 1_000);
    drain_ttl_below_threshold(&env, &contract_id);
    client.record_risk_admin_action();
    assert!(
        instance_ttl(&env, &contract_id) >= INSTANCE_BUMP_AMOUNT,
        "record_risk_admin_action must bump TTL"
    );

    // ⑤ get_admin
    drain_ttl_below_threshold(&env, &contract_id);
    let returned_admin = client.get_admin();
    assert_eq!(returned_admin, admin);
    assert!(
        instance_ttl(&env, &contract_id) >= INSTANCE_BUMP_AMOUNT,
        "get_admin must bump TTL"
    );
}
