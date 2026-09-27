// SPDX-License-Identifier: MIT

//! Integration tests for VRF commitment functionality.
//!
//! Tests the full workflow of committing to a VRF output and verifying
//! that risk scores are derived from the committed VRF output.

#![cfg(test)]

use creditra_credit::scoring::VrfCommitment;
use creditra_credit::types::ContractError;
use soroban_sdk::testutils::{Address as _, BytesN as _};
use soroban_sdk::{Address, BytesN, Env};

fn create_test_contract(env: &Env) -> creditra_credit::ContractClient {
    creditra_credit::ContractClient::new(
        env,
        &env.register(creditra_credit::Credit, ()),
    )
}

fn setup_contract<'a>(env: &'a Env, admin: &Address) -> creditra_credit::CreditClient<'a> {
    let contract = create_test_contract(env);
    contract.init(&admin);
    contract
}

#[test]
fn test_commit_vrf_output() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract = setup_contract(&env, &admin);

    // Create a VRF commitment hash
    let commitment_hash: BytesN<32> = BytesN::from_array(&env, &[1u8; 32]);

    // Commit the VRF output
    contract.commit_vrf_output(&borrower, &commitment_hash);

    // Verify the commitment was stored
    let commitment = contract.get_vrf_commitment(&borrower);
    assert!(commitment.is_some());
    let commitment = commitment.unwrap();
    assert_eq!(commitment.commitment_hash, commitment_hash);
    assert!(commitment.committed_at > 0);
}

#[test]
fn test_commit_vrf_output_twice_fails() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract = setup_contract(&env, &admin);

    let commitment_hash: BytesN<32> = BytesN::from_array(&env, &[1u8; 32]);

    // First commit should succeed
    contract.commit_vrf_output(&borrower, &commitment_hash);

    // Second commit should fail
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        contract.commit_vrf_output(&borrower, &commitment_hash);
    }));
    assert!(result.is_err());
}

#[test]
fn test_clear_vrf_commitment() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract = setup_contract(&env, &admin);

    let commitment_hash: BytesN<32> = BytesN::from_array(&env, &[1u8; 32]);

    // Commit the VRF output
    contract.commit_vrf_output(&borrower, &commitment_hash);
    assert!(contract.get_vrf_commitment(&borrower).is_some());

    // Clear the commitment
    contract.clear_vrf_commitment(&borrower);

    // Verify it was cleared
    assert!(contract.get_vrf_commitment(&borrower).is_none());
}

#[test]
fn test_update_risk_parameters_with_valid_vrf_commitment() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract = setup_contract(&env, &admin);

    // Set up liquidity token
    let token = Address::generate(&env);
    contract.set_liquidity_token(&token);

    // Open a credit line with initial score
    contract.open_credit_line(&borrower, &1000_i128, &500_u32, &50_u32);

    // Create a VRF commitment hash that will derive to score 75
    // sum of bytes = 75, so score = 75 % 101 = 75
    let mut hash_bytes = [0u8; 32];
    hash_bytes[0] = 75;
    let commitment_hash: BytesN<32> = BytesN::from_array(&env, &hash_bytes);

    // Commit the VRF output
    contract.commit_vrf_output(&borrower, &commitment_hash);

    // Update risk parameters with the derived score
    contract.update_risk_parameters(&borrower, &1000_i128, &500_u32, &75_u32);

    // Verify the score was updated
    let line = contract.get_credit_line(&borrower).unwrap();
    assert_eq!(line.risk_score, 75);
}

#[test]
fn test_update_risk_parameters_with_invalid_vrf_commitment_fails() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract = setup_contract(&env, &admin);

    // Set up liquidity token
    let token = Address::generate(&env);
    contract.set_liquidity_token(&token);

    // Open a credit line with initial score
    contract.open_credit_line(&borrower, &1000_i128, &500_u32, &50_u32);

    // Create a VRF commitment hash that will derive to score 75
    let mut hash_bytes = [0u8; 32];
    hash_bytes[0] = 75;
    let commitment_hash: BytesN<32> = BytesN::from_array(&env, &hash_bytes);

    // Commit the VRF output
    contract.commit_vrf_output(&borrower, &commitment_hash);

    // Try to update with a different score (not matching the commitment)
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        contract.update_risk_parameters(&borrower, &1000_i128, &500_u32, &80_u32);
    }));
    assert!(result.is_err());
}

