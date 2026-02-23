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
    pub last_accrual_timestamp: u64,
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
        env.storage().instance().set(&Symbol::new(&env, "admin"), &admin);
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
            last_accrual_timestamp: env.ledger().timestamp(),
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

    /// Repay credit (borrower).
    pub fn repay_credit(_env: Env, _borrower: Address, _amount: i128) -> () {
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

    /// Get credit line data for a borrower (view function).
    pub fn get_credit_line(env: Env, borrower: Address) -> Option<CreditLineData> {
        env.storage().persistent().get(&borrower)
    }

    // ========== INTEREST ACCRUAL FUNCTIONS ==========
    // These functions implement compound interest accrual based on ledger timestamp.
    // Formula: effective_debt = principal * (1 + rate)^time
    // Where rate is per-second rate derived from annual BPS.

    /// Accrue interest for a borrower's credit line.
    /// Updates the utilized_amount and last_accrual_timestamp in storage.
    /// This is called internally before any operation that reads or modifies debt.
    pub fn accrue_interest(env: Env, borrower: Address) -> () {
        let mut credit_line: CreditLineData = match env.storage().persistent().get(&borrower) {
            Some(cl) => cl,
            None => return,
        };

        if credit_line.utilized_amount == 0 {
            // No debt, just update timestamp
            credit_line.last_accrual_timestamp = env.ledger().timestamp();
            env.storage().persistent().set(&borrower, &credit_line);
            return;
        }

        let current_timestamp = env.ledger().timestamp();
        let elapsed_seconds = current_timestamp.saturating_sub(credit_line.last_accrual_timestamp);

        if elapsed_seconds == 0 {
            return; // No time elapsed, no accrual needed
        }

        // Calculate new debt with compound interest
        let new_debt = Self::calculate_accrued_debt(
            credit_line.utilized_amount,
            credit_line.interest_rate_bps,
            elapsed_seconds,
        );

        credit_line.utilized_amount = new_debt;
        credit_line.last_accrual_timestamp = current_timestamp;
        env.storage().persistent().set(&borrower, &credit_line);
    }

    /// Get the effective debt for a borrower (includes accrued interest).
    /// This is a view function that calculates interest without mutating state.
    pub fn get_effective_debt(env: Env, borrower: Address) -> i128 {
        let credit_line: CreditLineData = match env.storage().persistent().get(&borrower) {
            Some(cl) => cl,
            None => return 0,
        };

        if credit_line.utilized_amount == 0 {
            return 0;
        }

        let current_timestamp = env.ledger().timestamp();
        let elapsed_seconds = current_timestamp.saturating_sub(credit_line.last_accrual_timestamp);

        if elapsed_seconds == 0 {
            return credit_line.utilized_amount;
        }

        Self::calculate_accrued_debt(
            credit_line.utilized_amount,
            credit_line.interest_rate_bps,
            elapsed_seconds,
        )
    }

    /// Get the current borrow rate for a borrower in basis points.
    pub fn get_borrow_rate(env: Env, borrower: Address) -> i128 {
        let credit_line: CreditLineData = match env.storage().persistent().get(&borrower) {
            Some(cl) => cl,
            None => return 0,
        };
        credit_line.interest_rate_bps as i128
    }

    /// Set the borrow rate for a borrower (for testing purposes).
    /// In production, this would be controlled by risk parameters.
    pub fn set_borrow_rate(env: Env, borrower: Address, rate_bps: u32) -> () {
        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
            .expect("Credit line not found");
        
        credit_line.interest_rate_bps = rate_bps;
        env.storage().persistent().set(&borrower, &credit_line);
    }

    /// Set the utilized amount for a borrower (for testing purposes).
    /// In production, this would be controlled by draw_credit and repay_credit.
    pub fn set_utilized_amount(env: Env, borrower: Address, amount: i128) -> () {
        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
            .expect("Credit line not found");
        
        credit_line.utilized_amount = amount;
        credit_line.last_accrual_timestamp = env.ledger().timestamp();
        env.storage().persistent().set(&borrower, &credit_line);
    }

    /// Calculate accrued debt using compound interest formula.
    /// Formula: debt = principal * (1 + rate_per_second)^seconds
    /// 
    /// To avoid floating point, we use fixed-point arithmetic:
    /// - BPS (basis points) = rate * 10,000 (e.g., 500 BPS = 5% annual)
    /// - Annual rate = BPS / 10,000
    /// - Per-second rate = annual_rate / SECONDS_PER_YEAR
    /// - We use approximation: debt ≈ principal * (1 + rate_per_second * seconds) for small rates
    /// 
    /// For production, consider using a more sophisticated compound interest calculation
    /// or a library that handles fixed-point exponentiation.
    fn calculate_accrued_debt(principal: i128, rate_bps: u32, elapsed_seconds: u64) -> i128 {
        const SECONDS_PER_YEAR: u64 = 31_536_000; // 365 days
        const BPS_DIVISOR: i128 = 10_000;
        const MAX_RATE_BPS: u32 = 50_000; // 500% annual rate cap

        if rate_bps == 0 || elapsed_seconds == 0 {
            return principal;
        }

        // Cap rate to prevent overflow
        let capped_rate = rate_bps.min(MAX_RATE_BPS);

        // Calculate interest using simple interest approximation for safety
        // interest = principal * (rate_bps / BPS_DIVISOR) * (elapsed_seconds / SECONDS_PER_YEAR)
        // Rearranged to avoid overflow: interest = (principal * rate_bps * elapsed_seconds) / (BPS_DIVISOR * SECONDS_PER_YEAR)
        
        let rate_i128 = capped_rate as i128;
        let elapsed_i128 = elapsed_seconds as i128;
        
        // Check for potential overflow before multiplication
        let max_principal = i128::MAX / rate_i128 / elapsed_i128;
        if principal > max_principal {
            // Return capped value to prevent overflow
            return i128::MAX;
        }

        let interest_numerator = principal
            .saturating_mul(rate_i128)
            .saturating_mul(elapsed_i128);
        
        let interest_denominator = BPS_DIVISOR * (SECONDS_PER_YEAR as i128);
        let interest = interest_numerator / interest_denominator;

        principal.saturating_add(interest)
    }
}

#[cfg(test)]
mod tests;

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
}
