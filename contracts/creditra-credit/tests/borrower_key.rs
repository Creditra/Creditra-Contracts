// SPDX-License-Identifier: MIT

//! # Storage Key Safety and Encoding Verification Test Suite
//!
//! This test suite provides comprehensive verification that storage keys for
//! per-borrower data structures are deterministic, collision-resistant, stable,
//! and properly isolated across different borrower addresses.
//!
//! ## CosmWasm Key Encoding Overview
//!
//! The `creditra-credit` contract uses `cw_storage_plus::Map<Addr, …>` for
//! per-borrower storage.  Each `Addr` serialises to its canonical bech32
//! byte string, which is:
//!
//! 1. **Deterministic** — the same address string always produces the same bytes.
//! 2. **Collision-free** — different bech32 strings produce different bytes.
//! 3. **Stable** — bech32 is a Cosmos standard; the encoding does not drift.
//! 4. **Bijective** — the mapping `Addr → bytes → Addr` is invertible.
//!
//! The `cw_storage_plus::Map` prepends a per-map namespace prefix, so two
//! different `Map<Addr, _>` instances can *never* collide even when using
//! the same address key.
//!
//! ## Test Coverage
//!
//! | Property                  | What is tested                                     |
//! |---------------------------|----------------------------------------------------|
//! | Key stability             | Same address → same key across 100 invocations     |
//! | Key uniqueness            | 200+ distinct addresses → 0 collisions             |
//! | Map-isolation             | Two different `Map<Addr, _>` instances never clash |
//! | Adversarial resistance    | Similar addresses, edge-case addresses             |
//! | Integration (real state)  | Store & retrieve via the contract's `Map` entries  |
//! | Idempotency               | Repeated writes to the same key are safe           |
//! | Empty / zero-edge         | Empty key corner cases                             |

use cosmwasm_std::{
    testing::{message_info, mock_dependencies, mock_env, MockApi, MockQuerier, MockStorage},
    Addr, OwnedDeps, Uint128,
};
use cw_storage_plus::Map;
use std::collections::HashSet;

use creditra_credit::key::{borrower_key_bytes, BorrowerKey};
use creditra_credit::msg::{ExecuteMsg, InstantiateMsg};
use creditra_credit::state::{BORROWER_TO_ID, CREDIT_LINES};

// ═══════════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════════

fn make_addr(deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>, label: &str) -> Addr {
    deps.api.addr_make(label)
}

fn setup_contract(deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>) {
    let env = mock_env();
    let creator = make_addr(deps, "creator");
    let info = message_info(&creator, &[]);
    let msg = InstantiateMsg {
        owner: creator.to_string(),
    };
    creditra_credit::contract::instantiate(deps.as_mut(), env, info, msg).unwrap();
}

fn create_credit_line_for(
    deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
    borrower_label: &str,
    collateral_amount: &str,
    credit_amount: &str,
) -> u64 {
    let env = mock_env();
    let creator = make_addr(deps, "creator");
    let info = message_info(&creator, &[]);
    let borrower = make_addr(deps, borrower_label);

    // Read current count so we know the expected id
    let expected_id = creditra_credit::state::CREDIT_LINE_COUNT
        .load(deps.as_ref().storage)
        .unwrap_or(0);

    let msg = ExecuteMsg::CreateCreditLine {
        borrower: borrower.to_string(),
        collateral_denom: "ucollateral".to_string(),
        collateral_amount: collateral_amount.to_string(),
        credit_denom: "ucredit".to_string(),
        credit_amount: credit_amount.to_string(),
    };
    creditra_credit::contract::execute(deps.as_mut(), env, info, msg).unwrap();

    expected_id
}

// ═══════════════════════════════════════════════════════════════════════════
// Test 1: Key Stability — Same Address Produces the Same Key
// ═══════════════════════════════════════════════════════════════════════════

/// A single borrower address must produce the exact same key bytes on every
/// invocation.  The bech32 serialisation is stateless, so this is guaranteed.
#[test]
fn test_key_stability_same_address_produces_identical_keys() {
    let deps = mock_dependencies();
    let borrower = make_addr(&deps, "borrower");

    let iterations = 100;
    let mut keys = Vec::with_capacity(iterations);

    for _ in 0..iterations {
        let key = BorrowerKey::from_address(&borrower);
        keys.push(key);
    }

    let first = &keys[0];
    for (i, key) in keys.iter().enumerate() {
        assert_eq!(
            key, first,
            "Key at iteration {} differs from first key. Stability violated!",
            i
        );
    }

    // HashSet sanity check
    let unique: HashSet<&BorrowerKey> = keys.iter().collect();
    assert_eq!(unique.len(), 1, "Expected exactly 1 unique key");
}

