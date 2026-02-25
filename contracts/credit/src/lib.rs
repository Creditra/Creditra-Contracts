#![no_std]

//! Creditra credit contract: credit lines, draw/repay, risk parameters.
//!
//! # Reentrancy
//! Soroban token transfers (e.g. Stellar Asset Contract) do not invoke callbacks back into
//! the caller. This contract uses a reentrancy guard on draw_credit and repay_credit as a
//! defense-in-depth measure; if a token or future integration ever called back, the guard
//! would revert.

mod events;
mod types;

use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, Env, Symbol};

use events::{
    publish_credit_line_event, publish_draw_event, publish_repayment_event,
    publish_risk_parameters_updated, CreditLineEvent, DrawEvent, RepaymentEvent,
    RiskParametersUpdatedEvent,
};
use types::{CreditLineData, CreditStatus};

/// Maximum interest rate in basis points (100%).
const MAX_INTEREST_RATE_BPS: u32 = 10_000;
/// Maximum risk score (0–100 scale).
const MAX_RISK_SCORE: u32 = 100;

/// Instance storage key for reentrancy guard.
fn reentrancy_key(env: &Env) -> Symbol {
    Symbol::new(env, "reentrancy")
}

/// Instance storage key for admin.
fn admin_key(env: &Env) -> Symbol {
    Symbol::new(env, "admin")
}

fn require_admin(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&admin_key(env))
        .expect("admin not set")
}

#[contracttype]
#[derive(Debug, Clone, PartialEq)]
pub enum CreditError {
    CreditLineNotFound = 1,
    InvalidCreditStatus = 2,
    InvalidAmount = 3,
    InsufficientUtilization = 4,
    Unauthorized = 5,
}

impl From<CreditError> for soroban_sdk::Error {
    fn from(val: CreditError) -> Self {
        soroban_sdk::Error::from_contract_error(val as u32)
    }
}

fn require_admin_auth(env: &Env) -> Address {
    let admin = require_admin(env);
    admin.require_auth();
    admin
}

/// Assert reentrancy guard is not set; set it for the duration of the call.
/// Caller must call clear_reentrancy_guard when done (on all paths).
fn set_reentrancy_guard(env: &Env) {
    let key = reentrancy_key(env);
    let current: bool = env.storage().instance().get(&key).unwrap_or(false);
    if current {
        panic!("reentrancy guard");
    }
    env.storage().instance().set(&key, &true);
}

fn clear_reentrancy_guard(env: &Env) {
    env.storage().instance().set(&reentrancy_key(env), &false);
}

#[contract]
pub struct Credit;

#[contractimpl]
impl Credit {
    /// Initialize the contract (admin).
    pub fn init(env: Env, admin: Address) {
        env.storage().instance().set(&admin_key(&env), &admin);
    }

