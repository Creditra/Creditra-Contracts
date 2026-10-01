// SPDX-License-Identifier: MIT

//! Admin score pre-commitment hooks (the `Vrf` naming is legacy).
//!
//! # What
//!
//! [`commit_vrf_output`] lets the admin publish a 32-byte hash for a borrower.
//! Once a commitment exists, `update_risk_parameters` accepts only a score that
//! equals the deterministic reduction of that hash
//! ([`derive_score_from_hash`], `sum(bytes) % 101`), checked by
//! [`verify_vrf_commitment`].
//!
//! # What this is *not*
//!
//! Despite the `Vrf`-prefixed names (kept for ABI and storage compatibility),
//! **no VRF proof is verified on-chain and the protocol generates no
//! randomness**:
//!
//! - The admin supplies the hash *and* the score that must match it. The
//!   contract cannot distinguish a genuine VRF output from arbitrary bytes the
//!   admin chose.
//! - [`derive_score_from_hash`] is public and deterministic, so for any target
//!   score in `[0, 100]` a matching hash is trivial to construct (for example,
//!   the hash `[s, 0, .., 0]` reduces to `s`).
//! - The commitment is therefore a **pre-commitment / change-detection** device,
//!   not a source of unpredictability: it lets an observer detect a score that
//!   does not match the previously published hash, but it does not stop the
//!   admin from choosing a favourable hash up front.
//!
//! # Trust assumptions
//!
//! The (weaker) guarantee above holds only if **all** of the following are true.
//! None of them is enforced on-chain:
//!
//! 1. The committed hash really is the output of a VRF whose seed and proof the
//!    admin could not control or predict when choosing it.
//! 2. The admin commits *before* learning the information the score is meant to
//!    be independent of, and does not grind candidate hashes off-chain.
//! 3. [`clear_vrf_commitment`] (admin-only) is used as an exceptional recovery
//!    path, not a routine way to reset the commitment.
//! 4. Integrators present the commitment as *advisory* evidence of a
//!    pre-committed score, never as cryptographic proof of fairness.
//!
//! Commitments are also optional: when no commitment exists,
//! `update_risk_parameters` skips the score check entirely, so the mechanism
//! cannot constrain an admin who simply never commits.
//!
//! # Follow-ups
//!
//! On-chain verification of an actual VRF proof (or of an authenticated oracle
//! output) is not implemented; see issue #1447
//! (<https://github.com/Creditra/Creditra-Contracts/issues/1447>). The score
//! distribution of [`derive_score_from_hash`] is measured by the fixed-seed
//! test in `contracts/credit/tests/vrf_commitment.rs` (issue #1329).

#![warn(missing_docs)]

use crate::auth::require_admin_auth;
use crate::storage::{assert_not_paused, bump_vrf_commitment_ttl, DataKey};
use crate::types::ContractError;
use soroban_sdk::{Address, BytesN, Env};

/// Length of the pre-commitment hash in bytes (32).
pub const VRF_COMMITMENT_HASH_LEN: u32 = 32;

/// Score pre-commitment stored per borrower.
#[derive(Clone, Debug, Eq, PartialEq)]
#[soroban_sdk::contracttype]
pub struct VrfCommitment {
    /// Admin-supplied 32-byte value that the revealed score must reduce to.
    pub commitment_hash: BytesN<32>,
    /// Ledger timestamp when the commitment was made.
    pub committed_at: u64,
}

/// Commit to a 32-byte hash that a future score must reduce to (admin only).
///
/// # What it guarantees
/// Once stored, `update_risk_parameters` accepts only a score equal to
/// [`derive_score_from_hash`] of this hash, until the commitment is cleared.
/// Off-chain observers can therefore detect a score that does not match the
/// previously published hash.
///
/// # What it does *not* guarantee
/// No VRF proof is verified and the stored hash is not authenticated as a VRF
/// output — the admin chooses it. Because [`derive_score_from_hash`] is public,
/// the admin can pick a hash whose reduction is any desired score. This is a
/// pre-commitment, not proof of randomness; see the module docs for the trust
/// assumptions.
///
/// # Parameters
/// - `env`: The Soroban environment.
/// - `borrower`: Address of the borrower whose score is being pre-committed.
/// - `commitment_hash`: 32-byte value that the revealed score must reduce to.
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

    env.storage().persistent().set(&key, &commitment);
    bump_vrf_commitment_ttl(&env, &borrower);
}

