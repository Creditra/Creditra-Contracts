#[cfg(test)]
use super::*;
use soroban_sdk::testutils::Address as _;

#[test]
fn test_init_and_open_credit_line() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);
    client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);

    // Verify credit line was created
    let credit_line = client.get_credit_line(&borrower);
    assert!(credit_line.is_some());
    let credit_line = credit_line.unwrap();
    assert_eq!(credit_line.borrower, borrower);
    assert_eq!(credit_line.credit_limit, 1000);
    assert_eq!(credit_line.utilized_amount, 0);
    assert_eq!(credit_line.interest_rate_bps, 300);
    assert_eq!(credit_line.risk_score, 70);
    assert_eq!(credit_line.status, CreditStatus::Active);
}

#[test]
fn test_suspend_credit_line() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);
    client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
    client.suspend_credit_line(&borrower);

    // Verify status changed to Suspended
    let credit_line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(credit_line.status, CreditStatus::Suspended);
}

#[test]
fn test_close_credit_line() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);
    client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
    client.close_credit_line(&borrower);

    // Verify status changed to Closed
    let credit_line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(credit_line.status, CreditStatus::Closed);
}

#[test]
fn test_default_credit_line() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);
    client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
    client.default_credit_line(&borrower);

    // Verify status changed to Defaulted
    let credit_line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(credit_line.status, CreditStatus::Defaulted);
}

#[test]
fn test_full_lifecycle() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);

    // Open credit line
    client.open_credit_line(&borrower, &5000_i128, &500_u32, &80_u32);
    let credit_line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(credit_line.status, CreditStatus::Active);

    // Suspend credit line
    client.suspend_credit_line(&borrower);
    let credit_line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(credit_line.status, CreditStatus::Suspended);

    // Close credit line
    client.close_credit_line(&borrower);
    let credit_line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(credit_line.status, CreditStatus::Closed);
}

#[test]
fn test_event_data_integrity() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);
    client.open_credit_line(&borrower, &2000_i128, &400_u32, &75_u32);

    // Verify credit line data matches what was passed
    let credit_line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(credit_line.borrower, borrower);
    assert_eq!(credit_line.status, CreditStatus::Active);
    assert_eq!(credit_line.credit_limit, 2000);
    assert_eq!(credit_line.interest_rate_bps, 400);
    assert_eq!(credit_line.risk_score, 75);
}

#[test]
#[should_panic(expected = "Credit line not found")]
fn test_suspend_nonexistent_credit_line() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);
    client.suspend_credit_line(&borrower);
}

#[test]
#[should_panic(expected = "Credit line not found")]
fn test_close_nonexistent_credit_line() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);
    client.close_credit_line(&borrower);
}

#[test]
#[should_panic(expected = "Credit line not found")]
fn test_default_nonexistent_credit_line() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);
    client.default_credit_line(&borrower);
}

#[test]
fn test_multiple_borrowers() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let borrower1 = Address::generate(&env);
    let borrower2 = Address::generate(&env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);
    client.open_credit_line(&borrower1, &1000_i128, &300_u32, &70_u32);
    client.open_credit_line(&borrower2, &2000_i128, &400_u32, &80_u32);

    let credit_line1 = client.get_credit_line(&borrower1).unwrap();
    let credit_line2 = client.get_credit_line(&borrower2).unwrap();

    assert_eq!(credit_line1.credit_limit, 1000);
    assert_eq!(credit_line2.credit_limit, 2000);
    assert_eq!(credit_line1.status, CreditStatus::Active);
    assert_eq!(credit_line2.status, CreditStatus::Active);
}

#[test]
fn test_lifecycle_transitions() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);

    // Test Active -> Defaulted
    client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Active
    );

    client.default_credit_line(&borrower);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Defaulted
    );
}

#[test]
fn test_open_credit_line_success() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);
    client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);

    let credit_line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(credit_line.borrower, borrower);
    assert_eq!(credit_line.credit_limit, 1000);
    assert_eq!(credit_line.utilized_amount, 0);
    assert_eq!(credit_line.interest_rate_bps, 300);
    assert_eq!(credit_line.risk_score, 70);
    assert_eq!(credit_line.status, CreditStatus::Active);
}

#[test]
fn test_open_credit_line_utilized_amount_starts_at_zero() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);
    client.open_credit_line(&borrower, &9999_i128, &500_u32, &50_u32);

    let credit_line = client.get_credit_line(&borrower).unwrap();
    // utilized_amount must always start at 0 regardless of credit_limit
    assert_eq!(credit_line.utilized_amount, 0);
}