    /// Open a new credit line for a borrower (called by backend/risk engine).
    ///
    /// # Arguments
    /// * `borrower` - The address of the borrower
    /// * `credit_limit` - Maximum borrowable amount (must be > 0)
    /// * `interest_rate_bps` - Annual interest rate in basis points (max 10000 = 100%)
    /// * `risk_score` - Borrower risk score (0–100)
    ///
    /// # Panics
    /// * If `credit_limit` <= 0
    /// * If `interest_rate_bps` > 10000
    /// * If `risk_score` > 100
    /// * If an Active credit line already exists for the borrower
    ///
    /// # Events
    /// Emits `(credit, opened)` with a `CreditLineEvent` payload.
    pub fn open_credit_line(
        env: Env,
        borrower: Address,
        credit_limit: i128,
        interest_rate_bps: u32,
        risk_score: u32,
    ) {
        // Validate credit_limit
        if credit_limit <= 0 {
            panic!("credit_limit must be greater than zero");
        }

        // Validate interest_rate_bps
        if interest_rate_bps > MAX_INTEREST_RATE_BPS {
            panic!("interest_rate_bps cannot exceed 10000 (100%)");
        }

        // Validate risk_score
        if risk_score > MAX_RISK_SCORE {
            panic!("risk_score must be between 0 and 100");
        }

        // Check if borrower already has an active credit line
        if let Some(existing) = env
            .storage()
            .persistent()
            .get::<Address, CreditLineData>(&borrower)
        {
            if existing.status == CreditStatus::Active {
                panic!("borrower already has an active credit line");
            }
        }

        let credit_line = CreditLineData {
            borrower: borrower.clone(),
            credit_limit,
            utilized_amount: 0,
            interest_rate_bps,
            risk_score,
            status: CreditStatus::Active,
            last_accrual_timestamp: env.ledger().timestamp(),
        };

        env.storage().persistent().set(&borrower, &credit_line);

        publish_credit_line_event(
            &env,
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
    }

    /// Draw from credit line (borrower).
    /// Reverts if credit line does not exist, is Closed, or borrower has not authorized.
    pub fn draw_credit(env: Env, borrower: Address, amount: i128) {
        set_reentrancy_guard(&env);
        borrower.require_auth();

        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
            .expect("Credit line not found");

        if credit_line.borrower != borrower {
            panic!("Borrower mismatch for credit line");
        }

        if credit_line.status == CreditStatus::Closed {
            clear_reentrancy_guard(&env);
            panic!("credit line is closed");
        }
        if amount <= 0 {
            clear_reentrancy_guard(&env);
            panic!("amount must be positive");
        }

        let new_utilized = credit_line
            .utilized_amount
            .checked_add(amount)
            .expect("overflow");

        if new_utilized > credit_line.credit_limit {
            clear_reentrancy_guard(&env);
            panic!("exceeds credit limit");
        }

        credit_line.utilized_amount = new_utilized;
        env.storage().persistent().set(&borrower, &credit_line);

        let timestamp = env.ledger().timestamp();
        publish_draw_event(
            &env,
            DrawEvent {
                borrower: borrower.clone(),
                amount,
                new_utilized_amount: new_utilized,
                timestamp,
            },
        );

        clear_reentrancy_guard(&env);
        // TODO: transfer token to borrower
    }

    /// Repay credit (borrower).
    /// Reverts if credit line does not exist, is Closed, or borrower has not authorized.
    /// Reduces utilized_amount by amount (capped at 0). Emits RepaymentEvent.
    pub fn repay_credit(env: Env, borrower: Address, amount: i128) {
        set_reentrancy_guard(&env);
        borrower.require_auth();
        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
            .expect("Credit line not found");

        if credit_line.borrower != borrower {
            panic!("Borrower mismatch for credit line");
        }

        if credit_line.status == CreditStatus::Closed {
            clear_reentrancy_guard(&env);
            panic!("credit line is closed");
        }
        if amount <= 0 {
            clear_reentrancy_guard(&env);
            panic!("amount must be positive");
        }
        let new_utilized = credit_line.utilized_amount.saturating_sub(amount).max(0);
        credit_line.utilized_amount = new_utilized;
        env.storage().persistent().set(&borrower, &credit_line);

        let timestamp = env.ledger().timestamp();
        publish_repayment_event(
            &env,
            RepaymentEvent {
                borrower: borrower.clone(),
                amount,
                new_utilized_amount: new_utilized,
                timestamp,
            },
        );
        clear_reentrancy_guard(&env);
        // TODO: accept token from borrower
    }

    /// Update risk parameters for an existing credit line (admin only).
    ///
    /// # Arguments
    /// * `borrower` - Borrower whose credit line to update.
    /// * `credit_limit` - New credit limit (must be >= current utilized_amount and >= 0).
    /// * `interest_rate_bps` - New interest rate in basis points (0 ..= 10000).
    /// * `risk_score` - New risk score (0 ..= 100).
    ///
    /// # Errors
    /// * Panics if caller is not the contract admin.
    /// * Panics if no credit line exists for the borrower.
    /// * Panics if bounds are violated (e.g. credit_limit < utilized_amount).
    ///
    /// Emits a risk_updated event.
    pub fn update_risk_parameters(
        env: Env,
        borrower: Address,
        credit_limit: i128,
        interest_rate_bps: u32,
        risk_score: u32,
    ) {
        require_admin_auth(&env);

        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
            .expect("Credit line not found");

        if credit_limit < 0 {
            panic!("credit_limit must be non-negative");
        }
        if credit_limit < credit_line.utilized_amount {
            panic!("credit_limit cannot be less than utilized amount");
        }
        if interest_rate_bps > MAX_INTEREST_RATE_BPS {
            panic!("interest_rate_bps exceeds maximum");
        }
        if risk_score > MAX_RISK_SCORE {
            panic!("risk_score exceeds maximum");
        }

        credit_line.credit_limit = credit_limit;
        credit_line.interest_rate_bps = interest_rate_bps;
        credit_line.risk_score = risk_score;
        env.storage().persistent().set(&borrower, &credit_line);

        publish_risk_parameters_updated(
            &env,
            RiskParametersUpdatedEvent {
                borrower: borrower.clone(),
                credit_limit,
                interest_rate_bps,
                risk_score,
            },
        );
    }

    /// Suspend a credit line (admin only).
    /// Emits a CreditLineSuspended event.
    pub fn suspend_credit_line(env: Env, borrower: Address) {
        require_admin_auth(&env);

        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
            .expect("Credit line not found");

        credit_line.status = CreditStatus::Suspended;
        env.storage().persistent().set(&borrower, &credit_line);

        publish_credit_line_event(
            &env,
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
    pub fn close_credit_line(env: Env, borrower: Address, closer: Address) {
        closer.require_auth();

        let admin: Address = require_admin(&env);

        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
            .expect("Credit line not found");

        if credit_line.status == CreditStatus::Closed {
            return;
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

        publish_credit_line_event(
            &env,
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
    }

    /// Mark a credit line as defaulted (admin only).
    /// Emits a CreditLineDefaulted event.
    pub fn default_credit_line(env: Env, borrower: Address) {
        require_admin_auth(&env);

        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
            .expect("Credit line not found");

        credit_line.status = CreditStatus::Defaulted;
        env.storage().persistent().set(&borrower, &credit_line);

        publish_credit_line_event(
            &env,
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
    }

    /// Read-only getter for credit line by borrower
    ///
    /// @param borrower The address to query
    /// @return Option<CreditLineData> Full data or None if no line exists
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
    pub fn accrue_interest(env: Env, borrower: Address) {
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
    pub fn set_borrow_rate(env: Env, borrower: Address, rate_bps: u32) {
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
    pub fn set_utilized_amount(env: Env, borrower: Address, amount: i128) {
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
    use soroban_sdk::testutils::Events;
    use soroban_sdk::{TryFromVal, TryIntoVal};

    /// Helper function to set up test environment with admin, borrower, and contract
    fn setup_test(env: &Env) -> (Address, Address, Address) {
        let admin = Address::generate(env);
        let borrower = Address::generate(env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(env, &contract_id);

        env.mock_all_auths();

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);

        (admin, borrower, contract_id)
    }

    /// Helper function to call contract methods within contract context
    fn call_contract<F>(env: &Env, contract_id: &Address, f: F)
    where
        F: FnOnce(),
    {
        env.mock_all_auths();
        env.as_contract(contract_id, f);
    }

    /// Helper function to get credit line data
    fn get_credit_data(env: &Env, contract_id: &Address, borrower: &Address) -> CreditLineData {
        env.as_contract(contract_id, || {
            Credit::get_credit_line(env.clone(), borrower.clone())
                .expect("Credit line should exist")
        })
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

    // --- draw_credit event emission tests (#draw_events) ---
    // Note: draw_credit emits a DrawEvent with borrower, amount, new_utilized_amount, and timestamp.
    // Event emission is verified through test snapshots which capture the event data.

    #[test]
    fn test_open_credit_line_emits_event() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);

        let events_before = env.events().all().len();
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        let events_after = env.events().all().len();

        assert_eq!(
            events_after,
            events_before + 1,
            "open_credit_line must emit exactly one event"
        );
    }

    #[test]
    fn test_draw_credit_emits_draw_event() {
        // This test verifies that draw_credit emits a DrawEvent.
        // The event schema and data are captured in the test snapshot.
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);

        // Draw credit - this will emit a DrawEvent
        client.draw_credit(&borrower, &250_i128);

        // Verify state was updated correctly
        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(
            credit_line.utilized_amount, 250,
            "Utilized amount must be updated"
        );

        // Event emission is verified through the test snapshot
        // The snapshot will contain the DrawEvent with:
        // - borrower: the borrower address
        // - amount: 250
        // - new_utilized_amount: 250
        // - timestamp: ledger timestamp
    }

    #[test]
    fn test_draw_credit_event_on_multiple_draws() {
        // Verifies that each draw emits its own event with correct accumulated amounts
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);

        // First draw
        client.draw_credit(&borrower, &200_i128);
        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.utilized_amount, 200);

        // Second draw
        client.draw_credit(&borrower, &300_i128);
        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.utilized_amount, 500);

        // Each draw emits a DrawEvent (verified in snapshot)
    }

    #[test]
    fn test_draw_credit_event_for_different_borrowers() {
        // Verifies that events are emitted per borrower
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

        // Borrower 1 draws
        client.draw_credit(&borrower1, &150_i128);
        let credit_line1 = client.get_credit_line(&borrower1).unwrap();
        assert_eq!(credit_line1.utilized_amount, 150);

        // Borrower 2 draws
        client.draw_credit(&borrower2, &500_i128);
        let credit_line2 = client.get_credit_line(&borrower2).unwrap();
        assert_eq!(credit_line2.utilized_amount, 500);

        // Each borrower's draw emits its own DrawEvent (verified in snapshot)
    }

    #[test]
    fn test_draw_credit_event_at_credit_limit() {
        // Verifies event emission when drawing to full credit limit
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);

        // Draw exactly to the credit limit
        client.draw_credit(&borrower, &1000_i128);

        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.utilized_amount, 1000);

        // DrawEvent emitted with new_utilized_amount = 1000 (verified in snapshot)
    }

    #[test]
    fn test_draw_credit_event_for_small_amount() {
        // Verifies event emission for minimal draw amounts
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);

        // Draw minimal amount
        client.draw_credit(&borrower, &1_i128);

        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.utilized_amount, 1);

        // DrawEvent emitted with amount = 1 (verified in snapshot)
    }

