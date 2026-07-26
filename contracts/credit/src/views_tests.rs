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
    let page = client.get_credit_lines_paginated(&None, &10);
    assert_eq!(page.lines.len(), 0);
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
    let page = client.get_credit_lines_paginated(&None, &10);
    assert_eq!(page.lines.len(), 3);
    assert!(page.next_cursor.is_none());

    // Verify credit line data
    assert_eq!(page.lines.get(0).unwrap().credit_limit, 1000);
    assert_eq!(page.lines.get(1).unwrap().credit_limit, 2000);
    assert_eq!(page.lines.get(2).unwrap().credit_limit, 3000);
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
    let page1 = client.get_credit_lines_paginated(&None, &2);
    assert_eq!(page1.lines.len(), 2);
    assert!(page1.next_cursor.is_some());

    // Second page with 2 items
    let cursor1 = page1.next_cursor.unwrap();
    let page2 = client.get_credit_lines_paginated(&Some(cursor1), &2);
    assert_eq!(page2.lines.len(), 2);
    assert!(page2.next_cursor.is_some());

    // Third page with 1 item (last page)
    let cursor2 = page2.next_cursor.unwrap();
    let page3 = client.get_credit_lines_paginated(&Some(cursor2), &2);
    assert_eq!(page3.lines.len(), 1);
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
    let page = client.get_credit_lines_paginated(&None, &3);
    assert_eq!(page.lines.len(), 3);
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
        client.get_credit_lines_paginated(&None, &101);
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
    let page = client.get_credit_lines_paginated(&Some(100), &10);
    assert_eq!(page.lines.len(), 0);
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
    let page = client.get_credit_lines_paginated(&None, &10);
    assert_eq!(page.lines.len(), 3);
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
    let page1 = client.get_credit_lines_paginated(&None, &2);
    assert_eq!(page1.lines.len(), 2);
    let cursor1 = page1.next_cursor.unwrap();

    // Second page: continue from cursor
    let page2 = client.get_credit_lines_paginated(&Some(cursor1), &2);
    assert_eq!(page2.lines.len(), 2);
    assert!(page2.next_cursor.is_none());

    // Verify we got all 4 distinct lines
    let all_limits: Vec<i128> = page1
        .lines
        .iter()
        .chain(page2.lines.iter())
        .map(|line| line.credit_limit)
        .collect();
    assert_eq!(all_limits.len(), 4);
    assert!(all_limits.contains(&1000));
    assert!(all_limits.contains(&2000));
    assert!(all_limits.contains(&3000));
    assert!(all_limits.contains(&4000));
}

// ── Borrow capabilities tests ─────────────────────────────────────────────────

fn setup_caps_test(env: &Env) -> (CreditClient<'_>, Address, Address) {
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1000);

    let admin = Address::generate(env);
    let contract_id = env.register_contract(None, Credit);
    let client = CreditClient::new(env, &contract_id);

    let token = Address::generate(env);
    let source = Address::generate(env);
    client.init(&admin);
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&source);

    let borrower = Address::generate(env);
    client.open_credit_line(&borrower, &10_000, &500, &50);

    (client, admin, borrower)
}

/// No credit line exists → all capabilities are false.
#[test]
fn borrow_caps_no_line() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1000);

    let admin = Address::generate(&env);
    let contract_id = env.register_contract(None, Credit);
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let stranger = Address::generate(&env);
    let caps = client.borrow_capabilities(&stranger);
    assert!(!caps.can_draw, "no line → cannot draw");
    assert!(!caps.can_repay, "no line → cannot repay");
    assert!(!caps.can_self_suspend, "no line → cannot self-suspend");
}

/// Active credit line with no restrictions → all capabilities true.
#[test]
fn borrow_caps_active_line() {
    let env = Env::default();
    let (client, _admin, borrower) = setup_caps_test(&env);

    let caps = client.borrow_capabilities(&borrower);
    assert!(caps.can_draw, "active line → can draw");
    assert!(caps.can_repay, "active line → can repay");
    assert!(caps.can_self_suspend, "active line → can self-suspend");
}

/// Suspended credit line → draw and self-suspend blocked, repay still allowed.
#[test]
fn borrow_caps_suspended() {
    let env = Env::default();
    let (client, _admin, borrower) = setup_caps_test(&env);

    client.suspend_credit_line(&borrower);

    let caps = client.borrow_capabilities(&borrower);
    assert!(!caps.can_draw, "suspended → cannot draw");
    assert!(caps.can_repay, "suspended → can repay");
    assert!(!caps.can_self_suspend, "suspended → cannot self-suspend");
}

/// Defaulted credit line → draw and self-suspend blocked, repay allowed.
#[test]
fn borrow_caps_defaulted() {
    let env = Env::default();
    let (client, _admin, borrower) = setup_caps_test(&env);

    client.default_credit_line(&borrower);

    let caps = client.borrow_capabilities(&borrower);
    assert!(!caps.can_draw, "defaulted → cannot draw");
    assert!(caps.can_repay, "defaulted → can repay");
    assert!(!caps.can_self_suspend, "defaulted → cannot self-suspend");
}

/// Closed credit line → all capabilities false.
#[test]
fn borrow_caps_closed() {
    let env = Env::default();
    let (client, admin, borrower) = setup_caps_test(&env);

    client.close_credit_line(&borrower, &admin);

    let caps = client.borrow_capabilities(&borrower);
    assert!(!caps.can_draw, "closed → cannot draw");
    assert!(!caps.can_repay, "closed → cannot repay");
    assert!(!caps.can_self_suspend, "closed → cannot self-suspend");
}

