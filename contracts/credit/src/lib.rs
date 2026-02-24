#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, Env, Symbol};

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreditStatus {
    Active = 0,
    Suspended = 1,
    Defaulted = 2,
    Closed = 3,
}

#[contracttype]
pub struct CreditLineData {
    pub borrower: Address,
    pub credit_limit: i128,
    pub utilized_amount: i128,
    pub interest_rate_bps: u32,
    pub risk_score: u32,
    pub status: CreditStatus,
}

/// Event emitted when a credit line lifecycle event occurs
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreditLineEvent {
    pub event_type: Symbol,
    pub borrower: Address,
    pub status: CreditStatus,
    pub credit_limit: i128,
    pub interest_rate_bps: u32,
    pub risk_score: u32,
}

#[contract]
pub struct Credit;

#[contractimpl]
impl Credit {
    /// Initialize the contract (admin).
    pub fn init(env: Env, admin: Address) -> () {
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "admin"), &admin);
        ()
    }

    /// Open a new credit line for a borrower (called by backend/risk engine).
    /// Emits a CreditLineOpened event.
    pub fn open_credit_line(
        env: Env,
        borrower: Address,
        credit_limit: i128,
        interest_rate_bps: u32,
        risk_score: u32,
    ) -> () {
        let credit_line = CreditLineData {
            borrower: borrower.clone(),
            credit_limit,
            utilized_amount: 0,
            interest_rate_bps,
            risk_score,
            status: CreditStatus::Active,
        };

        env.storage().persistent().set(&borrower, &credit_line);

        // Emit CreditLineOpened event
        env.events().publish(
            (symbol_short!("credit"), symbol_short!("opened")),
            CreditLineEvent {
                event_type: symbol_short!("opened"),
                borrower: borrower.clone(),
                status: CreditStatus::Active,
                credit_limit,
                interest_rate_bps,
                risk_score,
            },
        );
        ()
    }

    /// @notice Draws credit for a borrower with an active credit line.
    /// @dev Enforces positive amount, active status, remaining-limit bounds, and overflow-safe math.
    /// @param borrower Borrower address that must authorize this call.
    /// @param amount Draw amount; must be strictly positive.
    ///
    /// # Arguments
    /// * `borrower` - Borrower address that must authorize this call.
    /// * `amount` - Draw amount; must be strictly positive.
    ///
    /// # Reverts
    /// * If the credit line does not exist.
    /// * If `borrower` is not authorized.
    /// * If the credit line status is not [`CreditStatus::Active`].
    /// * If `amount <= 0`.
    /// * If `amount` exceeds the remaining limit (`credit_limit - utilized_amount`).
    /// * If arithmetic overflows while computing remaining/utilized amounts.
    ///
    /// Note: Token transfer integration is intentionally deferred.
    pub fn draw_credit(env: Env, borrower: Address, amount: i128) -> () {
        borrower.require_auth();
        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
            .expect("Credit line not found");
        if credit_line.status != CreditStatus::Active {
            if credit_line.status == CreditStatus::Closed {
                panic!("credit line is closed");
            }
            panic!("credit line is not active");
        }
        if amount <= 0 {
            panic!("amount must be positive");
        }
        let remaining_limit = credit_line
            .credit_limit
            .checked_sub(credit_line.utilized_amount)
            .expect("invalid credit line accounting");
        if amount > remaining_limit {
            panic!("exceeds credit limit");
        }
        let new_utilized = credit_line
            .utilized_amount
            .checked_add(amount)
            .expect("overflow");
        credit_line.utilized_amount = new_utilized;
        env.storage().persistent().set(&borrower, &credit_line);
        // TODO: transfer token to borrower
        ()
    }

    /// Repay credit (borrower).
    /// Reverts if credit line does not exist, is Closed, or borrower has not authorized.
    pub fn repay_credit(env: Env, borrower: Address, _amount: i128) -> () {
        borrower.require_auth();
        let credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
            .expect("Credit line not found");
        if credit_line.status == CreditStatus::Closed {
            panic!("credit line is closed");
        }
        // TODO: accept token, reduce utilized_amount, accrue interest
        ()
    }

    /// Update risk parameters (admin/risk engine).
    pub fn update_risk_parameters(
        _env: Env,
        _borrower: Address,
        _credit_limit: i128,
        _interest_rate_bps: u32,
        _risk_score: u32,
    ) -> () {
        // TODO: update stored CreditLineData
        ()
    }

    /// Suspend a credit line (admin).
    /// Emits a CreditLineSuspended event.
    pub fn suspend_credit_line(env: Env, borrower: Address) -> () {
        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
            .expect("Credit line not found");

        credit_line.status = CreditStatus::Suspended;
        env.storage().persistent().set(&borrower, &credit_line);

        // Emit CreditLineSuspended event
        env.events().publish(
            (symbol_short!("credit"), symbol_short!("suspend")),
            CreditLineEvent {
                event_type: symbol_short!("suspend"),
                borrower: borrower.clone(),
                status: CreditStatus::Suspended,
                credit_limit: credit_line.credit_limit,
                interest_rate_bps: credit_line.interest_rate_bps,
                risk_score: credit_line.risk_score,
            },
        );
        ()
    }

    /// Close a credit line. Callable by admin (force-close) or by borrower when utilization is zero.
    ///
    /// # Arguments
    /// * `closer` - Address that must have authorized this call. Must be either the contract admin
    ///   (can close regardless of utilization) or the borrower (can close only when
    ///   `utilized_amount` is zero).
    ///
    /// # Errors
    /// * Panics if credit line does not exist, or if `closer` is not admin/borrower, or if
    ///   borrower closes while `utilized_amount != 0`.
    ///
    /// Emits a CreditLineClosed event.
    pub fn close_credit_line(env: Env, borrower: Address, closer: Address) -> () {
        closer.require_auth();

        let admin: Address = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, "admin"))
            .expect("admin not set");

        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
            .expect("Credit line not found");

        if credit_line.status == CreditStatus::Closed {
            return ();
        }

        let allowed = closer == admin || (closer == borrower && credit_line.utilized_amount == 0);

        if !allowed {
            if closer == borrower {
                panic!("cannot close: utilized amount not zero");
            }
            panic!("unauthorized");
        }

        credit_line.status = CreditStatus::Closed;
        env.storage().persistent().set(&borrower, &credit_line);

        env.events().publish(
            (symbol_short!("credit"), symbol_short!("closed")),
            CreditLineEvent {
                event_type: symbol_short!("closed"),
                borrower: borrower.clone(),
                status: CreditStatus::Closed,
                credit_limit: credit_line.credit_limit,
                interest_rate_bps: credit_line.interest_rate_bps,
                risk_score: credit_line.risk_score,
            },
        );
        ()
    }

    /// Mark a credit line as defaulted (admin).
    /// Emits a CreditLineDefaulted event.
    pub fn default_credit_line(env: Env, borrower: Address) -> () {
        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
            .expect("Credit line not found");

        credit_line.status = CreditStatus::Defaulted;
        env.storage().persistent().set(&borrower, &credit_line);

        // Emit CreditLineDefaulted event
        env.events().publish(
            (symbol_short!("credit"), symbol_short!("default")),
            CreditLineEvent {
                event_type: symbol_short!("default"),
                borrower: borrower.clone(),
                status: CreditStatus::Defaulted,
                credit_limit: credit_line.credit_limit,
                interest_rate_bps: credit_line.interest_rate_bps,
                risk_score: credit_line.risk_score,
            },
        );
        ()
    }

    /// Get credit line data for a borrower (view function).
    pub fn get_credit_line(env: Env, borrower: Address) -> Option<CreditLineData> {
        env.storage().persistent().get(&borrower)
    }
}

#[cfg(test)]
mod test {
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
        client.close_credit_line(&borrower, &admin);

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

    // --- close_credit_line: admin vs borrower, utilization ---

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
    #[should_panic(expected = "credit line is not active")]
    fn test_draw_credit_rejected_when_suspended() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.suspend_credit_line(&borrower);

        client.draw_credit(&borrower, &100_i128);
    }

    #[test]
    #[should_panic(expected = "credit line is not active")]
    fn test_draw_credit_rejected_when_defaulted() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.default_credit_line(&borrower);

        client.draw_credit(&borrower, &100_i128);
    }

    #[test]
    #[should_panic(expected = "amount must be positive")]
    fn test_draw_credit_rejected_when_amount_zero() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);

        client.draw_credit(&borrower, &0_i128);
    }

    #[test]
    #[should_panic(expected = "exceeds credit limit")]
    fn test_draw_credit_rejected_when_exceeds_remaining_limit() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.draw_credit(&borrower, &600_i128);
        client.draw_credit(&borrower, &300_i128);

        // Remaining limit is 100. Drawing 101 must revert.
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
}