#[test]
fn test_update_risk_parameters_without_commitment_succeeds() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract = setup_contract(&env, &admin);

    // Set up liquidity token
    let token = Address::generate(&env);
    contract.set_liquidity_token(&token);

    // Open a credit line with initial score
    contract.open_credit_line(&borrower, &1000_i128, &500_u32, &50_u32);

    // Update risk parameters without any VRF commitment (backward compatibility)
    contract.update_risk_parameters(&borrower, &1000_i128, &600_u32, &60_u32);

    // Verify the score was updated
    let line = contract.get_credit_line(&borrower).unwrap();
    assert_eq!(line.risk_score, 60);
}

#[test]
fn test_update_risk_parameters_same_score_no_verification() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract = setup_contract(&env, &admin);

    // Set up liquidity token
    let token = Address::generate(&env);
    contract.set_liquidity_token(&token);

    // Open a credit line with initial score
    contract.open_credit_line(&borrower, &1000_i128, &500_u32, &50_u32);

    // Create a VRF commitment hash
    let commitment_hash: BytesN<32> = BytesN::from_array(&env, &[1u8; 32]);

    // Commit the VRF output
    contract.commit_vrf_output(&borrower, &commitment_hash);

    // Update with the same score (should not trigger verification)
    contract.update_risk_parameters(&borrower, &1000_i128, &500_u32, &50_u32);

    // Verify the score remains the same
    let line = contract.get_credit_line(&borrower).unwrap();
    assert_eq!(line.risk_score, 50);
}

#[test]
fn test_commit_then_clear_then_update() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract = setup_contract(&env, &admin);

    // Set up liquidity token
    let token = Address::generate(&env);
    contract.set_liquidity_token(&token);

    // Open a credit line
    contract.open_credit_line(&borrower, &1000_i128, &500_u32, &50_u32);

    // Commit VRF output
    let commitment_hash: BytesN<32> = BytesN::from_array(&env, &[1u8; 32]);
    contract.commit_vrf_output(&borrower, &commitment_hash);

    // Clear the commitment
    contract.clear_vrf_commitment(&borrower);

    // Update without commitment (should succeed due to backward compatibility)
    contract.update_risk_parameters(&borrower, &1000_i128, &600_u32, &60_u32);

    // Verify the score was updated
    let line = contract.get_credit_line(&borrower).unwrap();
    assert_eq!(line.risk_score, 60);
}

#[test]
fn test_multiple_borrowers_independent_commitments() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower1 = Address::generate(&env);
    let borrower2 = Address::generate(&env);
    let contract = setup_contract(&env, &admin);

    // Set up liquidity token
    let token = Address::generate(&env);
    contract.set_liquidity_token(&token);

    // Open credit lines for both borrowers
    contract.open_credit_line(&borrower1, &1000_i128, &500_u32, &50_u32);
    contract.open_credit_line(&borrower2, &1000_i128, &500_u32, &50_u32);

    // Commit different VRF outputs for each borrower
    let hash1: BytesN<32> = BytesN::from_array(&env, &[75u8; 32]); // derives to 75
    let hash2: BytesN<32> = BytesN::from_array(&env, &[25u8; 32]); // derives to 25

    contract.commit_vrf_output(&borrower1, &hash1);
    contract.commit_vrf_output(&borrower2, &hash2);

    // Update each borrower with their respective scores
    contract.update_risk_parameters(&borrower1, &1000_i128, &500_u32, &75_u32);
    contract.update_risk_parameters(&borrower2, &1000_i128, &500_u32, &25_u32);

    // Verify both scores were updated correctly
    let line1 = contract.get_credit_line(&borrower1).unwrap();
    let line2 = contract.get_credit_line(&borrower2).unwrap();
    assert_eq!(line1.risk_score, 75);
    assert_eq!(line2.risk_score, 25);
}

#[test]
fn test_commit_requires_admin() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let non_admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract = setup_contract(&env, &admin);

    let commitment_hash: BytesN<32> = BytesN::from_array(&env, &[1u8; 32]);

    // Try to commit as non-admin
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        contract.commit_vrf_output(&borrower, &commitment_hash);
    }));
    assert!(result.is_err());
}