#[test]
fn test_open_credit_line_boundary_interest_rate() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);
    // interest_rate_bps = 10000 (100%) is the max allowed
    client.open_credit_line(&borrower, &1000_i128, &10_000_u32, &50_u32);

    let credit_line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(credit_line.interest_rate_bps, 10_000);
}

#[test]
fn test_open_credit_line_boundary_risk_score() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);
    // risk_score = 100 is the max allowed
    client.open_credit_line(&borrower, &1000_i128, &300_u32, &100_u32);

    let credit_line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(credit_line.risk_score, 100);
}

#[test]
fn test_open_credit_line_minimum_credit_limit() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);
    // credit_limit = 1 is the minimum allowed
    client.open_credit_line(&borrower, &1_i128, &300_u32, &50_u32);

    let credit_line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(credit_line.credit_limit, 1);
    assert_eq!(credit_line.status, CreditStatus::Active);
}

#[test]
#[should_panic(expected = "credit_limit must be greater than zero")]
fn test_open_credit_line_rejects_zero_credit_limit() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);
    // credit_limit = 0 must be rejected
    client.open_credit_line(&borrower, &0_i128, &300_u32, &50_u32);
}

#[test]
#[should_panic(expected = "credit_limit must be greater than zero")]
fn test_open_credit_line_rejects_negative_credit_limit() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);
    // negative credit_limit must be rejected
    client.open_credit_line(&borrower, &-1_i128, &300_u32, &50_u32);
}

#[test]
#[should_panic(expected = "interest_rate_bps cannot exceed 10000 (100%)")]
fn test_open_credit_line_rejects_interest_rate_above_max() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);
    // interest_rate_bps = 10001 exceeds the 10000 cap
    client.open_credit_line(&borrower, &1000_i128, &10_001_u32, &50_u32);
}

#[test]
#[should_panic(expected = "risk_score must be between 0 and 100")]
fn test_open_credit_line_rejects_risk_score_above_max() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);
    // risk_score = 101 exceeds the 100 cap
    client.open_credit_line(&borrower, &1000_i128, &300_u32, &101_u32);
}

#[test]
#[should_panic(expected = "borrower already has an active credit line")]
fn test_open_credit_line_rejects_duplicate_active_borrower() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);
    client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
    // second call for same borrower while Active must panic
    client.open_credit_line(&borrower, &2000_i128, &400_u32, &60_u32);
}

#[test]
fn test_open_credit_line_allowed_after_closed() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);
    client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
    client.close_credit_line(&borrower);

    // re-opening after Closed is allowed
    client.open_credit_line(&borrower, &2000_i128, &400_u32, &60_u32);

    let credit_line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(credit_line.credit_limit, 2000);
    assert_eq!(credit_line.status, CreditStatus::Active);
}

#[test]
fn test_open_credit_line_allowed_after_defaulted() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);
    client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
    client.default_credit_line(&borrower);

    // re-opening after Defaulted is allowed (e.g. borrower rehabilitated)
    client.open_credit_line(&borrower, &500_i128, &800_u32, &30_u32);

    let credit_line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(credit_line.credit_limit, 500);
    assert_eq!(credit_line.status, CreditStatus::Active);
}

#[test]
fn test_open_credit_line_allowed_after_suspended() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);
    client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
    client.suspend_credit_line(&borrower);

    // re-opening after Suspended is allowed (admin lifted suspension via new line)
    client.open_credit_line(&borrower, &1500_i128, &350_u32, &65_u32);

    let credit_line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(credit_line.credit_limit, 1500);
    assert_eq!(credit_line.status, CreditStatus::Active);
}

#[test]
fn test_open_credit_line_multiple_independent_borrowers() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let borrower_a = Address::generate(&env);
    let borrower_b = Address::generate(&env);
    let borrower_c = Address::generate(&env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);
    client.open_credit_line(&borrower_a, &1000_i128, &300_u32, &70_u32);
    client.open_credit_line(&borrower_b, &2000_i128, &400_u32, &80_u32);
    client.open_credit_line(&borrower_c, &3000_i128, &500_u32, &90_u32);

    // Each borrower has its own independent storage slot
    assert_eq!(
        client.get_credit_line(&borrower_a).unwrap().credit_limit,
        1000
    );
    assert_eq!(
        client.get_credit_line(&borrower_b).unwrap().credit_limit,
        2000
    );
    assert_eq!(
        client.get_credit_line(&borrower_c).unwrap().credit_limit,
        3000
    );
}

#[test]
fn test_get_credit_line_returns_none_for_unknown_borrower() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let unknown = Address::generate(&env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);
    // No credit line opened for this address
    assert!(client.get_credit_line(&unknown).is_none());
}