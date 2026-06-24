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

    let (addresses, next_cursor) = client.enumerate_credit_lines(&0, &10);
    assert_eq!(addresses.len(), 0);
    assert_eq!(next_cursor, None);
}

#[test]
fn test_enumerate_pagination() {
    let test_env = TestEnv::new();
    let client = test_env.client();

    let mut borrowers = Vec::new(&test_env.env);
    for _ in 0..5 {
        let borrower = Address::generate(&test_env.env);
        borrowers.push_back(borrower.clone());
        test_env.open_credit_line(&borrower, 1000);
    }

    // Page 1: start = 0, limit = 2
    let (addresses, next_cursor) = client.enumerate_credit_lines(&0, &2);
    assert_eq!(addresses.len(), 2);
    assert_eq!(addresses.get(0).unwrap(), borrowers.get(0).unwrap());
    assert_eq!(addresses.get(1).unwrap(), borrowers.get(1).unwrap());
    assert_eq!(next_cursor, Some(2));

    // Page 2: start = 2, limit = 2
    let (addresses2, next_cursor2) = client.enumerate_credit_lines(&2, &2);
    assert_eq!(addresses2.len(), 2);
    assert_eq!(addresses2.get(0).unwrap(), borrowers.get(2).unwrap());
    assert_eq!(addresses2.get(1).unwrap(), borrowers.get(3).unwrap());
    assert_eq!(next_cursor2, Some(4));

    // Page 3: start = 4, limit = 2
    let (addresses3, next_cursor3) = client.enumerate_credit_lines(&4, &2);
    assert_eq!(addresses3.len(), 1);
    assert_eq!(addresses3.get(0).unwrap(), borrowers.get(4).unwrap());
    assert_eq!(next_cursor3, None);
}

#[test]
fn test_enumerate_limit_capped() {
    let test_env = TestEnv::new();
    let client = test_env.client();

    let mut borrowers = Vec::new(&test_env.env);
    for _ in 0..60 {
        let borrower = Address::generate(&test_env.env);
        borrowers.push_back(borrower.clone());
        test_env.open_credit_line(&borrower, 1000);
    }

    let (addresses, next_cursor) = client.enumerate_credit_lines(&0, &100);
    assert_eq!(addresses.len(), 50); // Clamped at hard cap of 50
    assert_eq!(next_cursor, Some(50));
}

#[test]
fn test_enumerate_out_of_bounds() {
    let test_env = TestEnv::new();
    let client = test_env.client();

    for _ in 0..3 {
        let borrower = Address::generate(&test_env.env);
        test_env.open_credit_line(&borrower, 1000);
    }

    let (addresses, next_cursor) = client.enumerate_credit_lines(&5, &10);
    assert_eq!(addresses.len(), 0);
    assert_eq!(next_cursor, None);
}
