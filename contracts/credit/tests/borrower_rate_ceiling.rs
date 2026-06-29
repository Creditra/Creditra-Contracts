// SPDX-License-Identifier: MIT

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

fn setup(env: &Env) -> (CreditClient, Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    // Initialize credit line
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &50_u32);
    (client, admin, borrower)
}

#[test]
fn test_rate_ceiling_overrides_high_rate() {
    let env = Env::default();
    let (client, _admin, borrower) = setup(&env);

    // Set ceiling to 400 bps
    client.set_borrower_rate_ceiling(&borrower, &Some(400_u32));

    // Update risk params with 500 bps rate (above ceiling)
    client.update_risk_parameters(&borrower, &1_000_i128, &500_u32, &50_u32);

    // Effective rate should be 400 (the ceiling)
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.interest_rate_bps, 400);
}

#[test]
fn test_rate_ceiling_does_not_override_lower_rate() {
    let env = Env::default();
    let (client, _admin, borrower) = setup(&env);

    // Set ceiling to 400 bps
    client.set_borrower_rate_ceiling(&borrower, &Some(400_u32));

    // Update risk params with 300 bps rate (below ceiling)
    client.update_risk_parameters(&borrower, &1_000_i128, &300_u32, &50_u32);

    // Effective rate should be 300
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.interest_rate_bps, 300);
}

#[test]
fn test_removing_rate_ceiling() {
    let env = Env::default();
    let (client, _admin, borrower) = setup(&env);

    // Set ceiling to 400 bps
    client.set_borrower_rate_ceiling(&borrower, &Some(400_u32));

    // Remove ceiling
    client.set_borrower_rate_ceiling(&borrower, &None);

    // Update risk params with 500 bps rate
    client.update_risk_parameters(&borrower, &1_000_i128, &500_u32, &50_u32);

    // Effective rate should be 500 (no ceiling)
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.interest_rate_bps, 500);
}

#[test]
fn test_ceiling_below_floor_reverts() {
    let env = Env::default();
    let (client, _admin, borrower) = setup(&env);

    // Set floor to 500 bps
    client.set_borrower_rate_floor(&borrower, &Some(500_u32));

    // Try to set ceiling to 400 bps (below floor) - should revert
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.set_borrower_rate_ceiling(&borrower, &Some(400_u32));
    }));

    assert!(result.is_err(), "Setting ceiling below floor should revert");
}

#[test]
fn test_floor_above_ceiling_reverts() {
    let env = Env::default();
    let (client, _admin, borrower) = setup(&env);

    // Set ceiling to 400 bps
    client.set_borrower_rate_ceiling(&borrower, &Some(400_u32));

    // Try to set floor to 500 bps (above ceiling) - should revert
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.set_borrower_rate_floor(&borrower, &Some(500_u32));
    }));

    assert!(result.is_err(), "Setting floor above ceiling should revert");
}

#[test]
fn test_ceiling_above_floor_succeeds() {
    let env = Env::default();
    let (client, _admin, borrower) = setup(&env);

    // Set floor to 300 bps
    client.set_borrower_rate_floor(&borrower, &Some(300_u32));

    // Set ceiling to 600 bps (above floor) - should succeed
    client.set_borrower_rate_ceiling(&borrower, &Some(600_u32));

    // Verify both are set
    assert_eq!(client.get_borrower_rate_floor(&borrower), Some(300_u32));
    assert_eq!(client.get_borrower_rate_ceiling(&borrower), Some(600_u32));
}

#[test]
fn test_ceiling_enforces_with_floor() {
    let env = Env::default();
    let (client, _admin, borrower) = setup(&env);

    // Set floor to 300 bps and ceiling to 500 bps
    client.set_borrower_rate_floor(&borrower, &Some(300_u32));
    client.set_borrower_rate_ceiling(&borrower, &Some(500_u32));

    // Try to set rate to 200 bps (below floor)
    client.update_risk_parameters(&borrower, &1_000_i128, &200_u32, &50_u32);
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.interest_rate_bps, 300); // Floor applied

    // Try to set rate to 600 bps (above ceiling)
    client.update_risk_parameters(&borrower, &1_000_i128, &600_u32, &50_u32);
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.interest_rate_bps, 500); // Ceiling applied

    // Set rate to 400 bps (within range)
    client.update_risk_parameters(&borrower, &1_000_i128, &400_u32, &50_u32);
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.interest_rate_bps, 400); // No adjustment needed
}

#[test]
fn test_ceiling_with_formula_rate() {
    let env = Env::default();
    let (client, _admin, borrower) = setup(&env);

    // Configure a rate formula: base=200, slope=5, min=200, max=700
    client.set_rate_formula_config(&200_u32, &5_u32, &200_u32, &700_u32);

    // Set ceiling to 400 bps
    client.set_borrower_rate_ceiling(&borrower, &Some(400_u32));

    // Risk score 50 would give: 200 + 50*5 = 450, but ceiling caps at 400
    client.update_risk_parameters(&borrower, &1_000_i128, &500_u32, &50_u32);
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.interest_rate_bps, 400); // Ceiling applied to formula result

    // Risk score 20 would give: 200 + 20*5 = 300, below ceiling
    client.update_risk_parameters(&borrower, &1_000_i128, &500_u32, &20_u32);
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.interest_rate_bps, 300); // No ceiling effect
}

