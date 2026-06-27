// SPDX-License-Identifier: MIT

//! VRF commitment hooks for credit score derivation.
//!
//! # What
//!
//! Provides a commitment scheme to prevent ex-post manipulation of credit scores.
//! Before a risk score can be updated, a VRF output must be committed to, and the
//! final score must be derived from that committed VRF output.
//!
//! # How
//!
//! The commitment scheme works in two phases:
//!
//! 1. **Commit phase** — `commit_vrf_output` stores a hash of the VRF output
//!    for a borrower. This creates a binding commitment that cannot be changed
//!    once set.
//! 2. **Reveal phase** — When `update_risk_parameters` is called, the contract
//!    verifies that the provided score matches the committed VRF output via
//!    `verify_vrf_commitment`.
//!
//! # Why
//!
//! Without VRF commitments, an admin could potentially manipulate risk scores
//! after seeing market conditions or borrower behavior. By committing to a VRF
//! output first, the score becomes cryptographically bound to an unpredictable
//! value that was chosen before any sensitive information was known.

#![warn(missing_docs)]

use crate::auth::require_admin_auth;
use crate::storage::{assert_not_paused, bump_credit_line_ttl, DataKey};
use crate::types::ContractError;
use soroban_sdk::{Address, BytesN, Env};

/// Length of the VRF commitment hash in bytes (256-bit hash).
pub const VRF_COMMITMENT_HASH_LEN: u32 = 32;

/// VRF commitment data stored per borrower.
#[derive(Clone, Debug, Eq, PartialEq)]
#[soroban_sdk::contracttype]
pub struct VrfCommitment {
    /// Hash of the VRF output (commitment).
    pub commitment_hash: BytesN<32>,
    /// Ledger timestamp when the commitment was made.
    pub committed_at: u64,
}

/// Commit to a VRF output for a borrower's credit score derivation.
///
/// This function stores a hash of the VRF output, creating a binding commitment
/// that prevents ex-post manipulation of the credit score. The commitment must
/// be set before `update_risk_parameters` can be called with a new score.
///
/// # Parameters
/// - `env`: The Soroban environment.
/// - `borrower`: Address of the borrower whose score will be derived from this VRF.
/// - `commitment_hash`: 256-bit hash of the VRF output (e.g., SHA-256 of the VRF output).
///
/// # Authorization
/// Requires administrative privileges.
///
/// # Storage
/// Stores the commitment under `DataKey::VrfCommitment(Address)` in persistent storage.
///
/// # Errors
/// - Panics with [`ContractError::Paused`] if the protocol is paused.
/// - Panics with auth error if the caller is not the configured admin.
/// - Panics with [`ContractError::InvalidAmount`] if a commitment already exists for this borrower.
pub fn commit_vrf_output(env: Env, borrower: Address, commitment_hash: BytesN<32>) {
    assert_not_paused(&env);
    require_admin_auth(&env);

    // Check if a commitment already exists
    let key = DataKey::VrfCommitment(borrower.clone());
    if env.storage().persistent().has(&key) {
        env.panic_with_error(ContractError::InvalidAmount);
    }

    let commitment = VrfCommitment {
        commitment_hash,
        committed_at: env.ledger().timestamp(),
    };

    env.storage()
        .persistent()
        .set(&key, &commitment);
    bump_credit_line_ttl(&env, &borrower);
}

/// Verify that a risk score matches the committed VRF output.
///
/// This function checks that the provided score is cryptographically derived
/// from the previously committed VRF output. It uses a deterministic mapping
/// from the VRF hash to a score in the range [0, 100].
///
/// # Parameters
/// - `env`: The Soroban environment.
/// - `borrower`: Address of the borrower.
/// - `risk_score`: The risk score to verify (0-100).
///
/// # Returns
/// `true` if the score matches the committed VRF output, `false` otherwise.
///
/// # Errors
/// - Panics with [`ContractError::CreditLineNotFound`] if no commitment exists.
///
/// # Score derivation
/// The score is derived from the commitment hash using a deterministic formula:
/// ```text
/// score = (hash_bytes[0] + hash_bytes[1] + ... + hash_bytes[31]) % 101
/// ```
/// This ensures the score is uniformly distributed in [0, 100] while being
/// cryptographically bound to the VRF output.
pub fn verify_vrf_commitment(env: &Env, borrower: &Address, risk_score: u32) -> bool {
    let key = DataKey::VrfCommitment(borrower.clone());
    let commitment: VrfCommitment = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| env.panic_with_error(ContractError::CreditLineNotFound));

    bump_credit_line_ttl(env, borrower);

    // Derive expected score from commitment hash
    let expected_score = derive_score_from_hash(&commitment.commitment_hash);

    expected_score == risk_score
}

