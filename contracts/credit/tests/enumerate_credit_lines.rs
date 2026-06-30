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

    let (lines, next_cursor) = client.enumerate_credit_lines(&None, &10, &false);
    assert_eq!(lines.len(), 0);
    assert_eq!(next_cursor, None);
}

#[test]
fn test_enumerate_single_credit_line() {
    let test_env = TestEnv::new();
    let borrower = Address::generate(&test_env.env);
    let client = test_env.client();

    test_env.open_credit_line(&borrower, 1000);

    let count = client.get_credit_line_count();
    assert_eq!(count, 1);

    let (lines, next_cursor) = client.enumerate_credit_lines(&None, &10, &false);
    assert_eq!(lines.len(), 1);
    assert_eq!(next_cursor, None); // single line: page not full, no more pages
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

    let (lines, next_cursor) = client.enumerate_credit_lines(&None, &10, &false);
    assert_eq!(lines.len(), 3);
    assert_eq!(next_cursor, None); // 3 < limit (10): final page

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

    // Get first 2: the page is full, so the cursor should point to id 1.
    let (page1, next_cursor1) = client.enumerate_credit_lines(&None, &2, &false);
    assert_eq!(page1.len(), 2);
    assert_eq!(page1.get(0).unwrap().0, 0);
    assert_eq!(page1.get(1).unwrap().0, 1);
    assert_eq!(next_cursor1, Some(1));
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

    // Get first page; pass the returned cursor back to fetch the second page.
    let (page1, cursor1) = client.enumerate_credit_lines(&None, &2, &false);
    assert_eq!(cursor1, Some(1));

    let (page2, cursor2) = client.enumerate_credit_lines(&cursor1, &2, &false);
    assert_eq!(page2.len(), 2);
    assert_eq!(page2.get(0).unwrap().0, 2);
    assert_eq!(page2.get(1).unwrap().0, 3);
    assert_eq!(cursor2, Some(3));
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

    // Page through with size 2: ids 0,1 -> 2,3 -> 4.
    let (page1, cursor1) = client.enumerate_credit_lines(&None, &2, &false);
    let (page2, cursor2) = client.enumerate_credit_lines(&cursor1, &2, &false);
    let (page3, cursor3) = client.enumerate_credit_lines(&cursor2, &2, &false);

    assert_eq!(page1.len(), 2);
    assert_eq!(cursor1, Some(1));

    assert_eq!(page2.len(), 2);
    assert_eq!(cursor2, Some(3));

    assert_eq!(page3.len(), 1); // Only one remaining
    assert_eq!(page3.get(0).unwrap().0, 4);
    assert_eq!(cursor3, None); // Partial last page: no more pages
}

#[test]
fn test_enumerate_pagination_cursor_none_signals_end() {
    let test_env = TestEnv::new();
    let client = test_env.client();

    // Create 3 credit lines.
    for _ in 0..3 {
        let borrower = Address::generate(&test_env.env);
        test_env.open_credit_line(&borrower, 1000);
    }

    // Single-shot fetch where limit exceeds count: must return None cursor.
    let (_page, cursor) = client.enumerate_credit_lines(&None, &10, &false);
    assert_eq!(cursor, None);
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

    // Request more than MAX_ENUMERATION_LIMIT (100).
    // Should be capped by the contract (100), but only 10 records exist
    // so the result hits 10 records and the cursor is None.
    let (lines, next_cursor) = client.enumerate_credit_lines(&None, &200, &false);
    assert_eq!(lines.len(), 10);
    assert_eq!(next_cursor, None);
}

#[test]
fn test_enumerate_max_enumeration_limit_is_exactly_100() {
    let test_env = TestEnv::new();
    let client = test_env.client();

    // Create 105 credit lines so we cross the MAX_ENUMERATION_LIMIT = 100 cap.
    let mut borrowers = Vec::new(&test_env.env);
    for _ in 0..105 {
        borrowers.push_back(Address::generate(&test_env.env));
    }
    for i in 0..borrowers.len() {
        test_env.open_credit_line(&borrowers.get(i).unwrap(), 1000);
    }

    // Bug guard: request more than the cap; the result MUST be exactly 100 with
    // next_cursor pointing at id 99, NOT the full 105 set (which would indicate
    // that the per-row `returned` increment is missing and the cap is dead).
    let (page1, cursor1) = client.enumerate_credit_lines(&None, &200, &false);
    assert_eq!(page1.len(), 100, "cap must be exactly MAX_ENUMERATION_LIMIT = 100");
    assert_eq!(page1.get(0).unwrap().0, 0);
    assert_eq!(page1.get(99).unwrap().0, 99);
    assert_eq!(
        cursor1,
        Some(99),
        "after a full page the cursor points at the last returned id"
    );

    // Page 2 should return the remaining 5 borrowers (ids 100..=104) with None cursor.
    let (page2, cursor2) = client.enumerate_credit_lines(&cursor1, &200, &false);
    assert_eq!(page2.len(), 5);
    assert_eq!(page2.get(0).unwrap().0, 100);
    assert_eq!(page2.get(4).unwrap().0, 104);
    assert_eq!(cursor2, None);
}