/// `borrower_key_bytes` must also be stable across repeated calls.
#[test]
fn test_borrower_key_bytes_stability() {
    let deps = mock_dependencies();
    let borrower = make_addr(&deps, "borrower");

    let iterations = 100;
    let mut results = Vec::with_capacity(iterations);

    for _ in 0..iterations {
        results.push(borrower_key_bytes(&borrower));
    }

    let first = &results[0];
    for (i, r) in results.iter().enumerate() {
        assert_eq!(r, first, "borrower_key_bytes at {} differs", i);
    }
}

/// Key stability must hold even when other state changes between calls.
#[test]
fn test_key_stability_across_state_changes() {
    let deps = mock_dependencies();
    let borrower = make_addr(&deps, "borrower");

    let key_before = BorrowerKey::from_address(&borrower);

    // Simulate other operations (different address generation etc.)
    let _ = make_addr(&deps, "another");
    let _ = make_addr(&deps, "third");

    let key_after = BorrowerKey::from_address(&borrower);

    assert_eq!(
        key_before, key_after,
        "Key changed after unrelated state operations"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Test 2: Key Uniqueness — Different Addresses Produce Different Keys
// ═══════════════════════════════════════════════════════════════════════════

/// No two different addresses may produce the same key.  This is the
/// collision-resistance guarantee.
#[test]
fn test_key_uniqueness_different_addresses_produce_unique_keys() {
    let deps = mock_dependencies();

    let address_count = 100;
    let mut keys = Vec::with_capacity(address_count);

    for i in 0..address_count {
        let label = format!("borrower_{:04}", i);
        let addr = make_addr(&deps, &label);
        keys.push(BorrowerKey::from_address(&addr));
    }

    // Detect collisions with HashSet
    let unique: HashSet<&BorrowerKey> = keys.iter().collect();
    assert_eq!(
        unique.len(),
        address_count,
        "Collision! Expected {} unique keys, got {}",
        address_count,
        unique.len()
    );

    // Pairwise check
    for i in 0..keys.len() {
        for j in (i + 1)..keys.len() {
            assert_ne!(
                keys[i], keys[j],
                "Collision between address {} and {}",
                i, j
            );
        }
    }
}

/// Large-scale uniqueness test (200+ addresses).
#[test]
fn test_key_uniqueness_large_pool() {
    let deps = mock_dependencies();

    let address_count = 200;
    let mut keys = Vec::with_capacity(address_count);

    for i in 0..address_count {
        let label = format!("addr_{:05}", i);
        let addr = make_addr(&deps, &label);
        keys.push(borrower_key_bytes(&addr));
    }

    let unique: HashSet<&Vec<u8>> = keys.iter().collect();
    assert_eq!(
        unique.len(),
        address_count,
        "Large-pool collision: expected {}, got {}",
        address_count,
        unique.len()
    );
}

/// Addresses that differ by only one character must still map to different keys.
#[test]
fn test_key_uniqueness_similar_addresses() {
    let deps = mock_dependencies();

    let addr_a = Addr::unchecked("cosmos1qyqszqgpqyqszqgpqyqszqgpqyqszqgpjnp7du");
    let addr_b = Addr::unchecked("cosmos1qyqszqgpqyqszqgpqyqszqgpqyqszqgpjnp7dv");

    let key_a = BorrowerKey::from_address(&addr_a);
    let key_b = BorrowerKey::from_address(&addr_b);

    assert_ne!(key_a, key_b, "Addresses differing by 1 char collided!");
    assert_ne!(
        key_a.as_bytes(),
        key_b.as_bytes(),
        "Similar address key bytes collided!"
    );
}

/// Different-length addresses must not collide or produce empty keys.
#[test]
fn test_key_uniqueness_different_length_addresses() {
    // CosmWasm mock_api.addr_make generates fixed-length addresses, so test
    // with unchecked addresses of varying lengths.
    let short = Addr::unchecked("cosmos1short");
    let long = Addr::unchecked("cosmos1qyqszqgpqyqszqgpqyqszqgpqyqszqgpjnp7du");

    let key_short = BorrowerKey::from_address(&short);
    let key_long = BorrowerKey::from_address(&long);

    assert_ne!(key_short, key_long);
    assert!(!key_short.is_empty());
    assert!(!key_long.is_empty());
    assert_ne!(key_short.len(), key_long.len());
}

// ═══════════════════════════════════════════════════════════════════════════
// Test 3: Map Isolation — Different Maps Never Collide
// ═══════════════════════════════════════════════════════════════════════════

/// Two `cw_storage_plus::Map` instances with different namespace prefixes
/// must never collide, even when storing the same address key.
#[test]
fn test_map_isolation_different_namespaces() {
    let mut deps = mock_dependencies();

    let map_a: Map<Addr, u64> = Map::new("ns_a");
    let map_b: Map<Addr, u64> = Map::new("ns_b");

    let borrower = make_addr(&deps, "borrower");

    map_a
        .save(deps.as_mut().storage, borrower.clone(), &42)
        .unwrap();
    map_b
        .save(deps.as_mut().storage, borrower.clone(), &99)
        .unwrap();

    let val_a = map_a.load(deps.as_ref().storage, borrower.clone()).unwrap();
    let val_b = map_b.load(deps.as_ref().storage, borrower.clone()).unwrap();

    assert_eq!(val_a, 42, "Map A value corrupted");
    assert_eq!(val_b, 99, "Map B value corrupted");
    assert_ne!(val_a, val_b, "Different maps should store different values");
}

/// The same address stored in two different `Map` instances of the same type
/// must be correctly isolated by the per-map namespace prefix.
#[test]
fn test_map_isolation_same_address_different_map_instances() {
    let mut deps = mock_dependencies();

    let map1: Map<Addr, String> = Map::new("prefix_one");
    let map2: Map<Addr, String> = Map::new("prefix_two");

    let borrower = make_addr(&deps, "b");

    map1.save(
        deps.as_mut().storage,
        borrower.clone(),
        &"alpha".to_string(),
    )
    .unwrap();
    map2.save(deps.as_mut().storage, borrower.clone(), &"beta".to_string())
        .unwrap();

    assert_eq!(
        map1.load(deps.as_ref().storage, borrower.clone()).unwrap(),
        "alpha"
    );
    assert_eq!(
        map2.load(deps.as_ref().storage, borrower.clone()).unwrap(),
        "beta"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Test 4: Integration — Real Contract State Isolation
// ═══════════════════════════════════════════════════════════════════════════

/// When multiple borrowers exist, each must have their own isolated credit
/// line data with no cross-contamination.
#[test]
fn test_storage_isolation_multiple_borrowers() {
    let mut deps = mock_dependencies();
    setup_contract(&mut deps);

    let id_a = create_credit_line_for(&mut deps, "alice", "1000", "500");
    let id_b = create_credit_line_for(&mut deps, "bob", "2000", "800");
    let id_c = create_credit_line_for(&mut deps, "carol", "3000", "1200");

    assert_eq!(id_a, 0);
    assert_eq!(id_b, 1);
    assert_eq!(id_c, 2);

    // Verify each borrower's credit line is isolated
    let cl_a = CREDIT_LINES.load(deps.as_ref().storage, id_a).unwrap();
    let cl_b = CREDIT_LINES.load(deps.as_ref().storage, id_b).unwrap();
    let cl_c = CREDIT_LINES.load(deps.as_ref().storage, id_c).unwrap();

    let alice = make_addr(&deps, "alice");
    let bob = make_addr(&deps, "bob");
    let carol = make_addr(&deps, "carol");

    assert_eq!(cl_a.borrower, alice);
    assert_eq!(cl_b.borrower, bob);
    assert_eq!(cl_c.borrower, carol);

    assert_eq!(cl_a.collateral_amount, Uint128::from(1000u128));
    assert_eq!(cl_b.collateral_amount, Uint128::from(2000u128));
    assert_eq!(cl_c.collateral_amount, Uint128::from(3000u128));

    assert_eq!(cl_a.credit_amount, Uint128::from(500u128));
    assert_eq!(cl_b.credit_amount, Uint128::from(800u128));
    assert_eq!(cl_c.credit_amount, Uint128::from(1200u128));
}

/// The per-borrower index (BORROWER_TO_ID) must correctly map each borrower
/// to their credit line id with no collisions.
#[test]
fn test_borrower_to_id_index_correctness() {
    let mut deps = mock_dependencies();
    setup_contract(&mut deps);

    create_credit_line_for(&mut deps, "alice", "1000", "500");
    create_credit_line_for(&mut deps, "bob", "2000", "800");

    let alice = make_addr(&deps, "alice");
    let bob = make_addr(&deps, "bob");
    let carol = make_addr(&deps, "carol"); // never opened a line

    assert_eq!(
        BORROWER_TO_ID
            .load(deps.as_ref().storage, alice.clone())
            .unwrap(),
        0
    );
    assert_eq!(
        BORROWER_TO_ID
            .load(deps.as_ref().storage, bob.clone())
            .unwrap(),
        1
    );

    // Carol has no entry
    assert!(BORROWER_TO_ID
        .may_load(deps.as_ref().storage, carol.clone())
        .unwrap()
        .is_none());
}

/// Large-scale integration: 50 borrowers, each with isolated data.
#[test]
fn test_storage_isolation_large_scale() {
    let mut deps = mock_dependencies();
    setup_contract(&mut deps);

    let borrower_count = 50;

    for i in 0..borrower_count {
        let label = format!("b_{:03}", i);
        let collateral = (i as u128 + 1) * 100;
        let credit = (i as u128 + 1) * 50;
        let id = create_credit_line_for(
            &mut deps,
            &label,
            &collateral.to_string(),
            &credit.to_string(),
        );
        assert_eq!(id, i as u64);
    }

    // Verify all are correct and isolated
    for i in 0..borrower_count {
        let label = format!("b_{:03}", i);
        let expected_addr = make_addr(&deps, &label);
        let expected_collateral = Uint128::from((i as u128 + 1) * 100);
        let expected_credit = Uint128::from((i as u128 + 1) * 50);

        let cl = CREDIT_LINES.load(deps.as_ref().storage, i as u64).unwrap();
        assert_eq!(
            cl.borrower, expected_addr,
            "Borrower {} address mismatch",
            i
        );
        assert_eq!(
            cl.collateral_amount, expected_collateral,
            "Borrower {} collateral mismatch",
            i
        );
        assert_eq!(
            cl.credit_amount, expected_credit,
            "Borrower {} credit mismatch",
            i
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Test 5: Idempotency — Repeated Writes to the Same Key
// ═══════════════════════════════════════════════════════════════════════════

/// Overwriting the same storage slot with updated data must not corrupt the
/// key or leak into other keys.
#[test]
fn test_idempotent_write_same_borrower() {
    let mut deps = mock_dependencies();

    let map: Map<Addr, u64> = Map::new("test_map");
    let borrower = make_addr(&deps, "borrower");

    // Write initial
    map.save(deps.as_mut().storage, borrower.clone(), &1)
        .unwrap();
    assert_eq!(
        map.load(deps.as_ref().storage, borrower.clone()).unwrap(),
        1
    );

    // Overwrite
    map.save(deps.as_mut().storage, borrower.clone(), &2)
        .unwrap();
    assert_eq!(
        map.load(deps.as_ref().storage, borrower.clone()).unwrap(),
        2
    );

    // Overwrite again
    map.save(deps.as_mut().storage, borrower.clone(), &3)
        .unwrap();
    assert_eq!(
        map.load(deps.as_ref().storage, borrower.clone()).unwrap(),
        3
    );
}

/// Writing to borrower A must not affect borrower B.
#[test]
fn test_independent_writes_do_not_interfere() {
    let mut deps = mock_dependencies();
    setup_contract(&mut deps);

    create_credit_line_for(&mut deps, "alice", "1000", "500");

    let alice = make_addr(&deps, "alice");
    let bob = make_addr(&deps, "bob");

    // Alice should have id 0
    assert_eq!(
        BORROWER_TO_ID
            .load(deps.as_ref().storage, alice.clone())
            .unwrap(),
        0
    );
    // Bob should NOT exist yet
    assert!(BORROWER_TO_ID
        .may_load(deps.as_ref().storage, bob.clone())
        .unwrap()
        .is_none());

    // Now create Bob
    create_credit_line_for(&mut deps, "bob", "2000", "800");

    // Alice's mapping must be unchanged
    assert_eq!(
        BORROWER_TO_ID
            .load(deps.as_ref().storage, alice.clone())
            .unwrap(),
        0
    );
    assert_eq!(
        BORROWER_TO_ID
            .load(deps.as_ref().storage, bob.clone())
            .unwrap(),
        1
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Test 6: Edge Cases
// ═══════════════════════════════════════════════════════════════════════════

/// Zero collisions in a set of sequentially-generated mock addresses.
#[test]
fn test_edge_case_sequential_addresses() {
    let deps = mock_dependencies();

    let count = 50;
    let mut addresses = Vec::with_capacity(count);

    for i in 0..count {
        let label = format!("seq_{:04}", i);
        addresses.push(make_addr(&deps, &label));
    }

    let keys: Vec<Vec<u8>> = addresses.iter().map(|a| borrower_key_bytes(a)).collect();

    let unique: HashSet<&Vec<u8>> = keys.iter().collect();
    assert_eq!(
        unique.len(),
        count,
        "Sequential addresses produced collisions"
    );
}

/// The derived key length must be equal to the address byte length.
#[test]
fn test_key_length_matches_address_length() {
    let deps = mock_dependencies();
    let borrower = make_addr(&deps, "borrower");

    let key = BorrowerKey::from_address(&borrower);
    assert_eq!(key.len(), borrower.as_bytes().len());
    assert!(!key.is_empty());
}

/// An address with a very long bech32 string must still produce a valid key.
#[test]
fn test_edge_case_max_length_address() {
    // Bech32 allows up to ~90 chars for the data portion. Create a long one.
    let long_addr = Addr::unchecked(
        "cosmos1qyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpjnp7du",
    );

    let key = BorrowerKey::from_address(&long_addr);
    assert_eq!(key.as_bytes(), long_addr.as_bytes());
    assert_eq!(key.len(), long_addr.as_bytes().len());
    assert!(!key.is_empty());
}

// ═══════════════════════════════════════════════════════════════════════════
// Test 7: Comprehensive Summary
// ═══════════════════════════════════════════════════════════════════════════

/// Single-entry-point summary test that validates all core properties in one
/// pass: stability, uniqueness, map isolation, and integration.
#[test]
fn test_summary_comprehensive_key_encoding_validation() {
    let mut deps = mock_dependencies();
    setup_contract(&mut deps);

    // ── 1. Key stability ─────────────────────────────────────────────
    let borrower = make_addr(&deps, "testuser");
    let k1 = BorrowerKey::from_address(&borrower);
    let k2 = BorrowerKey::from_address(&borrower);
    assert_eq!(k1, k2, "Stability check failed");

    // ── 2. Key uniqueness ────────────────────────────────────────────
    let addr_a = make_addr(&deps, "user_a");
    let addr_b = make_addr(&deps, "user_b");
    let key_a = BorrowerKey::from_address(&addr_a);
    let key_b = BorrowerKey::from_address(&addr_b);
    assert_ne!(key_a, key_b, "Uniqueness check failed");

    // ── 3. Map isolation ─────────────────────────────────────────────
    let map_x: Map<Addr, u64> = Map::new("map_x");
    let map_y: Map<Addr, u64> = Map::new("map_y");
    let shared = make_addr(&deps, "shared");
    map_x
        .save(deps.as_mut().storage, shared.clone(), &10)
        .unwrap();
    map_y
        .save(deps.as_mut().storage, shared.clone(), &20)
        .unwrap();
    assert_eq!(
        map_x.load(deps.as_ref().storage, shared.clone()).unwrap(),
        10
    );
    assert_eq!(map_y.load(deps.as_ref().storage, shared).unwrap(), 20);

    // ── 4. Integration ───────────────────────────────────────────────
    create_credit_line_for(&mut deps, "alice", "100", "50");
    create_credit_line_for(&mut deps, "bob", "200", "100");

    let alice = make_addr(&deps, "alice");
    let bob = make_addr(&deps, "bob");

    let alice_id = BORROWER_TO_ID
        .load(deps.as_ref().storage, alice.clone())
        .unwrap();
    let bob_id = BORROWER_TO_ID
        .load(deps.as_ref().storage, bob.clone())
        .unwrap();
    assert_ne!(alice_id, bob_id, "Borrower ids must be unique");

    let cl_alice = CREDIT_LINES.load(deps.as_ref().storage, alice_id).unwrap();
    let cl_bob = CREDIT_LINES.load(deps.as_ref().storage, bob_id).unwrap();
    assert_eq!(cl_alice.borrower, alice);
    assert_eq!(cl_bob.borrower, bob);

    // ── 5. Large-scale uniqueness ────────────────────────────────────
    let count = 100;
    let mut unique_addrs = HashSet::new();
    let mut keys = Vec::with_capacity(count);
    for i in 0..count {
        let label = format!("scale_{:04}", i);
        let addr = make_addr(&deps, &label);
        unique_addrs.insert(addr.to_string());
        keys.push(BorrowerKey::from_address(&addr));
    }
    let unique_keys: HashSet<&BorrowerKey> = keys.iter().collect();
    assert_eq!(unique_keys.len(), count, "Large-scale uniqueness failed");
    assert_eq!(
        unique_addrs.len(),
        count,
        "Mock API generated duplicate addresses"
    );
}