/// Self-suspended credit line → draws blocked, repay allowed, self-suspend blocked.
#[test]
fn borrow_caps_self_suspended() {
    let env = Env::default();
    let (client, _admin, borrower) = setup_caps_test(&env);

    client.self_suspend_credit_line(&borrower);

    let caps = client.borrow_capabilities(&borrower);
    assert!(!caps.can_draw, "self-suspended → cannot draw");
    assert!(caps.can_repay, "self-suspended → can repay");
    assert!(!caps.can_self_suspend, "already suspended → cannot self-suspend again");
}

/// Protocol paused → draws blocked, repay and self-suspend still allowed.
#[test]
fn borrow_caps_protocol_paused() {
    let env = Env::default();
    let (client, _admin, borrower) = setup_caps_test(&env);

    client.set_protocol_paused(&true);

    let caps = client.borrow_capabilities(&borrower);
    assert!(!caps.can_draw, "paused → cannot draw");
    assert!(caps.can_repay, "paused → can repay");
    assert!(caps.can_self_suspend, "paused → can self-suspend");
}

/// Global draws frozen → draws blocked, repay and self-suspend still allowed.
#[test]
fn borrow_caps_global_draws_frozen() {
    let env = Env::default();
    let (client, _admin, borrower) = setup_caps_test(&env);

    client.freeze_draws();

    let caps = client.borrow_capabilities(&borrower);
    assert!(!caps.can_draw, "draws frozen → cannot draw");
    assert!(caps.can_repay, "draws frozen → can repay");
    assert!(caps.can_self_suspend, "draws frozen → can self-suspend");
}

/// Borrower blocked → draws blocked, repay and self-suspend still allowed.
#[test]
fn borrow_caps_borrower_blocked() {
    let env = Env::default();
    let (client, admin, borrower) = setup_caps_test(&env);

    client.block_borrower(&admin, &borrower);

    let caps = client.borrow_capabilities(&borrower);
    assert!(!caps.can_draw, "blocked → cannot draw");
    assert!(caps.can_repay, "blocked → can repay");
    assert!(caps.can_self_suspend, "blocked → can self-suspend");
}

/// Borrower temporarily frozen → draws blocked, repay and self-suspend still allowed.
#[test]
fn borrow_caps_borrower_frozen() {
    let env = Env::default();
    let (client, admin, borrower) = setup_caps_test(&env);

    // Freeze until far in the future
    client.freeze_borrower_until(&admin, &borrower, &1_000_000);

    let caps = client.borrow_capabilities(&borrower);
    assert!(!caps.can_draw, "frozen → cannot draw");
    assert!(caps.can_repay, "frozen → can repay");
    assert!(caps.can_self_suspend, "frozen → can self-suspend");
}

/// Credit line admin-frozen → draws blocked, repay and self-suspend still allowed.
#[test]
fn borrow_caps_credit_line_frozen() {
    let env = Env::default();
    let (client, _admin, borrower) = setup_caps_test(&env);

    client.freeze_credit_line(&borrower, &crate::FreezeReason::Compliance);

    let caps = client.borrow_capabilities(&borrower);
    assert!(!caps.can_draw, "line frozen → cannot draw");
    assert!(caps.can_repay, "line frozen → can repay");
    assert!(caps.can_self_suspend, "line frozen → can self-suspend");
}

/// Temporary freeze expiry → draws re-enabled.
#[test]
fn borrow_caps_freeze_expires() {
    let env = Env::default();
    let (client, admin, borrower) = setup_caps_test(&env);

    // Freeze until timestamp 2000
    client.freeze_borrower_until(&admin, &borrower, &2000);

    // Still frozen at t=1500
    env.ledger().with_mut(|li| li.timestamp = 1500);
    let caps = client.borrow_capabilities(&borrower);
    assert!(!caps.can_draw, "before expiry → cannot draw");

    // Expired at t=2000
    env.ledger().with_mut(|li| li.timestamp = 2000);
    let caps = client.borrow_capabilities(&borrower);
    assert!(caps.can_draw, "at expiry → can draw");

    // Well past expiry at t=3000
    env.ledger().with_mut(|li| li.timestamp = 3000);
    let caps = client.borrow_capabilities(&borrower);
    assert!(caps.can_draw, "after expiry → can draw");
}

/// Multiple blocking conditions: the most restrictive one applies.
#[test]
fn borrow_caps_multiple_blocks() {
    let env = Env::default();
    let (client, admin, borrower) = setup_caps_test(&env);

    // Paused + blocked + frozen = still just can_draw false
    client.set_protocol_paused(&true);
    client.block_borrower(&admin, &borrower);

    let caps = client.borrow_capabilities(&borrower);
    assert!(!caps.can_draw, "multiple blocks → cannot draw");
    assert!(caps.can_repay, "multiple blocks → still can repay");
    assert!(caps.can_self_suspend, "multiple blocks → still can self-suspend");
}

/// After unblocking, draws are restored.
#[test]
fn borrow_caps_unblock_restores_draws() {
    let env = Env::default();
    let (client, admin, borrower) = setup_caps_test(&env);

    client.block_borrower(&admin, &borrower);
    let caps = client.borrow_capabilities(&borrower);
    assert!(!caps.can_draw, "blocked → cannot draw");

    client.unblock_borrower(&admin, &borrower);
    let caps = client.borrow_capabilities(&borrower);
    assert!(caps.can_draw, "unblocked → can draw again");
}