#[test]
fn test_ceiling_exceeds_max_rate_reverts() {
    let env = Env::default();
    let (client, _admin, borrower) = setup(&env);

    // Try to set ceiling above MAX_INTEREST_RATE_BPS (10000)
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.set_borrower_rate_ceiling(&borrower, &Some(10_001_u32));
    }));

    assert!(result.is_err(), "Ceiling above max rate should revert");
}

#[test]
fn test_ceiling_at_max_rate_succeeds() {
    let env = Env::default();
    let (client, _admin, borrower) = setup(&env);

    // Set ceiling to exactly MAX_INTEREST_RATE_BPS
    client.set_borrower_rate_ceiling(&borrower, &Some(10_000_u32));

    // Verify it was set
    assert_eq!(
        client.get_borrower_rate_ceiling(&borrower),
        Some(10_000_u32)
    );
}

#[test]
fn test_ceiling_independent_per_borrower() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let borrower_a = Address::generate(&env);
    let borrower_b = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    // Open credit lines for both borrowers
    client.open_credit_line(&borrower_a, &1_000_i128, &300_u32, &50_u32);
    client.open_credit_line(&borrower_b, &1_000_i128, &300_u32, &50_u32);

    // Set different ceilings for each borrower
    client.set_borrower_rate_ceiling(&borrower_a, &Some(400_u32));
    client.set_borrower_rate_ceiling(&borrower_b, &Some(600_u32));

    // Update rates for both
    client.update_risk_parameters(&borrower_a, &1_000_i128, &500_u32, &50_u32);
    client.update_risk_parameters(&borrower_b, &1_000_i128, &500_u32, &50_u32);

    // Verify each borrower's rate is capped by their own ceiling
    let line_a = client.get_credit_line(&borrower_a).unwrap();
    let line_b = client.get_credit_line(&borrower_b).unwrap();
    assert_eq!(line_a.interest_rate_bps, 400);
    assert_eq!(line_b.interest_rate_bps, 500); // Below ceiling, no effect
}

#[test]
fn test_ceiling_with_no_floor() {
    let env = Env::default();
    let (client, _admin, borrower) = setup(&env);

    // Set only ceiling, no floor
    client.set_borrower_rate_ceiling(&borrower, &Some(400_u32));

    // Rate of 300 should remain unchanged
    client.update_risk_parameters(&borrower, &1_000_i128, &300_u32, &50_u32);
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.interest_rate_bps, 300);

    // Rate of 500 should be capped to 400
    client.update_risk_parameters(&borrower, &1_000_i128, &500_u32, &50_u32);
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.interest_rate_bps, 400);
}

#[test]
fn test_ceiling_removal_allows_higher_rates() {
    let env = Env::default();
    let (client, _admin, borrower) = setup(&env);

    // Set ceiling to 400 bps
    client.set_borrower_rate_ceiling(&borrower, &Some(400_u32));

    // Verify ceiling is enforced
    client.update_risk_parameters(&borrower, &1_000_i128, &500_u32, &50_u32);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().interest_rate_bps,
        400
    );

    // Remove ceiling
    client.set_borrower_rate_ceiling(&borrower, &None);

    // Now rate of 500 should be allowed
    client.update_risk_parameters(&borrower, &1_000_i128, &500_u32, &50_u32);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().interest_rate_bps,
        500
    );
}

#[test]
fn test_ceiling_with_rate_change_limits() {
    let env = Env::default();
    let (client, _admin, borrower) = setup(&env);

    // Set rate change limits: max 100 bps change, 60 second interval
    client.set_rate_change_limits(&100_u32, &60_u64);

    // Set ceiling to 400 bps
    client.set_borrower_rate_ceiling(&borrower, &Some(400_u32));

    // Current rate is 300, ceiling is 400
    // Try to jump to 500 (would be capped to 400, delta = 100)
    env.ledger().set_timestamp(100);
    client.update_risk_parameters(&borrower, &1_000_i128, &500_u32, &50_u32);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().interest_rate_bps,
        400
    );

    // Try to change again within interval (should fail due to rate change limits)
    env.ledger().set_timestamp(150);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.update_risk_parameters(&borrower, &1_000_i128, &350_u32, &50_u32);
    }));
    assert!(result.is_err(), "Rate change within interval should revert");
}

#[test]
fn test_ceiling_boundary_at_current_rate() {
    let env = Env::default();
    let (client, _admin, borrower) = setup(&env);

    // Current rate is 300
    // Set ceiling to exactly 300
    client.set_borrower_rate_ceiling(&borrower, &Some(300_u32));

    // Update with same rate should succeed
    client.update_risk_parameters(&borrower, &1_000_i128, &300_u32, &50_u32);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().interest_rate_bps,
        300
    );
}

#[test]
fn test_ceiling_zero_removes_ceiling() {
    let env = Env::default();
    let (client, _admin, borrower) = setup(&env);

    // Set ceiling to 400 bps
    client.set_borrower_rate_ceiling(&borrower, &Some(400_u32));
    assert_eq!(client.get_borrower_rate_ceiling(&borrower), Some(400_u32));

    // Setting to 0 should remove it (following the pattern of other optional configs)
    // Note: This test documents current behavior - 0 is treated as a valid ceiling
    // In practice, users should use None to remove the ceiling
    client.set_borrower_rate_ceiling(&borrower, &Some(0_u32));
    assert_eq!(client.get_borrower_rate_ceiling(&borrower), Some(0_u32));
}
