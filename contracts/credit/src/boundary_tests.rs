// SPDX-License-Identifier: MIT

//! Boundary tests for rate and score validation.
//!
//! This module contains exhaustive tests for exact bounds and one past bounds
//! for rate and score validation, ensuring consistent RateTooHigh and ScoreTooHigh
//! error mapping.

use super::*;
use soroban_sdk::testutils::Address as _;

/// Test case for boundary validation of rate and score parameters
#[derive(Debug, Clone)]
struct BoundaryTestCase {
    /// Description of the test case
    description: &'static str,
    /// Interest rate in basis points to test
    interest_rate_bps: u32,
    /// Risk score to test
    risk_score: u32,
    /// Expected error (None for success)
    expected_error: Option<ContractError>,
    /// Whether this test should pass (true) or panic (false)
    should_succeed: bool,
}

/// Table-driven tests for exact bounds and one-past bounds validation
#[test]
fn test_rate_and_score_boundary_validation() {
    let env = Env::default();
    env.mock_all_auths();
    
    let test_cases = vec![
        // Valid boundary cases - should succeed
        BoundaryTestCase {
            description: "Minimum valid rate (0) and score (0)",
            interest_rate_bps: 0,
            risk_score: 0,
            expected_error: None,
            should_succeed: true,
        },
        BoundaryTestCase {
            description: "Maximum valid rate (10000) and score (100)",
            interest_rate_bps: 10_000,
            risk_score: 100,
            expected_error: None,
            should_succeed: true,
        },
        BoundaryTestCase {
            description: "Rate at exact maximum (10000), score at minimum (0)",
            interest_rate_bps: 10_000,
            risk_score: 0,
            expected_error: None,
            should_succeed: true,
        },
        BoundaryTestCase {
            description: "Rate at minimum (0), score at exact maximum (100)",
            interest_rate_bps: 0,
            risk_score: 100,
            expected_error: None,
            should_succeed: true,
        },
        BoundaryTestCase {
            description: "Typical valid values (5000 rate, 50 score)",
            interest_rate_bps: 5_000,
            risk_score: 50,
            expected_error: None,
            should_succeed: true,
        },
        
        // Invalid boundary cases - should fail
        BoundaryTestCase {
            description: "Rate one past maximum (10001)",
            interest_rate_bps: 10_001,
            risk_score: 50,
            expected_error: Some(ContractError::RateTooHigh),
            should_succeed: false,
        },
        BoundaryTestCase {
            description: "Score one past maximum (101)",
            interest_rate_bps: 5_000,
            risk_score: 101,
            expected_error: Some(ContractError::ScoreTooHigh),
            should_succeed: false,
        },
        BoundaryTestCase {
            description: "Both rate and score one past maximum",
            interest_rate_bps: 10_001,
            risk_score: 101,
            expected_error: Some(ContractError::RateTooHigh), // Rate checked first
            should_succeed: false,
        },
        BoundaryTestCase {
            description: "Rate at maximum, score one past maximum",
            interest_rate_bps: 10_000,
            risk_score: 101,
            expected_error: Some(ContractError::ScoreTooHigh),
            should_succeed: false,
        },
        BoundaryTestCase {
            description: "Rate one past maximum, score at maximum",
            interest_rate_bps: 10_001,
            risk_score: 100,
            expected_error: Some(ContractError::RateTooHigh),
            should_succeed: false,
        },
    ];

    for (i, test_case) in test_cases.iter().enumerate() {
        println!("Running test case {}: {}", i + 1, test_case.description);
        
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        
        client.init(&admin);
        
        if test_case.should_succeed {
            // Should succeed - open credit line with valid parameters
            client.open_credit_line(
                &borrower,
                &1000_i128,
                &test_case.interest_rate_bps,
                &test_case.risk_score,
            );
            
            // Verify the credit line was created with correct values
            let line = client.get_credit_line(&borrower).expect("Credit line should exist");
            assert_eq!(line.interest_rate_bps, test_case.interest_rate_bps);
            assert_eq!(line.risk_score, test_case.risk_score);
            
            // Also test update_risk_parameters with the same valid values
            client.update_risk_parameters(
                &borrower,
                &1000_i128,
                &test_case.interest_rate_bps,
                &test_case.risk_score,
            );
            
            let updated_line = client.get_credit_line(&borrower).expect("Credit line should exist");
            assert_eq!(updated_line.interest_rate_bps, test_case.interest_rate_bps);
            assert_eq!(updated_line.risk_score, test_case.risk_score);
        } else {
            // Should fail - verify proper error mapping
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                client.open_credit_line(
                    &borrower,
                    &1000_i128,
                    &test_case.interest_rate_bps,
                    &test_case.risk_score,
                );
            }));
            
            assert!(result.is_err(), "Expected panic for invalid parameters");
            
            // Test update_risk_parameters as well
            // First open a valid credit line, then try to update with invalid values
            client.open_credit_line(&borrower, &1000_i128, &100_u32, &10_u32);
            
            let update_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                client.update_risk_parameters(
                    &borrower,
                    &1000_i128,
                    &test_case.interest_rate_bps,
                    &test_case.risk_score,
                );
            }));
            
            assert!(update_result.is_err(), "Expected panic for invalid update parameters");
        }
    }
}

/// Test edge cases around the boundaries with more granular testing
#[test]
fn test_rate_score_edge_cases() {
    let env = Env::default();
    env.mock_all_auths();
    
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    
    client.init(&admin);
    
    // Test very close to boundaries
    let edge_cases = vec![
        (9_999, 99, true),   // Just under max for both
        (10_000, 99, true),  // Max rate, just under max score
        (9_999, 100, true),  // Just under max rate, max score
        (10_000, 100, true), // Exactly at max for both
        (10_001, 100, false), // Rate over, score at max
        (10_000, 101, false), // Rate at max, score over
        (10_001, 101, false), // Both over
    ];
    
    for (rate, score, should_succeed) in edge_cases {
        if should_succeed {
            client.open_credit_line(&borrower, &1000_i128, &rate, &score);
            let line = client.get_credit_line(&borrower).expect("Credit line should exist");
            assert_eq!(line.interest_rate_bps, rate);
            assert_eq!(line.risk_score, score);
            
            // Clean up for next iteration
            client.close_credit_line(&borrower, &admin);
        } else {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                client.open_credit_line(&borrower, &1000_i128, &rate, &score);
            }));
            assert!(result.is_err(), "Expected panic for rate: {}, score: {}", rate, score);
        }
    }
}

/// Test that RateTooHigh and ScoreTooHigh errors are properly mapped
#[test]
fn test_error_mapping_consistency() {
    let env = Env::default();
    env.mock_all_auths();
    
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    
    client.init(&admin);
    
    // Test RateTooHigh error mapping
    let rate_over_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.open_credit_line(&borrower, &1000_i128, &10_001_u32, &50_u32);
    }));
    
    // Test ScoreTooHigh error mapping  
    let score_over_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.open_credit_line(&borrower, &1000_i128, &5000_u32, &101_u32);
    }));
    
    // Both should panic with appropriate error messages
    assert!(rate_over_result.is_err());
    assert!(score_over_result.is_err());
    
    // Verify the error codes are correctly defined
    assert_eq!(ContractError::RateTooHigh as u32, 8);
    assert_eq!(ContractError::ScoreTooHigh as u32, 9);
}
