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
