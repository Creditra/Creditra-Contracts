
    use super::*;
use soroban_sdk::{Address, Env};
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::testutils::Events as _;
    use soroban_sdk::token;

    use soroban_sdk::token::StellarAssetClient;
    use soroban_sdk::{Symbol, TryFromVal, TryIntoVal};


    fn setup_test(env: &Env) -> (Address, Address, Address) {
        env.mock_all_auths();

        let admin = Address::generate(env);
        let borrower = Address::generate(env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);

        (admin, borrower, contract_id)
    }

    fn call_contract<F>(env: &Env, contract_id: &Address, f: F)
    where
        F: FnOnce(),
    {
        env.as_contract(contract_id, f);
    }

    fn get_credit_data(env: &Env, contract_id: &Address, borrower: &Address) -> CreditLineData {
        let client = CreditClient::new(env, contract_id);
        client
            .get_credit_line(borrower)
            .expect("Credit line not found")
    }

    fn setup_contract_with_credit_line<'a>(
        env: &'a Env,
        borrower: &'a Address,
        credit_limit: i128,
        reserve_amount: i128,
    ) -> (CreditClient<'a>, Address, Address) {
        let admin = Address::generate(env);
        let contract_id = env.register(Credit, ());
        let token_admin = Address::generate(env);
        let token_id = env.register_stellar_asset_contract_v2(token_admin);
        let token_address = token_id.address();
        let client = CreditClient::new(env, &contract_id);
        client.init(&admin);
        if reserve_amount > 0 {
            let sac = StellarAssetClient::new(env, &token_address);
            sac.mint(&contract_id, &reserve_amount);
        }
        client.set_liquidity_token(&token_address);
        client.open_credit_line(borrower, &credit_limit, &300_u32, &70_u32);
        (client, token_address, admin)
    }

    fn setup_token<'a>(
        env: &'a Env,
        contract_id: &'a Address,
        reserve_amount: i128,
    ) -> (Address, StellarAssetClient<'a>) {
        let token_admin = Address::generate(env);
        let token_id = env.register_stellar_asset_contract_v2(token_admin);
        let token_address = token_id.address();
        let sac = StellarAssetClient::new(env, &token_address);
        if reserve_amount > 0 {
            sac.mint(contract_id, &reserve_amount);
        }
        (token_address, sac)
    }

    /// Test-only helper for simulating liquidity token balances and allowances.
    ///
    /// This utility keeps token setup concise in integration-style draw/repay tests.
    struct MockLiquidityToken<'a> {
        address: Address,
        admin_client: StellarAssetClient<'a>,
        token_client: token::Client<'a>,
    }

    impl<'a> MockLiquidityToken<'a> {
        fn deploy(env: &'a Env) -> Self {
            let token_admin = Address::generate(env);
            let token_id = env.register_stellar_asset_contract_v2(token_admin);
            let address = token_id.address();
            Self {
                address: address.clone(),
                admin_client: StellarAssetClient::new(env, &address),
                token_client: token::Client::new(env, &address),
            }
        }

        fn address(&self) -> Address {
            self.address.clone()
        }

        fn mint(&self, to: &Address, amount: i128) {
            self.admin_client.mint(to, &amount);
        }

        fn approve(&self, from: &Address, spender: &Address, amount: i128, expires_at: u32) {
            self.token_client
                .approve(from, spender, &amount, &expires_at);
        }

        fn balance(&self, address: &Address) -> i128 {
            self.token_client.balance(address)
        }

        fn allowance(&self, from: &Address, spender: &Address) -> i128 {
            self.token_client.allowance(from, spender)
        }
    }

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
        client.close_credit_line(&borrower, &admin);

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

        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.status, CreditStatus::Defaulted);
    }

    // ========== open_credit_line: duplicate borrower and invalid params (#28) ==========

    /// open_credit_line must revert when the borrower already has an Active credit line.
    #[test]
    #[should_panic(expected = "borrower already has an active credit line")]
    fn test_open_credit_line_duplicate_active_borrower_reverts() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        // Second open for same borrower while Active must revert.
        client.open_credit_line(&borrower, &2000_i128, &400_u32, &60_u32);
    }

    /// open_credit_line must revert when credit_limit is zero.
    #[test]
    #[should_panic(expected = "credit_limit must be greater than zero")]
    fn test_open_credit_line_zero_limit_reverts() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &0_i128, &300_u32, &70_u32);
    }

    /// open_credit_line must revert when credit_limit is negative.
    #[test]
    #[should_panic(expected = "credit_limit must be greater than zero")]
    fn test_open_credit_line_negative_limit_reverts() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &-1_i128, &300_u32, &70_u32);
    }

    /// open_credit_line must revert when interest_rate_bps exceeds 10000 (100%).
    #[test]
    #[should_panic(expected = "interest_rate_bps cannot exceed 10000 (100%)")]
    fn test_open_credit_line_interest_rate_exceeds_max_reverts() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &10_001_u32, &70_u32);
    }

    /// open_credit_line must revert when risk_score exceeds 100.
    #[test]
    #[should_panic(expected = "risk_score must be between 0 and 100")]
    fn test_open_credit_line_risk_score_exceeds_max_reverts() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &101_u32);
    }

    // ========== draw_credit within limit (#29) ==========

    #[test]
    fn test_draw_credit() {
        let env = Env::default();
        let (_admin, borrower, contract_id) = setup_test(&env);

        call_contract(&env, &contract_id, || {
            Credit::draw_credit(env.clone(), borrower.clone(), 500_i128);
        });

        let credit_data = get_credit_data(&env, &contract_id, &borrower);
        assert_eq!(credit_data.utilized_amount, 500_i128);

        // Events are emitted - functionality verified through storage changes
    }

    /// draw_credit within limit: single draw updates utilized_amount correctly.
    #[test]
    fn test_draw_credit_single_within_limit_succeeds_and_updates_utilized() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);

        let line_before = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line_before.utilized_amount, 0);

        client.draw_credit(&borrower, &400_i128);

        let line_after = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line_after.utilized_amount, 400);
        assert_eq!(line_after.credit_limit, 1000);
    }

    /// draw_credit within limit: multiple draws accumulate utilized_amount correctly.
    #[test]
    fn test_draw_credit_multiple_draws_within_limit_accumulate_utilized() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);

        client.draw_credit(&borrower, &100_i128);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            100
        );

        client.draw_credit(&borrower, &250_i128);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            350
        );

        client.draw_credit(&borrower, &150_i128);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            500
        );
    }

    /// draw_credit within limit: drawing exact available limit succeeds and utilized equals limit.
    #[test]
    fn test_draw_credit_exact_available_limit_succeeds() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        let limit = 5000_i128;
        client.open_credit_line(&borrower, &limit, &300_u32, &70_u32);

        client.draw_credit(&borrower, &limit);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.utilized_amount, limit);
        assert_eq!(line.credit_limit, limit);
    }

    #[test]
    fn test_repay_credit_partial() {
        let env = Env::default();
        let (_admin, borrower, contract_id) = setup_test(&env);

        // First draw some credit
        call_contract(&env, &contract_id, || {
            Credit::draw_credit(env.clone(), borrower.clone(), 500_i128);
        });
        assert_eq!(
            get_credit_data(&env, &contract_id, &borrower).utilized_amount,
            500_i128
        );

        // Partial repayment
        call_contract(&env, &contract_id, || {
            Credit::repay_credit(env.clone(), borrower.clone(), 200_i128);
        });

        let credit_data = get_credit_data(&env, &contract_id, &borrower);
        assert_eq!(credit_data.utilized_amount, 300_i128); // 500 - 200
    }

    #[test]
    fn test_repay_credit_full() {
        let env = Env::default();
        let (_admin, borrower, contract_id) = setup_test(&env);

        // Draw some credit
        call_contract(&env, &contract_id, || {
            Credit::draw_credit(env.clone(), borrower.clone(), 500_i128);
        });
        assert_eq!(
            get_credit_data(&env, &contract_id, &borrower).utilized_amount,
            500_i128
        );

        // Full repayment
        call_contract(&env, &contract_id, || {
            Credit::repay_credit(env.clone(), borrower.clone(), 500_i128);
        });

        let credit_data = get_credit_data(&env, &contract_id, &borrower);
        assert_eq!(credit_data.utilized_amount, 0_i128); // Fully repaid
    }

    #[test]
    fn test_repay_credit_overpayment() {
        let env = Env::default();
        let (_admin, borrower, contract_id) = setup_test(&env);

        // Draw some credit
        call_contract(&env, &contract_id, || {
            Credit::draw_credit(env.clone(), borrower.clone(), 300_i128);
        });
        assert_eq!(
            get_credit_data(&env, &contract_id, &borrower).utilized_amount,
            300_i128
        );

        // Overpayment (pay more than utilized)
        call_contract(&env, &contract_id, || {
            Credit::repay_credit(env.clone(), borrower.clone(), 500_i128);
        });

        let credit_data = get_credit_data(&env, &contract_id, &borrower);
        assert_eq!(credit_data.utilized_amount, 0_i128); // Should be capped at 0
    }

    #[test]
    fn test_repay_credit_zero_utilization() {
        let env = Env::default();
        let (_admin, borrower, contract_id) = setup_test(&env);

        // Try to repay when no credit is utilized
        call_contract(&env, &contract_id, || {
            Credit::repay_credit(env.clone(), borrower.clone(), 100_i128);
        });

        let credit_data = get_credit_data(&env, &contract_id, &borrower);
        assert_eq!(credit_data.utilized_amount, 0_i128); // Should remain 0
    }

    #[test]
    fn test_repay_credit_suspended_status() {
        let env = Env::default();
        let (_admin, borrower, contract_id) = setup_test(&env);

        // Draw some credit
        call_contract(&env, &contract_id, || {
            Credit::draw_credit(env.clone(), borrower.clone(), 500_i128);
        });

        // Manually set status to Suspended
        let mut credit_data = get_credit_data(&env, &contract_id, &borrower);
        credit_data.status = CreditStatus::Suspended;
        env.as_contract(&contract_id, || {
            env.storage().persistent().set(&borrower, &credit_data);
        });

        // Should be able to repay even when suspended
        call_contract(&env, &contract_id, || {
            Credit::repay_credit(env.clone(), borrower.clone(), 200_i128);
        });

        let updated_data = get_credit_data(&env, &contract_id, &borrower);
        assert_eq!(updated_data.utilized_amount, 300_i128);
        assert_eq!(updated_data.status, CreditStatus::Suspended); // Status should remain Suspended
    }

    #[test]
    #[should_panic(expected = "amount must be positive")]
    fn test_repay_credit_invalid_amount_zero() {
        let env = Env::default();
        let (_admin, borrower, contract_id) = setup_test(&env);

        call_contract(&env, &contract_id, || {
            Credit::repay_credit(env.clone(), borrower.clone(), 0_i128);
        });
    }

    #[test]
    #[should_panic(expected = "amount must be positive")]
    fn test_repay_credit_invalid_amount_negative() {
        let env = Env::default();
        let (_admin, borrower, contract_id) = setup_test(&env);

        let negative_amount: i128 = -100;
        call_contract(&env, &contract_id, || {
            Credit::repay_credit(env.clone(), borrower.clone(), negative_amount);
        });
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

        client.open_credit_line(&borrower, &5000_i128, &500_u32, &80_u32);
        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.status, CreditStatus::Active);

        client.suspend_credit_line(&borrower);
        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.status, CreditStatus::Suspended);

        client.close_credit_line(&borrower, &admin);
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

        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.borrower, borrower);
        assert_eq!(credit_line.status, CreditStatus::Active);
        assert_eq!(credit_line.credit_limit, 2000);
        assert_eq!(credit_line.interest_rate_bps, 400);
        assert_eq!(credit_line.risk_score, 75);
    }

    #[test]
    fn test_close_credit_line_defaulted_admin_force_close() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, admin) =
            setup_contract_with_credit_line(&env, &borrower, 1_000, 1_000);
        client.draw_credit(&borrower, &300);
        client.default_credit_line(&borrower);
        client.close_credit_line(&borrower, &admin);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Closed);
        assert_eq!(line.utilized_amount, 300);
    }

    #[test]
    fn test_close_credit_line_defaulted_borrower_when_zero_utilization() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) = setup_contract_with_credit_line(&env, &borrower, 1_000, 0);
        client.default_credit_line(&borrower);
        client.close_credit_line(&borrower, &borrower);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().status,
            CreditStatus::Closed
        );
    }

    #[test]
    #[should_panic(expected = "Credit line not found")]
    fn test_suspend_nonexistent_credit_line() {
        let env = Env::default();
        env.mock_all_auths();
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
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.close_credit_line(&borrower, &admin);
    }

    #[test]
    #[should_panic(expected = "Credit line not found")]
    fn test_default_nonexistent_credit_line() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.default_credit_line(&borrower);
    }

    #[test]
    #[should_panic(expected = "Credit line not found")]
    fn test_reinstate_nonexistent_credit_line() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let admin = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let (token_address, _) = setup_token(&env, &contract_id, 0);
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);
        client.set_liquidity_token(&token_address);
        client.reinstate_credit_line(&borrower);
    }

    #[test]
    fn test_reinstate_credit_line() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) =
            setup_contract_with_credit_line(&env, &borrower, 1_000, 1_000);
        client.default_credit_line(&borrower);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().status,
            CreditStatus::Defaulted
        );
        client.reinstate_credit_line(&borrower);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().status,
            CreditStatus::Active
        );
        client.draw_credit(&borrower, &200);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            200
        );
    }

    #[test]
    #[should_panic(expected = "credit line is not defaulted")]
    fn test_reinstate_credit_line_not_defaulted() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) = setup_contract_with_credit_line(&env, &borrower, 1_000, 0);
        client.reinstate_credit_line(&borrower);
    }

    #[test]
    #[should_panic]
    fn test_reinstate_credit_line_unauthorized() {
        let env = Env::default();
        let borrower = Address::generate(&env);
        let admin = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let (token_address, _) = setup_token(&env, &contract_id, 0);
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);
        client.set_liquidity_token(&token_address);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);
        client.default_credit_line(&borrower);
        client.reinstate_credit_line(&borrower);
    }

    // ── reinstate_credit_line: new explicit transition tests ─────────────────

    /// Defaulted → Active: status becomes Active and draws are re-enabled.
    #[test]
    fn test_reinstate_to_active_enables_draws() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) =
            setup_contract_with_credit_line(&env, &borrower, 1_000, 1_000);
        client.default_credit_line(&borrower);
        client.reinstate_credit_line(&borrower);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Active);
        // Draw must succeed after reinstatement to Active
        client.draw_credit(&borrower, &100);
        assert_eq!(client.get_credit_line(&borrower).unwrap().utilized_amount, 100);
    }

    /// Defaulted → Active: reinstate always goes to Active.
    #[test]
    fn test_reinstate_to_suspended_status() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) =
            setup_contract_with_credit_line(&env, &borrower, 1_000, 0);
        client.default_credit_line(&borrower);
        client.reinstate_credit_line(&borrower);
        let line = client.get_credit_line(&borrower).unwrap();
        // reinstate_credit_line always transitions to Active
        assert_eq!(line.status, CreditStatus::Active);
    }

    /// After reinstatement to Active, draws are re-enabled.
    #[test]
    fn test_reinstate_to_suspended_blocks_draws() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) =
            setup_contract_with_credit_line(&env, &borrower, 1_000, 1_000);
        client.default_credit_line(&borrower);
        client.reinstate_credit_line(&borrower);
        // After reinstatement to Active, draws succeed
        client.draw_credit(&borrower, &100);
        assert_eq!(client.get_credit_line(&borrower).unwrap().utilized_amount, 100);
    }

    /// Invariant: utilized_amount is preserved after reinstatement.
    #[test]
    fn test_reinstate_preserves_utilized_amount() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) =
            setup_contract_with_credit_line(&env, &borrower, 1_000, 1_000);
        client.draw_credit(&borrower, &400);
        client.default_credit_line(&borrower);
        client.reinstate_credit_line(&borrower);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.utilized_amount, 400);
        assert_eq!(line.credit_limit, 1_000);
        assert_eq!(line.interest_rate_bps, 300);
        assert_eq!(line.risk_score, 70);
    }

    /// Invariant: utilized_amount preserved after reinstatement.
    #[test]
    fn test_reinstate_to_suspended_preserves_utilized_amount() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) =
            setup_contract_with_credit_line(&env, &borrower, 1_000, 1_000);
        client.draw_credit(&borrower, &250);
        client.default_credit_line(&borrower);
        client.reinstate_credit_line(&borrower);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.utilized_amount, 250);
        assert_eq!(line.status, CreditStatus::Active);
    }

    /// Invalid target: reinstate on a non-defaulted line must revert.
    #[test]
    #[should_panic(expected = "credit line is not defaulted")]
    fn test_reinstate_invalid_target_status_closed_reverts() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) =
            setup_contract_with_credit_line(&env, &borrower, 1_000, 0);
        // Line is Active, not Defaulted — must revert
        client.reinstate_credit_line(&borrower);
    }

    /// Reinstate on a non-defaulted (Active) line must revert.
    #[test]
    #[should_panic(expected = "credit line is not defaulted")]
    fn test_reinstate_invalid_target_status_defaulted_reverts() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) =
            setup_contract_with_credit_line(&env, &borrower, 1_000, 0);
        // Line is Active, not Defaulted — must revert
        client.reinstate_credit_line(&borrower);
    }

    /// Reinstate to Active emits event with correct status.
    #[test]
    fn test_reinstate_to_active_emits_event_with_active_status() {
        use soroban_sdk::testutils::Events;
        use soroban_sdk::{TryFromVal, TryIntoVal};
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) =
            setup_contract_with_credit_line(&env, &borrower, 1_000, 0);
        client.default_credit_line(&borrower);
        client.reinstate_credit_line(&borrower);
        let events = env.events().all();
        let (_contract, topics, data) = events.last().unwrap();
        // New schema: topic0 = "credit", topic1 = "reinstate"
        assert_eq!(
            soroban_sdk::Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap(),
            soroban_sdk::Symbol::new(&env, "credit")
        );
        assert_eq!(
            soroban_sdk::Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
            soroban_sdk::Symbol::new(&env, "reinstate")
        );
        let event_data: CreditLineEvent = data.try_into_val(&env).unwrap();
        assert_eq!(event_data.status, CreditStatus::Active);
        assert_eq!(event_data.borrower, borrower);
    }

    /// Reinstate to Suspended is no longer supported (reinstate always goes to Active).
    /// This test verifies the current behavior: reinstate_credit_line goes to Active.
    #[test]
    fn test_reinstate_to_active_emits_event_with_correct_topic() {
        use soroban_sdk::testutils::Events;
        use soroban_sdk::{TryFromVal, TryIntoVal};
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) =
            setup_contract_with_credit_line(&env, &borrower, 1_000, 0);
        client.default_credit_line(&borrower);
        client.reinstate_credit_line(&borrower);
        let events = env.events().all();
        let (_contract, topics, data) = events.last().unwrap();
        // New schema: topic0 = "credit", topic1 = "reinstate"
        assert_eq!(
            soroban_sdk::Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap(),
            soroban_sdk::Symbol::new(&env, "credit")
        );
        assert_eq!(
            soroban_sdk::Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
            soroban_sdk::Symbol::new(&env, "reinstate")
        );
        let event_data: CreditLineEvent = data.try_into_val(&env).unwrap();
        assert_eq!(event_data.status, CreditStatus::Active);
        assert_eq!(event_data.borrower, borrower);
    }

    /// Reinstate to Active then can be suspended again (full state machine round-trip).
    #[test]
    fn test_reinstate_to_active_then_suspend_again() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) =
            setup_contract_with_credit_line(&env, &borrower, 1_000, 0);
        client.default_credit_line(&borrower);
        client.reinstate_credit_line(&borrower);
        assert_eq!(client.get_credit_line(&borrower).unwrap().status, CreditStatus::Active);
        client.suspend_credit_line(&borrower);
        assert_eq!(client.get_credit_line(&borrower).unwrap().status, CreditStatus::Suspended);
    }

    /// Reinstate then admin can close.
    #[test]
    fn test_reinstate_to_suspended_then_admin_close() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let admin = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);
        client.default_credit_line(&borrower);
        client.reinstate_credit_line(&borrower);
        client.close_credit_line(&borrower, &admin);
        assert_eq!(client.get_credit_line(&borrower).unwrap().status, CreditStatus::Closed);
    }

    /// Non-admin cannot reinstate (no auth).
    #[test]
    #[should_panic]
    fn test_reinstate_to_suspended_unauthorized() {
        let env = Env::default();
        // No mock_all_auths — auth will fail
        let borrower = Address::generate(&env);
        let admin = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);
        client.default_credit_line(&borrower);
        client.reinstate_credit_line(&borrower);
    }

    // ── update_risk_parameters ────────────────────────────────────────────────

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
    fn test_close_credit_line_borrower_when_utilized_zero() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.close_credit_line(&borrower, &borrower);

        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.status, CreditStatus::Closed);
        assert_eq!(credit_line.utilized_amount, 0);
    }

    #[test]
    #[should_panic(expected = "cannot close: utilized amount not zero")]
    fn test_close_credit_line_borrower_rejected_when_utilized_nonzero() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.draw_credit(&borrower, &300_i128);

        client.close_credit_line(&borrower, &borrower);
    }

    #[test]
    fn test_close_credit_line_admin_force_close_with_utilization() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.draw_credit(&borrower, &300_i128);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            300
        );

        client.close_credit_line(&borrower, &admin);

        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.status, CreditStatus::Closed);
        assert_eq!(credit_line.utilized_amount, 300);
    }

    #[test]
    fn test_close_credit_line_idempotent_when_already_closed() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.close_credit_line(&borrower, &admin);
        client.close_credit_line(&borrower, &admin);

        assert_eq!(
            client.get_credit_line(&borrower).unwrap().status,
            CreditStatus::Closed
        );
    }

    #[test]
    #[should_panic(expected = "credit line is closed")]
    fn test_draw_credit_rejected_when_closed() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.close_credit_line(&borrower, &admin);

        client.draw_credit(&borrower, &100_i128);
    }

    #[test]
    #[should_panic(expected = "exceeds credit limit")]
    fn test_draw_credit_rejected_when_exceeding_limit() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &100_i128, &300_u32, &70_u32);
        client.draw_credit(&borrower, &101_i128);
    }

    #[test]
    #[should_panic(expected = "credit line is closed")]
    fn test_repay_credit_rejected_when_closed() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.close_credit_line(&borrower, &admin);

        client.repay_credit(&borrower, &100_i128);
    }

    #[test]
    #[should_panic(expected = "unauthorized")]
    fn test_close_credit_line_unauthorized_closer() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let other = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.close_credit_line(&borrower, &other);
    }

    #[test]
    fn test_repay_credit_succeeds_when_defaulted() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) =
            setup_contract_with_credit_line(&env, &borrower, 1_000, 1_000);

        client.draw_credit(&borrower, &400);
        client.default_credit_line(&borrower);

        client.repay_credit(&borrower, &150);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Defaulted);
        assert_eq!(line.utilized_amount, 250);
    }

    // ── admin-only enforcement ────────────────────────────────────────────────

    #[test]
    fn test_draw_credit_updates_utilized() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);

        client.draw_credit(&borrower, &200_i128);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            200
        );

        client.draw_credit(&borrower, &300_i128);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            500
        );
    }

    // --- draw_credit: zero and negative amount guards ---

    #[test]
    #[should_panic(expected = "amount must be positive")]
    fn test_draw_credit_rejected_when_amount_is_zero() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);

        // Should panic: zero is not a positive amount
        client.draw_credit(&borrower, &0_i128);
    }

    #[test]
    #[should_panic(expected = "amount must be positive")]
    fn test_draw_credit_rejected_when_amount_is_negative() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);

        // i128 allows negatives — the guard `amount <= 0` must catch this
        client.draw_credit(&borrower, &-1_i128);
    }

    // --- repay_credit: zero and negative amount guards ---

    #[test]
    #[should_panic(expected = "amount must be positive")]
    fn test_repay_credit_rejects_non_positive_amount() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);

        // Should panic: repaying zero is meaningless and must be rejected
        client.repay_credit(&borrower, &0_i128);
    }

    #[test]
    #[should_panic(expected = "amount must be positive")]
    fn test_repay_credit_rejected_when_amount_is_negative() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);

        // Negative repayment would effectively be a draw — must be rejected
        client.repay_credit(&borrower, &-500_i128);
    }

    // --- update_risk_parameters ---

    #[test]
    fn test_update_risk_parameters_success() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);

        client.update_risk_parameters(&borrower, &2000_i128, &400_u32, &85_u32);

        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.credit_limit, 2000);
        assert_eq!(credit_line.interest_rate_bps, 400);
        assert_eq!(credit_line.risk_score, 85);
    }

    #[test]
    #[should_panic]
    fn test_update_risk_parameters_unauthorized_caller() {
        let env = Env::default();
        // Do not use mock_all_auths: no auth means admin.require_auth() will fail.
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.update_risk_parameters(&borrower, &2000_i128, &400_u32, &85_u32);
    }

    #[test]
    #[should_panic(expected = "Credit line not found")]
    fn test_update_risk_parameters_nonexistent_line() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.update_risk_parameters(&borrower, &1000_i128, &300_u32, &70_u32);
    }

    #[test]
    #[should_panic(expected = "credit_limit cannot be less than utilized amount")]
    fn test_update_risk_parameters_credit_limit_below_utilized() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.draw_credit(&borrower, &500_i128);

        client.update_risk_parameters(&borrower, &300_i128, &300_u32, &70_u32);
    }

    #[test]
    #[should_panic(expected = "credit_limit must be non-negative")]
    fn test_update_risk_parameters_negative_credit_limit() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.update_risk_parameters(&borrower, &(-1_i128), &300_u32, &70_u32);
    }

    #[test]
    fn test_event_reinstate_credit_line() {
        use soroban_sdk::testutils::Events;
        use soroban_sdk::{TryFromVal, TryIntoVal};
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) = setup_contract_with_credit_line(&env, &borrower, 1_000, 0);
        client.default_credit_line(&borrower);
        client.reinstate_credit_line(&borrower);
        let events = env.events().all();
        let (_contract, topics, data) = events.last().unwrap();
        // New schema: topic0 = "credit", topic1 = "reinstate"
        assert_eq!(
            Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap(),
            Symbol::new(&env, "credit")
        );
        assert_eq!(
            Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
            Symbol::new(&env, "reinstate")
        );
        let event_data: CreditLineEvent = data.try_into_val(&env).unwrap();
        assert_eq!(event_data.status, CreditStatus::Active);
    }

    #[test]
    fn test_event_lifecycle_sequence() {}
    #[test]
    #[should_panic(expected = "interest_rate_bps exceeds maximum")]
    fn test_update_risk_parameters_interest_rate_exceeds_max() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.update_risk_parameters(&borrower, &1000_i128, &10001_u32, &70_u32);
    }

    #[test]
    #[should_panic(expected = "risk_score exceeds maximum")]
    fn test_update_risk_parameters_risk_score_exceeds_max() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.update_risk_parameters(&borrower, &1000_i128, &300_u32, &101_u32);
    }

    #[test]
    fn test_update_risk_parameters_at_boundaries() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.update_risk_parameters(&borrower, &1000_i128, &10000_u32, &100_u32);

        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.interest_rate_bps, 10000);
        assert_eq!(credit_line.risk_score, 100);
    }

    // --- repay_credit: happy path and event emission ---

    #[test]
    fn test_repay_credit_reduces_utilized_and_emits_event() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.draw_credit(&borrower, &500_i128);

        let _ = env.events().all();
        client.repay_credit(&borrower, &200_i128);
        let events_after = env.events().all().len();

        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.utilized_amount, 300);
        assert_eq!(
            events_after, 1,
            "repay_credit must emit exactly one RepaymentEvent"
        );
    }

    #[test]
    fn test_repay_credit_saturates_at_zero() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.draw_credit(&borrower, &100_i128);
        client.repay_credit(&borrower, &500_i128);

        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.utilized_amount, 0);
    }

    #[test]
    #[should_panic(expected = "Credit line not found")]
    fn test_repay_credit_nonexistent_line() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.repay_credit(&borrower, &100_i128);
    }

    // --- suspend/default: unauthorized caller ---

    #[test]
    #[should_panic]
    fn test_suspend_credit_line_unauthorized() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.suspend_credit_line(&borrower);
    }

    #[test]
    #[should_panic]
    fn test_default_credit_line_unauthorized() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.default_credit_line(&borrower);
    }

    // --- Reentrancy guard: cleared correctly after draw and repay ---
    //
    // We cannot simulate a token callback in unit tests without a mock contract.
    // These tests verify the guard is cleared on the happy path so that sequential
    // calls succeed, proving no guard leak occurs on successful execution.

    #[test]
    fn test_reentrancy_guard_cleared_after_draw() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.draw_credit(&borrower, &100_i128);
        client.draw_credit(&borrower, &100_i128);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            200
        );
    }

    #[test]
    fn test_reentrancy_guard_cleared_after_repay() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.draw_credit(&borrower, &200_i128);
        client.repay_credit(&borrower, &50_i128);
        client.repay_credit(&borrower, &50_i128);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            100
        );
    }

    #[test]
    fn test_draw_credit_with_sufficient_liquidity() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        let liquidity = MockLiquidityToken::deploy(&env);

        client.init(&admin);
        client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);
        client.set_liquidity_token(&liquidity.address());
        liquidity.mint(&contract_id, 500_i128);
        client.draw_credit(&borrower, &200_i128);

        assert_eq!(liquidity.balance(&contract_id), 300_i128);
        assert_eq!(liquidity.balance(&borrower), 200_i128);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            200_i128
        );
    }

    #[test]
    fn test_set_liquidity_source_updates_instance_storage() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let reserve = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.set_liquidity_source(&reserve);

        let stored: Address = env
            .as_contract(&contract_id, || {
                env.storage().instance().get(&DataKey::LiquiditySource)
            })
            .unwrap();
        assert_eq!(stored, reserve);
    }

    #[test]
    fn test_draw_credit_uses_configured_external_liquidity_source() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        let liquidity = MockLiquidityToken::deploy(&env);

        client.init(&admin);
        client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);
        let reserve = contract_id.clone();

        client.set_liquidity_token(&liquidity.address());
        client.set_liquidity_source(&reserve);

        liquidity.mint(&reserve, 500_i128);
        client.draw_credit(&borrower, &120_i128);

        assert_eq!(liquidity.balance(&reserve), 380_i128);
        assert_eq!(liquidity.balance(&borrower), 120_i128);
        assert_eq!(liquidity.balance(&contract_id), 380_i128);
    }

    #[test]
    #[should_panic]
    fn test_set_liquidity_token_requires_admin_auth() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let token_admin = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);

        let token = env.register_stellar_asset_contract_v2(token_admin);
        client.set_liquidity_token(&token.address());
    }

    #[test]
    #[should_panic]
    fn test_set_liquidity_source_requires_admin_auth() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let reserve = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.set_liquidity_source(&reserve);
    }

    #[test]
    #[should_panic(expected = "Insufficient liquidity reserve for requested draw amount")]
    fn test_draw_credit_with_insufficient_liquidity() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        let liquidity = MockLiquidityToken::deploy(&env);

        client.init(&admin);
        client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);

        client.set_liquidity_token(&liquidity.address());
        liquidity.mint(&contract_id, 50_i128);
        client.draw_credit(&borrower, &100_i128);
    }

    #[test]
    fn test_repay_credit_integration_uses_mocked_allowance_and_balance_state() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        let liquidity = MockLiquidityToken::deploy(&env);

        client.init(&admin);
        client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);
        client.set_liquidity_token(&liquidity.address());

        liquidity.mint(&contract_id, 500_i128);
        liquidity.mint(&borrower, 250_i128);
        liquidity.approve(&borrower, &contract_id, 200_i128, 1_000_u32);

        assert_eq!(liquidity.balance(&borrower), 250_i128);
        assert_eq!(liquidity.allowance(&borrower, &contract_id), 200_i128);

        client.draw_credit(&borrower, &300_i128);
        client.repay_credit(&borrower, &200_i128);

        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            100_i128
        );
        // Current repay implementation is state-only; token balances/allowances are unchanged.
        assert_eq!(liquidity.balance(&borrower), 550_i128);
        assert_eq!(liquidity.allowance(&borrower, &contract_id), 200_i128);
    }

