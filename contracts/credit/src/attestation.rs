// SPDX-License-Identifier: MIT

//! Attestation batch aggregation for credit scoring.
//!
//! # What
//!
//! Allows an admin to aggregate multiple off-chain signed attestations (e.g.
//! income proofs, identity verifications, behavioural signals) into a single
//! compact on-chain record by committing the **Merkle root** of the leaf set.
//! Individual attestations can then be verified against the stored root by
//! supplying a standard binary-Merkle inclusion proof, without storing every
//! leaf on-chain.
//!
//! # How
//!
//! ## Commit
//! `commit_attestation_batch` stores an [`AttestationBatch`] keyed by
//! borrower in persistent storage.  A new call overwrites the previous batch,
//! allowing the admin to rotate the attestation set as the borrower's profile
//! evolves.
//!
//! ## Verify
//! `verify_attestation_proof` recomputes the Merkle root from a leaf and a
//! sibling proof path using the **sorted-pair** (a.k.a. order-independent)
//! convention: at each level the two nodes are lexicographically sorted before
//! hashing so that the proof path is valid regardless of left/right position.
//! The hash function is `SHA-256` via `env.crypto().sha256`.
//!
//! ## Leaf encoding
//! Callers pre-hash their attestation data off-chain with SHA-256 and supply
//! the resulting 32-byte leaf.  The contract never sees raw attestation
//! contents — only commitments.
//!
//! # Why
//!
//! Storing N raw attestations on-chain is expensive (N persistent entries,
//! N TTL bumps).  A Merkle root reduces N attestations to a single 32-byte
//! commitment while preserving the ability to prove any individual leaf in
//! O(log N) hashes.  This follows the same pattern used by Ethereum ERC-20
//! airdrop merkle-distributors and EIP-712 structured-data trees.
//!
//! # Security
//!
//! - **Second-preimage resistance**: Each internal node is hashed with a
//!   domain-separation prefix (`b"\x01"`) and each leaf is passed in already
//!   hashed by the caller, preventing length-extension / second-preimage
//!   attacks against the tree structure.
//! - **Replay protection**: The `committed_at` timestamp is stored with the
//!   root; callers can use it to detect stale batch rotations.
//! - **Admin-only writes**: `commit_attestation_batch` and
//!   `clear_attestation_batch` require admin authorization.
//! - **Permissionless reads/proofs**: `verify_attestation_proof` and
//!   `get_attestation_batch` are read-only and require no authorization.

#![warn(missing_docs)]

use crate::auth::require_admin_auth;
use crate::events::{publish_attestation_batch_committed, AttestationBatchCommittedEvent};
use crate::storage::{assert_not_paused, DataKey, LEDGER_BUMP_AMOUNT, LEDGER_BUMP_THRESHOLD};
use crate::types::ContractError;
use soroban_sdk::{Address, Bytes, BytesN, Env, Vec};

// ── Types ─────────────────────────────────────────────────────────────────────

/// On-chain record for an aggregated attestation batch.
///
/// Stores the Merkle root of all leaf hashes in the batch, the number of
/// leaves, and the ledger timestamp at which the batch was committed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[soroban_sdk::contracttype]
pub struct AttestationBatch {
    /// SHA-256 Merkle root of all leaf hashes in the batch.
    pub merkle_root: BytesN<32>,
    /// Number of leaf hashes committed in this batch (informational; not
    /// verified on-chain).
    pub count: u32,
    /// Ledger timestamp when this batch was committed.
    pub committed_at: u64,
}

// ── Internal Merkle helpers ───────────────────────────────────────────────────

/// Compare two `BytesN<32>` lexicographically, returning true when `a <= b`.
fn bytes32_le(a: &BytesN<32>, b: &BytesN<32>) -> bool {
    for i in 0u32..32 {
        let av = a.get(i).unwrap_or(0);
        let bv = b.get(i).unwrap_or(0);
        if av < bv {
            return true;
        }
        if av > bv {
            return false;
        }
    }
    true // equal
}

