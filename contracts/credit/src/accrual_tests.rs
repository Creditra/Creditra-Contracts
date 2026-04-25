// SPDX-License-Identifier: MIT

#[cfg(test)]
mod tests {
    use crate::Credit;
    use crate::CreditClient;
    use soroban_sdk::{
        testutils::{Address as _, Ledger},
        Address, Env,
    };

    fn setup_env() -> (Env, Address, Address, CreditClient<'static>) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);
        (env, admin, borrower, client)
    }

    #[test]
    fn test_accrual_initialization_on_first_touch() {
        let (env, _admin, borrower, client) = setup_env();

        // Open line
        client.open_credit_line(&borrower, &1000, &1000, &50); // 10% rate

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.last_accrual_ts, 0);

        // Advance time
        env.ledger().set_timestamp(100);

        // First touch (draw)
        client.draw_credit(&borrower, &500);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.last_accrual_ts, 100);
        assert_eq!(line.accrued_interest, 0);
        assert_eq!(line.utilized_amount, 500);
    }

    #[test]
    fn test_no_accrual_at_same_timestamp() {
        let (env, _admin, borrower, client) = setup_env();
        client.open_credit_line(&borrower, &1000, &1000, &50);

        env.ledger().set_timestamp(100);
        client.draw_credit(&borrower, &500);

        // Mutate again at same timestamp
        client.update_risk_parameters(&borrower, &1000, &1000, &50);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.last_accrual_ts, 100);
        assert_eq!(line.accrued_interest, 0);
    }

    #[test]
    fn test_positive_accrual() {
        let (env, _admin, borrower, client) = setup_env();
        // 10% annual rate = 1000 bps
        client.open_credit_line(&borrower, &1_000_000, &1000, &50);

        env.ledger().set_timestamp(100);
        client.draw_credit(&borrower, &100_000);

        // SECONDS_PER_YEAR = 31,536,000
        // Accrual after 1 year: 100,000 * 0.10 = 10,000
        env.ledger().set_timestamp(100 + 31_536_000);

        // Trigger accrual via a no-op update
        client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.last_accrual_ts, 100 + 31_536_000);
        assert_eq!(line.accrued_interest, 10_000);
        assert_eq!(line.utilized_amount, 110_000);
    }

    #[test]
    fn test_multi_period_accrual() {
        let (env, _admin, borrower, client) = setup_env();
        client.open_credit_line(&borrower, &1_000_000, &1000, &50);

        env.ledger().set_timestamp(100);
        client.draw_credit(&borrower, &100_000);

        // Accrue for 6 months (approx)
        env.ledger().set_timestamp(100 + 15_768_000);
        client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

        let line1 = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line1.accrued_interest, 5000);

        // Accrue for another 6 months
        env.ledger().set_timestamp(100 + 31_536_000);
        client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

        let line2 = client.get_credit_line(&borrower).unwrap();
        // utilized_amount increased, so interest increases slightly if compounding.
        // BUT our formula uses the CURRENT utilized_amount at the start of accrual.
        // Simple interest model:
        // Period 1: 100,000 * 1000 * 15,768,000 / (10,000 * 31,536,000) = 5,000
        // Utilized becomes 105,000.
        // Period 2: 105,000 * 1000 * 15,768,000 / (10,000 * 31,536,000) = 5,250
        // Total accrued: 5000 + 5250 = 10,250
        assert_eq!(line2.accrued_interest, 10_250);
    }

    #[test]
    fn test_interest_first_repayment() {
        let (env, _admin, borrower, client) = setup_env();
        client.open_credit_line(&borrower, &1_000_000, &1000, &50);

        env.ledger().set_timestamp(100);
        client.draw_credit(&borrower, &100_000);

        // Accrue 10,000
        env.ledger().set_timestamp(100 + 31_536_000);

        // Repay 5,000. This should trigger accrual first, then subtract from 110,000.
        // Accrued interest becomes 10,000.
        // Repay 5,000: accrued_interest becomes 5,000. utilized_amount becomes 105,000.
        client.repay_credit(&borrower, &5000);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.accrued_interest, 5000);
        assert_eq!(line.utilized_amount, 105_000);

        // Repay another 10,000
        // accrued_interest becomes 0. utilized_amount becomes 95,000.
        client.repay_credit(&borrower, &10_000);
        let line2 = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line2.accrued_interest, 0);
        assert_eq!(line2.utilized_amount, 95_000);
    }

    #[test]
    fn test_zero_utilization_no_accrual() {
        let (env, _admin, borrower, client) = setup_env();
        client.open_credit_line(&borrower, &1_000_000, &1000, &50);

        env.ledger().set_timestamp(100);
        client.update_risk_parameters(&borrower, &1_000_000, &1000, &50); // establishes checkpoint

        env.ledger().set_timestamp(100 + 31_536_000);
        client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.accrued_interest, 0);
        assert_eq!(line.utilized_amount, 0);
    }

    #[test]
    fn test_rounding_down() {
        let (env, _admin, borrower, client) = setup_env();
        client.open_credit_line(&borrower, &1_000_000, &1000, &50);

        env.ledger().set_timestamp(100);
        client.draw_credit(&borrower, &100_000);

        // Accrue for 1 second.
        // 100,000 * 1000 * 1 / (10,000 * 31,536,000) = 100,000,000 / 315,360,000,000 = 0.0003...
        // Should floor to 0.
        env.ledger().set_timestamp(101);
        client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.accrued_interest, 0);
        assert_eq!(line.last_accrual_ts, 101);
    }

    #[test]
    fn test_overflow_protection() {
        let (env, _admin, borrower, client) = setup_env();
        // Use very large utilized amount and rate
        client.open_credit_line(&borrower, &i128::MAX, &10000, &100);

        env.ledger().set_timestamp(100);
        client.draw_credit(&borrower, &1_000_000_000_000_000_000_i128); // 1e18

        // Advance time by 100 years
        env.ledger().set_timestamp(100 + 100 * 31_536_000);

        // This should not panic if using i128 correctly
        client.update_risk_parameters(&borrower, &i128::MAX, &10000, &100);

        let line = client.get_credit_line(&borrower).unwrap();
        assert!(line.accrued_interest > 0);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Grace period tests
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod grace_period_tests {
    use crate::types::{GracePeriodConfig, GraceWaiverMode};
    use crate::Credit;
    use crate::CreditClient;
    use soroban_sdk::{testutils::{Address as _, Ledger}, Address, Env};

    /// Helper: deploy contract, init admin, open a credit line, draw, then suspend.
    /// Returns (env, client, contract_id, borrower).
    fn setup_suspended<'a>(
        env: &'a Env,
        credit_limit: i128,
        draw_amount: i128,
        rate_bps: u32,
        suspend_ts: u64,
    ) -> (CreditClient<'a>, Address, Address) {
        env.mock_all_auths();
        let admin = Address::generate(env);
        let borrower = Address::generate(env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(env, &contract_id);
        client.init(&admin);
        client.open_credit_line(&borrower, &credit_limit, &rate_bps, &50_u32);

        // Draw at t=1 (non-zero) to establish a valid accrual checkpoint.
        env.ledger().set_timestamp(1);
        client.draw_credit(&borrower, &draw_amount);

        // Suspend at the given timestamp (must be >= 1).
        let actual_suspend_ts = if suspend_ts < 1 { 1 } else { suspend_ts };
        env.ledger().set_timestamp(actual_suspend_ts);
        client.suspend_credit_line(&borrower);

        (client, contract_id, borrower)
    }

    // ── Disabled by default ───────────────────────────────────────────────────

    /// Without a grace period config, a Suspended line accrues at the full rate.
    #[test]
    fn no_grace_config_suspended_line_accrues_at_full_rate() {
        let env = Env::default();
        // 1000 bps = 10% annual; principal = 100_000
        // Draw at t=1, suspend at t=1. Advance to t=1+31_536_000.
        // Elapsed = 31_536_000 s → interest = 10_000
        let (client, _contract_id, borrower) =
            setup_suspended(&env, 1_000_000, 100_000, 1000, 1);

        env.ledger().set_timestamp(1 + 31_536_000);
        client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.accrued_interest, 10_000);
        assert_eq!(line.utilized_amount, 110_000);
    }

    // ── FullWaiver: zero interest during grace window ─────────────────────────

    /// With FullWaiver, no interest accrues while inside the grace window.
    #[test]
    fn full_waiver_no_interest_inside_grace_window() {
        let env = Env::default();
        // Suspend at t=1; grace window = 1 year.
        let (client, _contract_id, borrower) =
            setup_suspended(&env, 1_000_000, 100_000, 1000, 1);

        client.set_grace_period_config(&31_536_000_u64, &GraceWaiverMode::FullWaiver, &0_u32);

        // Trigger accrual at t = 1 + half a year (inside grace window).
        env.ledger().set_timestamp(1 + 15_768_000);
        client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

        let line = client.get_credit_line(&borrower).unwrap();
        // No interest should have accrued.
        assert_eq!(line.accrued_interest, 0);
        assert_eq!(line.utilized_amount, 100_000);
    }

    /// With FullWaiver, no interest accrues for the entire grace window.
    #[test]
    fn full_waiver_no_interest_at_grace_boundary() {
        let env = Env::default();
        let (client, _contract_id, borrower) =
            setup_suspended(&env, 1_000_000, 100_000, 1000, 1);

        client.set_grace_period_config(&31_536_000_u64, &GraceWaiverMode::FullWaiver, &0_u32);

        // Trigger accrual exactly at the grace boundary (t = suspension_ts + grace_period_seconds).
        // suspension_ts = 1, grace_end = 1 + 31_536_000 = 31_536_001
        // now = 31_536_001 → now <= grace_end → still inside (Case 1).
        env.ledger().set_timestamp(31_536_001);
        client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.accrued_interest, 0);
        assert_eq!(line.utilized_amount, 100_000);
    }

    /// After the grace window expires, full-rate interest resumes.
    #[test]
    fn full_waiver_full_rate_resumes_after_grace_window() {
        let env = Env::default();
        // Suspend at t=1; grace = 1 year. grace_end = 1 + 31_536_000 = 31_536_001.
        let (client, _contract_id, borrower) =
            setup_suspended(&env, 1_000_000, 100_000, 1000, 1);

        client.set_grace_period_config(&31_536_000_u64, &GraceWaiverMode::FullWaiver, &0_u32);

        // Trigger accrual at t = 31_536_001 + 31_536_000 (1 year after grace end).
        // In-grace: 1 to 31_536_001 → 0 interest (FullWaiver).
        // Post-grace: 31_536_001 to 63_072_001 (31_536_000 s) → 10_000 interest.
        env.ledger().set_timestamp(63_072_001);
        client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.accrued_interest, 10_000);
        assert_eq!(line.utilized_amount, 110_000);
    }

    // ── ReducedRate: partial interest during grace window ────────────────────

    /// With ReducedRate, interest accrues at the reduced rate inside the window.
    #[test]
    fn reduced_rate_accrues_at_waiver_rate_inside_window() {
        let env = Env::default();
        // Full rate = 1000 bps (10%); reduced rate = 200 bps (2%).
        // Suspend at t=1; grace = 1 year. grace_end = 31_536_001.
        let (client, _contract_id, borrower) =
            setup_suspended(&env, 1_000_000, 100_000, 1000, 1);

        client.set_grace_period_config(&31_536_000_u64, &GraceWaiverMode::ReducedRate, &200_u32);

        // Trigger accrual at t = 31_536_001 (exactly at grace_end → still inside, Case 1).
        // Elapsed = 31_536_001 - 1 = 31_536_000 s at 200 bps.
        // Interest = 100_000 * 200 * 31_536_000 / (10_000 * 31_536_000) = 2_000
        env.ledger().set_timestamp(31_536_001);
        client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.accrued_interest, 2_000);
        assert_eq!(line.utilized_amount, 102_000);
    }

    // ── Boundary: window straddles grace end ─────────────────────────────────

    /// When the accrual window straddles the grace boundary, the in-grace portion
    /// uses the waiver rate and the post-grace portion uses the full rate.
    #[test]
    fn full_waiver_split_window_straddles_grace_boundary() {
        let env = Env::default();
        // Suspend at t=1; grace = 1 year. grace_end = 31_536_001.
        let (client, _contract_id, borrower) =
            setup_suspended(&env, 1_000_000, 100_000, 1000, 1);

        client.set_grace_period_config(&31_536_000_u64, &GraceWaiverMode::FullWaiver, &0_u32);

        // Trigger accrual at t = 31_536_001 + 15_768_000 (0.5 year after grace end).
        // last_accrual_ts = 1 (set during suspend_credit_line).
        // In-grace: 1 to 31_536_001 (31_536_000 s) → 0 interest (FullWaiver).
        // Post-grace: 31_536_001 to 47_304_001 (15_768_000 s) → 5_000 interest.
        env.ledger().set_timestamp(47_304_001);
        client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.accrued_interest, 5_000);
        assert_eq!(line.utilized_amount, 105_000);
    }

    #[test]
    fn reduced_rate_split_window_straddles_grace_boundary() {
        let env = Env::default();
        // Full rate = 1000 bps; reduced = 200 bps; grace = 1 year.
        // Suspend at t=1; grace_end = 31_536_001.
        let (client, _contract_id, borrower) =
            setup_suspended(&env, 1_000_000, 100_000, 1000, 1);

        client.set_grace_period_config(&31_536_000_u64, &GraceWaiverMode::ReducedRate, &200_u32);

        // Trigger accrual at t = 47_304_001 (1.5 years after t=1).
        // In-grace (1 to 31_536_001 = 31_536_000 s at 200 bps): 2_000
        // Post-grace (31_536_001 to 47_304_001 = 15_768_000 s at 1000 bps): 5_000
        // Total = 7_000
        env.ledger().set_timestamp(47_304_001);
        client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.accrued_interest, 7_000);
        assert_eq!(line.utilized_amount, 107_000);
    }

    // ── Grace period disabled (zero seconds) ─────────────────────────────────

    /// A grace period config with zero seconds is treated as disabled.
    #[test]
    fn zero_grace_period_seconds_disables_waiver() {
        let env = Env::default();
        let (client, _contract_id, borrower) =
            setup_suspended(&env, 1_000_000, 100_000, 1000, 1);

        // Set config but with 0 seconds — effectively disabled.
        client.set_grace_period_config(&0_u64, &GraceWaiverMode::FullWaiver, &0_u32);

        env.ledger().set_timestamp(1 + 31_536_000);
        client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

        let line = client.get_credit_line(&borrower).unwrap();
        // Full rate applies because grace_period_seconds == 0.
        assert_eq!(line.accrued_interest, 10_000);
    }

    // ── Active lines are unaffected ───────────────────────────────────────────

    /// Grace period config has no effect on Active lines.
    #[test]
    fn grace_period_does_not_affect_active_lines() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);
        client.open_credit_line(&borrower, &1_000_000, &1000, &50);

        // Draw at t=1 to establish a valid accrual checkpoint.
        env.ledger().set_timestamp(1);
        client.draw_credit(&borrower, &100_000);

        // Set a full-waiver grace period.
        client.set_grace_period_config(&31_536_000_u64, &GraceWaiverMode::FullWaiver, &0_u32);

        // Advance 1 year — line is still Active, not Suspended.
        env.ledger().set_timestamp(1 + 31_536_000);
        client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

        let line = client.get_credit_line(&borrower).unwrap();
        // Full rate applies because the line is Active.
        assert_eq!(line.accrued_interest, 10_000);
    }

    // ── Suspension timestamp recorded ────────────────────────────────────────

    /// suspension_ts is set when the line is suspended.
    #[test]
    fn suspension_ts_recorded_on_suspend() {
        let env = Env::default();
        let (client, _contract_id, borrower) =
            setup_suspended(&env, 1_000_000, 100_000, 1000, 12_345);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.suspension_ts, 12_345);
        assert_eq!(line.status, crate::types::CreditStatus::Suspended);
    }

    // ── Config round-trip ─────────────────────────────────────────────────────

    /// set_grace_period_config / get_grace_period_config round-trip.
    #[test]
    fn grace_period_config_roundtrip() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);

        assert!(client.get_grace_period_config().is_none());

        client.set_grace_period_config(&86_400_u64, &GraceWaiverMode::ReducedRate, &150_u32);

        let cfg = client.get_grace_period_config().unwrap();
        assert_eq!(cfg.grace_period_seconds, 86_400);
        assert_eq!(cfg.waiver_mode, GraceWaiverMode::ReducedRate);
        assert_eq!(cfg.reduced_rate_bps, 150);
    }

    /// set_grace_period_config requires admin auth.
    #[test]
    #[should_panic]
    fn set_grace_period_config_requires_admin_auth() {
        let env = Env::default();
        // No mock_all_auths — admin check fires.
        let admin = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        env.mock_all_auths();
        client.init(&admin);
        // Drop auths by creating a fresh env without mock_all_auths.
        let env2 = Env::default();
        let client2 = CreditClient::new(&env2, &contract_id);
        client2.set_grace_period_config(&1000_u64, &GraceWaiverMode::FullWaiver, &0_u32);
    }

    /// reduced_rate_bps > 10_000 is rejected.
    #[test]
    #[should_panic(expected = "Error(Contract, #8)")]
    fn set_grace_period_config_rejects_rate_too_high() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);
        client.set_grace_period_config(&1000_u64, &GraceWaiverMode::ReducedRate, &10_001_u32);
    }

    // ── Interaction: default during grace period ──────────────────────────────

    /// When a line is defaulted during the grace window, the grace period ends.
    /// After reinstatement, the line is Active and accrues at the full rate.
    #[test]
    fn default_during_grace_ends_grace_period() {
        let env = Env::default();
        // Suspend at t=0; grace = 1 year.
        let (client, _contract_id, borrower) =
            setup_suspended(&env, 1_000_000, 100_000, 1000, 0);

        client.set_grace_period_config(&31_536_000_u64, &GraceWaiverMode::FullWaiver, &0_u32);

        // Default at t = 0.5 years (inside grace window).
        env.ledger().set_timestamp(15_768_000);
        client.default_credit_line(&borrower);

        // Reinstate at t = 0.5 years (same timestamp, no additional accrual).
        client.reinstate_credit_line(&borrower);

        // Advance 1 more year — line is now Active, grace does not apply.
        env.ledger().set_timestamp(15_768_000 + 31_536_000);
        client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

        let line = client.get_credit_line(&borrower).unwrap();
        // Active line accrues at full rate for 1 year: 10_000 interest.
        assert_eq!(line.accrued_interest, 10_000);
        assert_eq!(line.status, crate::types::CreditStatus::Active);
    }

    /// suspension_ts is cleared when the line is reinstated.
    #[test]
    fn suspension_ts_cleared_on_reinstatement() {
        let env = Env::default();
        let (client, _contract_id, borrower) =
            setup_suspended(&env, 1_000_000, 100_000, 1000, 100);

        // Default then reinstate.
        env.ledger().set_timestamp(200);
        client.default_credit_line(&borrower);
        client.reinstate_credit_line(&borrower);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.suspension_ts, 0);
        assert_eq!(line.status, crate::types::CreditStatus::Active);
    }
}
