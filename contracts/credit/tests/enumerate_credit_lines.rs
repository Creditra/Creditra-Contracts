// SPDX-License-Identifier: MIT

//! Tests for cursor-based pagination via `enumerate_credit_lines`.
//!
//! Acceptance criteria:
//! - Cursor advances correctly
//! - Limit is capped at `MAX_ENUMERATION_LIMIT` (100)
//! - End-of-data returns `None` cursor
//! - `skip_closed` filters out closed lines

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{testutils::Address as _, Address, Env};

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

    let (lines, next_cursor) = client.enumerate_credit_lines(&0, &10, &false);
    assert_eq!(lines.len(), 0);
    assert!(next_cursor.is_none());
}

#[test]
fn test_enumerate_single_credit_line() {
    let test_env = TestEnv::new();
    let borrower = Address::generate(&test_env.env);
    let client = test_env.client();

    test_env.open_credit_line(&borrower, 1000);

    let (lines, next_cursor) = client.enumerate_credit_lines(&0, &10, &false);
    assert_eq!(lines.len(), 1);
    assert!(next_cursor.is_none());

    let line = lines.get(0).unwrap();
    assert_eq!(line.credit_limit, 1000);
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

    let (lines, next_cursor) = client.enumerate_credit_lines(&0, &10, &false);
    assert_eq!(lines.len(), 3);
    assert!(next_cursor.is_none());

    // Verify order (insertion order)
    assert_eq!(lines.get(0).unwrap().borrower, borrower_a);
    assert_eq!(lines.get(0).unwrap().credit_limit, 1000);
    assert_eq!(lines.get(1).unwrap().borrower, borrower_b);
    assert_eq!(lines.get(1).unwrap().credit_limit, 2000);
    assert_eq!(lines.get(2).unwrap().borrower, borrower_c);
    assert_eq!(lines.get(2).unwrap().credit_limit, 3000);
}

#[test]
fn test_enumerate_pagination_multiple_pages() {
    let test_env = TestEnv::new();
    let client = test_env.client();

    // Create 5 credit lines
    for _ in 0..5 {
        let borrower = Address::generate(&test_env.env);
        test_env.open_credit_line(&borrower, 1000);
    }

    // Page 1: cursor=0, limit=2 → expect 2 lines, next_cursor=Some(2)
    let (page1, next1) = client.enumerate_credit_lines(&0, &2, &false);
    assert_eq!(page1.len(), 2);
    assert_eq!(next1, Some(2));

    // Page 2: cursor=2, limit=2 → expect 2 lines, next_cursor=Some(4)
    let (page2, next2) = client.enumerate_credit_lines(&next1.unwrap(), &2, &false);
    assert_eq!(page2.len(), 2);
    assert_eq!(next2, Some(4));

    // Page 3: cursor=4, limit=2 → expect 1 line, next_cursor=None
    let (page3, next3) = client.enumerate_credit_lines(&next2.unwrap(), &2, &false);
    assert_eq!(page3.len(), 1);
    assert_eq!(next3, None);
}

#[test]
fn test_enumerate_limit_capped_at_max() {
    let test_env = TestEnv::new();
    let client = test_env.client();

    // Create 150 credit lines
    for _ in 0..150 {
        let borrower = Address::generate(&test_env.env);
        test_env.open_credit_line(&borrower, 1000);
    }

    // Request limit > MAX_ENUMERATION_LIMIT (100) → capped to 100
    let (lines, next_cursor) = client.enumerate_credit_lines(&0, &200, &false);
    assert_eq!(lines.len(), 100);
    assert!(next_cursor.is_some());
}

#[test]
fn test_enumerate_deterministic_ordering() {
    let test_env = TestEnv::new();
    let client = test_env.client();

    let b1 = Address::generate(&test_env.env);
    let b2 = Address::generate(&test_env.env);
    let b3 = Address::generate(&test_env.env);

    test_env.open_credit_line(&b1, 1000);
    test_env.open_credit_line(&b2, 2000);
    test_env.open_credit_line(&b3, 3000);

    // Enumerate multiple times - should always return same order
    let (lines1, _) = client.enumerate_credit_lines(&0, &10, &false);
    let (lines2, _) = client.enumerate_credit_lines(&0, &10, &false);
    let (lines3, _) = client.enumerate_credit_lines(&0, &10, &false);

    assert_eq!(lines1.len(), 3);
    assert_eq!(lines1, lines2);
    assert_eq!(lines2, lines3);
}

#[test]
fn test_enumerate_cursor_beyond_end() {
    let test_env = TestEnv::new();
    let client = test_env.client();

    // Create 3 credit lines
    for _ in 0..3 {
        let borrower = Address::generate(&test_env.env);
        test_env.open_credit_line(&borrower, 1000);
    }

    // Cursor beyond the last id → empty result, no next cursor
    let (lines, next_cursor) = client.enumerate_credit_lines(&100, &10, &false);
    assert_eq!(lines.len(), 0);
    assert!(next_cursor.is_none());
}

#[test]
fn test_enumerate_public_access() {
    let test_env = TestEnv::new();
    let client = test_env.client();

    let borrower = Address::generate(&test_env.env);
    test_env.open_credit_line(&borrower, 1000);

    // Anyone should be able to enumerate (no auth required for view functions)
    let (lines, _) = client.enumerate_credit_lines(&0, &10, &false);
    assert_eq!(lines.len(), 1);
}