/// Hash two sibling nodes using sorted-pair (order-independent) convention.
///
/// Nodes are lexicographically sorted before hashing so proof paths do not
/// need to encode left/right direction.  A `\x01` domain-separation byte is
/// prepended to distinguish internal nodes from leaves.
fn hash_pair(env: &Env, a: &BytesN<32>, b: &BytesN<32>) -> BytesN<32> {
    // Sort lexicographically to enforce canonical ordering.
    let (left, right) = if bytes32_le(a, b) { (a, b) } else { (b, a) };

    // Domain separation prefix for internal nodes.
    let mut buf = Bytes::new(env);
    buf.push_back(0x01u8);
    let left_bytes: Bytes = left.clone().into();
    let right_bytes: Bytes = right.clone().into();
    buf.append(&left_bytes);
    buf.append(&right_bytes);
    env.crypto().sha256(&buf).into()
}

/// Compute the Merkle root from a `leaf` and an ordered sibling `proof` path.
///
/// Each element of `proof` is a sibling hash at that level.  The root is
/// computed bottom-up using the sorted-pair convention so callers do not need
/// to supply direction bits.
///
/// # Parameters
/// - `env`:   The Soroban environment (needed for SHA-256).
/// - `leaf`:  The 32-byte leaf hash to prove inclusion of.
/// - `proof`: Ordered list of sibling hashes from leaf to root.
///
/// # Returns
/// The recomputed Merkle root.
pub fn compute_root(env: &Env, leaf: BytesN<32>, proof: &Vec<BytesN<32>>) -> BytesN<32> {
    let mut current = leaf;
    for sibling in proof.iter() {
        current = hash_pair(env, &current, &sibling);
    }
    current
}

// ── Public entrypoints ────────────────────────────────────────────────────────

/// Commit (or replace) an attestation batch for a borrower (admin only).
///
/// Stores the Merkle root of all attestation leaf hashes under
/// `DataKey::AttestationBatch(borrower)` in persistent storage.  A second
/// call for the same borrower **replaces** the previous batch, enabling
/// incremental profile updates without clearing first.
///
/// # Parameters
/// - `env`:         The Soroban environment.
/// - `borrower`:    Address of the borrower this batch describes.
/// - `merkle_root`: SHA-256 Merkle root of all leaf hashes in the batch.
/// - `count`:       Informational leaf count (not validated on-chain).
///
/// # Authorization
/// Requires administrative privileges.
///
/// # Errors
/// - `ContractError::Paused` if the protocol is paused.
/// - Auth panic if caller is not admin.
pub fn commit_attestation_batch(
    env: Env,
    borrower: Address,
    merkle_root: BytesN<32>,
    count: u32,
) {
    assert_not_paused(&env);
    require_admin_auth(&env);

    let batch = AttestationBatch {
        merkle_root: merkle_root.clone(),
        count,
        committed_at: env.ledger().timestamp(),
    };

    let key = DataKey::AttestationBatch(borrower.clone());
    env.storage().persistent().set(&key, &batch);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_BUMP_THRESHOLD, LEDGER_BUMP_AMOUNT);

    publish_attestation_batch_committed(
        &env,
        AttestationBatchCommittedEvent {
            borrower,
            merkle_root,
            count,
        },
    );
}

/// Verify that a leaf is included in the stored attestation batch.
///
/// Recomputes the Merkle root from `leaf` and `proof`, then checks it against
/// the root stored for `borrower`.
///
/// # Parameters
/// - `env`:      The Soroban environment.
/// - `borrower`: Address of the borrower whose batch to check against.
/// - `leaf`:     SHA-256 hash of the attestation to verify.
/// - `proof`:    Ordered sibling-hash path from the leaf to the root.
///
/// # Returns
/// `true` if the recomputed root matches the stored root; `false` otherwise.
///
/// # Errors
/// - `ContractError::AttestationBatchNotFound` if no batch has been committed
///   for this borrower.
pub fn verify_attestation_proof(
    env: Env,
    borrower: Address,
    leaf: BytesN<32>,
    proof: Vec<BytesN<32>>,
) -> bool {
    let key = DataKey::AttestationBatch(borrower.clone());
    let batch: AttestationBatch = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| env.panic_with_error(ContractError::AttestationBatchNotFound));

    // Bump TTL on read.
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_BUMP_THRESHOLD, LEDGER_BUMP_AMOUNT);

    let computed_root = compute_root(&env, leaf, &proof);
    computed_root == batch.merkle_root
}

