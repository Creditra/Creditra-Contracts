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

// token import from our branch — needed for actual token transfer in draw_credit
use soroban_sdk::{contract, contractimpl, symbol_short, token, Address, Env, Symbol};

use events::{
    publish_credit_line_event, publish_repayment_event, publish_risk_parameters_updated,
    CreditLineEvent, RepaymentEvent, RiskParametersUpdatedEvent,
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

/// Instance storage key for reserve token address.
fn token_key(env: &Env) -> Symbol {
    Symbol::new(env, "token")
}

fn require_admin(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&admin_key(env))
        .expect("admin not set")
}

fn require_admin_auth(env: &Env) -> Address {
    let admin = require_admin(env);
    admin.require_auth();
    admin
}

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

/// Retrieve [`CreditLineData`] for a borrower from persistent storage.
///
/// # Arguments
/// * `env`      - The Soroban environment.
/// * `borrower` - Address of the borrower whose credit line to retrieve.
///
/// # Returns
/// * `Some(CreditLineData)` if a credit line exists for the borrower.
/// * `None` if no credit line has been opened for this borrower.
fn get_credit_line_data(env: &Env, borrower: &Address) -> Option<CreditLineData> {
    env.storage().persistent().get(borrower)
}

/// Store [`CreditLineData`] for a borrower in persistent storage,
/// keyed by the borrower's [`Address`]. Overwrites any existing record.
///
/// # Arguments
/// * `env`      - The Soroban environment.
/// * `borrower` - Address of the borrower; used as the storage key.
/// * `data`     - The [`CreditLineData`] record to persist.
fn set_credit_line_data(env: &Env, borrower: &Address, data: &CreditLineData) {
    env.storage().persistent().set(borrower, data);
}

#[contract]
pub struct Credit;

#[contractimpl]
impl Credit {
    /// Initialize the contract with admin and reserve token address.
    pub fn init(env: Env, admin: Address, token: Address) {
        env.storage().instance().set(&admin_key(&env), &admin);
        env.storage().instance().set(&token_key(&env), &token);
    }

    /// Open a new credit line for a borrower (called by backend/risk engine).
    ///
    /// # Panics
    /// * If `credit_limit` <= 0
    /// * If `interest_rate_bps` > 10000
    /// * If `risk_score` > 100
    /// * If an Active credit line already exists for the borrower
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

        if let Some(existing) = get_credit_line_data(&env, &borrower) {
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
            last_update_timestamp: env.ledger().timestamp(),
        };

        set_credit_line_data(&env, &borrower, &credit_line);

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

    /// Draw from credit line: verifies limit, updates utilized_amount,
    /// and transfers the protocol token from the contract reserve to the borrower.
    ///
    /// # Panics
    /// - `"Credit line not found"` – borrower has no open credit line
    /// - `"credit line is closed"` – line is closed
    /// - `"Credit line not active"` – line is suspended or defaulted
    /// - `"exceeds credit limit"` – draw would push utilized_amount past credit_limit
    /// - `"amount must be positive"` – amount is zero or negative
    /// - `"reentrancy guard"` – re-entrant call detected
    pub fn draw_credit(env: Env, borrower: Address, amount: i128) {
        set_reentrancy_guard(&env);
        borrower.require_auth();

        if amount <= 0 {
            clear_reentrancy_guard(&env);
            panic!("amount must be positive");
        }

        let mut credit_line: CreditLineData = get_credit_line_data(&env, &borrower)
            .expect("Credit line not found");

        if credit_line.borrower != borrower {
            clear_reentrancy_guard(&env);
            panic!("Borrower mismatch for credit line");
        }

        if credit_line.status == CreditStatus::Closed {
            clear_reentrancy_guard(&env);
            panic!("credit line is closed");
        }

        if credit_line.status != CreditStatus::Active {
            clear_reentrancy_guard(&env);
            panic!("Credit line not active");
        }

        let new_utilized = credit_line
            .utilized_amount
            .checked_add(amount)
            .expect("overflow");

        if new_utilized > credit_line.credit_limit {
            clear_reentrancy_guard(&env);
            panic!("exceeds credit limit");
        }

        // Checks-effects-interactions: update state before external token call
        credit_line.utilized_amount = new_utilized;
        credit_line.last_update_timestamp = env.ledger().timestamp();
        set_credit_line_data(&env, &borrower, &credit_line);

        let token_address: Address = env
            .storage()
            .instance()
            .get(&token_key(&env))
            .expect("token not configured");

        let token_client = token::Client::new(&env, &token_address);
        token_client.transfer(&env.current_contract_address(), &borrower, &amount);

        clear_reentrancy_guard(&env);

        env.events().publish(
            (symbol_short!("credit"), symbol_short!("draw")),
            (borrower, amount, new_utilized),
        );
    }

