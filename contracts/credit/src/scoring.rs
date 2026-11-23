// SPDX-License-Identifier: MIT

//! VRF output commitments and on-chain oracle proof verification.
//!
//! # What
//!
//! Scores are committed through one of two paths:
//!
//! 1. **Verified (recommended).** [`commit_vrf_output_verified`] accepts a
//!    32-byte oracle output and a 64-byte Ed25519 proof. The contract rebuilds
//!    a domain-separated message binding the contract address, the borrower,
//!    and the output, verifies it against the configured VRF oracle public key
//!    with the host's Ed25519 verifier, and only then stores the commitment.
//!    `update_risk_parameters` derives the accepted score from that verified
//!    output ([`derive_score_from_hash`], `sum(bytes) % 101`).
//! 2. **Legacy pre-commitment.** [`commit_vrf_output`] stores a 32-byte value
//!    supplied by the admin, with no proof. It is kept for ABI compatibility
//!    but is rejected once an oracle key is configured
//!    ([`ContractError::VrfProofRequired`]), and
//!    `update_risk_parameters` rejects score changes that rely on it.
//!
//! The oracle key is managed with [`set_vrf_oracle_public_key`] (admin only).
//!
//! # Proof message
//!
//! The signed message is the concatenation of Soroban XDR values:
//!
//! ```text
//! b"CreditraVRF/v1"
//! || ScVal::Address(contract_id).to_xdr()
//! || ScVal::Address(borrower).to_xdr()
//! || ScVal::Bytes(output).to_xdr()
//! ```
//!
//! Binding the contract address prevents a proof for one deployment from being
//! replayed on another; binding the borrower prevents cross-borrower replay;
//! binding the output ties the signature to the exact 32 bytes that seed the
//! score. [`vrf_proof_message`] exposes the encoding so off-chain signers can
//! reproduce it byte-for-byte.
//!
//! # What this is *not*
//!
//! The contract does not run a VRF. It authenticates the output of an oracle
//! that is trusted to run one. [`derive_score_from_hash`] is a public,
//! deterministic, lossy reduction: it is not a hash and has no preimage
//! resistance. What the verified path adds is that the 32 bytes must carry a
//! valid oracle signature over this contract, this borrower, and this output,
//! so the admin cannot substitute a hash of their choosing.
//!
//! # Trust assumptions
//!
//! The guarantee holds only while all of the following hold:
//!
//! 1. **The oracle key is independent.** The key set with
//!    [`set_vrf_oracle_public_key`] belongs to an oracle whose signing policy is
//!    not controlled by the same party that sets scores. An admin who can
//!    rotate the key to one they control can forge outputs, so production
//!    deployments should place the admin role behind a governance contract or
//!    multisig with a timelock.
//! 2. **The oracle signs once per cycle.** The message binds the borrower and
//!    output but carries no epoch or freshness deadline, so the oracle must not
//!    sign competing outputs for the same borrower. If it does, whoever submits
//!    first (verified commits are permissionless) selects the output.
//! 3. **The oracle is live.** If the oracle stops signing, verified score
//!    changes are impossible. `clear_vrf_commitment` and
//!    `update_risk_parameters` remain usable without a commitment, and the
//!    legacy path is only blocked while an oracle key is configured.
//! 4. **Key rotation is deliberate.** [`is_vrf_commitment_verified`] requires
//!    the recorded proof to use the *currently* configured key, so rotating the
//!    key invalidates existing verified commitments until they are cleared and
//!    re-committed. This is intentional: it lets governance revoke a
//!    compromised key.
//!
//! Commitments are optional: when no commitment exists,
//! `update_risk_parameters` skips the score check entirely (issue #1240 tracks
//! making them mandatory).
//!
//! # Failure modes
//!
//! An invalid proof makes [`commit_vrf_output_verified`] revert with the
//! Soroban host's Ed25519 verification error
//! (`ScErrorType::Crypto` / `ScErrorCode::InvalidInput`, message
//! `"failed ED25519 verification"`). The contract cannot catch host crypto
//! errors, so this is the dedicated failure for a bad signature; the
//! contract-level errors [`ContractError::VrfOracleNotConfigured`] and
//! [`ContractError::VrfProofRequired`] cover the configuration and legacy-path
//! cases.
//!
//! # Follow-ups
//!
//! The score distribution of [`derive_score_from_hash`] is measured by the
//! fixed-seed test in `contracts/credit/tests/vrf_commitment.rs` (issue #1329).

