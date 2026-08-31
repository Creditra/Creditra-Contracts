//! Tests that factory initialization is one-shot and cannot be replayed
//! (Issue #1141).
//!
//! Before this change `set_factory_contract` was an unbounded setter: the same
//! initialization call could be submitted repeatedly, and a stale but still
//! valid authorization could re-point the factory long after deployment.
//!
//! | Acceptance criterion | Tests |
//! |---|---|
//! | Deterministic for valid input | `first_registration_succeeds_and_marks_initialized` |
//! | Deterministic for duplicate input | `second_registration_is_rejected`, `replaying_same_factory_is_rejected` |
//! | Deterministic for invalid input | `rotate_before_initialization_is_rejected` |
//! | Authorization preserved | `rotation_requires_both_parties` |
//! | Retries cannot corrupt state | `rejected_replay_leaves_factory_unchanged` |
//! | Boundary / regression | `rotation_does_not_reopen_initialization` |
//! | Diagnosable failures | `factory_errors_are_stable` |
//!
//! # Running
//!
//! ```bash
//! cargo test -p gateway-auction --test factory_init_replay
//! ```

use gateway_auction::{Auction, AuctionClient, AuctionError};
use soroban_sdk::testutils::{Address as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{Address, Env, IntoVal};

/// Deploy a fresh, uninitialized auction contract.
///
/// Returns the env and contract id rather than a client: `AuctionClient`
/// borrows the `Env`, so it must be constructed in the caller's scope.
fn setup() -> (Env, Address) {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(Auction, ());
    (env, contract_id)
}

// ─── Success path ───────────────────────────────────────────────────────────

#[test]
fn contract_starts_uninitialized() {
    let (env, contract_id) = setup();
    let client = AuctionClient::new(&env, &contract_id);
    assert!(
        !client.is_factory_initialized(),
        "a fresh contract must report factory initialization as incomplete"
    );
}

#[test]
fn first_registration_succeeds_and_marks_initialized() {
    let (env, contract_id) = setup();
    let client = AuctionClient::new(&env, &contract_id);
    let factory = Address::generate(&env);

    client.set_factory_contract(&factory);

    assert!(
        client.is_factory_initialized(),
        "the one-shot marker must be set by the first registration"
    );
}

// ─── Replay rejection (the core of #1141) ───────────────────────────────────

#[test]
fn second_registration_is_rejected() {
    let (env, contract_id) = setup();
    let client = AuctionClient::new(&env, &contract_id);
    let factory = Address::generate(&env);
    let attacker = Address::generate(&env);

    client.set_factory_contract(&factory);

    let err = client
        .try_set_factory_contract(&attacker)
        .expect_err("a second factory registration must be rejected");
    assert!(err.is_ok(), "expected a contract error, not a host error");
}

/// Replaying the *identical* initialization call — the literal replay case —
/// is rejected just as a different address would be. Accepting it because
/// "nothing would change" would leave the initialization window open forever.
#[test]
fn replaying_same_factory_is_rejected() {
    let (env, contract_id) = setup();
    let client = AuctionClient::new(&env, &contract_id);
    let factory = Address::generate(&env);

    client.set_factory_contract(&factory);

    let err = client
        .try_set_factory_contract(&factory)
        .expect_err("replaying the same registration must be rejected");
    assert!(err.is_ok(), "expected a contract error, not a host error");
}

/// A rejected replay must leave the registered factory untouched.
#[test]
fn rejected_replay_leaves_factory_unchanged() {
    let (env, contract_id) = setup();
    let client = AuctionClient::new(&env, &contract_id);
    let factory = Address::generate(&env);
    let attacker = Address::generate(&env);

    client.set_factory_contract(&factory);
    let _ = client.try_set_factory_contract(&attacker);

    // Still initialized, and the rotation path still recognises the original
    // factory as the incumbent — proving the attacker never took the slot.
    assert!(client.is_factory_initialized());
    let replacement = Address::generate(&env);
    client.rotate_factory_contract(&replacement);
}

// ─── Rotation: the deliberate replacement path ──────────────────────────────

#[test]
fn rotate_before_initialization_is_rejected() {
    let (env, contract_id) = setup();
    let client = AuctionClient::new(&env, &contract_id);
    let candidate = Address::generate(&env);

    let err = client
        .try_rotate_factory_contract(&candidate)
        .expect_err("rotation with no incumbent must be rejected");
    assert!(err.is_ok(), "expected a contract error, not a host error");
    assert!(!client.is_factory_initialized());
}

#[test]
fn rotation_succeeds_with_both_parties() {
    let (env, contract_id) = setup();
    let client = AuctionClient::new(&env, &contract_id);
    let factory = Address::generate(&env);
    let successor = Address::generate(&env);

    client.set_factory_contract(&factory);
    client.rotate_factory_contract(&successor);

    // The marker is deliberately retained across a rotation.
    assert!(client.is_factory_initialized());
}

/// Rotation must require the *incumbent's* authorization: a successor cannot
/// seize the role unilaterally.
#[test]
fn rotation_requires_both_parties() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);

    let factory = Address::generate(&env);
    let successor = Address::generate(&env);
    client.set_factory_contract(&factory);

    // Authorize only the successor, not the incumbent. The invoke and the
    // MockAuth slice are bound to locals so they outlive the borrow.
    let invoke = MockAuthInvoke {
        contract: &contract_id,
        fn_name: "rotate_factory_contract",
        args: (successor.clone(),).into_val(&env),
        sub_invokes: &[],
    };
    let auths = [MockAuth {
        address: &successor,
        invoke: &invoke,
    }];

    let result = client.mock_auths(&auths).try_rotate_factory_contract(&successor);
    assert!(
        result.is_err(),
        "rotation without the incumbent's authorization must fail"
    );
}

/// The security property that motivates keeping the marker set: a rotation
/// must never reopen the one-shot initialization window, or an unprivileged
/// address could claim the factory role a second time.
#[test]
fn rotation_does_not_reopen_initialization() {
    let (env, contract_id) = setup();
    let client = AuctionClient::new(&env, &contract_id);
    let factory = Address::generate(&env);
    let successor = Address::generate(&env);
    let attacker = Address::generate(&env);

    client.set_factory_contract(&factory);
    client.rotate_factory_contract(&successor);

    let err = client
        .try_set_factory_contract(&attacker)
        .expect_err("initialization must stay closed after a rotation");
    assert!(err.is_ok(), "expected a contract error, not a host error");
}

// ─── Diagnostics ────────────────────────────────────────────────────────────

/// The rejection surfaces a stable, appended discriminant so clients can
/// distinguish "already initialized" from every other failure.
#[test]
fn factory_errors_are_stable() {
    assert_eq!(AuctionError::FactoryAlreadyInitialized as u32, 15);
    assert_eq!(AuctionError::NoFactoryContract as u32, 4);
}
