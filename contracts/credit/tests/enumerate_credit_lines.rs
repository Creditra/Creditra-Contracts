// SPDX-License-Identifier: MIT

//! Tests for credit line enumeration with pagination.

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{testutils::Address as _, Address, Env, Vec};

pub struct TestEnv {
    env: Env,
    #[allow(dead_code)]
    admin: Address,
    contract_id: Address,
}

impl TestEnv {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);
        Self {
            env,
            admin,
            contract_id,
        }
    }

    fn client(&self) -> CreditClient<'_> {
        CreditClient::new(&self.env, &self.contract_id)
    }

    fn open_credit_line(&self, borrower: &Address, limit: i128) {
        self.client()
            .open_credit_line(borrower, &limit, &300_u32, &70_u32);
    }
}

#[test]
fn test_enumerate_empty_list() {
    let test_env = TestEnv::new();
    let client = test_env.client();

    let count = client.get_credit_line_count();
    assert_eq!(count, 0);

    let lines = client.enumerate_credit_lines(&None, &10);
    assert_eq!(lines.len(), 0);
}

#[test]
fn test_enumerate_single_credit_line() {
    let test_env = TestEnv::new();
    let borrower = Address::generate(&test_env.env);
    let client = test_env.client();

    test_env.open_credit_line(&borrower, 1000);

    let count = client.get_credit_line_count();
    assert_eq!(count, 1);

    let lines = client.enumerate_credit_lines(&None, &10);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines.get(0).unwrap().0, 0); // ID should be 0
    assert_eq!(lines.get(0).unwrap().1.borrower, borrower);
    assert_eq!(lines.get(0).unwrap().1.credit_limit, 1000);
}

#[test]
fn test_enumerate_multiple_credit_lines() {
    let test_env = TestEnv::new();
    let borrower_a = Address::generate(&test_env.env);
    let borrower_b = Address::generate(&test_env.env);
    let borrower_c = Address::generate(&test_env.env);
    let client = test_env.client();

    test_env.open_credit_line(&borrower_a, 1000);
    test_env.open_credit_line(&borrower_b, 2000);
    test_env.open_credit_line(&borrower_c, 3000);

    let count = client.get_credit_line_count();
    assert_eq!(count, 3);

    let lines = client.enumerate_credit_lines(&None, &10);
    assert_eq!(lines.len(), 3);

    // Verify order (insertion order)
    assert_eq!(lines.get(0).unwrap().0, 0);
    assert_eq!(lines.get(0).unwrap().1.borrower, borrower_a);
    assert_eq!(lines.get(0).unwrap().1.credit_limit, 1000);

    assert_eq!(lines.get(1).unwrap().0, 1);
    assert_eq!(lines.get(1).unwrap().1.borrower, borrower_b);
    assert_eq!(lines.get(1).unwrap().1.credit_limit, 2000);

    assert_eq!(lines.get(2).unwrap().0, 2);
    assert_eq!(lines.get(2).unwrap().1.borrower, borrower_c);
    assert_eq!(lines.get(2).unwrap().1.credit_limit, 3000);
}

#[test]
fn test_enumerate_pagination_first_page() {
    let test_env = TestEnv::new();
    let client = test_env.client();

    // Create 5 credit lines
    let mut borrowers = Vec::new(&test_env.env);
    for _ in 0..5 {
        borrowers.push_back(Address::generate(&test_env.env));
    }

    for i in 0..borrowers.len() {
        let borrower = borrowers.get(i).unwrap();
        test_env.open_credit_line(&borrower, 1000);
    }

    // Get first 2
    let page1 = client.enumerate_credit_lines(&None, &2);
    assert_eq!(page1.len(), 2);
    assert_eq!(page1.get(0).unwrap().0, 0);
    assert_eq!(page1.get(1).unwrap().0, 1);
}

#[test]
fn test_enumerate_pagination_second_page() {
    let test_env = TestEnv::new();
    let client = test_env.client();

    // Create 5 credit lines
    let mut borrowers = Vec::new(&test_env.env);
    for _ in 0..5 {
        borrowers.push_back(Address::generate(&test_env.env));
    }

    for i in 0..borrowers.len() {
        let borrower = borrowers.get(i).unwrap();
        test_env.open_credit_line(&borrower, 1000);
    }

    // Get first page
    let page1 = client.enumerate_credit_lines(&None, &2);
    let last_id = page1.get(1).unwrap().0;

    // Get second page using cursor
    let page2 = client.enumerate_credit_lines(&Some(last_id), &2);
    assert_eq!(page2.len(), 2);
    assert_eq!(page2.get(0).unwrap().0, 2);
    assert_eq!(page2.get(1).unwrap().0, 3);
}

