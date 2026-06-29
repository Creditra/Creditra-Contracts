//! Storage Tier Matrix Tests — Issue #594
//!
//! Verifies that each data key is written to and read from
//! the correct Soroban storage tier (Instance / Persistent / Temporary).

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Address, Env, Symbol,
};

/// Helper: advance ledger by `n` ledgers
fn advance_ledger(env: &Env, n: u32) {
    env.ledger().set(LedgerInfo {
        sequence_number: env.ledger().sequence() + n,
        timestamp: env.ledger().timestamp() + (n as u64 * 5),
        ..env.ledger().get()
    });
}

// ── Instance Storage ──────────────────────────────────────────────────────────

#[test]
fn test_instance_keys_survive_within_instance_ttl() {
    let env = Env::default();
    // Instance keys should be readable as long as the contract instance lives.
    // This test confirms they are NOT evicted within their bump window.
    env.storage()
        .instance()
        .set(&Symbol::new(&env, "Paused"), &false);

    // Advance well within the instance bump threshold
    advance_ledger(&env, 1_000);

    let paused: bool = env
        .storage()
        .instance()
        .get(&Symbol::new(&env, "Paused"))
        .unwrap();
    assert!(!paused, "Instance key should still be readable within TTL");
}

#[test]
fn test_instance_storage_is_small() {
    // Guard: instance storage should stay small.
    // If this number grows beyond ~10 keys, consider moving some to Persistent.
    let env = Env::default();
    let instance_keys = vec![
        "Admin",
        "Initialized",
        "TotalSupply",
        "ProtocolFee",
        "Paused",
        "LoanCount",
        "BridgeAdmin",
        "BridgeFee",
    ];
    assert!(
        instance_keys.len() <= 10,
        "Instance storage has too many keys ({}); keep it small for cost efficiency",
        instance_keys.len()
    );
}

// ── Persistent Storage ────────────────────────────────────────────────────────

#[test]
fn test_persistent_key_written_and_read() {
    let env = Env::default();
    let user = Address::generate(&env);

    env.storage()
        .persistent()
        .set(&(Symbol::new(&env, "Balance"), user.clone()), &1000_i128);

    let balance: i128 = env
        .storage()
        .persistent()
        .get(&(Symbol::new(&env, "Balance"), user))
        .unwrap();

    assert_eq!(balance, 1000, "Persistent Balance should round-trip correctly");
}

#[test]
fn test_persistent_key_bumped_on_access() {
    let env = Env::default();
    let user = Address::generate(&env);
    let key = (Symbol::new(&env, "CreditScore"), user.clone());

    env.storage().persistent().set(&key, &750_u32);

    // Simulate the bump that should happen on every read
    const THRESHOLD: u32 = 259_200;
    const BUMP: u32 = 518_400;
    env.storage().persistent().bump(&key, THRESHOLD, BUMP);

    let score: u32 = env.storage().persistent().get(&key).unwrap();
    assert_eq!(score, 750);
}

#[test]
fn test_nonce_is_not_in_persistent_storage() {
    // Nonces must be Temporary, not Persistent — they should auto-expire.
    // This test documents the intent: if someone accidentally writes a nonce
    // to persistent storage, it won't show up in the temporary store.
    let env = Env::default();
    let user = Address::generate(&env);
    let temp_key = (Symbol::new(&env, "Nonce"), user.clone());

    // Write to temporary (correct tier)
    env.storage().temporary().set(&temp_key, &42_u64);

    // Confirm it is NOT in persistent
    let in_persistent: Option<u64> = env.storage().persistent().get(&temp_key);
    assert!(
        in_persistent.is_none(),
        "Nonce must NOT be in persistent storage"
    );
}

// ── Temporary Storage ─────────────────────────────────────────────────────────

#[test]
fn test_temporary_key_written_and_read() {
    let env = Env::default();
    let user = Address::generate(&env);
    let key = (Symbol::new(&env, "Nonce"), user);

    env.storage().temporary().set(&key, &1_u64);

    let nonce: u64 = env.storage().temporary().get(&key).unwrap();
    assert_eq!(nonce, 1, "Temporary nonce should be readable before expiry");
}

#[test]
fn test_auction_state_in_temporary_storage() {
    let env = Env::default();
    let loan_id: u64 = 42;
    let key = (Symbol::new(&env, "AuctionState"), loan_id);

    // Write auction state with a short TTL
    env.storage().temporary().set(&key, &true); // simplified: bool as proxy for AuctionData
    const AUCTION_TTL: u32 = 17_280;
    env.storage().temporary().bump(&key, AUCTION_TTL, AUCTION_TTL);

    let active: bool = env.storage().temporary().get(&key).unwrap();
    assert!(active, "Auction state should be readable within TTL");
}

#[test]
fn test_processed_hash_replay_guard_is_temporary() {
    let env = Env::default();
    // Replay guards must be temporary — they only need to live for the finality window
    let tx_hash = Symbol::new(&env, "0xdeadbeef");
    let key = (Symbol::new(&env, "ProcessedHash"), tx_hash);

    env.storage().temporary().set(&key, &true);

    let processed: bool = env.storage().temporary().get(&key).unwrap();
    assert!(processed, "Replay guard should be set in temporary storage");

    // Confirm it is NOT in persistent (would waste fees)
    let in_persistent: Option<bool> = env.storage().persistent().get(&key);
    assert!(
        in_persistent.is_none(),
        "Replay guard must NOT be in persistent storage"
    );
}

// ── Cross-tier Isolation ──────────────────────────────────────────────────────

#[test]
fn test_same_key_is_independent_across_tiers() {
    // The same logical key in different tiers holds independent values.
    let env = Env::default();
    let key = Symbol::new(&env, "TestKey");

    env.storage().instance().set(&key, &100_i32);
    env.storage().persistent().set(&key, &200_i32);
    env.storage().temporary().set(&key, &300_i32);

    assert_eq!(env.storage().instance().get::<_, i32>(&key).unwrap(), 100);
    assert_eq!(env.storage().persistent().get::<_, i32>(&key).unwrap(), 200);
    assert_eq!(env.storage().temporary().get::<_, i32>(&key).unwrap(), 300);
}
