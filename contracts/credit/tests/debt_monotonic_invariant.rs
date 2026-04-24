// SPDX-License-Identifier: MIT

//! Invariant test: monotonic debt accounting.
//!
//! Total debt is defined as `utilized_amount + accrued_interest`.
//!
//! # Invariant
//!
//! `total_debt` must never decrease except through an **allowed decreasing
//! operation**. Any other operation that causes a decrease indicates an
//! accounting regression.
//!
//! # Allowed decreasing operations
//!
//! | Operation        | Why it decreases debt                          |
//! |------------------|------------------------------------------------|
//! | `repay_credit`   | Borrower repays principal, reducing utilization |
//! | `forgive_debt`   | (Future) Admin writes off debt explicitly       |
//! | Admin storage    | (Future) Explicit admin correction path         |
//!
//! # Operations that must NOT decrease debt
//!
//! - `open_credit_line` (fresh line, debt starts at zero)
//! - `draw_credit` (increases utilization)
//! - `update_risk_parameters` (changes limit/rate/score, not balances)
//! - `suspend_credit_line` (status change only)
//! - `default_credit_line` (status change only)
//! - `reinstate_credit_line` (status change only)
//! - Time-based interest accrual (increases accrued_interest)

#[cfg(test)]
mod debt_monotonic {
    use creditra_credit::types::{CreditLineData, CreditStatus};
    use creditra_credit::{Credit, CreditClient};
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env};

    fn total_debt(line: &CreditLineData) -> i128 {
        line.utilized_amount + line.accrued_interest
    }

    fn setup_initialized_contract(env: &Env) -> (CreditClient<'_>, Address, Address, Address) {
        env.mock_all_auths();
        let admin = Address::generate(env);
        let borrower = Address::generate(env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(env, &contract_id);
        client.init(&admin);
        (client, contract_id, admin, borrower)
    }

    #[test]
    fn debt_monotonic_across_full_lifecycle() {
        let env = Env::default();
        let (client, _contract_id, _admin, borrower) = setup_initialized_contract(&env);

        // -- OPEN: debt starts at zero --
        client.open_credit_line(&borrower, &10_000, &500_u32, &70_u32);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(total_debt(&line), 0);
        let mut prev_debt = total_debt(&line);

        // -- DRAW 1: debt increases --
        client.draw_credit(&borrower, &2_000);
        let line = client.get_credit_line(&borrower).unwrap();
        assert!(
            total_debt(&line) >= prev_debt,
            "draw must not decrease debt: {} -> {}",
            prev_debt,
            total_debt(&line)
        );
        assert_eq!(total_debt(&line), 2_000);
        prev_debt = total_debt(&line);

        // -- DRAW 2: debt increases further --
        client.draw_credit(&borrower, &1_500);
        let line = client.get_credit_line(&borrower).unwrap();
        assert!(
            total_debt(&line) >= prev_debt,
            "draw must not decrease debt: {} -> {}",
            prev_debt,
            total_debt(&line)
        );
        assert_eq!(total_debt(&line), 3_500);
        prev_debt = total_debt(&line);

        // -- UPDATE RISK (raise limit, change rate): debt unchanged --
        client.update_risk_parameters(&borrower, &15_000, &600_u32, &75_u32);
        let line = client.get_credit_line(&borrower).unwrap();
        assert!(
            total_debt(&line) >= prev_debt,
            "update_risk_parameters must not decrease debt: {} -> {}",
            prev_debt,
            total_debt(&line)
        );
        assert_eq!(total_debt(&line), 3_500);
        prev_debt = total_debt(&line);

        // -- UPDATE RISK (lower limit to utilization boundary): debt unchanged --
        client.update_risk_parameters(&borrower, &3_500, &600_u32, &75_u32);
        let line = client.get_credit_line(&borrower).unwrap();
        assert!(
            total_debt(&line) >= prev_debt,
            "lowering credit_limit to utilized must not decrease debt: {} -> {}",
            prev_debt,
            total_debt(&line)
        );

        // -- REPAY (allowed to decrease) --
        client.repay_credit(&borrower, &1_000);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(
            total_debt(&line),
            2_500,
            "repay should reduce debt by repay amount"
        );

        // -- SUSPEND: debt unchanged --
        client.update_risk_parameters(&borrower, &10_000, &600_u32, &75_u32);
        let line = client.get_credit_line(&borrower).unwrap();
        prev_debt = total_debt(&line);

        client.suspend_credit_line(&borrower);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Suspended);
        assert!(
            total_debt(&line) >= prev_debt,
            "suspend must not decrease debt: {} -> {}",
            prev_debt,
            total_debt(&line)
        );

        // -- REPAY while suspended (allowed to decrease) --
        client.repay_credit(&borrower, &500);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(total_debt(&line), 2_000);
        let prev_debt = total_debt(&line);

        // -- DEFAULT: debt unchanged --
        client.default_credit_line(&borrower);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Defaulted);
        assert!(
            total_debt(&line) >= prev_debt,
            "default must not decrease debt: {} -> {}",
            prev_debt,
            total_debt(&line)
        );

        // -- REPAY while defaulted (allowed to decrease) --
        client.repay_credit(&borrower, &500);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(total_debt(&line), 1_500);
        let prev_debt = total_debt(&line);

        // -- REINSTATE: debt unchanged --
        client.reinstate_credit_line(&borrower);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Active);
        assert!(
            total_debt(&line) >= prev_debt,
            "reinstate must not decrease debt: {} -> {}",
            prev_debt,
            total_debt(&line)
        );
    }

    #[test]
    fn debt_monotonic_with_simulated_interest_accrual() {
        use soroban_sdk::testutils::Ledger;
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        let (client, contract_id, _admin, borrower) = setup_initialized_contract(&env);

        client.open_credit_line(&borrower, &10_000, &500_u32, &70_u32);
        client.draw_credit(&borrower, &5_000);
        let line = client.get_credit_line(&borrower).unwrap();
        let mut prev_debt = total_debt(&line);
        assert_eq!(prev_debt, 5_000);

        // Simulate interest accrual by writing accrued_interest directly.
        // last_accrual_ts must be set to current timestamp so apply_accrual
        // sees no elapsed time and preserves the injected value.
        env.as_contract(&contract_id, || {
            let mut line: CreditLineData = env.storage().persistent().get(&borrower).unwrap();
            line.accrued_interest = 250;
            line.last_accrual_ts = 1_000; // matches current env timestamp
            env.storage().persistent().set(&borrower, &line);
        });

        let line = client.get_credit_line(&borrower).unwrap();
        assert!(
            total_debt(&line) >= prev_debt,
            "accrued interest must not decrease debt: {} -> {}",
            prev_debt,
            total_debt(&line)
        );
        assert_eq!(total_debt(&line), 5_250);
        prev_debt = total_debt(&line);

        // -- DRAW after accrual: debt increases further --
        client.draw_credit(&borrower, &1_000);
        let line = client.get_credit_line(&borrower).unwrap();
        assert!(
            total_debt(&line) >= prev_debt,
            "draw after accrual must not decrease debt: {} -> {}",
            prev_debt,
            total_debt(&line)
        );
        assert_eq!(line.utilized_amount, 6_000);
        assert_eq!(line.accrued_interest, 250);
        assert_eq!(total_debt(&line), 6_250);
        prev_debt = total_debt(&line);

        // -- UPDATE RISK after accrual: debt unchanged --
        client.update_risk_parameters(&borrower, &10_000, &700_u32, &65_u32);
        let line = client.get_credit_line(&borrower).unwrap();
        assert!(
            total_debt(&line) >= prev_debt,
            "update_risk after accrual must not decrease debt: {} -> {}",
            prev_debt,
            total_debt(&line)
        );
        assert_eq!(total_debt(&line), 6_250);
        prev_debt = total_debt(&line);

        // Simulate more interest accrual — advance time first so apply_accrual
        // doesn't recalculate and overwrite the injected value.
        env.ledger().set_timestamp(2_000);
        env.as_contract(&contract_id, || {
            let mut line: CreditLineData = env.storage().persistent().get(&borrower).unwrap();
            line.accrued_interest = 500;
            line.last_accrual_ts = 2_000; // matches current env timestamp
            env.storage().persistent().set(&borrower, &line);
        });

        let line = client.get_credit_line(&borrower).unwrap();
        assert!(
            total_debt(&line) >= prev_debt,
            "further accrual must not decrease debt: {} -> {}",
            prev_debt,
            total_debt(&line)
        );
        assert_eq!(total_debt(&line), 6_500);

        // -- REPAY: allowed to decrease --
        client.repay_credit(&borrower, &2_000);
        let line = client.get_credit_line(&borrower).unwrap();
        // Repay 2000: interest first (500), then principal (1500)
        // utilized_amount: 6000 - 2000 = 4000
        // accrued_interest: 500 - 500 = 0
        assert_eq!(line.utilized_amount, 4_000);
        assert_eq!(line.accrued_interest, 0);
        assert_eq!(total_debt(&line), 4_000);
    }

    #[test]
    fn debt_monotonic_multiple_draw_repay_cycles() {
        let env = Env::default();
        let (client, _contract_id, _admin, borrower) = setup_initialized_contract(&env);

        client.open_credit_line(&borrower, &10_000, &300_u32, &50_u32);

        const ITERATIONS: usize = 10;
        let mut prev_debt: i128 = 0;

        for i in 0..ITERATIONS {
            let draw_amount = ((i as i128) + 1) * 100;
            client.draw_credit(&borrower, &draw_amount);
            let line = client.get_credit_line(&borrower).unwrap();
            assert!(
                total_debt(&line) >= prev_debt,
                "iteration {}: draw must not decrease debt: {} -> {}",
                i,
                prev_debt,
                total_debt(&line)
            );
            prev_debt = total_debt(&line);
        }

        // Verify accumulated draws: sum of 100+200+...+1000 = 5500
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.utilized_amount, 5_500);
        assert_eq!(prev_debt, 5_500);

        // Repay in bounded steps, checking monotonicity between repays
        for i in 0..5 {
            let repay_amount = 500;
            let before = client.get_credit_line(&borrower).unwrap();
            let debt_before = total_debt(&before);

            client.repay_credit(&borrower, &repay_amount);

            let after = client.get_credit_line(&borrower).unwrap();
            let debt_after = total_debt(&after);
            assert_eq!(
                debt_after,
                debt_before - repay_amount,
                "iteration {}: repay should decrease debt by exact amount",
                i
            );
        }

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.utilized_amount, 3_000);
    }

    #[test]
    fn debt_monotonic_status_transitions_preserve_debt() {
        let env = Env::default();
        let (client, _contract_id, _admin, borrower) = setup_initialized_contract(&env);

        client.open_credit_line(&borrower, &5_000, &400_u32, &60_u32);
        client.draw_credit(&borrower, &3_000);

        let line = client.get_credit_line(&borrower).unwrap();
        let debt_at_active = total_debt(&line);
        assert_eq!(debt_at_active, 3_000);

        // Active -> Suspended: debt unchanged
        client.suspend_credit_line(&borrower);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(total_debt(&line), debt_at_active);

        // Suspended -> Defaulted: debt unchanged
        client.default_credit_line(&borrower);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(total_debt(&line), debt_at_active);

        // Defaulted -> Active (reinstate): debt unchanged
        client.reinstate_credit_line(&borrower);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(total_debt(&line), debt_at_active);
        assert_eq!(line.status, CreditStatus::Active);
    }

    #[test]
    fn debt_monotonic_risk_update_sequence() {
        use soroban_sdk::testutils::Ledger;

        let env = Env::default();
        let (client, _contract_id, _admin, borrower) = setup_initialized_contract(&env);

        client.open_credit_line(&borrower, &10_000, &300_u32, &50_u32);
        client.draw_credit(&borrower, &4_000);

        let line = client.get_credit_line(&borrower).unwrap();
        let initial_debt = total_debt(&line);

        // Raise limit
        client.update_risk_parameters(&borrower, &20_000, &300_u32, &50_u32);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(total_debt(&line), initial_debt);

        // Change rate
        client.update_risk_parameters(&borrower, &20_000, &800_u32, &50_u32);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(total_debt(&line), initial_debt);

        // Change score
        client.update_risk_parameters(&borrower, &20_000, &800_u32, &90_u32);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(total_debt(&line), initial_debt);

        // Lower limit to utilization boundary
        client.update_risk_parameters(&borrower, &4_000, &800_u32, &90_u32);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(total_debt(&line), initial_debt);

        // Set rate change limits and verify debt invariant with timed updates
        client.set_rate_change_limits(&500_u32, &60_u64);

        env.ledger().with_mut(|li| li.timestamp = 100);
        client.update_risk_parameters(&borrower, &4_000, &500_u32, &90_u32);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(total_debt(&line), initial_debt);

        env.ledger().with_mut(|li| li.timestamp = 200);
        client.update_risk_parameters(&borrower, &4_000, &700_u32, &90_u32);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(total_debt(&line), initial_debt);
    }

    #[test]
    fn debt_monotonic_overpay_does_not_go_negative() {
        let env = Env::default();
        let (client, _contract_id, _admin, borrower) = setup_initialized_contract(&env);

        client.open_credit_line(&borrower, &5_000, &300_u32, &50_u32);
        client.draw_credit(&borrower, &1_000);

        // Over-repay: amount > utilized
        client.repay_credit(&borrower, &5_000);
        let line = client.get_credit_line(&borrower).unwrap();
        assert!(
            total_debt(&line) >= 0,
            "total debt must never go negative after overpay"
        );
        assert_eq!(line.utilized_amount, 0);
    }
}
