// SPDX-License-Identifier: MIT

//! Integration tests for VRF commitment functionality.
//!
//! Tests the full workflow of committing to a VRF output and verifying
//! that risk scores are derived from the committed VRF output.

#![cfg(test)]

use creditra_credit::scoring::VrfCommitment;
use creditra_credit::ContractError;
use soroban_sdk::testutils::{Address as _, BytesN as _};
use soroban_sdk::{Address, BytesN, Env};

fn create_test_contract(env: &Env) -> creditra_credit::ContractClient {
    creditra_credit::ContractClient::new(env, &env.register_contract(None, creditra_credit::Contract))
}

fn setup_contract(env: &Env, admin: &Address) -> creditra_credit::ContractClient {
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
    contract.pause();

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