#[test]
fn test_clear_requires_admin() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract = setup_contract(&env, &admin);

    let commitment_hash: BytesN<32> = BytesN::from_array(&env, &[1u8; 32]);

    // Commit as admin
    contract.commit_vrf_output(&borrower, &commitment_hash);

    // Try to clear as non-admin
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        contract.clear_vrf_commitment(&borrower);
    }));
    assert!(result.is_err());
}

#[test]
fn test_commit_when_paused_fails() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract = setup_contract(&env, &admin);

    // Pause the contract
    contract.set_protocol_paused(&true);

    let commitment_hash: BytesN<32> = BytesN::from_array(&env, &[1u8; 32]);

    // Try to commit while paused
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        contract.commit_vrf_output(&borrower, &commitment_hash);
    }));
    assert!(result.is_err());
}

#[test]
fn test_score_derivation_edge_cases() {
    let env = Env::default();

    // Test score = 0 (all bytes sum to 0)
    let hash_zero: BytesN<32> = BytesN::from_array(&env, &[0u8; 32]);
    let score_zero = creditra_credit::scoring::derive_score_from_hash_test_helper(&hash_zero);
    assert_eq!(score_zero, 0);

    // Test score = 100 (sum = 100)
    let mut hash_100 = [0u8; 32];
    hash_100[0] = 100;
    let hash_100: BytesN<32> = BytesN::from_array(&env, &hash_100);
    let score_100 = creditra_credit::scoring::derive_score_from_hash_test_helper(&hash_100);
    assert_eq!(score_100, 100);

    // Test score = 100 (sum = 201, 201 % 101 = 100)
    let mut hash_100_alt = [0u8; 32];
    hash_100_alt[0] = 200;
    hash_100_alt[1] = 1;
    let hash_100_alt: BytesN<32> = BytesN::from_array(&env, &hash_100_alt);
    let score_100_alt = creditra_credit::scoring::derive_score_from_hash_test_helper(&hash_100_alt);
    assert_eq!(score_100_alt, 100);
}

#[test]
fn test_get_vrf_commitment_none_when_not_set() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract = setup_contract(&env, &admin);

    // Get commitment when none exists
    let commitment = contract.get_vrf_commitment(&borrower);
    assert!(commitment.is_none());
}

// ── score-derivation distribution coverage (issue #1329) ─────────────────────
//
// `derive_score_from_hash` is `sum(bytes) % 101`. The *sum* of 32 uniform bytes
// is bell-shaped (mean 4080, sd ~= 418), so the interesting question is what
// `% 101` does to that shape. These tests answer it with a fixed seed instead
// of assuming an answer, and pin the boundary values.

/// Deterministic PRNG (xorshift64) so the distribution measurement is
/// reproducible on every platform and CI run.
struct Xorshift64(u64);

