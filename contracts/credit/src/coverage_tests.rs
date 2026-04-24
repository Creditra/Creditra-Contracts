// SPDX-License-Identifier: MIT

#[cfg(test)]
mod tests {
    use crate::events::{
        publish_admin_rotation_accepted, publish_admin_rotation_proposed,
        publish_borrower_blocked_event, publish_credit_line_event_v2, publish_drawn_event_v2,
        publish_interest_accrued_event, publish_rate_formula_config_event,
        publish_repayment_event_v2, AdminRotationAcceptedEvent, AdminRotationProposedEvent,
        BorrowerBlockedEvent, CreditLineEventV2, DrawnEventV2, InterestAccruedEvent,
        RateFormulaConfigEvent, RepaymentEventV2,
    };
    use crate::storage::is_borrower_blocked;
    use crate::types::{CreditStatus, RateFormulaConfig};
    use crate::Credit;
    use crate::CreditClient;
    use soroban_sdk::{
        symbol_short,
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
    fn test_borrower_blocking() {
        let (env, _admin, borrower, client) = setup_env();

        client.open_credit_line(&borrower, &1000, &1000, &50);

        // Block borrower
        client.set_borrower_blocked(&borrower, &true);
        assert!(env.as_contract(&client.address, || is_borrower_blocked(&env, &borrower)));

        // Drawing should fail
        let result = client.try_draw_credit(&borrower, &100);
        assert!(result.is_err());

        // Unblock borrower
        client.set_borrower_blocked(&borrower, &false);
        assert!(!env.as_contract(&client.address, || is_borrower_blocked(&env, &borrower)));

        // Drawing should succeed
        client.draw_credit(&borrower, &100);
    }

    #[test]
    fn test_max_draw_amount() {
        let (_env, _admin, borrower, client) = setup_env();
        client.open_credit_line(&borrower, &1000, &1000, &50);

        client.set_max_draw_amount(&50);

        // Drawing more than max should fail
        let result = client.try_draw_credit(&borrower, &100);
        assert!(result.is_err());

        // Drawing exactly max should succeed
        client.draw_credit(&borrower, &50);
    }

    #[test]
    fn test_rate_change_limits() {
        let (env, _admin, borrower, client) = setup_env();

        // Set a non-zero timestamp initially so last_rate_update_ts is not 0
        env.ledger().set_timestamp(100);

        client.open_credit_line(&borrower, &1000, &1000, &50);

        client.set_rate_change_limits(&100, &3600);

        // Update rate by more than 100 bps should fail
        let result = client.try_update_risk_parameters(&borrower, &1000, &1200, &50);
        assert!(result.is_err());

        // Update rate within limits
        env.ledger().set_timestamp(1000); // Advance slightly
        client.update_risk_parameters(&borrower, &1000, &1050, &50);

        // Updating again before interval should fail
        env.ledger().set_timestamp(2000); // 1000s passed, but interval is 3600
        let result = client.try_update_risk_parameters(&borrower, &1000, &1100, &50);
        assert!(result.is_err());

        // Advance time past 3600 interval (1000 + 3601)
        env.ledger().set_timestamp(5000);
        client.update_risk_parameters(&borrower, &1000, &1100, &50);
    }

    #[test]
    fn test_rate_formula() {
        let (_env, _admin, borrower, client) = setup_env();
        client.open_credit_line(&borrower, &10000, &1000, &50);

        let formula = RateFormulaConfig {
            base_rate_bps: 100,
            slope_bps_per_score: 10,
            min_rate_bps: 200,
            max_rate_bps: 5000,
        };
        client.set_rate_formula(&formula);

        // Update with score 10. raw_rate = 100 + 10*10 = 200. Clamped to [200, 5000] -> 200.
        client.update_risk_parameters(&borrower, &10000, &0, &10);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.interest_rate_bps, 200);

        // Update with score 90. raw_rate = 100 + 10*90 = 1000.
        client.update_risk_parameters(&borrower, &10000, &0, &90);
        let line2 = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line2.interest_rate_bps, 1000);
    }

    #[test]
    fn test_admin_rotation() {
        let (env, _admin, _borrower, client) = setup_env();
        let new_admin = Address::generate(&env);

        env.ledger().set_timestamp(1000);
        client.propose_admin(&new_admin, &3600);

        // Accept too early (at 2000, expected after 4600)
        env.ledger().set_timestamp(2000);
        let result = client.try_accept_admin();
        assert!(result.is_err());

        // Accept after delay
        env.ledger().set_timestamp(5000);
        client.accept_admin();
    }

    #[test]
    fn test_lifecycle_reinstate() {
        let (_env, _admin, borrower, client) = setup_env();
        client.open_credit_line(&borrower, &1000, &1000, &50);

        client.suspend_credit_line(&borrower);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Suspended);

        client.default_credit_line(&borrower);
        let line2 = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line2.status, CreditStatus::Defaulted);

        client.reinstate_credit_line(&borrower);
        let line3 = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line3.status, CreditStatus::Active);
    }

    #[test]
    fn test_event_publishers() {
        let env = Env::default();
        let borrower = Address::generate(&env);
        let admin = Address::generate(&env);

        // Target uncovered publish_* functions in events.rs
        publish_credit_line_event_v2(
            &env,
            (symbol_short!("c"), symbol_short!("o")),
            CreditLineEventV2 {
                event_type: symbol_short!("opened"),
                borrower: borrower.clone(),
                status: CreditStatus::Active,
                credit_limit: 100,
                interest_rate_bps: 10,
                risk_score: 5,
                timestamp: 1,
                actor: admin.clone(),
                amount: 0,
            },
        );

        publish_repayment_event_v2(
            &env,
            RepaymentEventV2 {
                borrower: borrower.clone(),
                payer: borrower.clone(),
                amount: 100,
                interest_repaid: 10,
                principal_repaid: 90,
                new_utilized_amount: 0,
                new_accrued_interest: 0,
                timestamp: 1,
            },
        );

        publish_drawn_event_v2(
            &env,
            DrawnEventV2 {
                borrower: borrower.clone(),
                recipient: borrower.clone(),
                reserve_source: admin.clone(),
                amount: 100,
                new_utilized_amount: 100,
                timestamp: 1,
            },
        );

        publish_interest_accrued_event(
            &env,
            InterestAccruedEvent {
                borrower: borrower.clone(),
                accrued_amount: 5,
                total_accrued_interest: 5,
                new_utilized_amount: 105,
                timestamp: 1,
            },
        );

        publish_rate_formula_config_event(
            &env,
            RateFormulaConfigEvent {
                base_rate_bps: 100,
                slope_bps_per_score: 10,
                min_rate_bps: 100,
                max_rate_bps: 1000,
                timestamp: 1,
            },
        );

        publish_admin_rotation_proposed(
            &env,
            AdminRotationProposedEvent {
                current_admin: admin.clone(),
                proposed_admin: borrower.clone(),
                accept_after: 100,
            },
        );

        publish_admin_rotation_accepted(
            &env,
            AdminRotationAcceptedEvent {
                previous_admin: admin.clone(),
                new_admin: borrower.clone(),
            },
        );

        publish_borrower_blocked_event(
            &env,
            BorrowerBlockedEvent {
                borrower: borrower.clone(),
                blocked: true,
            },
        );

        publish_borrower_blocked_event(
            &env,
            BorrowerBlockedEvent {
                borrower: borrower.clone(),
                blocked: false,
            },
        );
    }
}