/// Get the stored attestation batch for a borrower, if any.
///
/// # Parameters
/// - `env`:      The Soroban environment.
/// - `borrower`: Address of the borrower.
///
/// # Returns
/// `Some(AttestationBatch)` if a batch exists; `None` otherwise.
pub fn get_attestation_batch(env: Env, borrower: Address) -> Option<AttestationBatch> {
    let key = DataKey::AttestationBatch(borrower.clone());
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_BUMP_THRESHOLD, LEDGER_BUMP_AMOUNT);
        env.storage().persistent().get(&key)
    } else {
        None
    }
}

/// Clear the attestation batch for a borrower (admin only).
///
/// Removes the stored batch, freeing persistent storage.  Useful when a
/// borrower's profile is reset or the credit line is closed.
///
/// # Authorization
/// Requires administrative privileges.
///
/// # Errors
/// - `ContractError::Paused` if the protocol is paused.
/// - Auth panic if caller is not admin.
pub fn clear_attestation_batch(env: Env, borrower: Address) {
    assert_not_paused(&env);
    require_admin_auth(&env);

    let key = DataKey::AttestationBatch(borrower);
    env.storage().persistent().remove(&key);
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, vec, Env};

    // ── helpers ──────────────────────────────────────────────────────────────

    /// SHA-256 hash of a single byte value — used as a test leaf.
    fn leaf(env: &Env, pattern: u8) -> BytesN<32> {
        let mut data = Bytes::new(env);
        data.push_back(pattern);
        env.crypto().sha256(&data)
    }

    /// Merkle root of two leaves via `hash_pair` (which sorts internally).
    fn two_leaf_root(env: &Env, l0: BytesN<32>, l1: BytesN<32>) -> BytesN<32> {
        hash_pair(env, &l0, &l1)
    }

    fn setup_admin(env: &Env) {
        let admin = Address::generate(env);
        env.storage()
            .instance()
            .set(&crate::storage::admin_key(env), &admin);
    }

    // ── compute_root ─────────────────────────────────────────────────────────

    #[test]
    fn compute_root_single_leaf_empty_proof() {
        let env = Env::default();
        let l = leaf(&env, 0xAB);
        // Empty proof: root == leaf.
        let root = compute_root(&env, l.clone(), &vec![&env]);
        assert_eq!(root, l);
    }

    #[test]
    fn compute_root_two_leaves_correct() {
        let env = Env::default();
        let l0 = leaf(&env, 0x01);
        let l1 = leaf(&env, 0x02);
        let expected_root = two_leaf_root(&env, l0.clone(), l1.clone());

        let root = compute_root(&env, l0, &vec![&env, l1]);
        assert_eq!(root, expected_root);
    }

    #[test]
    fn compute_root_sorted_pair_commutative() {
        // Sorted-pair hashing means both leaves produce the same root.
        let env = Env::default();
        let l0 = leaf(&env, 0x01);
        let l1 = leaf(&env, 0x02);

        let root_from_l0 = compute_root(&env, l0.clone(), &vec![&env, l1.clone()]);
        let root_from_l1 = compute_root(&env, l1, &vec![&env, l0]);
        assert_eq!(root_from_l0, root_from_l1);
    }

    #[test]
    fn compute_root_four_leaves() {
        let env = Env::default();
        let l0 = leaf(&env, 0x00);
        let l1 = leaf(&env, 0x01);
        let l2 = leaf(&env, 0x02);
        let l3 = leaf(&env, 0x03);

        let n01 = two_leaf_root(&env, l0.clone(), l1.clone());
        let n23 = two_leaf_root(&env, l2.clone(), l3.clone());
        let expected_root = two_leaf_root(&env, n01.clone(), n23.clone());

        // Prove l0 with proof = [l1, n23]
        let root = compute_root(&env, l0, &vec![&env, l1, n23.clone()]);
        assert_eq!(root, expected_root);

        // Prove l2 with proof = [l3, n01]
        let root2 = compute_root(&env, l2, &vec![&env, l3, n01]);
        assert_eq!(root2, expected_root);
    }

    // ── commit / get / clear ──────────────────────────────────────────────────

    #[test]
    fn commit_and_get_attestation_batch() {
        let env = Env::default();
        env.mock_all_auths();
        setup_admin(&env);

        let borrower = Address::generate(&env);
        let root = leaf(&env, 0xAA);

        commit_attestation_batch(env.clone(), borrower.clone(), root.clone(), 3);

        let batch = get_attestation_batch(env, borrower).expect("batch should exist");
        assert_eq!(batch.merkle_root, root);
        assert_eq!(batch.count, 3);
    }

    #[test]
    fn commit_overwrites_previous_batch() {
        let env = Env::default();
        env.mock_all_auths();
        setup_admin(&env);

        let borrower = Address::generate(&env);
        let root1 = leaf(&env, 0x11);
        let root2 = leaf(&env, 0x22);

        commit_attestation_batch(env.clone(), borrower.clone(), root1, 1);
        commit_attestation_batch(env.clone(), borrower.clone(), root2.clone(), 2);

        let batch = get_attestation_batch(env, borrower).expect("batch should exist");
        assert_eq!(batch.merkle_root, root2);
        assert_eq!(batch.count, 2);
    }

    #[test]
    fn clear_attestation_batch_removes_entry() {
        let env = Env::default();
        env.mock_all_auths();
        setup_admin(&env);

        let borrower = Address::generate(&env);
        commit_attestation_batch(env.clone(), borrower.clone(), leaf(&env, 0xBB), 1);
        clear_attestation_batch(env.clone(), borrower.clone());

        assert!(get_attestation_batch(env, borrower).is_none());
    }

    #[test]
    fn get_nonexistent_batch_returns_none() {
        let env = Env::default();
        let borrower = Address::generate(&env);
        assert!(get_attestation_batch(env, borrower).is_none());
    }

    // ── verify_attestation_proof ──────────────────────────────────────────────

    #[test]
    fn verify_single_leaf_batch() {
        let env = Env::default();
        env.mock_all_auths();
        setup_admin(&env);

        let borrower = Address::generate(&env);
        let l = leaf(&env, 0xCC);

        // Single-leaf tree: root == leaf, proof is empty.
        commit_attestation_batch(env.clone(), borrower.clone(), l.clone(), 1);

        assert!(verify_attestation_proof(
            env.clone(),
            borrower,
            l,
            vec![&env]
        ));
    }

    #[test]
    fn verify_two_leaf_batch_both_leaves() {
        let env = Env::default();
        env.mock_all_auths();
        setup_admin(&env);

        let borrower = Address::generate(&env);
        let l0 = leaf(&env, 0x01);
        let l1 = leaf(&env, 0x02);
        let root = two_leaf_root(&env, l0.clone(), l1.clone());

        commit_attestation_batch(env.clone(), borrower.clone(), root, 2);

        assert!(verify_attestation_proof(
            env.clone(),
            borrower.clone(),
            l0.clone(),
            vec![&env, l1.clone()]
        ));
        assert!(verify_attestation_proof(
            env.clone(),
            borrower,
            l1,
            vec![&env, l0]
        ));
    }

    #[test]
    fn verify_wrong_leaf_returns_false() {
        let env = Env::default();
        env.mock_all_auths();
        setup_admin(&env);

        let borrower = Address::generate(&env);
        let l0 = leaf(&env, 0x01);
        let l1 = leaf(&env, 0x02);
        let wrong_leaf = leaf(&env, 0xFF);
        let root = two_leaf_root(&env, l0, l1.clone());

        commit_attestation_batch(env.clone(), borrower.clone(), root, 2);

        assert!(!verify_attestation_proof(
            env.clone(),
            borrower,
            wrong_leaf,
            vec![&env, l1]
        ));
    }

    #[test]
    #[should_panic]
    fn verify_no_batch_panics() {
        let env = Env::default();
        let borrower = Address::generate(&env);
        let l = leaf(&env, 0xDD);
        // No batch committed — must panic with AttestationBatchNotFound.
        verify_attestation_proof(env.clone(), borrower, l, vec![&env]);
    }
}