#[test]
fn test_enumerate_with_draws_and_repays() {
    let test_env = TestEnv::new();
    let client = test_env.client();

    let token_id = test_env
        .env
        .register_stellar_asset_contract_v2(Address::generate(&test_env.env));
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);
    client.set_liquidity_source(&test_env.contract_id);
    client.set_min_collateral_ratio_bps(&0);
    soroban_sdk::token::StellarAssetClient::new(&test_env.env, &token_address)
        .mint(&test_env.contract_id, &10000);

    let borrower = Address::generate(&test_env.env);
    test_env.open_credit_line(&borrower, 5000);

    client.draw_credit(&borrower, &1000);
    soroban_sdk::token::Client::new(&test_env.env, &token_address).approve(
        &borrower,
        &test_env.contract_id,
        &500_i128,
        &1_000_000_u32,
    );
    client.repay_credit(&borrower, &500);

    let (lines, _) = client.enumerate_credit_lines(&0, &10, &false);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines.get(0).unwrap().utilized_amount, 500);
}

#[test]
fn test_enumerate_skip_closed() {
    let test_env = TestEnv::new();
    let client = test_env.client();
    let admin = test_env.admin.clone();

    let b1 = Address::generate(&test_env.env);
    let b2 = Address::generate(&test_env.env);
    let b3 = Address::generate(&test_env.env);

    test_env.open_credit_line(&b1, 1000);
    test_env.open_credit_line(&b2, 2000);
    test_env.open_credit_line(&b3, 3000);

    // Close b2
    client.close_credit_line(&b2, &admin);

    // Without skip_closed → all 3 lines returned
    let (all_lines, _) = client.enumerate_credit_lines(&0, &10, &false);
    assert_eq!(all_lines.len(), 3);

    // With skip_closed → 2 lines (b2 omitted)
    let (open_lines, _) = client.enumerate_credit_lines(&0, &10, &true);
    assert_eq!(open_lines.len(), 2);
    // Verify b2 is not in the result
    for i in 0..open_lines.len() {
        assert_ne!(open_lines.get(i).unwrap().borrower, b2);
    }
}

#[test]
fn test_enumerate_zero_limit() {
    let test_env = TestEnv::new();
    let client = test_env.client();

    let borrower = Address::generate(&test_env.env);
    test_env.open_credit_line(&borrower, 1000);

    let (lines, next_cursor) = client.enumerate_credit_lines(&0, &0, &false);
    assert_eq!(lines.len(), 0);
    assert!(next_cursor.is_none());
}

#[test]
fn test_enumerate_cursor_exact_end() {
    let test_env = TestEnv::new();
    let client = test_env.client();

    for _ in 0..3 {
        let borrower = Address::generate(&test_env.env);
        test_env.open_credit_line(&borrower, 1000);
    }

    // Cursor at count (= end) → empty
    let (lines, next_cursor) = client.enumerate_credit_lines(&3, &10, &false);
    assert_eq!(lines.len(), 0);
    assert!(next_cursor.is_none());
}

#[test]
fn test_enumerate_cursor_exact_last_item() {
    let test_env = TestEnv::new();
    let client = test_env.client();

    for _ in 0..3 {
        let borrower = Address::generate(&test_env.env);
        test_env.open_credit_line(&borrower, 1000);
    }

    // Cursor at id=2 (last item), limit=1 → returns 1 line, next_cursor=None
    let (lines, next_cursor) = client.enumerate_credit_lines(&2, &1, &false);
    assert_eq!(lines.len(), 1);
    assert!(next_cursor.is_none());
}

#[test]
fn test_enumerate_reaches_end_exactly() {
    let test_env = TestEnv::new();
    let client = test_env.client();

    for _ in 0..4 {
        let borrower = Address::generate(&test_env.env);
        test_env.open_credit_line(&borrower, 1000);
    }

    // Walk exactly to the end with page size 2
    let (page1, next1) = client.enumerate_credit_lines(&0, &2, &false);
    assert_eq!(page1.len(), 2);
    assert_eq!(next1, Some(2));

    let (page2, next2) = client.enumerate_credit_lines(&next1.unwrap(), &2, &false);
    assert_eq!(page2.len(), 2);
    assert_eq!(next2, None);
}

#[test]
fn test_enumerate_skip_closed_all_closed() {
    let test_env = TestEnv::new();
    let client = test_env.client();
    let admin = test_env.admin.clone();

    let b1 = Address::generate(&test_env.env);
    let b2 = Address::generate(&test_env.env);

    test_env.open_credit_line(&b1, 1000);
    test_env.open_credit_line(&b2, 2000);

    client.close_credit_line(&b1, &admin);
    client.close_credit_line(&b2, &admin);

    // All closed, skip_closed=true → empty
    let (lines, next_cursor) = client.enumerate_credit_lines(&0, &10, &true);
    assert_eq!(lines.len(), 0);
    assert!(next_cursor.is_none());
}

#[test]
fn test_enumerate_skip_closed_cursor_advances_past_closed() {
    let test_env = TestEnv::new();
    let client = test_env.client();
    let admin = test_env.admin.clone();

    let b1 = Address::generate(&test_env.env);
    let b2 = Address::generate(&test_env.env);
    let b3 = Address::generate(&test_env.env);

    test_env.open_credit_line(&b1, 1000);
    test_env.open_credit_line(&b2, 2000);
    test_env.open_credit_line(&b3, 3000);

    // Close middle line
    client.close_credit_line(&b2, &admin);

    // skip_closed with limit=2 should skip b2 and return b1, b3
    let (lines, next_cursor) = client.enumerate_credit_lines(&0, &2, &true);
    assert_eq!(lines.len(), 2);
    assert_eq!(lines.get(0).unwrap().borrower, b1);
    assert_eq!(lines.get(1).unwrap().borrower, b3);
    // next_cursor should be past b2 and b3
    assert!(next_cursor.is_none());
}
