#![cfg(test)]

use crate::{Credit, CreditClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env,
};

#[test]
fn test_protocol_summary_view_active_lines() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1000);

    let admin = Address::generate(&env);
    let contract_id = env.register_contract(None, Credit);
    let client = CreditClient::new(&env, &contract_id);

    // Initialize with dummy token/source
    let token = Address::generate(&env);
    let source = Address::generate(&env);
    client.init(&admin);
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&source);

    // Initial summary
    let summary = client.get_protocol_summary_view();
    assert_eq!(summary.active_line_count, 0);

    let b1 = Address::generate(&env);
    let b2 = Address::generate(&env);
    let b3 = Address::generate(&env);

    // Open b1 -> count 1
    client.open_credit_line(&b1, &1000, &500, &10);
    assert_eq!(client.get_protocol_summary_view().active_line_count, 1);

    // Open b2 -> count 2
    client.open_credit_line(&b2, &1000, &500, &10);
    assert_eq!(client.get_protocol_summary_view().active_line_count, 2);

    // Open b3 -> count 3
    client.open_credit_line(&b3, &1000, &500, &10);
    assert_eq!(client.get_protocol_summary_view().active_line_count, 3);

    // Suspend b2 -> count 2
    client.suspend_credit_line(&b2);
    assert_eq!(client.get_protocol_summary_view().active_line_count, 2);

    // Default b1 -> count 1
    client.default_credit_line(&b1);
    assert_eq!(client.get_protocol_summary_view().active_line_count, 1);

    // Close b3 -> count 0
    client.close_credit_line(&b3, &admin);
    assert_eq!(client.get_protocol_summary_view().active_line_count, 0);
}

#[test]
fn test_proof_of_reserve_empty() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1000);

    let admin = Address::generate(&env);
    let contract_id = env.register_contract(None, Credit);
    let client = CreditClient::new(&env, &contract_id);

    let token = Address::generate(&env);
    let source = Address::generate(&env);
    client.init(&admin);
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&source);

    let por = client.get_proof_of_reserve();
    assert_eq!(por.treasury_balance, 0);
    assert_eq!(por.bounty_balance, 0);
}

#[test]
fn test_proof_of_reserve_reads_existing_balances() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1000);

    let admin = Address::generate(&env);
    let contract_id = env.register_contract(None, Credit);
    let client = CreditClient::new(&env, &contract_id);

    let token = Address::generate(&env);
    let source = Address::generate(&env);
    client.init(&admin);
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&source);

    // Set balances directly via storage
    env.as_contract(&contract_id, || {
        env.storage()
            .instance()
            .set(&crate::storage::DataKey::TreasuryBalance, &42_i128);
        env.storage()
            .instance()
            .set(&crate::storage::DataKey::BountyBalance, &7_i128);
    });

    let por = client.get_proof_of_reserve();
    assert_eq!(por.treasury_balance, 42);
    assert_eq!(por.bounty_balance, 7);
}

#[test]
fn test_credit_lines_paginated_empty() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1000);

    let admin = Address::generate(&env);
    let contract_id = env.register_contract(None, Credit);
    let client = CreditClient::new(&env, &contract_id);

    let token = Address::generate(&env);
    let source = Address::generate(&env);
    client.init(&admin);
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&source);

    // Empty result with no credit lines
    let page = client.get_credit_lines_paginated(None, &10);
    assert_eq!(page.credit_lines.len(), 0);
    assert!(page.next_cursor.is_none());
}

#[test]
fn test_credit_lines_paginated_single_page() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1000);

    let admin = Address::generate(&env);
    let contract_id = env.register_contract(None, Credit);
    let client = CreditClient::new(&env, &contract_id);

    let token = Address::generate(&env);
    let source = Address::generate(&env);
    client.init(&admin);
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&source);

    // Create 3 credit lines
    let b1 = Address::generate(&env);
    let b2 = Address::generate(&env);
    let b3 = Address::generate(&env);

    client.open_credit_line(&b1, &1000, &500, &10);
    client.open_credit_line(&b2, &2000, &600, &20);
    client.open_credit_line(&b3, &3000, &700, &30);

    // Request all 3 in one page
    let page = client.get_credit_lines_paginated(None, &10);
    assert_eq!(page.credit_lines.len(), 3);
    assert!(page.next_cursor.is_none());

    // Verify credit line data
    assert_eq!(page.credit_lines.get(0).unwrap().credit_limit, 1000);
    assert_eq!(page.credit_lines.get(1).unwrap().credit_limit, 2000);
    assert_eq!(page.credit_lines.get(2).unwrap().credit_limit, 3000);
}

#[test]
fn test_credit_lines_paginated_multiple_pages() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1000);

    let admin = Address::generate(&env);
    let contract_id = env.register_contract(None, Credit);
    let client = CreditClient::new(&env, &contract_id);

    let token = Address::generate(&env);
    let source = Address::generate(&env);
    client.init(&admin);
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&source);

    // Create 5 credit lines
    let borrowers: Vec<Address> = (0..5).map(|_| Address::generate(&env)).collect();
    for (i, borrower) in borrowers.iter().enumerate() {
        client.open_credit_line(borrower, &(1000 * (i as i128 + 1)), &500, &(10 * (i as u32 + 1)));
    }

    // First page with 2 items
    let page1 = client.get_credit_lines_paginated(None, &2);
    assert_eq!(page1.credit_lines.len(), 2);
    assert!(page1.next_cursor.is_some());

    // Second page with 2 items
    let cursor1 = page1.next_cursor.unwrap();
    let page2 = client.get_credit_lines_paginated(Some(cursor1), &2);
    assert_eq!(page2.credit_lines.len(), 2);
    assert!(page2.next_cursor.is_some());

    // Third page with 1 item (last page)
    let cursor2 = page2.next_cursor.unwrap();
    let page3 = client.get_credit_lines_paginated(Some(cursor2), &2);
    assert_eq!(page3.credit_lines.len(), 1);
    assert!(page3.next_cursor.is_none());
}