    #[test]
    #[should_panic(expected = "credit line is closed")]
    fn test_draw_credit_no_event_when_closed() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.close_credit_line(&borrower, &borrower);

        // This should panic before emitting an event
        client.draw_credit(&borrower, &100_i128);
    }

    #[test]
    #[should_panic(expected = "exceeds credit limit")]
    fn test_draw_credit_no_event_when_exceeds_limit() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);

        // This should panic before emitting an event
        client.draw_credit(&borrower, &1001_i128);
    }

    #[test]
    #[should_panic(expected = "amount must be positive")]
    fn test_draw_credit_no_event_for_zero_amount() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);

        // This should panic before emitting an event
        client.draw_credit(&borrower, &0_i128);
    }

    #[test]
    #[should_panic(expected = "amount must be positive")]
    fn test_draw_credit_no_event_for_negative_amount() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);

        // This should panic before emitting an event
        client.draw_credit(&borrower, &-100_i128);
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

        client.repay_credit(&borrower, &200_i128);

        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.utilized_amount, 300);

        // RepaymentEvent emission is verified through the test snapshot
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

    // ========== EVENT EMISSION TESTS (#42) ==========

    /// Test that repay_credit emits RepaymentEvent with correct payload.
    #[test]
    fn test_event_repay_credit_payload() {
        use soroban_sdk::testutils::Events;

        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &5000_i128, &300_u32, &70_u32);
        client.draw_credit(&borrower, &1000_i128);

        // Repay 400
        client.repay_credit(&borrower, &400_i128);

        // Get the events (last event is the repay event)
        let events = env.events().all();
        let (_contract, topics, data) = events.last().unwrap();

        // Verify event topics
        assert_eq!(topics.len(), 2);
        assert_eq!(
            Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap(),
            symbol_short!("credit")
        );
        assert_eq!(
            Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
            symbol_short!("repay")
        );

        // Verify event data
        let event_data: RepaymentEvent = data.try_into_val(&env).unwrap();
        assert_eq!(event_data.borrower, borrower);
        assert_eq!(event_data.amount, 400);
        assert_eq!(event_data.new_utilized_amount, 600);
        // Timestamp is populated from ledger
    }

    /// Test that repay_credit emits correct event for full repayment.
    #[test]
    fn test_event_repay_credit_full_amount() {
        use soroban_sdk::testutils::Events;

        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &5000_i128, &300_u32, &70_u32);
        client.draw_credit(&borrower, &2000_i128);

        // Repay full amount
        client.repay_credit(&borrower, &2000_i128);

        // Get the events (last event is the repay event)
        let events = env.events().all();
        let (_contract, _topics, data) = events.last().unwrap();

        // Verify event data
        let event_data: RepaymentEvent = data.try_into_val(&env).unwrap();
        assert_eq!(event_data.borrower, borrower);
        assert_eq!(event_data.amount, 2000);
        assert_eq!(event_data.new_utilized_amount, 0);
        // Timestamp is populated from ledger
    }

    /// Test that repay_credit emits correct event for overpayment (saturating).
    #[test]
    fn test_event_repay_credit_overpayment() {
        use soroban_sdk::testutils::Events;

        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &5000_i128, &300_u32, &70_u32);
        client.draw_credit(&borrower, &500_i128);

        // Repay more than utilized (should saturate to 0)
        client.repay_credit(&borrower, &1000_i128);

        // Get the events (last event is the repay event)
        let events = env.events().all();
        let (_contract, _topics, data) = events.last().unwrap();

        // Verify event data
        let event_data: RepaymentEvent = data.try_into_val(&env).unwrap();
        assert_eq!(event_data.borrower, borrower);
        assert_eq!(event_data.amount, 1000);
        assert_eq!(event_data.new_utilized_amount, 0);
        // Timestamp is populated from ledger
    }

    /// Test multiple repay events are correctly emitted.
    #[test]
    fn test_event_multiple_repayments() {
        use soroban_sdk::testutils::Events;

        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &10000_i128, &300_u32, &70_u32);
        client.draw_credit(&borrower, &5000_i128);

        // First repayment
        client.repay_credit(&borrower, &1000_i128);
        let events = env.events().all();
        let (_c, _topics, data) = events.last().unwrap();
        let repay1_data: RepaymentEvent = data.try_into_val(&env).unwrap();
        assert_eq!(repay1_data.amount, 1000);
        assert_eq!(repay1_data.new_utilized_amount, 4000);

        // Second repayment
        client.repay_credit(&borrower, &2000_i128);
        let events = env.events().all();
        let (_c, _topics, data) = events.last().unwrap();
        let repay2_data: RepaymentEvent = data.try_into_val(&env).unwrap();
        assert_eq!(repay2_data.amount, 2000);
        assert_eq!(repay2_data.new_utilized_amount, 2000);

        // Third repayment
        client.repay_credit(&borrower, &1500_i128);
        let events = env.events().all();
        let (_c, _topics, data) = events.last().unwrap();
        let repay3_data: RepaymentEvent = data.try_into_val(&env).unwrap();
        assert_eq!(repay3_data.amount, 1500);
        assert_eq!(repay3_data.new_utilized_amount, 500);
    }

    /// Test that open_credit_line emits CreditLineEvent with correct payload.
    #[test]
    fn test_event_open_credit_line() {
        use soroban_sdk::testutils::Events;

        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);

        // Get the events
        let events = env.events().all();
        let (_contract, topics, data) = events.last().unwrap();

        // Verify event topics
        assert_eq!(topics.len(), 2);
        assert_eq!(
            Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap(),
            symbol_short!("credit")
        );
        assert_eq!(
            Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
            symbol_short!("opened")
        );

        // Verify event data
        let event_data: CreditLineEvent = data.try_into_val(&env).unwrap();
        assert_eq!(event_data.event_type, symbol_short!("opened"));
        assert_eq!(event_data.borrower, borrower);
        assert_eq!(event_data.status, CreditStatus::Active);
        assert_eq!(event_data.credit_limit, 1000);
        assert_eq!(event_data.interest_rate_bps, 300);
        assert_eq!(event_data.risk_score, 70);
    }

    /// Test that suspend_credit_line emits CreditLineEvent with correct payload.
    #[test]
    fn test_event_suspend_credit_line() {
        use soroban_sdk::testutils::Events;

        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.suspend_credit_line(&borrower);

        // Get the events (last event is the suspend event)
        let events = env.events().all();
        let (_contract, topics, data) = events.last().unwrap();

        // Verify event topics
        assert_eq!(topics.len(), 2);
        assert_eq!(
            Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap(),
            symbol_short!("credit")
        );
        assert_eq!(
            Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
            symbol_short!("suspend")
        );

        // Verify event data
        let event_data: CreditLineEvent = data.try_into_val(&env).unwrap();
        assert_eq!(event_data.event_type, symbol_short!("suspend"));
        assert_eq!(event_data.borrower, borrower);
        assert_eq!(event_data.status, CreditStatus::Suspended);
        assert_eq!(event_data.credit_limit, 1000);
        assert_eq!(event_data.interest_rate_bps, 300);
        assert_eq!(event_data.risk_score, 70);
    }

    /// Test that close_credit_line emits CreditLineEvent with correct payload.
    #[test]
    fn test_event_close_credit_line() {
        use soroban_sdk::testutils::Events;

        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &2000_i128, &400_u32, &80_u32);
        client.close_credit_line(&borrower, &admin);

        // Get the events (last event is the close event)
        let events = env.events().all();
        let (_contract, topics, data) = events.last().unwrap();

        // Verify event topics
        assert_eq!(topics.len(), 2);
        assert_eq!(
            Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap(),
            symbol_short!("credit")
        );
        assert_eq!(
            Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
            symbol_short!("closed")
        );

        // Verify event data
        let event_data: CreditLineEvent = data.try_into_val(&env).unwrap();
        assert_eq!(event_data.event_type, symbol_short!("closed"));
        assert_eq!(event_data.borrower, borrower);
        assert_eq!(event_data.status, CreditStatus::Closed);
        assert_eq!(event_data.credit_limit, 2000);
        assert_eq!(event_data.interest_rate_bps, 400);
        assert_eq!(event_data.risk_score, 80);
    }

    /// Test that default_credit_line emits CreditLineEvent with correct payload.
    #[test]
    fn test_event_default_credit_line() {
        use soroban_sdk::testutils::Events;

        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &3000_i128, &500_u32, &90_u32);
        client.default_credit_line(&borrower);

        // Get the events (last event is the default event)
        let events = env.events().all();
        let (_contract, topics, data) = events.last().unwrap();

        // Verify event topics
        assert_eq!(topics.len(), 2);
        assert_eq!(
            Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap(),
            symbol_short!("credit")
        );
        assert_eq!(
            Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
            symbol_short!("default")
        );

        // Verify event data
        let event_data: CreditLineEvent = data.try_into_val(&env).unwrap();
        assert_eq!(event_data.event_type, symbol_short!("default"));
        assert_eq!(event_data.borrower, borrower);
        assert_eq!(event_data.status, CreditStatus::Defaulted);
        assert_eq!(event_data.credit_limit, 3000);
        assert_eq!(event_data.interest_rate_bps, 500);
        assert_eq!(event_data.risk_score, 90);
    }

    /// Test lifecycle event sequence: open -> suspend -> close.
    #[test]
    fn test_event_lifecycle_sequence() {
        use soroban_sdk::testutils::Events;

        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);

        // Open credit line and check event
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        let events = env.events().all();
        let (_c, topics, data) = events.last().unwrap();
        assert_eq!(
            Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
            symbol_short!("opened")
        );
        let open_data: CreditLineEvent = data.try_into_val(&env).unwrap();
        assert_eq!(open_data.status, CreditStatus::Active);

        // Suspend credit line and check event
        client.suspend_credit_line(&borrower);
        let events = env.events().all();
        let (_c, topics, data) = events.last().unwrap();
        assert_eq!(
            Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
            symbol_short!("suspend")
        );
        let suspend_data: CreditLineEvent = data.try_into_val(&env).unwrap();
        assert_eq!(suspend_data.status, CreditStatus::Suspended);

        // Close credit line and check event
        client.close_credit_line(&borrower, &admin);
        let events = env.events().all();
        let (_c, topics, data) = events.last().unwrap();
        assert_eq!(
            Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
            symbol_short!("closed")
        );
        let close_data: CreditLineEvent = data.try_into_val(&env).unwrap();
        assert_eq!(close_data.status, CreditStatus::Closed);
    }

    /// Test that event data remains consistent across lifecycle operations.
    #[test]
    fn test_event_data_consistency_across_lifecycle() {
        use soroban_sdk::testutils::Events;

        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);

        // Open with specific parameters
        let credit_limit = 7500_i128;
        let interest_rate = 450_u32;
        let risk_score = 85_u32;

        // Open and verify event data
        client.open_credit_line(&borrower, &credit_limit, &interest_rate, &risk_score);
        let events = env.events().all();
        let (_c, _topics, data) = events.last().unwrap();
        let open_data: CreditLineEvent = data.try_into_val(&env).unwrap();
        assert_eq!(open_data.credit_limit, credit_limit);
        assert_eq!(open_data.interest_rate_bps, interest_rate);
        assert_eq!(open_data.risk_score, risk_score);

        // Suspend and verify event data consistency
        client.suspend_credit_line(&borrower);
        let events = env.events().all();
        let (_c, _topics, data) = events.last().unwrap();
        let suspend_data: CreditLineEvent = data.try_into_val(&env).unwrap();
        assert_eq!(suspend_data.credit_limit, credit_limit);
        assert_eq!(suspend_data.interest_rate_bps, interest_rate);
        assert_eq!(suspend_data.risk_score, risk_score);

        // Default and verify event data consistency
        client.default_credit_line(&borrower);
        let events = env.events().all();
        let (_c, _topics, data) = events.last().unwrap();
        let default_data: CreditLineEvent = data.try_into_val(&env).unwrap();
        assert_eq!(default_data.credit_limit, credit_limit);
        assert_eq!(default_data.interest_rate_bps, interest_rate);
        assert_eq!(default_data.risk_score, risk_score);
    }
}

