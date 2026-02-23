#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, token, Address, Env, Symbol,
};

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
    /// Sets admin and the Stellar token contract used for repayments.
    pub fn init(env: Env, admin: Address, token: Address) -> () {
        env.storage().instance().set(&Symbol::new(&env, "admin"), &admin);
        env.storage().instance().set(&Symbol::new(&env, "token"), &token);
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

        env.storage()
            .persistent()
            .set(&borrower, &credit_line);

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

    /// Draw from credit line (borrower).
    pub fn draw_credit(_env: Env, _borrower: Address, _amount: i128) -> () {
        // TODO: check limit, update utilized_amount, transfer token to borrower
        ()
    }

    /// Transfers `amount` from borrower to this contract via token transfer_from, then reduces utilized_amount.
    /// Caller must be the borrower. Fails if credit line missing, not Active/Suspended, amount <= 0, or amount > utilized_amount.
    pub fn repay_credit(env: Env, borrower: Address, amount: i128) -> () {
        borrower.require_auth();
        let token: Address = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, "token"))
            .expect("token not set");
        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
            .expect("Credit line not found");
        if credit_line.status != CreditStatus::Active && credit_line.status != CreditStatus::Suspended
        {
            panic!("Credit line not repayable");
        }
        if amount <= 0 {
            panic!("amount must be positive");
        }
        if amount > credit_line.utilized_amount {
            panic!("amount exceeds utilized");
        }
        let contract = env.current_contract_address();
        let token_client = token::Client::new(&env, &token);
        token_client.transfer_from(&contract, &borrower, &contract, &amount);
        credit_line.utilized_amount -= amount;
        env.storage().persistent().set(&borrower, &credit_line);
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
        env.storage()
            .persistent()
            .set(&borrower, &credit_line);

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

    /// Close a credit line (admin or borrower when utilized is 0).
    /// Emits a CreditLineClosed event.
    pub fn close_credit_line(env: Env, borrower: Address) -> () {
        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
            .expect("Credit line not found");

        credit_line.status = CreditStatus::Closed;
        env.storage()
            .persistent()
            .set(&borrower, &credit_line);

        // Emit CreditLineClosed event
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
        env.storage()
            .persistent()
            .set(&borrower, &credit_line);

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

    pub fn get_credit_line(env: Env, borrower: Address) -> Option<CreditLineData> {
        env.storage().persistent().get(&borrower)
    }

    #[cfg(test)]
    pub fn test_set_utilized(env: Env, borrower: Address, amount: i128) -> () {
        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
            .expect("Credit line not found");
        credit_line.utilized_amount = amount;
        env.storage().persistent().set(&borrower, &credit_line);
        ()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::token::StellarAssetClient;

    fn setup_contract(env: &Env) -> (Address, Address, Address, CreditClient<'_>) {
        let admin = Address::generate(env);
        let token = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(env, &contract_id);
        client.init(&admin, &token);
        (admin, token, contract_id, client)
    }

    #[test]
    fn test_init_and_open_credit_line() {
        let env = Env::default();
        env.mock_all_auths();
        let (_, _, _, client) = setup_contract(&env);
        let borrower = Address::generate(&env);

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
        let (_, _, _, client) = setup_contract(&env);
        let borrower = Address::generate(&env);

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
        let (_, _, _, client) = setup_contract(&env);
        let borrower = Address::generate(&env);

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
        let (_, _, _, client) = setup_contract(&env);
        let borrower = Address::generate(&env);

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
        let (_, _, _, client) = setup_contract(&env);
        let borrower = Address::generate(&env);

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
        let (_, _, _, client) = setup_contract(&env);
        let borrower = Address::generate(&env);

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
        env.mock_all_auths();
        let (_, _, _, client) = setup_contract(&env);
        let borrower = Address::generate(&env);

        client.suspend_credit_line(&borrower);
    }

    #[test]
    #[should_panic(expected = "Credit line not found")]
    fn test_close_nonexistent_credit_line() {
        let env = Env::default();
        env.mock_all_auths();
        let (_, _, _, client) = setup_contract(&env);
        let borrower = Address::generate(&env);

        client.close_credit_line(&borrower);
    }

    #[test]
    #[should_panic(expected = "Credit line not found")]
    fn test_default_nonexistent_credit_line() {
        let env = Env::default();
        env.mock_all_auths();
        let (_, _, _, client) = setup_contract(&env);
        let borrower = Address::generate(&env);

        client.default_credit_line(&borrower);
    }

    #[test]
    fn test_multiple_borrowers() {
        let env = Env::default();
        env.mock_all_auths();
        let (_, _, _, client) = setup_contract(&env);
        let borrower1 = Address::generate(&env);
        let borrower2 = Address::generate(&env);

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
        let (_, _, _, client) = setup_contract(&env);
        let borrower = Address::generate(&env);

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
    fn test_repay_credit_success() {
        let env = Env::default();
        env.mock_all_auths();
        let (_admin, token, contract_id, client) = setup_contract(&env);
        let borrower = Address::generate(&env);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.test_set_utilized(&borrower, &500_i128);

        let stellar_token = StellarAssetClient::new(&env, &token);
        stellar_token.mint(&borrower, &500_i128);
        let token_client = token::Client::new(&env, &token);
        let exp = env.ledger().sequence() + 100;
        token_client.approve(&borrower, &contract_id, &500_i128, &exp);

        client.repay_credit(&borrower, &300_i128);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.utilized_amount, 200);
        assert_eq!(token_client.balance(&contract_id), 300);
        assert_eq!(token_client.balance(&borrower), 200);
    }

    #[test]
    fn test_repay_credit_full_repay() {
        let env = Env::default();
        env.mock_all_auths();
        let (_, token, contract_id, client) = setup_contract(&env);
        let borrower = Address::generate(&env);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.test_set_utilized(&borrower, &400_i128);

        let stellar_token = StellarAssetClient::new(&env, &token);
        stellar_token.mint(&borrower, &400_i128);
        let token_client = token::Client::new(&env, &token);
        let exp = env.ledger().sequence() + 100;
        token_client.approve(&borrower, &contract_id, &400_i128, &exp);

        client.repay_credit(&borrower, &400_i128);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.utilized_amount, 0);
        assert_eq!(token_client.balance(&contract_id), 400);
    }

    #[test]
    #[should_panic(expected = "Credit line not found")]
    fn test_repay_credit_nonexistent_line() {
        let env = Env::default();
        env.mock_all_auths();
        let (_, _, _, client) = setup_contract(&env);
        let borrower = Address::generate(&env);
        client.repay_credit(&borrower, &100_i128);
    }

    #[test]
    #[should_panic(expected = "amount must be positive")]
    fn test_repay_credit_zero_amount() {
        let env = Env::default();
        env.mock_all_auths();
        let (_, _, _, client) = setup_contract(&env);
        let borrower = Address::generate(&env);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.test_set_utilized(&borrower, &100_i128);
        client.repay_credit(&borrower, &0_i128);
    }

    #[test]
    #[should_panic(expected = "amount exceeds utilized")]
    fn test_repay_credit_amount_exceeds_utilized() {
        let env = Env::default();
        env.mock_all_auths();
        let (_, token, contract_id, client) = setup_contract(&env);
        let borrower = Address::generate(&env);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.test_set_utilized(&borrower, &100_i128);

        let stellar_token = StellarAssetClient::new(&env, &token);
        stellar_token.mint(&borrower, &500_i128);
        let token_client = token::Client::new(&env, &token);
        let exp = env.ledger().sequence() + 100;
        token_client.approve(&borrower, &contract_id, &500_i128, &exp);

        client.repay_credit(&borrower, &200_i128);
    }

    #[test]
    #[should_panic(expected = "Credit line not repayable")]
    fn test_repay_credit_closed_not_repayable() {
        let env = Env::default();
        env.mock_all_auths();
        let (_, _, _, client) = setup_contract(&env);
        let borrower = Address::generate(&env);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.test_set_utilized(&borrower, &100_i128);
        client.close_credit_line(&borrower);
        client.repay_credit(&borrower, &100_i128);
    }

    #[test]
    #[should_panic(expected = "Credit line not repayable")]
    fn test_repay_credit_defaulted_not_repayable() {
        let env = Env::default();
        env.mock_all_auths();
        let (_, _, _, client) = setup_contract(&env);
        let borrower = Address::generate(&env);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.test_set_utilized(&borrower, &100_i128);
        client.default_credit_line(&borrower);
        client.repay_credit(&borrower, &100_i128);
    }

    #[test]
    fn test_repay_credit_suspended_allowed() {
        let env = Env::default();
        env.mock_all_auths();
        let (_, token, contract_id, client) = setup_contract(&env);
        let borrower = Address::generate(&env);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.test_set_utilized(&borrower, &200_i128);
        client.suspend_credit_line(&borrower);

        let stellar_token = StellarAssetClient::new(&env, &token);
        stellar_token.mint(&borrower, &200_i128);
        let token_client = token::Client::new(&env, &token);
        let exp = env.ledger().sequence() + 100;
        token_client.approve(&borrower, &contract_id, &200_i128, &exp);

        client.repay_credit(&borrower, &200_i128);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.utilized_amount, 0);
        assert_eq!(token_client.balance(&contract_id), 200);
    }

    #[test]
    #[should_panic]
    fn test_repay_credit_insufficient_allowance() {
        let env = Env::default();
        env.mock_all_auths();
        let (_, token, _, client) = setup_contract(&env);
        let borrower = Address::generate(&env);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.test_set_utilized(&borrower, &100_i128);
        let stellar_token = StellarAssetClient::new(&env, &token);
        stellar_token.mint(&borrower, &100_i128);
        client.repay_credit(&borrower, &100_i128);
    }

    #[test]
    #[should_panic]
    fn test_repay_credit_insufficient_balance() {
        let env = Env::default();
        env.mock_all_auths();
        let (_, token, contract_id, client) = setup_contract(&env);
        let borrower = Address::generate(&env);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.test_set_utilized(&borrower, &100_i128);
        let stellar_token = StellarAssetClient::new(&env, &token);
        stellar_token.mint(&borrower, &50_i128);
        let token_client = token::Client::new(&env, &token);
        let exp = env.ledger().sequence() + 100;
        token_client.approve(&borrower, &contract_id, &100_i128, &exp);
        client.repay_credit(&borrower, &100_i128);
    }
}