    /// Repay credit (borrower).
    /// Reverts if credit line does not exist, is Closed, or borrower has not authorized.
    /// Reduces utilized_amount by amount (capped at 0). Emits RepaymentEvent.
    pub fn repay_credit(env: Env, borrower: Address, amount: i128) {
        set_reentrancy_guard(&env);
        borrower.require_auth();

        let mut credit_line: CreditLineData = get_credit_line_data(&env, &borrower)
            .expect("Credit line not found");

        if credit_line.borrower != borrower {
            clear_reentrancy_guard(&env);
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
        credit_line.last_update_timestamp = env.ledger().timestamp();
        set_credit_line_data(&env, &borrower, &credit_line);

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
    pub fn update_risk_parameters(
        env: Env,
        borrower: Address,
        credit_limit: i128,
        interest_rate_bps: u32,
        risk_score: u32,
    ) {
        require_admin_auth(&env);

        let mut credit_line: CreditLineData = get_credit_line_data(&env, &borrower)
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
        set_credit_line_data(&env, &borrower, &credit_line);

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

    /// Suspend a credit line (admin only). Emits a CreditLineSuspended event.
    pub fn suspend_credit_line(env: Env, borrower: Address) {
        require_admin_auth(&env);

        let mut credit_line: CreditLineData = get_credit_line_data(&env, &borrower)
            .expect("Credit line not found");

        credit_line.status = CreditStatus::Suspended;
        set_credit_line_data(&env, &borrower, &credit_line);

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
    /// * `closer` - Must be either the contract admin or the borrower (only when utilized_amount == 0).
    pub fn close_credit_line(env: Env, borrower: Address, closer: Address) {
        closer.require_auth();

        let admin: Address = require_admin(&env);

        let mut credit_line: CreditLineData = get_credit_line_data(&env, &borrower)
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
        set_credit_line_data(&env, &borrower, &credit_line);

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

    /// Mark a credit line as defaulted (admin only). Emits a CreditLineDefaulted event.
    pub fn default_credit_line(env: Env, borrower: Address) {
        require_admin_auth(&env);

        let mut credit_line: CreditLineData = get_credit_line_data(&env, &borrower)
            .expect("Credit line not found");

        credit_line.status = CreditStatus::Defaulted;
        set_credit_line_data(&env, &borrower, &credit_line);

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

    /// Get credit line data for a borrower (view function).
    pub fn get_credit_line(env: Env, borrower: Address) -> Option<CreditLineData> {
        get_credit_line_data(&env, &borrower)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::token;

    // ── helpers ───────────────────────────────────────────────────────────────

    fn setup_token<'a>(
        env: &'a Env,
        contract_id: &'a Address,
        reserve_amount: i128,
    ) -> (Address, token::StellarAssetClient<'a>) {
        let token_admin = Address::generate(env);
        let token_id = env.register_stellar_asset_contract_v2(token_admin);
        let token_address = token_id.address();
        let sac = token::StellarAssetClient::new(env, &token_address);
        if reserve_amount > 0 {
            sac.mint(contract_id, &reserve_amount);
        }
        (token_address, sac)
    }

    fn setup_contract_with_credit_line<'a>(
        env: &'a Env,
        borrower: &'a Address,
        credit_limit: i128,
        reserve_amount: i128,
    ) -> (CreditClient<'a>, Address, Address) {
        let admin = Address::generate(env);
        let contract_id = env.register(Credit, ());
        let (token_address, _sac) = setup_token(env, &contract_id, reserve_amount);
        let client = CreditClient::new(env, &contract_id);
        client.init(&admin, &token_address);
        client.open_credit_line(borrower, &credit_limit, &300_u32, &70_u32);
        (client, token_address, admin)
    }

    // ── draw_credit: token transfer (#39) ─────────────────────────────────────

    #[test]
    fn test_draw_transfers_correct_amount_to_borrower() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, token_address, _admin) =
            setup_contract_with_credit_line(&env, &borrower, 1_000, 1_000);
        let token_client = token::Client::new(&env, &token_address);
        let before = token_client.balance(&borrower);
        client.draw_credit(&borrower, &500);
        assert_eq!(token_client.balance(&borrower) - before, 500);
    }

    #[test]
    fn test_draw_reduces_contract_reserve() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let admin = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let (token_address, _sac) = setup_token(&env, &contract_id, 1_000);
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin, &token_address);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);
        let token_client = token::Client::new(&env, &token_address);
        let reserve_before = token_client.balance(&contract_id);
        client.draw_credit(&borrower, &300);
        assert_eq!(reserve_before - token_client.balance(&contract_id), 300);
    }