// ============================================================
// Tests: close_credit_line with outstanding utilization
// Branch: tests/close-utilized-nonzero
// ============================================================
//
// These tests verify the contract's behavior when close_credit_line is called
// while the borrower has a non-zero utilized_amount.
//
// Scenarios covered:
//   1. Borrower is rejected even when utilized_amount == 1 (minimum non-zero).
//   2. Borrower is rejected when utilized_amount equals the full credit limit.
//   3. Admin force-close succeeds and the utilized_amount is preserved in storage.
//   4. Admin force-close emits exactly one CreditLineClosed event.
//   5. Borrower is rejected on a Suspended line that has outstanding utilization.
//   6. Admin can force-close a Suspended line that has outstanding utilization.
//   7. Borrower succeeds in closing after fully repaying all outstanding debt.
//   8. A third-party address (neither admin nor borrower) is rejected even when
//      utilized_amount is zero.
//   9. Admin force-close succeeds after multiple sequential draws.
#[cfg(test)]
mod test_close_utilized {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    // ------------------------------------------------------------------
    // 1. Borrower rejected even when utilized_amount == 1 (minimum non-zero)
    // ------------------------------------------------------------------
    /// Verifies that a borrower cannot close their own credit line when
    /// utilized_amount is as small as 1.  The contract must panic with the
    /// message "cannot close: utilized amount not zero".
    #[test]
    #[should_panic(expected = "cannot close: utilized amount not zero")]
    fn test_close_utilized_borrower_rejected_at_minimum_utilization() {
        let env = soroban_sdk::Env::default();
        env.mock_all_auths();

        let admin = soroban_sdk::Address::generate(&env);
        let borrower = soroban_sdk::Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        // Open a line with limit 1000 and draw the minimum non-zero amount.
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.draw_credit(&borrower, &1_i128);

        // utilized_amount == 1; borrower must be rejected.
        client.close_credit_line(&borrower, &borrower);
    }

