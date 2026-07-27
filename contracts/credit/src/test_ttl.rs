#[cfg(test)]
mod test {
    use crate::storage::CREDIT_LINE_TTL_EXTEND_TO;
    use crate::{Credit, CreditClient};
    use soroban_sdk::testutils::storage::Persistent;
    use soroban_sdk::token::StellarAssetClient;
    use soroban_sdk::{
        testutils::{Address as _, Ledger},
        Address, Env,
    };
    use std::panic::{catch_unwind, AssertUnwindSafe};

    fn setup<'a>(env: &'a Env) -> (CreditClient<'a>, Address, Address, Address) {
        env.mock_all_auths();
        let admin = Address::generate(env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(env, &contract_id);
        client.init(&admin);

        let borrower = Address::generate(env);

        (client, contract_id, admin, borrower)
    }

    fn check_ttl(env: &Env, contract_id: &Address, borrower: &Address) -> u32 {
        env.as_contract(contract_id, || env.storage().persistent().get_ttl(borrower))
    }

    fn advance_ledger(env: &Env, contract_id: &Address, borrower: &Address) {
        env.as_contract(contract_id, || {
            env.storage().instance().extend_ttl(500_000, 500_000);
        });
        // Advance sequence by enough to drop the remaining TTL below CREDIT_LINE_TTL_THRESHOLD.
        // If current TTL is 432,000, advancing by 432,000 - 100 makes remaining TTL 100.
        env.ledger()
            .set_sequence_number(env.ledger().sequence() + 432_000 - 100);
    }

    #[test]
    fn test_borrow_read_path_bumps_ttl_before_panic() {
        let env = Env::default();
        let (client, contract_id, _admin, borrower) = setup(&env);

        client.open_credit_line(&borrower, &1000, &300, &70);
        advance_ledger(&env, &contract_id);

        let impostor = Address::generate(&env);
        let result = catch_unwind(AssertUnwindSafe(|| {
            client.draw_credit(&impostor, &100);
        }));

        assert!(result.is_err());
        assert_eq!(
            check_ttl(&env, &contract_id, &borrower),
            CREDIT_LINE_TTL_EXTEND_TO
        );
    }

    #[test]
    fn test_all_interactions_bump_ttl() {
        let env = Env::default();
        let (client, contract_id, admin, borrower) = setup(&env);

        // 1. open_credit_line
        client.open_credit_line(&borrower, &1000, &300, &70);
        assert_eq!(
            check_ttl(&env, &contract_id, &borrower),
            CREDIT_LINE_TTL_EXTEND_TO
        );

        advance_ledger(&env, &contract_id);

        // 2. get_credit_line
        client.get_credit_line(&borrower);
        assert_eq!(
            check_ttl(&env, &contract_id, &borrower),
            CREDIT_LINE_TTL_EXTEND_TO
        );

        advance_ledger(&env, &contract_id);

        // 3. update_risk_parameters
        client.update_risk_parameters(&borrower, &1000, &350, &75);
        assert_eq!(
            check_ttl(&env, &contract_id, &borrower),
            CREDIT_LINE_TTL_EXTEND_TO
        );

        advance_ledger(&env, &contract_id);

        // 4. draw_credit
        client.draw_credit(&borrower, &100);
        assert_eq!(
            check_ttl(&env, &contract_id, &borrower),
            CREDIT_LINE_TTL_EXTEND_TO
        );

        advance_ledger(&env, &contract_id);

        // 5. repay_credit
        client.repay_credit(&borrower, &50);
        assert_eq!(
            check_ttl(&env, &contract_id, &borrower),
            CREDIT_LINE_TTL_EXTEND_TO
        );

        advance_ledger(&env, &contract_id);

        // 6. suspend_credit_line
        client.suspend_credit_line(&borrower);
        assert_eq!(
            check_ttl(&env, &contract_id, &borrower),
            CREDIT_LINE_TTL_EXTEND_TO
        );

        advance_ledger(&env, &contract_id);

        // 7. default_credit_line (susp -> default)
        client.default_credit_line(&borrower);
        assert_eq!(
            check_ttl(&env, &contract_id, &borrower),
            CREDIT_LINE_TTL_EXTEND_TO
        );

        advance_ledger(&env, &contract_id);

        // 8. reinstate_credit_line
        client.reinstate_credit_line(&borrower, &crate::types::CreditStatus::Active);
        assert_eq!(check_ttl(&env, &contract_id, &borrower), CREDIT_LINE_TTL_EXTEND_TO);
        
        advance_ledger(&env, &contract_id, &borrower);

        // 9. close_credit_line (by admin)
        client.close_credit_line(&borrower, &admin);
        assert_eq!(
            check_ttl(&env, &contract_id, &borrower),
            CREDIT_LINE_TTL_EXTEND_TO
        );
    }
}
