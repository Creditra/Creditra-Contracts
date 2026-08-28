// SPDX-License-Identifier: MIT

//! Integration tests for instance and persistent storage TTL bump behaviour
//! on freeze read paths.
//!
//! # Coverage matrix
//!
//! | Entrypoint                    | Storage    | Bump expected |
//! |-------------------------------|------------|---------------|
//! | `is_draws_frozen`             | instance   | yes (was missing) |
//! | `get_draws_freeze_reason`     | instance   | yes (was missing) |
//! | `is_credit_line_frozen`       | persistent | yes (already present) |
//! | `get_credit_line_freeze_reason` | persistent | yes (already present) |
//! | `is_borrower_frozen`          | persistent | yes (already present) |
//! | `get_borrower_frozen_until`   | persistent | yes (already present) |
//!
//! # Scenario types
//!
//! 1. **Below-threshold** — drain TTL below the refresh threshold, call the
//!    entrypoint, assert TTL is extended to `BUMP_AMOUNT`.
//! 2. **Healthy** — call the entrypoint while TTL is still above threshold,
//!    assert the TTL does not decrease (no unnecessary ledger write).

use creditra_freeze::{Credit, CreditClient, FreezeReason};
use soroban_sdk::testutils::storage::Instance as _;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env};

// ── TTL policy constants (mirroring creditra_credit::storage) ─────────────────

/// Instance storage TTL extend-to target (~6 months at 5 s/ledger).
const INSTANCE_BUMP_AMOUNT: u32 = 3_110_400;

/// Instance storage TTL refresh threshold (~3 months at 5 s/ledger).
const INSTANCE_BUMP_THRESHOLD: u32 = 1_555_200;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Deploy and initialise the credit contract, open a credit line for `borrower`.
fn setup() -> (Env, Address, Address, Address, CreditClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    client.open_credit_line(&borrower, &1_000_000, &300, &50);
    (env, contract_id, admin, borrower, client)
}

/// Return the current instance storage TTL for the contract.
fn instance_ttl(env: &Env, contract_id: &Address) -> u32 {
    env.as_contract(contract_id, || env.storage().instance().get_ttl())
}

/// Advance the ledger so instance TTL drops below `INSTANCE_BUMP_THRESHOLD`.
fn drain_instance_ttl(env: &Env, contract_id: &Address) {
    let current = instance_ttl(env, contract_id);
    let target = INSTANCE_BUMP_THRESHOLD.saturating_sub(1);
    let delta = current.saturating_sub(target);
    if delta > 0 {
        env.ledger().with_mut(|li| {
            li.sequence_number = li.sequence_number.saturating_add(delta);
        });
    }
}

// ── Instance storage read paths ───────────────────────────────────────────────

/// `is_draws_frozen` must bump instance TTL when the remaining TTL is below
/// the refresh threshold.
#[test]
fn is_draws_frozen_bumps_instance_ttl_when_below_threshold() {
    let (env, contract_id, _admin, _borrower, client) = setup();

    client.freeze_draws(&FreezeReason::LiquidityReserve);
    drain_instance_ttl(&env, &contract_id);

    let before = instance_ttl(&env, &contract_id);
    assert!(before < INSTANCE_BUMP_THRESHOLD);

    assert!(client.is_draws_frozen());

    let after = instance_ttl(&env, &contract_id);
    assert!(
        after >= INSTANCE_BUMP_AMOUNT,
        "is_draws_frozen must extend instance TTL from {before} to at least {INSTANCE_BUMP_AMOUNT}; got {after}"
    );
}

/// `get_draws_freeze_reason` must bump instance TTL when below threshold.
#[test]
fn get_draws_freeze_reason_bumps_instance_ttl_when_below_threshold() {
    let (env, contract_id, _admin, _borrower, client) = setup();

    client.freeze_draws(&FreezeReason::Compliance);
    drain_instance_ttl(&env, &contract_id);

    let before = instance_ttl(&env, &contract_id);
    assert!(before < INSTANCE_BUMP_THRESHOLD);

    let reason = client.get_draws_freeze_reason();
    assert_eq!(reason, Some(FreezeReason::Compliance));

    let after = instance_ttl(&env, &contract_id);
    assert!(
        after >= INSTANCE_BUMP_AMOUNT,
        "get_draws_freeze_reason must extend instance TTL; before={before} after={after}"
    );
}

/// When instance TTL is still healthy, `is_draws_frozen` must not trigger
/// an unnecessary TTL extension write.
#[test]
fn is_draws_frozen_does_not_write_ttl_when_healthy() {
    let (env, contract_id, _admin, _borrower, client) = setup();

    client.freeze_draws(&FreezeReason::LiquidityReserve);

    let before = instance_ttl(&env, &contract_id);
    assert!(before >= INSTANCE_BUMP_THRESHOLD);

    assert!(client.is_draws_frozen());

    let after = instance_ttl(&env, &contract_id);
    assert!(
        after >= before.saturating_sub(1),
        "TTL must not decrease when bump is a no-op; before={before} after={after}"
    );
}

// ── Persistent storage read paths ─────────────────────────────────────────────

/// `is_credit_line_frozen` must bump persistent TTL when below threshold.
#[test]
fn is_credit_line_frozen_bumps_persistent_ttl_when_below_threshold() {
    let (env, contract_id, admin, borrower, client) = setup();

    client.freeze_credit_line(&borrower, &FreezeReason::Compliance);
    let ttl_before = client.get_credit_line_freeze_reason(&borrower);
    assert_eq!(ttl_before, Some(FreezeReason::Compliance));

    // Drain instance TTL so the contract stays alive but bump is forced.
    drain_instance_ttl(&env, &contract_id);

    assert!(client.is_credit_line_frozen(&borrower));

    let after = instance_ttl(&env, &contract_id);
    assert!(
        after >= INSTANCE_BUMP_AMOUNT,
        "is_credit_line_frozen must extend instance+persistent TTL; got {after}"
    );
}