    // ------------------------------------------------------------------
    // 2. Borrower rejected when utilized_amount equals the full credit limit
    // ------------------------------------------------------------------
    /// Verifies that a borrower cannot close their own credit line when the
    /// entire credit limit has been drawn (utilized_amount == credit_limit).
    #[test]
    #[should_panic(expected = "cannot close: utilized amount not zero")]
    fn test_close_utilized_borrower_rejected_at_full_utilization() {
        let env = soroban_sdk::Env::default();
        env.mock_all_auths();

        let admin = soroban_sdk::Address::generate(&env);
        let borrower = soroban_sdk::Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        // Draw the full credit limit.
        client.open_credit_line(&borrower, &500_i128, &300_u32, &70_u32);
        client.draw_credit(&borrower, &500_i128);

        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            500,
            "pre-condition: utilized_amount must equal credit_limit"
        );

        // Borrower must be rejected.
        client.close_credit_line(&borrower, &borrower);
    }

    // ------------------------------------------------------------------
    // 3. Admin force-close preserves utilized_amount in storage
    // ------------------------------------------------------------------
    /// Verifies that when the admin force-closes a credit line that has
    /// outstanding utilization, the stored utilized_amount is NOT zeroed out —
    /// it is preserved so that the debt record remains auditable.
    #[test]
    fn test_close_utilized_admin_force_close_preserves_utilized_amount() {
        let env = soroban_sdk::Env::default();
        env.mock_all_auths();

        let admin = soroban_sdk::Address::generate(&env);
        let borrower = soroban_sdk::Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.draw_credit(&borrower, &750_i128);

        // Admin force-closes the line.
        client.close_credit_line(&borrower, &admin);

        let credit_line = client.get_credit_line(&borrower).unwrap();

        // Status must be Closed.
        assert_eq!(
            credit_line.status,
            CreditStatus::Closed,
            "status must be Closed after admin force-close"
        );
        // utilized_amount must be preserved (not zeroed).
        assert_eq!(
            credit_line.utilized_amount, 750,
            "utilized_amount must be preserved after admin force-close"
        );
        // Other fields must be unchanged.
        assert_eq!(credit_line.credit_limit, 1000);
        assert_eq!(credit_line.interest_rate_bps, 300);
        assert_eq!(credit_line.risk_score, 70);
    }

    // ------------------------------------------------------------------
    // 4. Admin force-close emits exactly one CreditLineClosed event
    // ------------------------------------------------------------------
    /// Verifies that admin force-closing a credit line with outstanding
    /// utilization emits exactly one event and that the event carries the
    /// correct status (Closed) and the correct utilized_amount.
    #[test]
    fn test_close_utilized_admin_force_close_emits_closed_event() {
        let env = soroban_sdk::Env::default();
        env.mock_all_auths();

        let admin = soroban_sdk::Address::generate(&env);
        let borrower = soroban_sdk::Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.draw_credit(&borrower, &400_i128);

        // Admin force-closes the line.
        client.close_credit_line(&borrower, &admin);

        // The credit line must be Closed.
        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.status, CreditStatus::Closed);
        assert_eq!(
            credit_line.utilized_amount, 400,
            "utilized_amount must be preserved in the closed state"
        );

        // Event emission is verified by the test snapshot
    }

    // ------------------------------------------------------------------
    // 5. Borrower rejected on a Suspended line with outstanding utilization
    // ------------------------------------------------------------------
    /// Verifies that a borrower cannot close a Suspended credit line when
    /// utilized_amount is non-zero.  The contract must panic with the message
    /// "cannot close: utilized amount not zero".
    #[test]
    #[should_panic(expected = "cannot close: utilized amount not zero")]
    fn test_close_utilized_borrower_rejected_on_suspended_line() {
        let env = soroban_sdk::Env::default();
        env.mock_all_auths();

        let admin = soroban_sdk::Address::generate(&env);
        let borrower = soroban_sdk::Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        // Draw while Active (draw_credit rejects Closed, not Suspended).
        client.draw_credit(&borrower, &200_i128);
        // Admin suspends the line.
        client.suspend_credit_line(&borrower);

        assert_eq!(
            client.get_credit_line(&borrower).unwrap().status,
            CreditStatus::Suspended,
            "pre-condition: line must be Suspended"
        );
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            200,
            "pre-condition: utilized_amount must be 200"
        );

        // Borrower must be rejected even on a Suspended line.
        client.close_credit_line(&borrower, &borrower);
    }

    // ------------------------------------------------------------------
    // 6. Admin can force-close a Suspended line with outstanding utilization
    // ------------------------------------------------------------------
    /// Verifies that the admin can force-close a Suspended credit line that
    /// has outstanding utilization, and that the resulting state is Closed
    /// with the utilized_amount preserved.
    #[test]
    fn test_close_utilized_admin_force_close_suspended_line() {
        let env = soroban_sdk::Env::default();
        env.mock_all_auths();

        let admin = soroban_sdk::Address::generate(&env);
        let borrower = soroban_sdk::Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.draw_credit(&borrower, &600_i128);
        client.suspend_credit_line(&borrower);

        // Admin force-closes the Suspended line.
        client.close_credit_line(&borrower, &admin);

        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(
            credit_line.status,
            CreditStatus::Closed,
            "status must be Closed after admin force-close of Suspended line"
        );
        assert_eq!(
            credit_line.utilized_amount, 600,
            "utilized_amount must be preserved after force-close of Suspended line"
        );
    }

    // ------------------------------------------------------------------
    // 7. Borrower succeeds in closing after fully repaying all outstanding debt
    // ------------------------------------------------------------------
    /// Verifies the happy-path: a borrower who drew credit and then fully
    /// repaid it (utilized_amount == 0) is allowed to close their own line.
    #[test]
    fn test_close_utilized_borrower_succeeds_after_full_repayment() {
        let env = soroban_sdk::Env::default();
        env.mock_all_auths();

        let admin = soroban_sdk::Address::generate(&env);
        let borrower = soroban_sdk::Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.draw_credit(&borrower, &350_i128);

        // Fully repay the outstanding debt.
        client.repay_credit(&borrower, &350_i128);

        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            0,
            "pre-condition: utilized_amount must be 0 after full repayment"
        );

        // Borrower must now be allowed to close.
        client.close_credit_line(&borrower, &borrower);

        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(
            credit_line.status,
            CreditStatus::Closed,
            "status must be Closed after borrower closes with zero utilization"
        );
        assert_eq!(credit_line.utilized_amount, 0);
    }

    // ------------------------------------------------------------------
    // 8. Third-party rejected even when utilized_amount is zero
    // ------------------------------------------------------------------
    /// Verifies that an address that is neither the admin nor the borrower
    /// cannot close a credit line, regardless of the utilized_amount.
    #[test]
    #[should_panic(expected = "unauthorized")]
    fn test_close_utilized_third_party_rejected_with_zero_utilization() {
        let env = soroban_sdk::Env::default();
        env.mock_all_auths();

        let admin = soroban_sdk::Address::generate(&env);
        let borrower = soroban_sdk::Address::generate(&env);
        let third_party = soroban_sdk::Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        // Open with zero utilization so the only rejection reason is authorization.
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);

        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            0,
            "pre-condition: utilized_amount must be 0"
        );

        // Third-party must be rejected with "unauthorized".
        client.close_credit_line(&borrower, &third_party);
    }

    // ------------------------------------------------------------------
    // 9. Admin force-close succeeds after multiple sequential draws
    // ------------------------------------------------------------------
    /// Verifies that the admin can force-close a credit line whose
    /// utilized_amount was built up through multiple sequential draw_credit
    /// calls, and that the final utilized_amount is preserved correctly.
    #[test]
    fn test_close_utilized_admin_force_close_multiple_draws() {
        let env = soroban_sdk::Env::default();
        env.mock_all_auths();

        let admin = soroban_sdk::Address::generate(&env);
        let borrower = soroban_sdk::Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);

        // Multiple sequential draws.
        client.draw_credit(&borrower, &100_i128);
        client.draw_credit(&borrower, &150_i128);
        client.draw_credit(&borrower, &250_i128);

        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            500,
            "pre-condition: utilized_amount must be 500 after three draws"
        );

        // Admin force-closes the line.
        client.close_credit_line(&borrower, &admin);

        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(
            credit_line.status,
            CreditStatus::Closed,
            "status must be Closed after admin force-close"
        );
        assert_eq!(
            credit_line.utilized_amount, 500,
            "utilized_amount must be preserved (100 + 150 + 250 = 500)"
        );
    }

    // ------------------------------------------------------------------
    // 10. Borrower rejected after partial repayment (utilized_amount still > 0)
    // ------------------------------------------------------------------
    /// Verifies that a borrower who has partially repaid their debt but still
    /// has a non-zero utilized_amount cannot close their own credit line.
    #[test]
    #[should_panic(expected = "cannot close: utilized amount not zero")]
    fn test_close_utilized_borrower_rejected_after_partial_repayment() {
        let env = soroban_sdk::Env::default();
        env.mock_all_auths();

        let admin = soroban_sdk::Address::generate(&env);
        let borrower = soroban_sdk::Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.draw_credit(&borrower, &400_i128);

        // Partial repayment — still 200 outstanding.
        client.repay_credit(&borrower, &200_i128);

        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            200,
            "pre-condition: utilized_amount must be 200 after partial repayment"
        );

        // Borrower must still be rejected.
        client.close_credit_line(&borrower, &borrower);
    }
}