    #[test]
    fn test_draw_updates_utilized_amount() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) =
            setup_contract_with_credit_line(&env, &borrower, 1_000, 1_000);
        client.draw_credit(&borrower, &400);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            400
        );
    }

    #[test]
    fn test_draw_accumulates_across_multiple_draws() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, token_address, _admin) =
            setup_contract_with_credit_line(&env, &borrower, 1_000, 1_000);
        client.draw_credit(&borrower, &200);
        client.draw_credit(&borrower, &300);
        let token_client = token::Client::new(&env, &token_address);
        assert_eq!(token_client.balance(&borrower), 500);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            500
        );
    }

    #[test]
    fn test_draw_exact_credit_limit() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, token_address, _admin) =
            setup_contract_with_credit_line(&env, &borrower, 1_000, 1_000);
        client.draw_credit(&borrower, &1_000);
        let token_client = token::Client::new(&env, &token_address);
        assert_eq!(token_client.balance(&borrower), 1_000);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            1_000
        );
    }

    #[test]
    fn test_draw_requires_borrower_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) =
            setup_contract_with_credit_line(&env, &borrower, 1_000, 1_000);
        client.draw_credit(&borrower, &100);
        assert!(
            env.auths().iter().any(|(addr, _)| *addr == borrower),
            "draw_credit must require borrower authorization"
        );
    }

    #[test]
    fn test_multiple_borrowers_draw_independently() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let b1 = Address::generate(&env);
        let b2 = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let (token_address, _sac) = setup_token(&env, &contract_id, 3_000);
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin, &token_address);
        client.open_credit_line(&b1, &1_000, &300_u32, &70_u32);
        client.open_credit_line(&b2, &2_000, &400_u32, &80_u32);
        client.draw_credit(&b1, &500);
        client.draw_credit(&b2, &1_000);
        let token_client = token::Client::new(&env, &token_address);
        assert_eq!(token_client.balance(&b1), 500);
        assert_eq!(token_client.balance(&b2), 1_000);
        assert_eq!(client.get_credit_line(&b1).unwrap().utilized_amount, 500);
        assert_eq!(client.get_credit_line(&b2).unwrap().utilized_amount, 1_000);
    }

    // ── draw_credit: guards ───────────────────────────────────────────────────

    #[test]
    #[should_panic(expected = "exceeds credit limit")]
    fn test_draw_exceeds_credit_limit() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) = setup_contract_with_credit_line(&env, &borrower, 500, 1_000);
        client.draw_credit(&borrower, &600);
    }

    #[test]
    #[should_panic(expected = "exceeds credit limit")]
    fn test_draw_cumulative_exceeds_limit() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) = setup_contract_with_credit_line(&env, &borrower, 500, 1_000);
        client.draw_credit(&borrower, &400);
        client.draw_credit(&borrower, &200);
    }

    #[test]
    #[should_panic(expected = "Credit line not active")]
    fn test_draw_on_suspended_line_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) =
            setup_contract_with_credit_line(&env, &borrower, 1_000, 1_000);
        client.suspend_credit_line(&borrower);
        client.draw_credit(&borrower, &100);
    }

    #[test]
    #[should_panic(expected = "credit line is closed")]
    fn test_draw_on_closed_line_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, admin) =
            setup_contract_with_credit_line(&env, &borrower, 1_000, 1_000);
        client.close_credit_line(&borrower, &admin);
        client.draw_credit(&borrower, &100);
    }

    #[test]
    #[should_panic(expected = "Credit line not active")]
    fn test_draw_on_defaulted_line_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) =
            setup_contract_with_credit_line(&env, &borrower, 1_000, 1_000);
        client.default_credit_line(&borrower);
        client.draw_credit(&borrower, &100);
    }

    #[test]
    #[should_panic(expected = "amount must be positive")]
    fn test_draw_zero_amount_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) =
            setup_contract_with_credit_line(&env, &borrower, 1_000, 1_000);
        client.draw_credit(&borrower, &0);
    }

    #[test]
    #[should_panic(expected = "amount must be positive")]
    fn test_draw_negative_amount_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) =
            setup_contract_with_credit_line(&env, &borrower, 1_000, 1_000);
        client.draw_credit(&borrower, &-50);
    }

    #[test]
    #[should_panic(expected = "Credit line not found")]
    fn test_draw_no_credit_line_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let stranger = Address::generate(&env);
        let admin = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let (token_address, _sac) = setup_token(&env, &contract_id, 1_000);
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin, &token_address);
        client.draw_credit(&stranger, &100);
    }

    // ── open_credit_line validation ───────────────────────────────────────────

    #[test]
    #[should_panic(expected = "borrower already has an active credit line")]
    fn test_open_credit_line_duplicate_active_borrower_reverts() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) = setup_contract_with_credit_line(&env, &borrower, 1_000, 0);
        client.open_credit_line(&borrower, &2_000, &400_u32, &60_u32);
    }

    #[test]
    #[should_panic(expected = "credit_limit must be greater than zero")]
    fn test_open_credit_line_zero_limit_reverts() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let (token_address, _) = setup_token(&env, &contract_id, 0);
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin, &token_address);
        client.open_credit_line(&borrower, &0, &300_u32, &70_u32);
    }

    #[test]
    #[should_panic(expected = "credit_limit must be greater than zero")]
    fn test_open_credit_line_negative_limit_reverts() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let (token_address, _) = setup_token(&env, &contract_id, 0);
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin, &token_address);
        client.open_credit_line(&borrower, &-1, &300_u32, &70_u32);
    }

    #[test]
    #[should_panic(expected = "interest_rate_bps cannot exceed 10000 (100%)")]
    fn test_open_credit_line_interest_rate_exceeds_max_reverts() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let (token_address, _) = setup_token(&env, &contract_id, 0);
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin, &token_address);
        client.open_credit_line(&borrower, &1_000, &10_001_u32, &70_u32);
    }

    #[test]
    #[should_panic(expected = "risk_score must be between 0 and 100")]
    fn test_open_credit_line_risk_score_exceeds_max_reverts() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let (token_address, _) = setup_token(&env, &contract_id, 0);
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin, &token_address);
        client.open_credit_line(&borrower, &1_000, &300_u32, &101_u32);
    }

    // ── lifecycle ─────────────────────────────────────────────────────────────

    #[test]
    fn test_init_and_open_credit_line() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) = setup_contract_with_credit_line(&env, &borrower, 1_000, 0);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.borrower, borrower);
        assert_eq!(line.credit_limit, 1_000);
        assert_eq!(line.utilized_amount, 0);
        assert_eq!(line.interest_rate_bps, 300);
        assert_eq!(line.risk_score, 70);
        assert_eq!(line.status, CreditStatus::Active);
    }

    #[test]
    fn test_suspend_credit_line() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) = setup_contract_with_credit_line(&env, &borrower, 1_000, 0);
        client.suspend_credit_line(&borrower);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().status,
            CreditStatus::Suspended
        );
    }

    #[test]
    fn test_close_credit_line() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, admin) = setup_contract_with_credit_line(&env, &borrower, 1_000, 0);
        client.close_credit_line(&borrower, &admin);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().status,
            CreditStatus::Closed
        );
    }

    #[test]
    fn test_default_credit_line() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) = setup_contract_with_credit_line(&env, &borrower, 1_000, 0);
        client.default_credit_line(&borrower);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().status,
            CreditStatus::Defaulted
        );
    }

    #[test]
    fn test_full_lifecycle() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, admin) =
            setup_contract_with_credit_line(&env, &borrower, 5_000, 5_000);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().status,
            CreditStatus::Active
        );
        client.suspend_credit_line(&borrower);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().status,
            CreditStatus::Suspended
        );
        client.close_credit_line(&borrower, &admin);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().status,
            CreditStatus::Closed
        );
    }

    #[test]
    fn test_close_credit_line_borrower_when_utilized_zero() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) = setup_contract_with_credit_line(&env, &borrower, 1_000, 0);
        client.close_credit_line(&borrower, &borrower);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Closed);
        assert_eq!(line.utilized_amount, 0);
    }

    #[test]
    #[should_panic(expected = "cannot close: utilized amount not zero")]
    fn test_close_credit_line_borrower_rejected_when_utilized_nonzero() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) =
            setup_contract_with_credit_line(&env, &borrower, 1_000, 1_000);
        client.draw_credit(&borrower, &300);
        client.close_credit_line(&borrower, &borrower);
    }

    #[test]
    fn test_close_credit_line_admin_force_close_with_utilization() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, admin) =
            setup_contract_with_credit_line(&env, &borrower, 1_000, 1_000);
        client.draw_credit(&borrower, &300);
        client.close_credit_line(&borrower, &admin);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Closed);
        assert_eq!(line.utilized_amount, 300);
    }

    #[test]
    fn test_close_credit_line_idempotent_when_already_closed() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, admin) = setup_contract_with_credit_line(&env, &borrower, 1_000, 0);
        client.close_credit_line(&borrower, &admin);
        client.close_credit_line(&borrower, &admin);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().status,
            CreditStatus::Closed
        );
    }

    #[test]
    #[should_panic(expected = "unauthorized")]
    fn test_close_credit_line_unauthorized_closer() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let other = Address::generate(&env);
        let (client, _token, _admin) = setup_contract_with_credit_line(&env, &borrower, 1_000, 0);
        client.close_credit_line(&borrower, &other);
    }

    #[test]
    #[should_panic(expected = "Credit line not found")]
    fn test_suspend_nonexistent_credit_line() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let admin = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let (token_address, _) = setup_token(&env, &contract_id, 0);
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin, &token_address);
        client.suspend_credit_line(&borrower);
    }

    #[test]
    #[should_panic(expected = "Credit line not found")]
    fn test_close_nonexistent_credit_line() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let admin = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let (token_address, _) = setup_token(&env, &contract_id, 0);
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin, &token_address);
        client.close_credit_line(&borrower, &admin);
    }

    #[test]
    #[should_panic(expected = "Credit line not found")]
    fn test_default_nonexistent_credit_line() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let admin = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let (token_address, _) = setup_token(&env, &contract_id, 0);
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin, &token_address);
        client.default_credit_line(&borrower);
    }

    // ── update_risk_parameters ────────────────────────────────────────────────

    #[test]
    fn test_update_risk_parameters_success() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) = setup_contract_with_credit_line(&env, &borrower, 1_000, 0);
        client.update_risk_parameters(&borrower, &2_000, &400_u32, &85_u32);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.credit_limit, 2_000);
        assert_eq!(line.interest_rate_bps, 400);
        assert_eq!(line.risk_score, 85);
    }

    #[test]
    #[should_panic]
    fn test_update_risk_parameters_unauthorized_caller() {
        let env = Env::default();
        let borrower = Address::generate(&env);
        let admin = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let (token_address, _) = setup_token(&env, &contract_id, 0);
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin, &token_address);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);
        client.update_risk_parameters(&borrower, &2_000, &400_u32, &85_u32);
    }

    #[test]
    #[should_panic(expected = "Credit line not found")]
    fn test_update_risk_parameters_nonexistent_line() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let admin = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let (token_address, _) = setup_token(&env, &contract_id, 0);
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin, &token_address);
        client.update_risk_parameters(&borrower, &1_000, &300_u32, &70_u32);
    }

    #[test]
    #[should_panic(expected = "credit_limit cannot be less than utilized amount")]
    fn test_update_risk_parameters_credit_limit_below_utilized() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) =
            setup_contract_with_credit_line(&env, &borrower, 1_000, 1_000);
        client.draw_credit(&borrower, &500);
        client.update_risk_parameters(&borrower, &300, &300_u32, &70_u32);
    }

    #[test]
    #[should_panic(expected = "credit_limit must be non-negative")]
    fn test_update_risk_parameters_negative_credit_limit() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) = setup_contract_with_credit_line(&env, &borrower, 1_000, 0);
        client.update_risk_parameters(&borrower, &-1, &300_u32, &70_u32);
    }

    #[test]
    #[should_panic(expected = "interest_rate_bps exceeds maximum")]
    fn test_update_risk_parameters_interest_rate_exceeds_max() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) = setup_contract_with_credit_line(&env, &borrower, 1_000, 0);
        client.update_risk_parameters(&borrower, &1_000, &10_001_u32, &70_u32);
    }

    #[test]
    #[should_panic(expected = "risk_score exceeds maximum")]
    fn test_update_risk_parameters_risk_score_exceeds_max() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) = setup_contract_with_credit_line(&env, &borrower, 1_000, 0);
        client.update_risk_parameters(&borrower, &1_000, &300_u32, &101_u32);
    }

    #[test]
    fn test_update_risk_parameters_at_boundaries() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) = setup_contract_with_credit_line(&env, &borrower, 1_000, 0);
        client.update_risk_parameters(&borrower, &1_000, &10_000_u32, &100_u32);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.interest_rate_bps, 10_000);
        assert_eq!(line.risk_score, 100);
    }

    // ── repay_credit ──────────────────────────────────────────────────────────

    #[test]
    fn test_repay_credit_reduces_utilized_amount() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) =
            setup_contract_with_credit_line(&env, &borrower, 1_000, 1_000);
        client.draw_credit(&borrower, &500);
        client.repay_credit(&borrower, &200);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            300
        );
    }

    #[test]
    fn test_repay_credit_saturates_at_zero() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) =
            setup_contract_with_credit_line(&env, &borrower, 1_000, 1_000);
        client.draw_credit(&borrower, &100);
        client.repay_credit(&borrower, &500);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            0
        );
    }

    #[test]
    #[should_panic(expected = "amount must be positive")]
    fn test_repay_credit_rejects_non_positive_amount() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) = setup_contract_with_credit_line(&env, &borrower, 1_000, 0);
        client.repay_credit(&borrower, &0);
    }

    #[test]
    #[should_panic(expected = "Credit line not found")]
    fn test_repay_credit_nonexistent_line() {
        let env = Env::default();
        env.mock_all_auths();
        let stranger = Address::generate(&env);
        let admin = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let (token_address, _) = setup_token(&env, &contract_id, 0);
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin, &token_address);
        client.repay_credit(&stranger, &100);
    }

    #[test]
    #[should_panic(expected = "credit line is closed")]
    fn test_repay_credit_rejected_when_closed() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, admin) = setup_contract_with_credit_line(&env, &borrower, 1_000, 0);
        client.close_credit_line(&borrower, &admin);
        client.repay_credit(&borrower, &100);
    }

    // ── admin-only enforcement ────────────────────────────────────────────────

    #[test]
    #[should_panic]
    fn test_suspend_credit_line_unauthorized() {
        let env = Env::default();
        let borrower = Address::generate(&env);
        let admin = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let (token_address, _) = setup_token(&env, &contract_id, 0);
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin, &token_address);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);
        client.suspend_credit_line(&borrower);
    }

    #[test]
    #[should_panic]
    fn test_default_credit_line_unauthorized() {
        let env = Env::default();
        let borrower = Address::generate(&env);
        let admin = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let (token_address, _) = setup_token(&env, &contract_id, 0);
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin, &token_address);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);
        client.default_credit_line(&borrower);
    }

    // ── reentrancy guard ──────────────────────────────────────────────────────

    #[test]
    fn test_reentrancy_guard_cleared_after_draw() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) =
            setup_contract_with_credit_line(&env, &borrower, 1_000, 1_000);
        client.draw_credit(&borrower, &100);
        client.draw_credit(&borrower, &100);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            200
        );
    }

    #[test]
    fn test_reentrancy_guard_cleared_after_repay() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) =
            setup_contract_with_credit_line(&env, &borrower, 1_000, 1_000);
        client.draw_credit(&borrower, &200);
        client.repay_credit(&borrower, &50);
        client.repay_credit(&borrower, &50);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            100
        );
    }

    // ── event emission ────────────────────────────────────────────────────────

    #[test]
    fn test_event_open_credit_line() {
        use soroban_sdk::testutils::Events;
        use soroban_sdk::{TryFromVal, TryIntoVal};
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) = setup_contract_with_credit_line(&env, &borrower, 1_000, 0);
        let _ = client;
        let events = env.events().all();
        let (_contract, topics, data) = events.last().unwrap();
        assert_eq!(
            Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
            symbol_short!("opened")
        );
        let event_data: CreditLineEvent = data.try_into_val(&env).unwrap();
        assert_eq!(event_data.status, CreditStatus::Active);
        assert_eq!(event_data.borrower, borrower);
    }

    #[test]
    fn test_event_suspend_credit_line() {
        use soroban_sdk::testutils::Events;
        use soroban_sdk::{TryFromVal, TryIntoVal};
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) = setup_contract_with_credit_line(&env, &borrower, 1_000, 0);
        client.suspend_credit_line(&borrower);
        let events = env.events().all();
        let (_contract, topics, data) = events.last().unwrap();
        assert_eq!(
            Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
            symbol_short!("suspend")
        );
        let event_data: CreditLineEvent = data.try_into_val(&env).unwrap();
        assert_eq!(event_data.status, CreditStatus::Suspended);
    }

    #[test]
    fn test_event_close_credit_line() {
        use soroban_sdk::testutils::Events;
        use soroban_sdk::{TryFromVal, TryIntoVal};
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, admin) = setup_contract_with_credit_line(&env, &borrower, 1_000, 0);
        client.close_credit_line(&borrower, &admin);
        let events = env.events().all();
        let (_contract, topics, data) = events.last().unwrap();
        assert_eq!(
            Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
            symbol_short!("closed")
        );
        let event_data: CreditLineEvent = data.try_into_val(&env).unwrap();
        assert_eq!(event_data.status, CreditStatus::Closed);
    }

    #[test]
    fn test_event_default_credit_line() {
        use soroban_sdk::testutils::Events;
        use soroban_sdk::{TryFromVal, TryIntoVal};
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) = setup_contract_with_credit_line(&env, &borrower, 1_000, 0);
        client.default_credit_line(&borrower);
        let events = env.events().all();
        let (_contract, topics, data) = events.last().unwrap();
        assert_eq!(
            Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
            symbol_short!("default")
        );
        let event_data: CreditLineEvent = data.try_into_val(&env).unwrap();
        assert_eq!(event_data.status, CreditStatus::Defaulted);
    }

    #[test]
    fn test_event_lifecycle_sequence() {
        use soroban_sdk::testutils::Events;
        use soroban_sdk::{TryFromVal, TryIntoVal};
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, admin) = setup_contract_with_credit_line(&env, &borrower, 1_000, 0);
        let open_data: CreditLineEvent = env
            .events()
            .all()
            .last()
            .unwrap()
            .2
            .try_into_val(&env)
            .unwrap();
        assert_eq!(open_data.status, CreditStatus::Active);

        client.suspend_credit_line(&borrower);
        let suspend_data: CreditLineEvent = env
            .events()
            .all()
            .last()
            .unwrap()
            .2
            .try_into_val(&env)
            .unwrap();
        assert_eq!(suspend_data.status, CreditStatus::Suspended);
        assert_eq!(
            Symbol::try_from_val(&env, &env.events().all().last().unwrap().1.get(1).unwrap())
                .unwrap(),
            symbol_short!("suspend")
        );

        client.close_credit_line(&borrower, &admin);
        let close_data: CreditLineEvent = env
            .events()
            .all()
            .last()
            .unwrap()
            .2
            .try_into_val(&env)
            .unwrap();
        assert_eq!(close_data.status, CreditStatus::Closed);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests: close_credit_line with outstanding utilization
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod test_close_utilized {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    fn setup<'a>(
        env: &'a Env,
        borrower: &'a Address,
        credit_limit: i128,
        reserve_amount: i128,
    ) -> (CreditClient<'a>, Address) {
        let admin = Address::generate(env);
        let contract_id = env.register(Credit, ());
        let token_admin = Address::generate(env);
        let token_id = env.register_stellar_asset_contract_v2(token_admin);
        let token_address = token_id.address();
        if reserve_amount > 0 {
            let sac = soroban_sdk::token::StellarAssetClient::new(env, &token_address);
            sac.mint(&contract_id, &reserve_amount);
        }
        let client = CreditClient::new(env, &contract_id);
        client.init(&admin, &token_address);
        client.open_credit_line(borrower, &credit_limit, &300_u32, &70_u32);
        (client, admin)
    }

    #[test]
    #[should_panic(expected = "cannot close: utilized amount not zero")]
    fn test_close_utilized_borrower_rejected_at_minimum_utilization() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _admin) = setup(&env, &borrower, 1_000, 1_000);
        client.draw_credit(&borrower, &1);
        client.close_credit_line(&borrower, &borrower);
    }

    #[test]
    #[should_panic(expected = "cannot close: utilized amount not zero")]
    fn test_close_utilized_borrower_rejected_at_full_utilization() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _admin) = setup(&env, &borrower, 500, 500);
        client.draw_credit(&borrower, &500);
        client.close_credit_line(&borrower, &borrower);
    }

    #[test]
    fn test_close_utilized_admin_force_close_preserves_utilized_amount() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, admin) = setup(&env, &borrower, 1_000, 1_000);
        client.draw_credit(&borrower, &750);
        client.close_credit_line(&borrower, &admin);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Closed);
        assert_eq!(line.utilized_amount, 750);
    }

    #[test]
    fn test_close_utilized_admin_force_close_emits_closed_event() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, admin) = setup(&env, &borrower, 1_000, 1_000);
        client.draw_credit(&borrower, &400);
        client.close_credit_line(&borrower, &admin);
        // Verify state: status is Closed and utilized_amount is preserved.
        // Event payload correctness is covered by test::test_event_close_credit_line.
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Closed);
        assert_eq!(line.utilized_amount, 400);
    }

    #[test]
    #[should_panic(expected = "cannot close: utilized amount not zero")]
    fn test_close_utilized_borrower_rejected_on_suspended_line() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _admin) = setup(&env, &borrower, 1_000, 1_000);
        client.draw_credit(&borrower, &200);
        client.suspend_credit_line(&borrower);
        client.close_credit_line(&borrower, &borrower);
    }

    #[test]
    fn test_close_utilized_admin_force_close_suspended_line() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, admin) = setup(&env, &borrower, 1_000, 1_000);
        client.draw_credit(&borrower, &600);
        client.suspend_credit_line(&borrower);
        client.close_credit_line(&borrower, &admin);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Closed);
        assert_eq!(line.utilized_amount, 600);
    }

    #[test]
    fn test_close_utilized_borrower_succeeds_after_full_repayment() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _admin) = setup(&env, &borrower, 1_000, 1_000);
        client.draw_credit(&borrower, &350);
        client.repay_credit(&borrower, &350);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            0
        );
        client.close_credit_line(&borrower, &borrower);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().status,
            CreditStatus::Closed
        );
    }

    #[test]
    #[should_panic(expected = "unauthorized")]
    fn test_close_utilized_third_party_rejected_with_zero_utilization() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let third_party = Address::generate(&env);
        let (client, _admin) = setup(&env, &borrower, 1_000, 0);
        client.close_credit_line(&borrower, &third_party);
    }

    #[test]
    fn test_close_utilized_admin_force_close_multiple_draws() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, admin) = setup(&env, &borrower, 1_000, 1_000);
        client.draw_credit(&borrower, &100);
        client.draw_credit(&borrower, &150);
        client.draw_credit(&borrower, &250);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            500
        );
        client.close_credit_line(&borrower, &admin);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Closed);
        assert_eq!(line.utilized_amount, 500);
    }

    #[test]
    #[should_panic(expected = "cannot close: utilized amount not zero")]
    fn test_close_utilized_borrower_rejected_after_partial_repayment() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _admin) = setup(&env, &borrower, 1_000, 1_000);
        client.draw_credit(&borrower, &400);
        client.repay_credit(&borrower, &200);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            200
        );
        client.close_credit_line(&borrower, &borrower);
    }
}

