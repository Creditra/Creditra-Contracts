#![no_std]
#![allow(clippy::unused_unit)]

//! Creditra credit contract: credit lines, draw/repay, risk parameters.
//!
//! # Reentrancy
//! Soroban token transfers (e.g. Stellar Asset Contract) do not invoke callbacks back into
//! the caller. This contract uses a reentrancy guard on draw_credit and repay_credit as a
//! defense-in-depth measure; if a token or future integration ever called back, the guard
//! would revert.

mod events;
mod types;

use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, token, Address, Env, Symbol,
};

use events::{
    publish_credit_line_event, publish_drawn_event, publish_repayment_event,
    publish_risk_parameters_updated, CreditLineEvent, DrawnEvent, RepaymentEvent,
    RiskParametersUpdatedEvent,
};
use types::{CreditLineData, CreditStatus, RateChangeConfig};

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

/// Instance storage key for rate-change limit configuration.
fn rate_cfg_key(env: &Env) -> Symbol {
    Symbol::new(env, "rate_cfg")
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

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    LiquidityToken,
    LiquiditySource,
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
    /// @notice Initializes contract-level configuration.
    /// @dev Sets admin and defaults liquidity source to this contract address.
    pub fn init(env: Env, admin: Address) -> () {
        env.storage().instance().set(&admin_key(&env), &admin);
        env.storage()
            .instance()
            .set(&DataKey::LiquiditySource, &env.current_contract_address());
        ()
    }

    /// @notice Sets the token contract used for reserve/liquidity checks and draw transfers.
    /// @dev Admin-only.
    pub fn set_liquidity_token(env: Env, token_address: Address) -> () {
        require_admin_auth(&env);
        env.storage()
            .instance()
            .set(&DataKey::LiquidityToken, &token_address);
        ()
    }

    /// @notice Sets the address that provides liquidity for draw operations.
    /// @dev Admin-only. If unset, init config uses the contract address.
    pub fn set_liquidity_source(env: Env, reserve_address: Address) -> () {
        require_admin_auth(&env);
        env.storage()
            .instance()
            .set(&DataKey::LiquiditySource, &reserve_address);
        ()
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
        assert!(credit_limit > 0, "credit_limit must be greater than zero");
        assert!(
            interest_rate_bps <= 10_000,
            "interest_rate_bps cannot exceed 10000 (100%)"
        );
        assert!(risk_score <= 100, "risk_score must be between 0 and 100");

        // Prevent overwriting an existing Active credit line
        if let Some(existing) = env
            .storage()
            .persistent()
            .get::<Address, CreditLineData>(&borrower)
        {
            assert!(
                existing.status != CreditStatus::Active,
                "borrower already has an active credit line"
            );
        }
        let credit_line = CreditLineData {
            borrower: borrower.clone(),
            credit_limit,
            utilized_amount: 0,
            interest_rate_bps,
            risk_score,
            status: CreditStatus::Active,
            last_rate_update_ts: 0,
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

    /// @notice Draws credit by transferring liquidity tokens to the borrower.
    /// @dev Enforces status/limit/liquidity checks and uses a reentrancy guard.
    pub fn draw_credit(env: Env, borrower: Address, amount: i128) -> () {
        set_reentrancy_guard(&env);
        borrower.require_auth();

        if amount <= 0 {
            clear_reentrancy_guard(&env);
            panic!("amount must be positive");
        }

        let token_address: Option<Address> = env.storage().instance().get(&DataKey::LiquidityToken);
        let reserve_address: Address = env
            .storage()
            .instance()
            .get(&DataKey::LiquiditySource)
            .unwrap_or(env.current_contract_address());

        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
            .expect("Credit line not found");

        if credit_line.status == CreditStatus::Closed {
            clear_reentrancy_guard(&env);
            panic!("credit line is closed");
        }

        let updated_utilized = credit_line
            .utilized_amount
            .checked_add(amount)
            .expect("overflow");

        if updated_utilized > credit_line.credit_limit {
            clear_reentrancy_guard(&env);
            panic!("exceeds credit limit");
        }

        if let Some(token_address) = token_address {
            let token_client = token::Client::new(&env, &token_address);
            let reserve_balance = token_client.balance(&reserve_address);
            if reserve_balance < amount {
                clear_reentrancy_guard(&env);
                panic!("Insufficient liquidity reserve for requested draw amount");
            }

            token_client.transfer(&reserve_address, &borrower, &amount);
        }

        credit_line.utilized_amount = updated_utilized;
        env.storage().persistent().set(&borrower, &credit_line);
        let timestamp = env.ledger().timestamp();
        publish_drawn_event(
            &env,
            DrawnEvent {
                borrower,
                amount,
                new_utilized_amount: updated_utilized,
                timestamp,
            },
        );
        clear_reentrancy_guard(&env);
        ()
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
    /// # Rate-change limits
    /// When a `RateChangeConfig` has been set via `set_rate_change_limits`, the
    /// following additional checks are enforced whenever the interest rate is
    /// actually changing:
    /// * The absolute delta `|new_rate - old_rate|` must be ≤
    ///   `max_rate_change_bps`.
    /// * If a minimum interval is configured and a previous rate change
    ///   timestamp exists, the elapsed time since the last change must be ≥
    ///   `rate_change_min_interval`.
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

        // --- Rate-change limit enforcement (#17) ---
        let rate_cfg: Option<RateChangeConfig> = env.storage().instance().get(&rate_cfg_key(&env));
        if let Some(cfg) = rate_cfg {
            let old_rate = credit_line.interest_rate_bps;
            if interest_rate_bps != old_rate {
                // Check minimum interval between rate changes.
                if credit_line.last_rate_update_ts > 0 && cfg.rate_change_min_interval > 0 {
                    let now = env.ledger().timestamp();
                    let elapsed = now.saturating_sub(credit_line.last_rate_update_ts);
                    if elapsed < cfg.rate_change_min_interval {
                        panic!("rate change too soon: minimum interval not elapsed");
                    }
                }
                // Check absolute delta cap.
                let delta = interest_rate_bps.abs_diff(old_rate);
                if delta > cfg.max_rate_change_bps {
                    panic!("rate change exceeds maximum allowed delta");
                }
                // Record the timestamp of this rate change.
                credit_line.last_rate_update_ts = env.ledger().timestamp();
            }
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

    /// Set rate-change limits (admin only).
    ///
    /// Configures the maximum allowed interest-rate change per call and the
    /// minimum time interval between consecutive rate changes.
    pub fn set_rate_change_limits(
        env: Env,
        max_rate_change_bps: u32,
        rate_change_min_interval: u64,
    ) {
        require_admin_auth(&env);
        let cfg = RateChangeConfig {
            max_rate_change_bps,
            rate_change_min_interval,
        };
        env.storage().instance().set(&rate_cfg_key(&env), &cfg);
    }

    /// Get the current rate-change limit configuration (view function).
    pub fn get_rate_change_limits(env: Env) -> Option<RateChangeConfig> {
        env.storage().instance().get(&rate_cfg_key(&env))
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
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::testutils::Events;
    use soroban_sdk::token::StellarAssetClient;

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
        let token_admin = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);

        let token = env.register_stellar_asset_contract_v2(token_admin);
        let token_admin_client = StellarAssetClient::new(&env, &token.address());
        let token_client = token::Client::new(&env, &token.address());

        client.set_liquidity_token(&token.address());

        token_admin_client.mint(&contract_id, &500_i128);
        client.draw_credit(&borrower, &200_i128);

        assert_eq!(token_client.balance(&contract_id), 300_i128);
        assert_eq!(token_client.balance(&borrower), 200_i128);
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
        let token_admin = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);

        let token = env.register_stellar_asset_contract_v2(token_admin);
        let token_admin_client = StellarAssetClient::new(&env, &token.address());
        let token_client = token::Client::new(&env, &token.address());
        let reserve = contract_id.clone();

        client.set_liquidity_token(&token.address());
        client.set_liquidity_source(&reserve);

        token_admin_client.mint(&reserve, &500_i128);
        client.draw_credit(&borrower, &120_i128);

        assert_eq!(token_client.balance(&reserve), 380_i128);
        assert_eq!(token_client.balance(&borrower), 120_i128);
        assert_eq!(token_client.balance(&contract_id), 380_i128);
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
        let token_admin = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);

        let token = env.register_stellar_asset_contract_v2(token_admin);
        let token_admin_client = StellarAssetClient::new(&env, &token.address());

        client.set_liquidity_token(&token.address());

        token_admin_client.mint(&contract_id, &50_i128);
        client.draw_credit(&borrower, &100_i128);
    }

    /// CreditError::from converts each variant to a contract error code.
    #[test]
    fn test_credit_error_from_conversion() {
        let err: soroban_sdk::Error = soroban_sdk::Error::from(CreditError::CreditLineNotFound);
        assert_eq!(err, soroban_sdk::Error::from_contract_error(1));

        let err2: soroban_sdk::Error = soroban_sdk::Error::from(CreditError::InvalidCreditStatus);
        assert_eq!(err2, soroban_sdk::Error::from_contract_error(2));

        let err3: soroban_sdk::Error = soroban_sdk::Error::from(CreditError::InvalidAmount);
        assert_eq!(err3, soroban_sdk::Error::from_contract_error(3));

        let err4: soroban_sdk::Error =
            soroban_sdk::Error::from(CreditError::InsufficientUtilization);
        assert_eq!(err4, soroban_sdk::Error::from_contract_error(4));

        let err5: soroban_sdk::Error = soroban_sdk::Error::from(CreditError::Unauthorized);
        assert_eq!(err5, soroban_sdk::Error::from_contract_error(5));
    }

    /// draw_credit panics with "overflow" when utilized_amount + amount overflows i128.
    #[test]
    #[should_panic(expected = "overflow")]
    fn test_draw_credit_overflow_panics() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        // Open with i128::MAX credit limit so the limit check won't fire first.
        client.init(&admin);
        client.open_credit_line(&borrower, &i128::MAX, &300_u32, &70_u32);

        // Manually set utilized_amount to i128::MAX so the next draw overflows.
        env.as_contract(&contract_id, || {
            let mut line: CreditLineData = env
                .storage()
                .persistent()
                .get::<Address, CreditLineData>(&borrower)
                .unwrap();
            line.utilized_amount = i128::MAX;
            env.storage().persistent().set(&borrower, &line);
        });

        // Any positive draw now causes checked_add to return None → panic "overflow".
        client.draw_credit(&borrower, &1_i128);
    }

    /// draw_credit succeeds on a Defaulted credit line (only Closed is blocked).
    #[test]
    fn test_draw_credit_allowed_on_defaulted_line() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.default_credit_line(&borrower);

        // Draw should succeed because draw_credit only blocks Closed status.
        client.draw_credit(&borrower, &100_i128);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.utilized_amount, 100);
        assert_eq!(line.status, CreditStatus::Defaulted);
    }

    /// repay_credit succeeds on a Defaulted credit line.
    #[test]
    fn test_repay_credit_allowed_on_defaulted_line() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.draw_credit(&borrower, &500_i128);
        client.default_credit_line(&borrower);

        client.repay_credit(&borrower, &200_i128);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.utilized_amount, 300);
        assert_eq!(line.status, CreditStatus::Defaulted);
    }

    /// open_credit_line allows re-opening a previously Closed credit line.
    #[test]
    fn test_open_credit_line_after_closed_succeeds() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.close_credit_line(&borrower, &admin);

        // Re-opening a Closed line should succeed.
        client.open_credit_line(&borrower, &2000_i128, &400_u32, &60_u32);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.credit_limit, 2000);
        assert_eq!(line.status, CreditStatus::Active);
    }

    /// open_credit_line allows re-opening a Defaulted credit line.
    #[test]
    fn test_open_credit_line_after_defaulted_succeeds() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.default_credit_line(&borrower);

        // Re-opening a Defaulted line should succeed.
        client.open_credit_line(&borrower, &1500_i128, &350_u32, &65_u32);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.credit_limit, 1500);
        assert_eq!(line.status, CreditStatus::Active);
    }

    /// Admin can force-close a Defaulted credit line.
    #[test]
    fn test_close_credit_line_defaulted_admin_force_close() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.default_credit_line(&borrower);

        client.close_credit_line(&borrower, &admin);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Closed);
    }

    /// Admin can force-close a Suspended credit line.
    #[test]
    fn test_close_credit_line_suspended_admin_force_close() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.suspend_credit_line(&borrower);

        client.close_credit_line(&borrower, &admin);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Closed);
    }

    /// open_credit_line allows re-opening a Suspended credit line.
    #[test]
    fn test_open_credit_line_after_suspended_succeeds() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.suspend_credit_line(&borrower);

        // Re-opening a Suspended line should succeed.
        client.open_credit_line(&borrower, &2000_i128, &400_u32, &60_u32);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.credit_limit, 2000);
        assert_eq!(line.status, CreditStatus::Active);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests: rate-change limits (issue #17)
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod test_rate_change_limits {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::testutils::Ledger;

    fn setup<'a>(
        env: &'a Env,
        borrower: &'a Address,
        credit_limit: i128,
        _reserve_amount: i128,
    ) -> (CreditClient<'a>, Address) {
        let admin = Address::generate(env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(env, &contract_id);
        client.init(&admin);
        client.open_credit_line(borrower, &credit_limit, &300_u32, &70_u32);
        (client, admin)
    }

    #[test]
    fn test_rate_change_within_limit_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _admin) = setup(&env, &borrower, 5_000, 0);

        client.set_rate_change_limits(&100_u32, &0_u64);
        client.update_risk_parameters(&borrower, &5_000_i128, &350_u32, &70_u32);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.interest_rate_bps, 350);
    }

    #[test]
    #[should_panic(expected = "rate change exceeds maximum allowed delta")]
    fn test_rate_change_over_limit_reverts() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _admin) = setup(&env, &borrower, 5_000, 0);

        client.set_rate_change_limits(&50_u32, &0_u64);
        // Current rate is 300; 300 + 51 = 351 → delta 51 > 50
        client.update_risk_parameters(&borrower, &5_000_i128, &351_u32, &70_u32);
    }

    #[test]
    #[should_panic(expected = "rate change exceeds maximum allowed delta")]
    fn test_rate_decrease_over_limit_reverts() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _admin) = setup(&env, &borrower, 5_000, 0);

        client.set_rate_change_limits(&50_u32, &0_u64);
        // Current rate is 300; 300 - 51 = 249 → delta 51 > 50
        client.update_risk_parameters(&borrower, &5_000_i128, &249_u32, &70_u32);
    }

    #[test]
    fn test_rate_change_at_exact_limit_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _admin) = setup(&env, &borrower, 5_000, 0);

        client.set_rate_change_limits(&50_u32, &0_u64);
        // Current rate 300; 300 + 50 = 350 → delta == limit
        client.update_risk_parameters(&borrower, &5_000_i128, &350_u32, &70_u32);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.interest_rate_bps, 350);
    }

    #[test]
    #[should_panic(expected = "rate change exceeds maximum allowed delta")]
    fn test_rate_change_one_over_limit_reverts() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _admin) = setup(&env, &borrower, 5_000, 0);

        client.set_rate_change_limits(&50_u32, &0_u64);
        // Current rate 300; 300 + 51 = 351 → delta 51 > 50
        client.update_risk_parameters(&borrower, &5_000_i128, &351_u32, &70_u32);
    }

    #[test]
    #[should_panic(expected = "rate change too soon: minimum interval not elapsed")]
    fn test_rate_change_within_interval_reverts() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _admin) = setup(&env, &borrower, 5_000, 0);

        // Allow up to 100 bps change but only every 3600 seconds (1 hour).
        client.set_rate_change_limits(&100_u32, &3600_u64);

        // First update at t=100.
        env.ledger().with_mut(|li| li.timestamp = 100);
        client.update_risk_parameters(&borrower, &5_000_i128, &350_u32, &70_u32);

        // Second update at t=200 (only 100 s later, < 3600).
        env.ledger().with_mut(|li| li.timestamp = 200);
        client.update_risk_parameters(&borrower, &5_000_i128, &330_u32, &70_u32);
    }

    #[test]
    fn test_rate_change_after_interval_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _admin) = setup(&env, &borrower, 5_000, 0);

        client.set_rate_change_limits(&100_u32, &3600_u64);

        env.ledger().with_mut(|li| li.timestamp = 100);
        client.update_risk_parameters(&borrower, &5_000_i128, &350_u32, &70_u32);

        // Advance past the interval.
        env.ledger().with_mut(|li| li.timestamp = 3701);
        client.update_risk_parameters(&borrower, &5_000_i128, &330_u32, &70_u32);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.interest_rate_bps, 330);
    }

    #[test]
    fn test_rate_change_at_exact_interval_boundary_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _admin) = setup(&env, &borrower, 5_000, 0);

        client.set_rate_change_limits(&100_u32, &3600_u64);

        env.ledger().with_mut(|li| li.timestamp = 100);
        client.update_risk_parameters(&borrower, &5_000_i128, &350_u32, &70_u32);

        // Exactly on the interval boundary: elapsed == 3600.
        env.ledger().with_mut(|li| li.timestamp = 3700);
        client.update_risk_parameters(&borrower, &5_000_i128, &330_u32, &70_u32);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.interest_rate_bps, 330);
        assert_eq!(line.last_rate_update_ts, 3700);
    }

    #[test]
    fn test_rate_change_first_update_ignores_interval() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _admin) = setup(&env, &borrower, 5_000, 0);

        // Interval set but first update should always pass (last_rate_update_ts == 0).
        client.set_rate_change_limits(&100_u32, &86400_u64);
        env.ledger().with_mut(|li| li.timestamp = 10);
        client.update_risk_parameters(&borrower, &5_000_i128, &350_u32, &70_u32);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.interest_rate_bps, 350);
    }

    #[test]
    fn test_zero_interval_disables_timing_check_after_first_update() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _admin) = setup(&env, &borrower, 5_000, 0);

        client.set_rate_change_limits(&100_u32, &0_u64);

        env.ledger().with_mut(|li| li.timestamp = 100);
        client.update_risk_parameters(&borrower, &5_000_i128, &350_u32, &70_u32);

        // Immediate subsequent update should still pass because interval == 0 disables the gate.
        env.ledger().with_mut(|li| li.timestamp = 101);
        client.update_risk_parameters(&borrower, &5_000_i128, &330_u32, &70_u32);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.interest_rate_bps, 330);
        assert_eq!(line.last_rate_update_ts, 101);
    }

    #[test]
    fn test_same_rate_bypasses_limits() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _admin) = setup(&env, &borrower, 5_000, 0);

        // Strict limits: 0 bps max change, huge interval.
        client.set_rate_change_limits(&0_u32, &999_999_u64);

        // Same rate (300 → 300) should still succeed.
        client.update_risk_parameters(&borrower, &5_000_i128, &300_u32, &70_u32);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.interest_rate_bps, 300);
    }

    #[test]
    fn test_no_rate_limits_configured_backward_compat() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _admin) = setup(&env, &borrower, 5_000, 0);

        // No set_rate_change_limits call → unlimited changes.
        client.update_risk_parameters(&borrower, &5_000_i128, &9_999_u32, &70_u32);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.interest_rate_bps, 9_999);
    }

    #[test]
    fn test_set_and_get_rate_change_limits() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);

        client.set_rate_change_limits(&200_u32, &7200_u64);
        let cfg = client.get_rate_change_limits().unwrap();

        assert_eq!(cfg.max_rate_change_bps, 200);
        assert_eq!(cfg.rate_change_min_interval, 7200);
    }

    #[test]
    fn test_rate_change_timestamp_recorded() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _admin) = setup(&env, &borrower, 5_000, 0);

        client.set_rate_change_limits(&100_u32, &0_u64);
        env.ledger().with_mut(|li| li.timestamp = 42);
        client.update_risk_parameters(&borrower, &5_000_i128, &350_u32, &70_u32);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.last_rate_update_ts, 42);
    }

    #[test]
    fn test_rate_change_multiple_sequential_within_limits() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _admin) = setup(&env, &borrower, 5_000, 0);

        client.set_rate_change_limits(&50_u32, &60_u64);

        // First update at t=100: 300 → 350
        env.ledger().with_mut(|li| li.timestamp = 100);
        client.update_risk_parameters(&borrower, &5_000_i128, &350_u32, &70_u32);

        // Second update at t=161: 350 → 320 (delta 30 ≤ 50)
        env.ledger().with_mut(|li| li.timestamp = 161);
        client.update_risk_parameters(&borrower, &5_000_i128, &320_u32, &65_u32);

        // Third update at t=222: 320 → 370 (delta 50 == limit)
        env.ledger().with_mut(|li| li.timestamp = 222);
        client.update_risk_parameters(&borrower, &5_000_i128, &370_u32, &60_u32);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.interest_rate_bps, 370);
        assert_eq!(line.risk_score, 60);
    }

    #[test]
    #[should_panic(expected = "Unauthorized")]
    fn test_set_rate_change_limits_unauthorized() {
        let env = Env::default();
        // NOTE: no mock_all_auths → admin auth will fail.
        let admin = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);

        client.set_rate_change_limits(&100_u32, &0_u64);
    }
}