#[test]
fn test_credit_lines_paginated_limit_enforcement() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1000);

    let admin = Address::generate(&env);
    let contract_id = env.register_contract(None, Credit);
    let client = CreditClient::new(&env, &contract_id);

    let token = Address::generate(&env);
    let source = Address::generate(&env);
    client.init(&admin);
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&source);

    // Create 5 credit lines
    let borrowers: Vec<Address> = (0..5).map(|_| Address::generate(&env)).collect();
    for (i, borrower) in borrowers.iter().enumerate() {
        client.open_credit_line(borrower, &(1000 * (i as i128 + 1)), &500, &(10 * (i as u32 + 1)));
    }

    // Request exactly 3 items
    let page = client.get_credit_lines_paginated(None, &3);
    assert_eq!(page.credit_lines.len(), 3);
    assert!(page.next_cursor.is_some());
}

#[test]
fn test_credit_lines_paginated_limit_exceeds_max() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1000);

    let admin = Address::generate(&env);
    let contract_id = env.register_contract(None, Credit);
    let client = CreditClient::new(&env, &contract_id);

    let token = Address::generate(&env);
    let source = Address::generate(&env);
    client.init(&admin);
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&source);

    // Request limit > MAX_ENUMERATION_LIMIT (100) should panic
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.get_credit_lines_paginated(None, &101);
    }));
    assert!(result.is_err());
}

#[test]
fn test_credit_lines_paginated_cursor_beyond_end() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1000);

    let admin = Address::generate(&env);
    let contract_id = env.register_contract(None, Credit);
    let client = CreditClient::new(&env, &contract_id);

    let token = Address::generate(&env);
    let source = Address::generate(&env);
    client.init(&admin);
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&source);

    // Create 2 credit lines
    let b1 = Address::generate(&env);
    let b2 = Address::generate(&env);
    client.open_credit_line(&b1, &1000, &500, &10);
    client.open_credit_line(&b2, &2000, &600, &20);

    // Request with cursor beyond the last ID
    let page = client.get_credit_lines_paginated(Some(100), &10);
    assert_eq!(page.credit_lines.len(), 0);
    assert!(page.next_cursor.is_none());
}

#[test]
fn test_credit_lines_paginated_with_closed_lines() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1000);

    let admin = Address::generate(&env);
    let contract_id = env.register_contract(None, Credit);
    let client = CreditClient::new(&env, &contract_id);

    let token = Address::generate(&env);
    let source = Address::generate(&env);
    client.init(&admin);
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&source);

    // Create 3 credit lines
    let b1 = Address::generate(&env);
    let b2 = Address::generate(&env);
    let b3 = Address::generate(&env);

    client.open_credit_line(&b1, &1000, &500, &10);
    client.open_credit_line(&b2, &2000, &600, &20);
    client.open_credit_line(&b3, &3000, &700, &30);

    // Close one line
    client.close_credit_line(&b2, &admin);

    // Pagination should still return all lines (including closed)
    let page = client.get_credit_lines_paginated(None, &10);
    assert_eq!(page.credit_lines.len(), 3);
}

#[test]
fn test_credit_lines_paginated_cursor_continuation() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1000);

    let admin = Address::generate(&env);
    let contract_id = env.register_contract(None, Credit);
    let client = CreditClient::new(&env, &contract_id);

    let token = Address::generate(&env);
    let source = Address::generate(&env);
    client.init(&admin);
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&source);

    // Create 4 credit lines with distinct limits for identification
    let b1 = Address::generate(&env);
    let b2 = Address::generate(&env);
    let b3 = Address::generate(&env);
    let b4 = Address::generate(&env);

    client.open_credit_line(&b1, &1000, &500, &10);
    client.open_credit_line(&b2, &2000, &600, &20);
    client.open_credit_line(&b3, &3000, &700, &30);
    client.open_credit_line(&b4, &4000, &800, &40);

    // First page: 2 items
    let page1 = client.get_credit_lines_paginated(None, &2);
    assert_eq!(page1.credit_lines.len(), 2);
    let cursor1 = page1.next_cursor.unwrap();

    // Second page: continue from cursor
    let page2 = client.get_credit_lines_paginated(Some(cursor1), &2);
    assert_eq!(page2.credit_lines.len(), 2);
    assert!(page2.next_cursor.is_none());

    // Verify we got all 4 distinct lines
    let all_limits: Vec<i128> = page1
        .credit_lines
        .iter()
        .chain(page2.credit_lines.iter())
        .map(|line| line.credit_limit)
        .collect();
    assert_eq!(all_limits.len(), 4);
    assert!(all_limits.contains(&1000));
    assert!(all_limits.contains(&2000));
    assert!(all_limits.contains(&3000));
    assert!(all_limits.contains(&4000));
}
