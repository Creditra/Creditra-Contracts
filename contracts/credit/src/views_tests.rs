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
fn test_credit_lines_pagination_first_page() {
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

    // Open 5 credit lines
    for i in 0..5 {
        let borrower = Address::generate(&env);
        client.open_credit_line(&borrower, &(1000 + i as i128), &500, &10 + i);
    }

    // Get first page with limit 2
    let page1 = client.get_credit_lines_paginated(None, &2);
    assert_eq!(page1.lines.len(), 2);
    assert!(page1.next_cursor.is_some());
    assert!(page1.has_more);
}

#[test]
fn test_credit_lines_pagination_multiple_pages() {
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

    // Open 5 credit lines
    for i in 0..5 {
        let borrower = Address::generate(&env);
        client.open_credit_line(&borrower, &(1000 + i as i128), &500, &10 + i);
    }

    // Get first page with limit 2
    let page1 = client.get_credit_lines_paginated(None, &2);
    assert_eq!(page1.lines.len(), 2);
    assert!(page1.next_cursor.is_some());
    assert!(page1.has_more);

    // Get second page using cursor
    let cursor = page1.next_cursor.unwrap();
    let page2 = client.get_credit_lines_paginated(Some(cursor), &2);
    assert_eq!(page2.lines.len(), 2);
    assert!(page2.next_cursor.is_some());
    assert!(page2.has_more);

    // Get third page using cursor
    let cursor = page2.next_cursor.unwrap();
    let page3 = client.get_credit_lines_paginated(Some(cursor), &2);
    assert_eq!(page3.lines.len(), 1);
    assert!(page3.next_cursor.is_none());
    assert!(!page3.has_more);
}

#[test]
fn test_credit_lines_pagination_empty_result() {
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

    // Get first page with no credit lines
    let page = client.get_credit_lines_paginated(None, &10);
    assert_eq!(page.lines.len(), 0);
    assert!(page.next_cursor.is_none());
    assert!(!page.has_more);
}

#[test]
fn test_credit_lines_pagination_limit_enforcement() {
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

    // Open 5 credit lines
    for i in 0..5 {
        let borrower = Address::generate(&env);
        client.open_credit_line(&borrower, &(1000 + i as i128), &500, &10 + i);
    }

    // Request limit larger than MAX_ENUMERATION_LIMIT should panic
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.get_credit_lines_paginated(None, &101);
    }));
    assert!(result.is_err());
}

#[test]
fn test_credit_lines_pagination_cursor_beyond_end() {
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

    // Open 3 credit lines
    for i in 0..3 {
        let borrower = Address::generate(&env);
        client.open_credit_line(&borrower, &(1000 + i as i128), &500, &10 + i);
    }

    // Request page with cursor beyond the last credit line
    let page = client.get_credit_lines_paginated(Some(100), &10);
    assert_eq!(page.lines.len(), 0);
    assert!(page.next_cursor.is_none());
    assert!(!page.has_more);
}