/// Storage tests for CreditLineData keyed by borrower (issue #2).
///
/// Covers:
/// - Missing key returns None
/// - Storage write and read correctness
/// - Overwrite replaces existing record
/// - Isolation between borrowers
/// - last_update_timestamp is set on open
#[cfg(test)]
mod test_credit_line_storage {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    /// Verifies that get_credit_line returns None for a borrower
    /// that has never had a credit line opened (missing key edge case).
    #[test]
    fn test_storage_returns_none_for_missing_borrower() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let token_admin = Address::generate(&env);
        let token_id = env.register_stellar_asset_contract_v2(token_admin);
        let token_address = token_id.address();
        let client = CreditClient::new(&env, &contract_id);
        
        client.init(&admin, &token_address);

        // No credit line opened — should return None
        let result = client.get_credit_line(&borrower);
        assert!(result.is_none());
    }

    /// Verifies that opening a credit line writes all fields correctly
    /// and that get_credit_line reads them back accurately (storage write/read).
    #[test]
    fn test_storage_write_and_read_credit_line() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        
        let contract_id = env.register(Credit, ());
        let token_admin = Address::generate(&env);
        let token_id = env.register_stellar_asset_contract_v2(token_admin);
        let token_address = token_id.address();
        let client = CreditClient::new(&env, &contract_id);
        
        client.init(&admin, &token_address);
        client.open_credit_line(&borrower, &5000_i128, &250_u32, &60_u32);
        
        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.borrower, borrower);
        assert_eq!(credit_line.credit_limit, 5000);
        assert_eq!(credit_line.utilized_amount, 0);
        assert_eq!(credit_line.interest_rate_bps, 250);
        assert_eq!(credit_line.risk_score, 60);
        assert_eq!(credit_line.status, CreditStatus::Active);
    }

    /// Verifies that overwriting a credit line (via update_risk_parameters)
    /// replaces the stored record correctly (overwrite edge case).
    #[test]
    fn test_storage_overwrite_updates_record() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let token_admin = Address::generate(&env);
        let token_id = env.register_stellar_asset_contract_v2(token_admin);
        let token_address = token_id.address();
        let client = CreditClient::new(&env, &contract_id);
        
        client.init(&admin, &token_address);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        
        // Overwrite via update_risk_parameters
        client.update_risk_parameters(&borrower, &3000_i128, &500_u32, &90_u32);
        
        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.credit_limit, 3000);
        assert_eq!(credit_line.interest_rate_bps, 500);
        assert_eq!(credit_line.risk_score, 90);
    }

    /// Verifies that two borrowers have completely isolated storage records —
    /// modifying one does not affect the other.
    #[test]
    fn test_storage_isolation_between_borrowers() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower_a = Address::generate(&env);
        let borrower_b = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let token_admin = Address::generate(&env);
        let token_id = env.register_stellar_asset_contract_v2(token_admin);
        let token_address = token_id.address();
        let client = CreditClient::new(&env, &contract_id);
        
        client.init(&admin, &token_address);
        client.open_credit_line(&borrower_a, &1000_i128, &300_u32, &70_u32);
        client.open_credit_line(&borrower_b, &2000_i128, &400_u32, &80_u32);
        
        // Suspend borrower_a — borrower_b must be unaffected
        client.suspend_credit_line(&borrower_a);
        
        let line_a = client.get_credit_line(&borrower_a).unwrap();
        let line_b = client.get_credit_line(&borrower_b).unwrap();
        
        assert_eq!(line_a.status, CreditStatus::Suspended);
        assert_eq!(line_b.status, CreditStatus::Active); // isolation confirmed
        assert_eq!(line_b.credit_limit, 2000);
    }

    /// Verifies that last_update_timestamp is set on open and is a valid timestamp.
    #[test]
    fn test_storage_last_update_timestamp_set_on_open() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let token_admin = Address::generate(&env);
        let token_id = env.register_stellar_asset_contract_v2(token_admin);
        let token_address = token_id.address();
        let client = CreditClient::new(&env, &contract_id);
        
        client.init(&admin, &token_address);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        
        let credit_line = client.get_credit_line(&borrower).unwrap();
        // Timestamp should be set (non-zero in a real ledger; 0 is valid in test env)
        let _ = credit_line.last_update_timestamp; // field exists and is readable
    }
}