#[test]
fn test_enumerate_limit_zero_returns_empty() {
    let test_env = TestEnv::new();
    let client = test_env.client();

    for _ in 0..3 {
        let borrower = Address::generate(&test_env.env);
        test_env.open_credit_line(&borrower, 1000);
    }

    // limit = 0 must short-circuit: empty Vec, None cursor.
    let (lines, next_cursor) = client.enumerate_credit_lines(&None, &0, &false);
    assert_eq!(lines.len(), 0);
    assert_eq!(next_cursor, None);
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
    let (lines1, _) = client.enumerate_credit_lines(&None, &10, &false);
    let (lines2, _) = client.enumerate_credit_lines(&None, &10, &false);
    let (lines3, _) = client.enumerate_credit_lines(&None, &10, &false);

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

    // Start after the last ID -> empty Vec, None cursor (end of data).
    let (lines, next_cursor) = client.enumerate_credit_lines(&Some(100), &10, &false);
    assert_eq!(lines.len(), 0);
    assert_eq!(next_cursor, None);
}

#[test]
fn test_enumerate_public_access() {
    let test_env = TestEnv::new();
    let client = test_env.client();

    // Create a credit line
    let borrower = Address::generate(&test_env.env);
    test_env.open_credit_line(&borrower, 1000);

    // Anyone should be able to enumerate (no auth required for view functions)
    let (lines, _) = client.enumerate_credit_lines(&None, &10, &false);
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

    let (lines, _) = client.enumerate_credit_lines(&None, &10, &false);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines.get(0).unwrap().1.utilized_amount, 500);
}

#[test]
fn test_enumerate_skip_closed_excludes_closed_lines() {
    let test_env = TestEnv::new();
    let client = test_env.client();
    let admin = Address::generate(&test_env.env);

    let b_active_a = Address::generate(&test_env.env);
    let b_close = Address::generate(&test_env.env);
    let b_active_b = Address::generate(&test_env.env);
    let b_suspended = Address::generate(&test_env.env);

    test_env.open_credit_line(&b_active_a, 1_000);
    test_env.open_credit_line(&b_close, 1_000);
    test_env.open_credit_line(&b_active_b, 1_000);
    test_env.open_credit_line(&b_suspended, 1_000);

    // Close one line and suspend another; both should still be present
    // when skip_closed == false.
    client.close_credit_line(&b_close, &admin);
    client.suspend_credit_line(&b_suspended);

    let (lines_all, _) = client.enumerate_credit_lines(&None, &10, &false);
    assert_eq!(
        lines_all.len(),
        4,
        "skip_closed=false must yield every indexed line"
    );

    // With skip_closed=true, only the Closed line is omitted; Suspended remains.
    let (lines_open, cursor_open) = client.enumerate_credit_lines(&None, &10, &true);
    assert_eq!(
        lines_open.len(),
        3,
        "skip_closed=true must drop only the Closed line"
    );
    let borrowers_seen: std::vec::Vec<Address> = (0..lines_open.len())
        .map(|i| lines_open.get(i).unwrap().1.borrower.clone())
        .collect();
    assert!(borrowers_seen.contains(&b_active_a));
    assert!(borrowers_seen.contains(&b_active_b));
    assert!(borrowers_seen.contains(&b_suspended));
    assert!(!borrowers_seen.contains(&b_close));
    assert_eq!(cursor_open, None);
}

#[test]
fn test_enumerate_skip_closed_pages_through_filtered_set() {
    let test_env = TestEnv::new();
    let client = test_env.client();
    let admin = Address::generate(&test_env.env);

    // Create 5 borrowers; close ids 1 and 3 (so the filter would skip 2 rows).
    let mut borrowers = Vec::new(&test_env.env);
    for _ in 0..5 {
        borrowers.push_back(Address::generate(&test_env.env));
    }
    for i in 0..borrowers.len() {
        test_env.open_credit_line(&borrowers.get(i).unwrap(), 1_000);
    }
    client.close_credit_line(&borrowers.get(1).unwrap(), &admin);
    client.close_credit_line(&borrowers.get(3).unwrap(), &admin);

    // Page with size 2 and skip_closed=true: expect ids 0,2 -> 4.
    let (page1, cursor1) = client.enumerate_credit_lines(&None, &2, &true);
    let (page2, cursor2) = client.enumerate_credit_lines(&cursor1, &2, &true);

    assert_eq!(page1.len(), 2);
    assert_eq!(page1.get(0).unwrap().0, 0);
    assert_eq!(page1.get(1).unwrap().0, 2);
    assert_eq!(cursor1, Some(2));

    assert_eq!(page2.len(), 1);
    assert_eq!(page2.get(0).unwrap().0, 4);
    assert_eq!(cursor2, None);
}

#[test]
fn test_enumerate_skip_closed_all_closed_yields_empty_and_none_cursor() {
    let test_env = TestEnv::new();
    let client = test_env.client();
    let admin = Address::generate(&test_env.env);

    for _ in 0..3 {
        let borrower = Address::generate(&test_env.env);
        test_env.open_credit_line(&borrower, 1_000);
        client.close_credit_line(&borrower, &admin);
    }

    let (lines, cursor) = client.enumerate_credit_lines(&None, &10, &true);
    assert_eq!(lines.len(), 0);
    assert_eq!(
        cursor, None,
        "Empty result with skip_closed=true must signal end of data"
    );
}