#![warn(missing_docs)]

use crate::auth::require_admin_auth;
use crate::events::{
    publish_vrf_commitment_verified_event, publish_vrf_oracle_key_set_event,
    VrfCommitmentVerifiedEvent, VrfOracleKeySetEvent,
};
use crate::storage::{
    assert_not_paused, bump_vrf_commitment_ttl, bump_vrf_verification_ttl,
    get_vrf_oracle_public_key as read_vrf_oracle_public_key,
    set_vrf_oracle_public_key as store_vrf_oracle_public_key, DataKey,
};
use crate::types::ContractError;
use soroban_sdk::xdr::ToXdr;
use soroban_sdk::{Address, Bytes, BytesN, Env};

/// Domain separator prefixed to every signed VRF proof message.
pub const VRF_PROOF_DOMAIN: &[u8] = b"CreditraVRF/v1";

/// Length of the oracle output / commitment value in bytes (32).
pub const VRF_COMMITMENT_HASH_LEN: u32 = 32;

/// Length of an Ed25519 proof in bytes (64).
pub const VRF_PROOF_LEN: u32 = 64;

/// Score pre-commitment stored per borrower.
#[derive(Clone, Debug, Eq, PartialEq)]
#[soroban_sdk::contracttype]
pub struct VrfCommitment {
    /// 32-byte value that the revealed score must reduce to.
    ///
    /// For a verified commitment this is the oracle output whose Ed25519 proof
    /// was checked at commit time. For a legacy commitment it is the
    /// unauthenticated value supplied by the admin.
    pub commitment_hash: BytesN<32>,
    /// Ledger timestamp when the commitment was made.
    pub committed_at: u64,
}

/// Oracle attestation backing a verified VRF commitment.
#[derive(Clone, Debug, Eq, PartialEq)]
#[soroban_sdk::contracttype]
pub struct VrfVerification {
    /// The oracle output that was verified.
    pub output: BytesN<32>,
    /// Oracle public key that signed the proof message.
    pub oracle_public_key: BytesN<32>,
    /// Ledger timestamp when the proof was verified.
    pub verified_at: u64,
}