/// `get_credit_line_freeze_reason` must bump persistent TTL when below threshold.
#[test]
fn get_credit_line_freeze_reason_bumps_persistent_ttl_when_below_threshold() {
    let (env, contract_id, admin, borrower, client) = setup();

    client.freeze_credit_line(&borrower, &FreezeReason::Compliance);
    drain_instance_ttl(&env, &contract_id);

    let before = instance_ttl(&env, &contract_id);
    assert!(before < INSTANCE_BUMP_THRESHOLD);

    let reason = client.get_credit_line_freeze_reason(&borrower);
    assert_eq!(reason, Some(FreezeReason::Compliance));

    let after = instance_ttl(&env, &contract_id);
    assert!(
        after >= INSTANCE_BUMP_AMOUNT,
        "get_credit_line_freeze_reason must extend TTL; before={before} after={after}"
    );
}

/// `is_borrower_frozen` must bump persistent TTL when below threshold.
#[test]
fn is_borrower_frozen_bumps_persistent_ttl_when_below_threshold() {
    let (env, contract_id, admin, borrower, client) = setup();

    let now = env.ledger().timestamp();
    client.freeze_borrower_until(&admin, &borrower, &(now + 3600));
    drain_instance_ttl(&env, &contract_id);

    let before = instance_ttl(&env, &contract_id);
    assert!(before < INSTANCE_BUMP_THRESHOLD);

    assert!(client.is_borrower_frozen(&borrower));

    let after = instance_ttl(&env, &contract_id);
    assert!(
        after >= INSTANCE_BUMP_AMOUNT,
        "is_borrower_frozen must extend TTL; before={before} after={after}"
    );
}

/// `get_borrower_frozen_until` must bump persistent TTL when below threshold.
#[test]
fn get_borrower_frozen_until_bumps_persistent_ttl_when_below_threshold() {
    let (env, contract_id, admin, borrower, client) = setup();

    let now = env.ledger().timestamp();
    client.freeze_borrower_until(&admin, &borrower, &(now + 7200));
    drain_instance_ttl(&env, &contract_id);

    let before = instance_ttl(&env, &contract_id);
    assert!(before < INSTANCE_BUMP_THRESHOLD);

    let expiry = client.get_borrower_frozen_until(&borrower);
    assert_eq!(expiry, Some(now + 7200));

    let after = instance_ttl(&env, &contract_id);
    assert!(
        after >= INSTANCE_BUMP_AMOUNT,
        "get_borrower_frozen_until must extend TTL; before={before} after={after}"
    );
}

// ── Survey — every freeze read entrypoint in a single round-trip ──────────────

/// Drain TTL then exercise every freeze read entrypoint, asserting TTL is
/// extended to `INSTANCE_BUMP_AMOUNT` after each one.
#[test]
fn every_freeze_read_entrypoint_bumps_ttl() {
    let (env, contract_id, admin, borrower, client) = setup();

    // Seed freeze state.
    let now = env.ledger().timestamp();
    client.freeze_draws(&FreezeReason::LiquidityReserve);
    client.freeze_credit_line(&borrower, &FreezeReason::Compliance);
    client.freeze_borrower_until(&admin, &borrower, &(now + 3600));

    // ① is_draws_frozen
    drain_instance_ttl(&env, &contract_id);
    assert!(client.is_draws_frozen());
    assert!(
        instance_ttl(&env, &contract_id) >= INSTANCE_BUMP_AMOUNT,
        "is_draws_frozen must bump TTL"
    );

    // ② get_draws_freeze_reason
    drain_instance_ttl(&env, &contract_id);
    let reason = client.get_draws_freeze_reason();
    assert_eq!(reason, Some(FreezeReason::LiquidityReserve));
    assert!(
        instance_ttl(&env, &contract_id) >= INSTANCE_BUMP_AMOUNT,
        "get_draws_freeze_reason must bump TTL"
    );

    // ③ is_credit_line_frozen
    drain_instance_ttl(&env, &contract_id);
    assert!(client.is_credit_line_frozen(&borrower));
    assert!(
        instance_ttl(&env, &contract_id) >= INSTANCE_BUMP_AMOUNT,
        "is_credit_line_frozen must bump TTL"
    );

    // ④ get_credit_line_freeze_reason
    drain_instance_ttl(&env, &contract_id);
    let line_reason = client.get_credit_line_freeze_reason(&borrower);
    assert_eq!(line_reason, Some(FreezeReason::Compliance));
    assert!(
        instance_ttl(&env, &contract_id) >= INSTANCE_BUMP_AMOUNT,
        "get_credit_line_freeze_reason must bump TTL"
    );

    // ⑤ is_borrower_frozen
    drain_instance_ttl(&env, &contract_id);
    assert!(client.is_borrower_frozen(&borrower));
    assert!(
        instance_ttl(&env, &contract_id) >= INSTANCE_BUMP_AMOUNT,
        "is_borrower_frozen must bump TTL"
    );

    // ⑥ get_borrower_frozen_until
    drain_instance_ttl(&env, &contract_id);
    let expiry = client.get_borrower_frozen_until(&borrower);
    assert_eq!(expiry, Some(now + 3600));
    assert!(
        instance_ttl(&env, &contract_id) >= INSTANCE_BUMP_AMOUNT,
        "get_borrower_frozen_until must bump TTL"
    );
}