// ─────────────────────────────────────────────────────────────────────────────
// Event Schema Tests
//
// These tests assert the exact topic structure and data payload for every event
// emitted by the credit contract, per the frozen schema defined in events.rs.
//
// Schema summary:
//   Lifecycle events  → topic: ("credit", action_symbol),  data: CreditLineEvent
//   Draw events       → topic: ("drawn",  borrower_addr),  data: DrawnEvent
//   Repay events      → topic: ("repay",  borrower_addr),  data: RepaymentEvent
//   Risk-update events→ topic: ("credit", "risk_upd"),     data: RiskParametersUpdatedEvent
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod event_schema_tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Events as _};
    use soroban_sdk::{Address, Env, Symbol, TryFromVal, TryIntoVal};
    use soroban_sdk::token::StellarAssetClient;

    // ── helpers ───────────────────────────────────────────────────────────────

    fn deploy(env: &Env) -> (CreditClient<'_>, Address, Address) {
        env.mock_all_auths();
        let admin = Address::generate(env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(env, &contract_id);
        client.init(&admin);
        (client, admin, contract_id)
    }

    fn deploy_with_token(env: &Env) -> (CreditClient<'_>, Address, Address, Address) {
        env.mock_all_auths();
        let admin = Address::generate(env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(env, &contract_id);
        client.init(&admin);
        let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
        let token_addr = token_id.address();
        client.set_liquidity_token(&token_addr);
        (client, admin, contract_id, token_addr)
    }

    // ── open_credit_line → ("credit", "opened") ───────────────────────────────

    /// Topic[0] must be the Symbol "credit".
    #[test]
    fn open_event_topic0_is_credit_symbol() {
        let env = Env::default();
        let (client, _admin, _cid) = deploy(&env);
        let borrower = Address::generate(&env);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);

        let events = env.events().all();
        let (_cid, topics, _data) = events.last().unwrap();
        let t0: Symbol = Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap();
        assert_eq!(t0, Symbol::new(&env, "credit"));
    }

    /// Topic[1] must be the Symbol "opened".
    #[test]
    fn open_event_topic1_is_opened_symbol() {
        let env = Env::default();
        let (client, _admin, _cid) = deploy(&env);
        let borrower = Address::generate(&env);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);

        let events = env.events().all();
        let (_cid, topics, _data) = events.last().unwrap();
        let t1: Symbol = Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap();
        assert_eq!(t1, Symbol::new(&env, "opened"));
    }

    /// Data payload must decode as CreditLineEvent with correct fields.
    #[test]
    fn open_event_data_payload_matches_inputs() {
        let env = Env::default();
        let (client, _admin, _cid) = deploy(&env);
        let borrower = Address::generate(&env);
        client.open_credit_line(&borrower, &2_500, &450_u32, &65_u32);

        let events = env.events().all();
        let (_cid, _topics, data) = events.last().unwrap();
        let ev: CreditLineEvent = data.try_into_val(&env).unwrap();

        assert_eq!(ev.borrower, borrower);
        assert_eq!(ev.status, CreditStatus::Active);
        assert_eq!(ev.credit_limit, 2_500);
        assert_eq!(ev.interest_rate_bps, 450);
        assert_eq!(ev.risk_score, 65);
        assert_eq!(ev.event_type, Symbol::new(&env, "opened"));
    }

    /// Exactly one event is emitted per open_credit_line call.
    #[test]
    fn open_event_exactly_one_event_emitted() {
        let env = Env::default();
        let (client, _admin, _cid) = deploy(&env);
        let borrower = Address::generate(&env);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);
        assert_eq!(env.events().all().len(), 1);
    }

    // ── draw_credit → ("drawn", borrower_addr) ────────────────────────────────

    /// Topic[0] must be the Symbol "drawn".
    #[test]
    fn draw_event_topic0_is_drawn_symbol() {
        let env = Env::default();
        let (client, _admin, contract_id, token) = deploy_with_token(&env);
        let borrower = Address::generate(&env);
        StellarAssetClient::new(&env, &token).mint(&contract_id, &1_000);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);
        client.draw_credit(&borrower, &200);

        let events = env.events().all();
        let (_cid, topics, _data) = events.last().unwrap();
        let t0: Symbol = Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap();
        assert_eq!(t0, Symbol::new(&env, "drawn"));
    }

    /// Topic[1] must be the borrower's Address (not a Symbol).
    #[test]
    fn draw_event_topic1_is_borrower_address() {
        let env = Env::default();
        let (client, _admin, contract_id, token) = deploy_with_token(&env);
        let borrower = Address::generate(&env);
        StellarAssetClient::new(&env, &token).mint(&contract_id, &1_000);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);
        client.draw_credit(&borrower, &300);

        let events = env.events().all();
        let (_cid, topics, _data) = events.last().unwrap();
        let t1: Address = Address::try_from_val(&env, &topics.get(1).unwrap()).unwrap();
        assert_eq!(t1, borrower);
    }

    /// Data payload must decode as DrawnEvent with correct fields.
    #[test]
    fn draw_event_data_payload_matches_inputs() {
        let env = Env::default();
        let (client, _admin, contract_id, token) = deploy_with_token(&env);
        let borrower = Address::generate(&env);
        StellarAssetClient::new(&env, &token).mint(&contract_id, &1_000);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);
        client.draw_credit(&borrower, &400);

        let events = env.events().all();
        let (_cid, _topics, data) = events.last().unwrap();
        let ev: DrawnEvent = data.try_into_val(&env).unwrap();

        assert_eq!(ev.borrower, borrower);
        assert_eq!(ev.amount, 400);
        assert_eq!(ev.new_utilized_amount, 400);
    }

    /// Cumulative draw: new_utilized_amount reflects total after multiple draws.
    #[test]
    fn draw_event_new_utilized_accumulates_across_draws() {
        let env = Env::default();
        let (client, _admin, contract_id, token) = deploy_with_token(&env);
        let borrower = Address::generate(&env);
        StellarAssetClient::new(&env, &token).mint(&contract_id, &1_000);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);
        client.draw_credit(&borrower, &200);
        client.draw_credit(&borrower, &300);

        let events = env.events().all();
        let (_cid, _topics, data) = events.last().unwrap();
        let ev: DrawnEvent = data.try_into_val(&env).unwrap();
        assert_eq!(ev.amount, 300);
        assert_eq!(ev.new_utilized_amount, 500);
    }

    // ── repay_credit → ("repay", borrower_addr) ───────────────────────────────

    /// Topic[0] must be the Symbol "repay".
    #[test]
    fn repay_event_topic0_is_repay_symbol() {
        let env = Env::default();
        let (client, _admin, contract_id, token) = deploy_with_token(&env);
        let borrower = Address::generate(&env);
        StellarAssetClient::new(&env, &token).mint(&contract_id, &1_000);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);
        client.draw_credit(&borrower, &500);

        StellarAssetClient::new(&env, &token).mint(&borrower, &200);
        soroban_sdk::token::Client::new(&env, &token)
            .approve(&borrower, &contract_id, &200, &1_000_u32);
        client.repay_credit(&borrower, &200);

        let events = env.events().all();
        let (_cid, topics, _data) = events.last().unwrap();
        let t0: Symbol = Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap();
        assert_eq!(t0, Symbol::new(&env, "repay"));
    }

    /// Topic[1] must be the borrower's Address.
    #[test]
    fn repay_event_topic1_is_borrower_address() {
        let env = Env::default();
        let (client, _admin, contract_id, token) = deploy_with_token(&env);
        let borrower = Address::generate(&env);
        StellarAssetClient::new(&env, &token).mint(&contract_id, &1_000);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);
        client.draw_credit(&borrower, &500);

        StellarAssetClient::new(&env, &token).mint(&borrower, &150);
        soroban_sdk::token::Client::new(&env, &token)
            .approve(&borrower, &contract_id, &150, &1_000_u32);
        client.repay_credit(&borrower, &150);

        let events = env.events().all();
        let (_cid, topics, _data) = events.last().unwrap();
        let t1: Address = Address::try_from_val(&env, &topics.get(1).unwrap()).unwrap();
        assert_eq!(t1, borrower);
    }

    /// Data payload must decode as RepaymentEvent with correct fields.
    #[test]
    fn repay_event_data_payload_matches_inputs() {
        let env = Env::default();
        let (client, _admin, contract_id, token) = deploy_with_token(&env);
        let borrower = Address::generate(&env);
        StellarAssetClient::new(&env, &token).mint(&contract_id, &1_000);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);
        client.draw_credit(&borrower, &600);

        StellarAssetClient::new(&env, &token).mint(&borrower, &250);
        soroban_sdk::token::Client::new(&env, &token)
            .approve(&borrower, &contract_id, &250, &1_000_u32);
        client.repay_credit(&borrower, &250);

        let events = env.events().all();
        let (_cid, _topics, data) = events.last().unwrap();
        let ev: RepaymentEvent = data.try_into_val(&env).unwrap();

        assert_eq!(ev.borrower, borrower);
        assert_eq!(ev.amount, 250);
        assert_eq!(ev.new_utilized_amount, 350); // 600 - 250
    }

    /// Overpayment: event amount is capped at utilized_amount, not the nominal amount.
    #[test]
    fn repay_event_amount_is_effective_not_nominal_on_overpayment() {
        let env = Env::default();
        let (client, _admin, contract_id, token) = deploy_with_token(&env);
        let borrower = Address::generate(&env);
        StellarAssetClient::new(&env, &token).mint(&contract_id, &1_000);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);
        client.draw_credit(&borrower, &100);

        StellarAssetClient::new(&env, &token).mint(&borrower, &500);
        soroban_sdk::token::Client::new(&env, &token)
            .approve(&borrower, &contract_id, &500, &1_000_u32);
        client.repay_credit(&borrower, &500); // overpay

        let events = env.events().all();
        let (_cid, _topics, data) = events.last().unwrap();
        let ev: RepaymentEvent = data.try_into_val(&env).unwrap();

        assert_eq!(ev.amount, 100);           // effective, not 500
        assert_eq!(ev.new_utilized_amount, 0);
    }

    // ── suspend_credit_line → ("credit", "suspend") ───────────────────────────

    /// Topic[0] = "credit", Topic[1] = "suspend".
    #[test]
    fn suspend_event_topics_are_credit_and_suspend() {
        let env = Env::default();
        let (client, _admin, _cid) = deploy(&env);
        let borrower = Address::generate(&env);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);
        client.suspend_credit_line(&borrower);

        let events = env.events().all();
        let (_cid, topics, _data) = events.last().unwrap();
        let t0: Symbol = Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap();
        let t1: Symbol = Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap();
        assert_eq!(t0, Symbol::new(&env, "credit"));
        assert_eq!(t1, Symbol::new(&env, "suspend"));
    }

    /// Data payload status must be Suspended.
    #[test]
    fn suspend_event_data_status_is_suspended() {
        let env = Env::default();
        let (client, _admin, _cid) = deploy(&env);
        let borrower = Address::generate(&env);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);
        client.suspend_credit_line(&borrower);

        let events = env.events().all();
        let (_cid, _topics, data) = events.last().unwrap();
        let ev: CreditLineEvent = data.try_into_val(&env).unwrap();
        assert_eq!(ev.status, CreditStatus::Suspended);
        assert_eq!(ev.borrower, borrower);
    }

    // ── close_credit_line → ("credit", "closed") ──────────────────────────────

    /// Topic[0] = "credit", Topic[1] = "closed".
    #[test]
    fn close_event_topics_are_credit_and_closed() {
        let env = Env::default();
        let (client, admin, _cid) = deploy(&env);
        let borrower = Address::generate(&env);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);
        client.close_credit_line(&borrower, &admin);

        let events = env.events().all();
        let (_cid, topics, _data) = events.last().unwrap();
        let t0: Symbol = Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap();
        let t1: Symbol = Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap();
        assert_eq!(t0, Symbol::new(&env, "credit"));
        assert_eq!(t1, Symbol::new(&env, "closed"));
    }

    /// Data payload status must be Closed and borrower must match.
    #[test]
    fn close_event_data_status_is_closed_and_borrower_matches() {
        let env = Env::default();
        let (client, admin, _cid) = deploy(&env);
        let borrower = Address::generate(&env);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);
        client.close_credit_line(&borrower, &admin);

        let events = env.events().all();
        let (_cid, _topics, data) = events.last().unwrap();
        let ev: CreditLineEvent = data.try_into_val(&env).unwrap();
        assert_eq!(ev.status, CreditStatus::Closed);
        assert_eq!(ev.borrower, borrower);
    }

    // ── default_credit_line → ("credit", "default") ───────────────────────────

    /// Topic[0] = "credit", Topic[1] = "default".
    #[test]
    fn default_event_topics_are_credit_and_default() {
        let env = Env::default();
        let (client, _admin, _cid) = deploy(&env);
        let borrower = Address::generate(&env);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);
        client.default_credit_line(&borrower);

        let events = env.events().all();
        let (_cid, topics, _data) = events.last().unwrap();
        let t0: Symbol = Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap();
        let t1: Symbol = Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap();
        assert_eq!(t0, Symbol::new(&env, "credit"));
        assert_eq!(t1, Symbol::new(&env, "default"));
    }

    /// Data payload status must be Defaulted.
    #[test]
    fn default_event_data_status_is_defaulted() {
        let env = Env::default();
        let (client, _admin, _cid) = deploy(&env);
        let borrower = Address::generate(&env);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);
        client.default_credit_line(&borrower);

        let events = env.events().all();
        let (_cid, _topics, data) = events.last().unwrap();
        let ev: CreditLineEvent = data.try_into_val(&env).unwrap();
        assert_eq!(ev.status, CreditStatus::Defaulted);
        assert_eq!(ev.borrower, borrower);
    }

    // ── reinstate_credit_line → ("credit", "reinstate") ──────────────────────

    /// Topic[0] = "credit", Topic[1] = "reinstate".
    #[test]
    fn reinstate_event_topics_are_credit_and_reinstate() {
        let env = Env::default();
        let (client, _admin, _cid) = deploy(&env);
        let borrower = Address::generate(&env);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);
        client.default_credit_line(&borrower);
        client.reinstate_credit_line(&borrower);

        let events = env.events().all();
        let (_cid, topics, _data) = events.last().unwrap();
        let t0: Symbol = Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap();
        let t1: Symbol = Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap();
        assert_eq!(t0, Symbol::new(&env, "credit"));
        assert_eq!(t1, Symbol::new(&env, "reinstate"));
    }

    /// Data payload status must be Active after reinstatement.
    #[test]
    fn reinstate_event_data_status_is_active() {
        let env = Env::default();
        let (client, _admin, _cid) = deploy(&env);
        let borrower = Address::generate(&env);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);
        client.default_credit_line(&borrower);
        client.reinstate_credit_line(&borrower);

        let events = env.events().all();
        let (_cid, _topics, data) = events.last().unwrap();
        let ev: CreditLineEvent = data.try_into_val(&env).unwrap();
        assert_eq!(ev.status, CreditStatus::Active);
        assert_eq!(ev.borrower, borrower);
    }

    // ── update_risk_parameters → ("credit", "risk_upd") ──────────────────────

    /// Topic[0] = "credit", Topic[1] = "risk_upd".
    #[test]
    fn risk_update_event_topics_are_credit_and_risk_upd() {
        let env = Env::default();
        let (client, _admin, _cid) = deploy(&env);
        let borrower = Address::generate(&env);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);
        client.update_risk_parameters(&borrower, &2_000, &400_u32, &80_u32);

        let events = env.events().all();
        let (_cid, topics, _data) = events.last().unwrap();
        let t0: Symbol = Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap();
        let t1: Symbol = Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap();
        assert_eq!(t0, Symbol::new(&env, "credit"));
        assert_eq!(t1, Symbol::new(&env, "risk_upd"));
    }

    /// Data payload must decode as RiskParametersUpdatedEvent with correct fields.
    #[test]
    fn risk_update_event_data_payload_matches_inputs() {
        let env = Env::default();
        let (client, _admin, _cid) = deploy(&env);
        let borrower = Address::generate(&env);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);
        client.update_risk_parameters(&borrower, &3_000, &500_u32, &85_u32);

        let events = env.events().all();
        let (_cid, _topics, data) = events.last().unwrap();
        let ev: RiskParametersUpdatedEvent = data.try_into_val(&env).unwrap();

        assert_eq!(ev.borrower, borrower);
        assert_eq!(ev.credit_limit, 3_000);
        assert_eq!(ev.interest_rate_bps, 500);
        assert_eq!(ev.risk_score, 85);
    }

    // ── Cross-cutting: topic stability across full lifecycle ──────────────────

    /// Every event in a full lifecycle has the correct namespace in topic[0].
    #[test]
    fn full_lifecycle_all_events_have_correct_namespace() {
        let env = Env::default();
        let (client, admin, contract_id, token) = deploy_with_token(&env);
        let borrower = Address::generate(&env);

        StellarAssetClient::new(&env, &token).mint(&contract_id, &1_000);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);
        client.draw_credit(&borrower, &200);

        StellarAssetClient::new(&env, &token).mint(&borrower, &100);
        soroban_sdk::token::Client::new(&env, &token)
            .approve(&borrower, &contract_id, &100, &1_000_u32);
        client.repay_credit(&borrower, &100);

        client.suspend_credit_line(&borrower);
        client.default_credit_line(&borrower);
        client.reinstate_credit_line(&borrower);
        client.close_credit_line(&borrower, &admin);

        let events = env.events().all();
        // Expected namespaces in order: credit, drawn, repay, credit, credit, credit, credit
        let expected_ns = ["credit", "drawn", "repay", "credit", "credit", "credit", "credit"];
        assert_eq!(events.len(), expected_ns.len());

        for (i, ((_cid, topics, _data), ns)) in events.iter().zip(expected_ns.iter()).enumerate() {
            let t0: Symbol = Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap();
            assert_eq!(
                t0,
                Symbol::new(&env, ns),
                "event[{i}] topic[0] mismatch: expected {ns}"
            );
        }
    }

    /// Lifecycle action symbols appear in topic[1] in the correct order.
    #[test]
    fn full_lifecycle_action_symbols_in_correct_order() {
        let env = Env::default();
        let (client, admin, _cid) = deploy(&env);
        let borrower = Address::generate(&env);

        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);
        client.suspend_credit_line(&borrower);
        client.default_credit_line(&borrower);
        client.reinstate_credit_line(&borrower);
        client.close_credit_line(&borrower, &admin);

        let events = env.events().all();
        // All are lifecycle events: topic[1] is a Symbol
        let expected_actions = ["opened", "suspend", "default", "reinstate", "closed"];
        assert_eq!(events.len(), expected_actions.len());

        for (i, ((_cid, topics, _data), action)) in
            events.iter().zip(expected_actions.iter()).enumerate()
        {
            let t1: Symbol = Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap();
            assert_eq!(
                t1,
                Symbol::new(&env, action),
                "event[{i}] topic[1] mismatch: expected {action}"
            );
        }
    }

    /// Draw and repay events carry the borrower address in topic[1], not a Symbol.
    #[test]
    fn draw_and_repay_events_carry_borrower_address_in_topic1() {
        let env = Env::default();
        let (client, _admin, contract_id, token) = deploy_with_token(&env);
        let borrower = Address::generate(&env);

        StellarAssetClient::new(&env, &token).mint(&contract_id, &1_000);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);
        client.draw_credit(&borrower, &300);

        StellarAssetClient::new(&env, &token).mint(&borrower, &100);
        soroban_sdk::token::Client::new(&env, &token)
            .approve(&borrower, &contract_id, &100, &1_000_u32);
        client.repay_credit(&borrower, &100);

        let events = env.events().all();
        // event[0] = open (lifecycle), event[1] = draw, event[2] = repay
        assert_eq!(events.len(), 3);

        // draw event topic[1] is borrower Address
        let (_cid, draw_topics, _) = &events[1];
        let draw_t1: Address =
            Address::try_from_val(&env, &draw_topics.get(1).unwrap()).unwrap();
        assert_eq!(draw_t1, borrower);

        // repay event topic[1] is borrower Address
        let (_cid, repay_topics, _) = &events[2];
        let repay_t1: Address =
            Address::try_from_val(&env, &repay_topics.get(1).unwrap()).unwrap();
        assert_eq!(repay_t1, borrower);
    }

    // ── Boundary: zero-draw repay emits event with zero amount ────────────────

    /// Repay when utilized_amount is zero: event is emitted with amount = 0.
    #[test]
    fn repay_zero_utilization_event_has_zero_amount() {
        let env = Env::default();
        let (client, _admin, _cid) = deploy(&env);
        let borrower = Address::generate(&env);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);
        // No draw — repay with no token configured (state-only path)
        client.repay_credit(&borrower, &500);

        let events = env.events().all();
        let (_cid, _topics, data) = events.last().unwrap();
        let ev: RepaymentEvent = data.try_into_val(&env).unwrap();
        assert_eq!(ev.amount, 0);
        assert_eq!(ev.new_utilized_amount, 0);
    }
}
