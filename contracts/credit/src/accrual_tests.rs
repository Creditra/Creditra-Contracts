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