/// Commit to a 32-byte value with no proof (legacy, admin only).
///
/// This is the original pre-commitment path. It stores an unauthenticated
/// 32-byte value that the revealed score must reduce to. Because
/// [`derive_score_from_hash`] is public, an admin can construct a value that
/// reduces to any desired score, so this path provides change detection only.
///
/// Prefer [`commit_vrf_output_verified`]. Once a VRF oracle public key is
/// configured with [`set_vrf_oracle_public_key`], this entrypoint reverts with
/// [`ContractError::VrfProofRequired`] so an unverified commitment cannot
/// shadow the verified path.
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
/// - Panics with [`ContractError::VrfProofRequired`] if an oracle key is configured.
/// - Panics with [`ContractError::InvalidAmount`] if a commitment already exists for this borrower.
pub fn commit_vrf_output(env: Env, borrower: Address, commitment_hash: BytesN<32>) {
    assert_not_paused(&env);
    require_admin_auth(&env);

    if read_vrf_oracle_public_key(&env).is_some() {
        env.panic_with_error(ContractError::VrfProofRequired);
    }

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

/// Configure or rotate the trusted VRF oracle Ed25519 public key (admin only).
///
/// The key authenticates oracle outputs submitted through
/// [`commit_vrf_output_verified`]. It is stored in instance storage and cannot
/// be removed: once set, the legacy unverified path stays closed.
///
/// Rotating the key invalidates existing verified commitments, because
/// [`is_vrf_commitment_verified`] requires the recorded proof to use the current
/// key. See the module docs for the trust model around key management.
///
/// # Parameters
/// - `env`: The Soroban environment.
/// - `public_key`: 32-byte Ed25519 public key of the VRF oracle.
///
/// # Authorization
/// Requires administrative privileges.
///
/// # Errors
/// - Panics with [`ContractError::Paused`] if the protocol is paused.
/// - Panics with auth error if the caller is not the configured admin.
/// - Panics with [`ContractError::InvalidAmount`] for an all-zero key.
pub fn set_vrf_oracle_public_key(env: Env, public_key: BytesN<32>) {
    assert_not_paused(&env);
    require_admin_auth(&env);

    if is_zero_public_key(&public_key) {
        env.panic_with_error(ContractError::InvalidAmount);
    }

    store_vrf_oracle_public_key(&env, &public_key);
    publish_vrf_oracle_key_set_event(
        &env,
        VrfOracleKeySetEvent {
            oracle_public_key: public_key,
            timestamp: env.ledger().timestamp(),
        },
    );
}

/// Return the configured VRF oracle public key, if any.
pub fn get_vrf_oracle_public_key(env: &Env) -> Option<BytesN<32>> {
    read_vrf_oracle_public_key(env)
}

/// Build the domain-separated message the oracle signs for `(borrower, output)`.
///
/// The encoding is `VRF_PROOF_DOMAIN` followed by the Soroban XDR encoding of
/// the contract address, the borrower address, and the output bytes, in that
/// order. Off-chain signers must reproduce these bytes exactly.
pub fn vrf_proof_message(env: &Env, borrower: &Address, output: &BytesN<32>) -> Bytes {
    let mut message = Bytes::from_slice(env, VRF_PROOF_DOMAIN);
    message.append(&env.current_contract_address().to_xdr(env));
    message.append(&borrower.clone().to_xdr(env));
    message.append(&output.clone().to_xdr(env));
    message
}

/// Verify an oracle proof and store the resulting commitment (permissionless).
///
/// Anyone may submit a valid oracle-signed output: the proof is the
/// authorization, so the admin is not a gatekeeper for verified commitments.
/// The signature must cover this contract, `borrower`, and `output`; see
/// [`vrf_proof_message`].
///
/// An invalid proof reverts with the Soroban host's Ed25519 verification error
/// (`ScErrorType::Crypto` / `ScErrorCode::InvalidInput`).
///
/// # Parameters
/// - `env`: The Soroban environment.
/// - `borrower`: Address of the borrower whose score is being committed.
/// - `output`: 32-byte oracle output from which the score is derived.
/// - `proof`: 64-byte Ed25519 signature over the proof message.
///
/// # Storage
/// Writes `DataKey::VrfCommitment(Address)` and `DataKey::VrfVerification(Address)`
/// in persistent storage.
///
/// # Errors
/// - Panics with [`ContractError::Paused`] if the protocol is paused.
/// - Panics with [`ContractError::VrfOracleNotConfigured`] if no oracle key is set.
/// - Panics with [`ContractError::InvalidAmount`] if a commitment already exists.
/// - Reverts with the host Ed25519 error if the proof is invalid.
pub fn commit_vrf_output_verified(
    env: Env,
    borrower: Address,
    output: BytesN<32>,
    proof: BytesN<64>,
) {
    assert_not_paused(&env);

    let commitment_key = DataKey::VrfCommitment(borrower.clone());
    if env.storage().persistent().has(&commitment_key) {
        env.panic_with_error(ContractError::InvalidAmount);
    }

    let oracle_public_key = read_vrf_oracle_public_key(&env)
        .unwrap_or_else(|| env.panic_with_error(ContractError::VrfOracleNotConfigured));

    let message = vrf_proof_message(&env, &borrower, &output);
    env.crypto()
        .ed25519_verify(&oracle_public_key, &message, &proof);

    let now = env.ledger().timestamp();
    env.storage().persistent().set(
        &commitment_key,
        &VrfCommitment {
            commitment_hash: output.clone(),
            committed_at: now,
        },
    );
    bump_vrf_commitment_ttl(&env, &borrower);

    env.storage().persistent().set(
        &DataKey::VrfVerification(borrower.clone()),
        &VrfVerification {
            output: output.clone(),
            oracle_public_key: oracle_public_key.clone(),
            verified_at: now,
        },
    );
    bump_vrf_verification_ttl(&env, &borrower);

    publish_vrf_commitment_verified_event(
        &env,
        VrfCommitmentVerifiedEvent {
            borrower,
            output,
            oracle_public_key,
            timestamp: now,
        },
    );
}

/// Return the oracle attestation backing a borrower's commitment, if any.
pub fn get_vrf_verification(env: &Env, borrower: &Address) -> Option<VrfVerification> {
    let key = DataKey::VrfVerification(borrower.clone());
    if env.storage().persistent().has(&key) {
        bump_vrf_verification_ttl(env, borrower);
        env.storage().persistent().get(&key)
    } else {
        None
    }
}

/// Return whether the borrower's commitment is backed by an oracle proof.
///
/// The recorded output must equal the stored commitment and the proof must have
/// been verified under the currently configured oracle key. A legacy commitment
/// or a commitment verified before a key rotation returns `false`.
pub fn is_vrf_commitment_verified(env: &Env, borrower: &Address) -> bool {
    let Some(commitment) = get_vrf_commitment(env, borrower) else {
        return false;
    };
    let Some(verification) = get_vrf_verification(env, borrower) else {
        return false;
    };
    let Some(current_key) = read_vrf_oracle_public_key(env) else {
        return false;
    };

    verification.output == commitment.commitment_hash
        && verification.oracle_public_key == current_key
}

fn is_zero_public_key(public_key: &BytesN<32>) -> bool {
    for i in 0u32..VRF_COMMITMENT_HASH_LEN {
        if public_key.get(i) != Some(0) {
            return false;
        }
    }
    true
}

/// Return whether `risk_score` equals the reduction of the committed value.
///
/// # What "verify" means here
/// This is an equality check against [`derive_score_from_hash`] of the stored
/// 32 bytes. It does **not** inspect a proof. Pair it with
/// [`is_vrf_commitment_verified`] when the caller needs to know that the stored
/// value was oracle-attested.
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
/// The score is approximately uniform over `[0, 100]`, but not exactly. The
/// sum of 32 uniform bytes is bell-shaped (mean 4080, standard deviation
/// ~418), and `8161 = 101 * 80 + 81` leaves a small modulo bias (~1.25%).
/// Folding the bell shape modulo 101 still averages most of it out because the
/// deviation is more than four times the modulus, so the fixed-seed chi-square
/// test below does not reject uniformity.
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
/// Removes the commitment and its oracle attestation so
/// `update_risk_parameters` stops checking the score until a new commitment is
/// made. This is intended as an exceptional recovery path (for example, when
/// the off-chain score process has to be restarted, or to re-commit after a
/// key rotation). Because it is admin-only and resets the check, routine use
/// would defeat the change-detection property; see the module docs.
///
/// # Parameters
/// - `env`: The Soroban environment.
/// - `borrower`: Address of the borrower.
///
/// # Authorization
/// Requires administrative privileges.
///
/// # Storage
/// Removes `DataKey::VrfCommitment(Address)` and `DataKey::VrfVerification(Address)`.
///
/// # Errors
/// - Panics with [`ContractError::Paused`] if the protocol is paused.
/// - Panics with auth error if the caller is not the configured admin.
pub fn clear_vrf_commitment(env: Env, borrower: Address) {
    assert_not_paused(&env);
    require_admin_auth(&env);

    env.storage()
        .persistent()
        .remove(&DataKey::VrfCommitment(borrower.clone()));
    env.storage()
        .persistent()
        .remove(&DataKey::VrfVerification(borrower.clone()));
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