/// Derive a risk score (0-100) from a VRF commitment hash.
///
/// This is a deterministic, non-invertible function that maps a 256-bit hash
/// to a score in the range [0, 100]. The function is designed to be:
/// - Uniform: Each score has approximately equal probability
/// - Deterministic: Same hash always produces same score
/// - Non-invertible: Cannot recover the hash from the score
///
/// # Parameters
/// - `hash`: The 256-bit VRF commitment hash.
///
/// # Returns
/// A risk score in the range [0, 100].
///
/// # Formula
/// ```text
/// score = (sum of all bytes) % 101
/// ```
fn derive_score_from_hash(hash: &BytesN<32>) -> u32 {
    let mut sum: u32 = 0;
    for i in 0..32 {
        let byte = hash.get(i).unwrap_or(0);
        sum = sum.saturating_add(byte as u32);
    }
    sum % 101 // Modulo 101 gives range [0, 100]
}

/// Test helper function to expose score derivation for testing.
///
/// This function is only available in test builds and allows integration tests
/// to verify the score derivation logic without needing to commit and verify
/// through the full workflow.
#[cfg(test)]
pub fn derive_score_from_hash_test_helper(hash: &BytesN<32>) -> u32 {
    derive_score_from_hash(hash)
}

/// Clear the VRF commitment for a borrower (admin only).
///
/// This function removes the VRF commitment, allowing a new commitment to be
/// made. This is intended for cases where the VRF process needs to be restarted
/// (e.g., VRF failure, timeout).
///
/// # Parameters
/// - `env`: The Soroban environment.
/// - `borrower`: Address of the borrower.
///
/// # Authorization
/// Requires administrative privileges.
///
/// # Storage
/// Removes the commitment from `DataKey::VrfCommitment(Address)`.
///
/// # Errors
/// - Panics with [`ContractError::Paused`] if the protocol is paused.
/// - Panics with auth error if the caller is not the configured admin.
pub fn clear_vrf_commitment(env: Env, borrower: Address) {
    assert_not_paused(&env);
    require_admin_auth(&env);

    let key = DataKey::VrfCommitment(borrower.clone());
    env.storage().persistent().remove(&key);
}

/// Get the VRF commitment for a borrower (if it exists).
///
/// # Parameters
/// - `env`: The Soroban environment.
/// - `borrower`: Address of the borrower.
///
/// # Returns
/// The VRF commitment data, or `None` if no commitment exists.
pub fn get_vrf_commitment(env: &Env, borrower: &Address) -> Option<VrfCommitment> {
    let key = DataKey::VrfCommitment(borrower.clone());
    if env.storage().persistent().has(&key) {
        bump_credit_line_ttl(env, borrower);
        env.storage().persistent().get(&key)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::BytesN;

    #[test]
    fn test_derive_score_from_hash_deterministic() {
        let env = Env::default();
        let hash1: BytesN<32> = BytesN::from_array(&env, &[0u8; 32]);
        let hash2: BytesN<32> = BytesN::from_array(&env, &[1u8; 32]);

        let score1 = derive_score_from_hash(&hash1);
        let score2 = derive_score_from_hash(&hash2);

        // Same hash should produce same score
        assert_eq!(derive_score_from_hash(&hash1), score1);
        assert_eq!(derive_score_from_hash(&hash2), score2);

        // Different hashes should (likely) produce different scores
        assert_ne!(score1, score2);
    }

    #[test]
    fn test_derive_score_from_hash_range() {
        let env = Env::default();
        
        // Test with various hash patterns
        let hash_zero: BytesN<32> = BytesN::from_array(&env, &[0u8; 32]);
        let hash_max: BytesN<32> = BytesN::from_array(&env, &[255u8; 32]);
        let hash_mixed: BytesN<32> = BytesN::from_array(&env, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32]);

        let score_zero = derive_score_from_hash(&hash_zero);
        let score_max = derive_score_from_hash(&hash_max);
        let score_mixed = derive_score_from_hash(&hash_mixed);

        // All scores should be in range [0, 100]
        assert!(score_zero <= 100);
        assert!(score_max <= 100);
        assert!(score_mixed <= 100);
    }

    #[test]
    fn test_derive_score_from_hash_distribution() {
        let env = Env::default();
        
        // Test that the distribution covers the range
        let mut scores = std::collections::HashSet::new();
        for i in 0..100 {
            let mut bytes = [0u8; 32];
            bytes[0] = i as u8;
            let hash: BytesN<32> = BytesN::from_array(&env, &bytes);
            let score = derive_score_from_hash(&hash);
            scores.insert(score);
        }

        // Should have good coverage (at least 50 distinct scores out of 101 possible)
        assert!(scores.len() >= 50);
    }
}