/// Return whether `risk_score` equals the reduction of the committed hash.
///
/// # What "verify" means here
/// This is an equality check against [`derive_score_from_hash`] of the stored
/// 32 bytes. It does **not** verify a VRF proof and does **not** establish that
/// the stored hash came from a VRF — see the module docs.
///
/// # Parameters
/// - `env`: The Soroban environment.
/// - `borrower`: Address of the borrower.
/// - `risk_score`: The risk score to compare (0-100).
///
/// # Returns
/// `true` if `risk_score` equals the derived score, `false` otherwise.
///
/// # Errors
/// - Panics with [`ContractError::MissingVrfCommitment`] if no commitment exists.
///
/// # Score derivation
/// ```text
/// score = (hash_bytes[0] + hash_bytes[1] + ... + hash_bytes[31]) % 101
/// ```
/// The reduction is deterministic but carries no cryptographic binding. Its
/// distribution is characterised — and measured — for
/// [`derive_score_from_hash`]; see the `# Distribution` section there and the
/// fixed-seed distribution test in `contracts/credit/tests/vrf_commitment.rs`.
pub fn verify_vrf_commitment(env: &Env, borrower: &Address, risk_score: u32) -> bool {
    let key = DataKey::VrfCommitment(borrower.clone());
    let commitment: VrfCommitment = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| env.panic_with_error(ContractError::MissingVrfCommitment));

    bump_vrf_commitment_ttl(env, borrower);

    // Derive expected score from commitment hash
    let expected_score = derive_score_from_hash(&commitment.commitment_hash);

    expected_score == risk_score
}

/// Reduce a 32-byte commitment hash to a score in `[0, 100]`.
///
/// # Formula
/// ```text
/// score = (sum of all 32 bytes) % 101
/// ```
///
/// # Distribution
///
/// The score is approximately uniform over `[0, 100]`, but not because the
/// byte sum is: the sum of 32 uniform bytes is bell-shaped (mean 4080,
/// standard deviation ~418), and folding that shape modulo 101 averages it
/// out because the deviation is more than four times the modulus.
///
/// This is measured rather than assumed. The fixed-seed distribution test in
/// `contracts/credit/tests/vrf_commitment.rs`
/// (`test_derive_score_distribution_uniformity_result_with_fixed_seed`) draws
/// 20 000 hashes, verifies the byte sum really is bell-shaped, and applies a
/// chi-square goodness-of-fit test against the uniform hypothesis, which it
/// does not reject. Boundary hashes are pinned by
/// `test_derive_score_boundary_hashes_are_pinned` in the same file.
///
/// Those tests reach this private helper through
/// [`derive_score_from_hash_test_helper`].
///
/// # Properties
/// - **Deterministic**: the same hash always maps to the same score.
/// - **Public and cheap**: there is no secret input, so anyone can compute it.
/// - **Not a cryptographic function**: this is a lossy reduction with no
///   preimage resistance. Many hashes collapse to the same score, and a hash
///   reducing to any chosen score is trivial to construct. It must not be
///   described as "cryptographically bound" or "non-invertible".
///
/// # Parameters
/// - `hash`: The 32-byte commitment hash.
///
/// # Returns
/// A risk score in the range `[0, 100]`.
fn derive_score_from_hash(hash: &BytesN<32>) -> u32 {
    let mut sum: u32 = 0;
    for i in 0u32..32 {
        let byte = hash.get(i);
        if let Some(b) = byte {
            sum = sum.saturating_add(b as u32);
        }
    }
    sum % 101 // Modulo 101 gives range [0, 100]
}

/// Test helper function to expose score derivation for testing.
///
/// This function allows integration tests to verify the score derivation logic
/// without needing to commit and verify through the full workflow.
#[doc(hidden)]
pub fn derive_score_from_hash_test_helper(hash: &BytesN<32>) -> u32 {
    derive_score_from_hash(hash)
}

/// Clear the score pre-commitment for a borrower (admin only).
///
/// Removes the commitment so `update_risk_parameters` stops checking the score
/// against a previously published hash until a new commitment is made. This is
/// intended as an exceptional recovery path (for example, when the
/// off-chain score process has to be restarted). Because it is admin-only and
/// resets the check, routine use would defeat the change-detection property;
/// see the module docs for trust assumptions.
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
        bump_vrf_commitment_ttl(env, borrower);
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
        let hash_mixed: BytesN<32> = BytesN::from_array(
            &env,
            &[
                1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
                24, 25, 26, 27, 28, 29, 30, 31, 32,
            ],
        );

        // Test with various hash patterns
        let hash_zero: BytesN<32> = BytesN::from_array(&env, &[0u8; 32]);
        let hash_max: BytesN<32> = BytesN::from_array(&env, &[255u8; 32]);
        let hash_mixed: BytesN<32> = BytesN::from_array(
            &env,
            &[
                1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
                24, 25, 26, 27, 28, 29, 30, 31, 32,
            ],
        );

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
        for i in 0u32..100 {
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