impl Xorshift64 {
    fn new(seed: u64) -> Self {
        assert_ne!(seed, 0, "xorshift64 requires a non-zero seed");
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn next_bytes32(&mut self) -> [u8; 32] {
        let mut out = [0u8; 32];
        for chunk in out.chunks_mut(8) {
            chunk.copy_from_slice(&self.next_u64().to_le_bytes());
        }
        out
    }
}

/// Pass a byte buffer through the contract's score derivation.
fn score_of(env: &Env, bytes: &[u8; 32]) -> u32 {
    let hash: BytesN<32> = BytesN::from_array(env, bytes);
    creditra_credit::scoring::derive_score_from_hash_test_helper(&hash)
}

/// Raw sum of the 32 hash bytes — the input to the `% 101` step.
fn byte_sum(hash: &BytesN<32>) -> u32 {
    let mut total = 0u32;
    for i in 0u32..32 {
        total = total.saturating_add(hash.get(i).unwrap() as u32);
    }
    total
}

#[test]
fn test_derive_score_boundary_hashes_are_pinned() {
    let env = Env::default();

    // All-zero hash: the byte sum is 0, so the score is 0.
    let all_zero = [0u8; 32];
    assert_eq!(byte_sum(&BytesN::from_array(&env, &all_zero)), 0);
    assert_eq!(score_of(&env, &all_zero), 0);

    // All-0xFF hash: the byte sum is 32 * 255 = 8160, and 8160 % 101 == 80.
    let all_ff = [255u8; 32];
    assert_eq!(byte_sum(&BytesN::from_array(&env, &all_ff)), 8_160);
    assert_eq!(8160u32 % 101, 80);
    assert_eq!(score_of(&env, &all_ff), 80);

    // Single-byte sums pin the identity mapping below the modulus.
    let mut sum_100 = [0u8; 32];
    sum_100[0] = 100;
    assert_eq!(score_of(&env, &sum_100), 100);

    // Sums above the modulus wrap: 255 % 101 == 53 and 202 % 101 == 0.
    let mut sum_255 = [0u8; 32];
    sum_255[0] = 255;
    assert_eq!(score_of(&env, &sum_255), 53);

    let mut sum_202 = [0u8; 32];
    sum_202[0] = 200;
    sum_202[1] = 2;
    assert_eq!(score_of(&env, &sum_202), 0);
}

#[test]
fn test_derive_score_distribution_uniformity_result_with_fixed_seed() {
    const SAMPLES: usize = 20_000;
    const BUCKETS: usize = 5;
    /// Lines per band: `score * 5 / 101` yields bands of 21/20/20/20/20.
    const BAND_SIZES: [f64; BUCKETS] = [21.0, 20.0, 20.0, 20.0, 20.0];

    let env = Env::default();
    let mut rng = Xorshift64::new(0x5EED_1329_0000_0001);

    let mut sum_total: u64 = 0;
    let mut sum_sq_total: f64 = 0.0;
    let mut score_total: u128 = 0;
    let mut bucket_counts = [0u32; BUCKETS];

    for _ in 0..SAMPLES {
        let bytes = rng.next_bytes32();
        let score = score_of(&env, &bytes);
        assert!(score <= 100, "score out of range: {score}");

        let byte_total = byte_sum(&BytesN::from_array(&env, &bytes));
        sum_total += byte_total as u64;
        sum_sq_total += (byte_total as f64) * (byte_total as f64);
        score_total += score as u128;

        let bucket = ((score as usize) * BUCKETS) / 101;
        bucket_counts[bucket] += 1;
    }

    // ---- the raw byte sum is bell-shaped ----------------------------------
    // Theoretical mean 32 * 255 / 2 = 4080, variance
    // 32 * (256^2 - 1) / 12 = 174 760, so sd ~= 418.0.
    let observed_sum_mean = sum_total as f64 / SAMPLES as f64;
    let observed_sum_sd =
        (sum_sq_total / SAMPLES as f64 - observed_sum_mean * observed_sum_mean).sqrt();
    assert!(
        (observed_sum_mean - 4_080.0).abs() < 30.0,
        "byte-sum mean {observed_sum_mean} deviates from the theoretical 4080"
    );
    assert!(
        (380.0..=455.0).contains(&observed_sum_sd),
        "byte-sum sd {observed_sum_sd} deviates from the theoretical ~418"
    );

    // ---- the derived score is (approximately) uniform ---------------------
    // Folding a bell curve whose sd (~418) is more than four times the modulus
    // (101) averages the shape out almost exactly, so a chi-square goodness-of-
    // fit against the uniform hypothesis must NOT reject it.
    let observed_score_mean = score_total as f64 / SAMPLES as f64;
    assert!(
        (observed_score_mean - 50.0).abs() < 2.0,
        "score mean {observed_score_mean} should sit near the uniform mean of 50"
    );

    let chi_square: f64 = bucket_counts
        .iter()
        .enumerate()
        .map(|(bucket, observed)| {
            let expected = SAMPLES as f64 * BAND_SIZES[bucket] / 101.0;
            let delta = *observed as f64 - expected;
            delta * delta / expected
        })
        .sum();

    // df = 4. 21.666 is the 99.9% critical value, so a fixed-seed failure here
    // would mean the derivation really is biased rather than unlucky.
    assert!(
        chi_square < 21.666,
        "chi-square {chi_square} rejects uniformity (buckets {bucket_counts:?})"
    );

    // Every band must actually be reachable — a degenerate mapping funnelling
    // everything into one band would otherwise slip past the check above.
    assert!(
        bucket_counts.iter().all(|count| *count > 0),
        "all five score bands must be reachable: {bucket_counts:?}"
    );
}