#[test]
fn test_enumerate_pagination_last_page_partial() {
    let test_env = TestEnv::new();
    let client = test_env.client();

    // Create 5 credit lines
    let mut borrowers = Vec::new(&test_env.env);
    for _ in 0..5 {
        borrowers.push_back(Address::generate(&test_env.env));
    }

    for i in 0..borrowers.len() {
        let borrower = borrowers.get(i).unwrap();
        test_env.open_credit_line(&borrower, 1000);
    }

    // Get pages of 2
    let page1 = client.enumerate_credit_lines(&None, &2);
    let page2 = client.enumerate_credit_lines(&Some(1), &2);
    let page3 = client.enumerate_credit_lines(&Some(3), &2);

    assert_eq!(page1.len(), 2);
    assert_eq!(page2.len(), 2);
    assert_eq!(page3.len(), 1); // Only one remaining
    assert_eq!(page3.get(0).unwrap().0, 4);
}

#[test]
fn test_enumerate_limit_capped_at_max() {
    let test_env = TestEnv::new();
    let client = test_env.client();

    // Create 10 credit lines
    for _ in 0..10 {
        let borrower = Address::generate(&test_env.env);
        test_env.open_credit_line(&borrower, 1000);
    }

    // Request more than MAX_ENUMERATION_LIMIT (100)
    // Should be capped
    let lines = client.enumerate_credit_lines(&None, &200);
    assert_eq!(lines.len(), 10); // Only 10 exist, so we get all 10
}

#[test]
fn test_enumerate_deterministic_ordering() {
    let test_env = TestEnv::new();
    let client = test_env.client();

    // Create credit lines in specific order
    let b1 = Address::generate(&test_env.env);
    let b2 = Address::generate(&test_env.env);
    let b3 = Address::generate(&test_env.env);

    test_env.open_credit_line(&b1, 1000);
    test_env.open_credit_line(&b2, 2000);
    test_env.open_credit_line(&b3, 3000);

    // Enumerate multiple times - should always return same order
    let lines1 = client.enumerate_credit_lines(&None, &10);
    let lines2 = client.enumerate_credit_lines(&None, &10);
    let lines3 = client.enumerate_credit_lines(&None, &10);

    assert_eq!(lines1, lines2);
    assert_eq!(lines2, lines3);
}

#[test]
fn test_enumerate_start_after_beyond_end() {
    let test_env = TestEnv::new();
    let client = test_env.client();

    // Create 3 credit lines
    for _ in 0..3 {
        let borrower = Address::generate(&test_env.env);
        test_env.open_credit_line(&borrower, 1000);
    }

    // Start after the last ID
    let lines = client.enumerate_credit_lines(&Some(100), &10);
    assert_eq!(lines.len(), 0);
}

#[test]
fn test_enumerate_public_access() {
    let test_env = TestEnv::new();
    let client = test_env.client();

    // Create a credit line
    let borrower = Address::generate(&test_env.env);
    test_env.open_credit_line(&borrower, 1000);

    // Anyone should be able to enumerate (no auth required for view functions)
    let lines = client.enumerate_credit_lines(&None, &10);
    assert_eq!(lines.len(), 1);

    let count = client.get_credit_line_count();
    assert_eq!(count, 1);
}

#[test]
fn test_enumerate_with_draws_and_repays() {
    let test_env = TestEnv::new();
    let client = test_env.client();

    // Set up token for draws/repays
    let token_id = test_env
        .env
        .register_stellar_asset_contract_v2(Address::generate(&test_env.env));
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);
    soroban_sdk::token::StellarAssetClient::new(&test_env.env, &token_address)
        .mint(&test_env.contract_id, &10000);

    let borrower = Address::generate(&test_env.env);
    test_env.open_credit_line(&borrower, 5000);

    // Draw and repay shouldn't affect enumeration
    client.draw_credit(&borrower, &1000);
    soroban_sdk::token::Client::new(&test_env.env, &token_address).approve(
        &borrower,
        &test_env.contract_id,
        &500_i128,
        &1_000_000_u32,
    );
    client.repay_credit(&borrower, &500);

    let lines = client.enumerate_credit_lines(&None, &10);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines.get(0).unwrap().1.utilized_amount, 500);
}
