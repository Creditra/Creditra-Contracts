#![no_std]
//! # Creditra Credit Line Contract
//!
//! This contract manages credit lines for borrowers in the Creditra adaptive credit protocol.
//!
//! ## Features
//!
//! - Open and manage credit lines with validated parameters
//! - Track credit utilization and status
//! - Update risk parameters with bounds checking
//! - Lifecycle management (Active, Suspended, Defaulted, Closed)
//!
//! ## Parameter Bounds
//!
//! ### Credit Limit
//! - **Minimum**: 1 (must be greater than 0)
//! - **Maximum**: 100,000,000 units
//! - **Validation**: Applied in `open_credit_line` and `update_risk_parameters`
//!
//! ### Interest Rate (in basis points)
//! - **Minimum**: 0 bps (0%) - implicit as u32 type
//! - **Maximum**: 10,000 bps (100%)
//! - **Note**: 1 basis point = 0.01%
//! - **Validation**: Applied in `open_credit_line` and `update_risk_parameters`
//!
//! ## Security
//!
//! All input parameters are validated before storage. Invalid parameters will cause
//! the transaction to revert with a clear error message indicating the validation failure.

use soroban_sdk::{contract, contractimpl, symbol_short, Address, Env, Symbol};

mod events;
mod types;

use events::{
    publish_credit_line_event, publish_drawn_event, publish_repayment_event,
    publish_risk_parameters_updated, CreditLineEvent, DrawnEvent, RepaymentEvent,
    RiskParametersUpdatedEvent,
};
use types::{CreditLineData, CreditStatus};

// Validation bounds for credit parameters
/// Maximum credit limit allowed (100 million units)
const MAX_CREDIT_LIMIT: i128 = 100_000_000;

/// Maximum interest rate in basis points (100% = 10,000 bps)
/// Note: Minimum is 0 bps (implicit, as interest_rate_bps is u32)
const MAX_INTEREST_RATE_BPS: u32 = 10_000;

/// Validate credit limit bounds
fn validate_credit_limit(credit_limit: i128) {
    if credit_limit <= 0 {
        panic!("Credit limit must be greater than 0");
    }
    if credit_limit > MAX_CREDIT_LIMIT {
        panic!("Credit limit exceeds maximum allowed");
    }
}

/// Validate interest rate bounds (u32 is always >= 0)
fn validate_interest_rate(interest_rate_bps: u32) {
    if interest_rate_bps > MAX_INTEREST_RATE_BPS {
        panic!("Interest rate exceeds maximum allowed");
    }
}

// ── Storage keys ──────────────────────────────────────────────────────────────

fn admin_key(env: &Env) -> Symbol {
    symbol_short!("admin")
}

fn token_key(env: &Env) -> Symbol {
    symbol_short!("token")
}

fn reentrancy_key(env: &Env) -> Symbol {
    symbol_short!("reent")
}

// ── Reentrancy guard ──────────────────────────────────────────────────────────

fn set_reentrancy_guard(env: &Env) {
    if env.storage().instance().has(&reentrancy_key(env)) {
        panic!("reentrancy guard");
    }
    env.storage().instance().set(&reentrancy_key(env), &true);
}

fn clear_reentrancy_guard(env: &Env) {
    env.storage().instance().remove(&reentrancy_key(env));
}

// ── Admin helpers ─────────────────────────────────────────────────────────────

fn require_admin(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&admin_key(env))
        .expect("admin not set")
}

fn require_admin_auth(env: &Env) {
    let admin = require_admin(env);
    admin.require_auth();
}

// ── Token client ──────────────────────────────────────────────────────────────

mod token {
    soroban_sdk::contractimport!(
        file = "../../target/wasm32-unknown-unknown/release/soroban_token_contract.wasm"
    );
}

#[contract]
pub struct Credit;

#[contractimpl]
impl Credit {
    /// Initialize the contract with admin and reserve token address.
    pub fn init(env: Env, admin: Address, token: Address) {
        if env.storage().instance().has(&admin_key(&env)) {
            panic!("Already initialized");
        }
        env.storage().instance().set(&admin_key(&env), &admin);
        env.storage().instance().set(&token_key(&env), &token);
    }

    /// Open a new credit line for a borrower (called by backend/risk engine).
    ///
    /// # Parameters
    /// - `borrower`: Address of the borrower
    /// - `credit_limit`: Credit limit in base units (must be > 0 and <= MAX_CREDIT_LIMIT)
    /// - `interest_rate_bps`: Interest rate in basis points (must be <= MAX_INTEREST_RATE_BPS, min is 0 as u32)
    /// - `risk_score`: Risk score for the borrower
    ///
    /// # Panics
    /// - If credit_limit is <= 0
    /// - If credit_limit exceeds MAX_CREDIT_LIMIT
    /// - If interest_rate_bps is outside the valid range
    /// - If risk_score is > 100
    ///
    /// # Events
    /// Emits a CreditLineOpened event on success.
    pub fn open_credit_line(
        env: Env,
        borrower: Address,
        credit_limit: i128,
        interest_rate_bps: u32,
        risk_score: u32,
    ) {
        require_admin_auth(&env);

        validate_credit_limit(credit_limit);
        validate_interest_rate(interest_rate_bps);

        assert!(risk_score <= 100, "risk_score must be between 0 and 100");

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

        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
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
        env.storage().persistent().set(&borrower, &credit_line);

        let token_address: Address = env
            .storage()
            .instance()
            .get(&token_key(&env))
            .expect("token not configured");

        let token_client = token::Client::new(&env, &token_address);
        token_client.transfer(&env.current_contract_address(), &borrower, &amount);

        clear_reentrancy_guard(&env);

        let timestamp = env.ledger().timestamp();
        publish_drawn_event(
            &env,
            DrawnEvent {
                borrower: borrower.clone(),
                amount,
                new_utilized_amount: credit_line.utilized_amount,
                timestamp,
            },
        );

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

        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
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

    /// Update risk parameters (admin/risk engine).
    ///
    /// # Parameters
    /// - `borrower`: Address of the borrower
    /// - `credit_limit`: New credit limit in base units (must be > 0 and <= MAX_CREDIT_LIMIT)
    /// - `interest_rate_bps`: New interest rate in basis points (must be <= MAX_INTEREST_RATE_BPS, min is 0 as u32)
    /// - `risk_score`: New risk score for the borrower
    ///
    /// # Panics
    /// - If credit line does not exist
    /// - If credit_limit is <= 0
    /// - If credit_limit exceeds MAX_CREDIT_LIMIT
    /// - If interest_rate_bps is outside the valid range
    ///
    /// # Events
    /// Emits a RiskParametersUpdated event on success.
    pub fn update_risk_parameters(
        env: Env,
        borrower: Address,
        credit_limit: i128,
        interest_rate_bps: u32,
        risk_score: u32,
    ) {
        require_admin_auth(&env);

        validate_credit_limit(credit_limit);
        validate_interest_rate(interest_rate_bps);

        // Get existing credit line
        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
            .expect("Credit line not found");

        // Update parameters
        credit_line.credit_limit = credit_limit;
        credit_line.interest_rate_bps = interest_rate_bps;
        credit_line.risk_score = risk_score;

        env.storage().persistent().set(&borrower, &credit_line);

        // Emit RiskParametersUpdated event
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

    /// Suspend a credit line (admin).
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

        // Emit CreditLineSuspended event
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

        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
            .expect("Credit line not found");

        // Admin can force-close; borrower can close only if utilized_amount == 0
        if closer != admin && closer != borrower {
            panic!("Unauthorized closer");
        }
        if closer == borrower && credit_line.utilized_amount != 0 {
            panic!("Borrower cannot close with nonzero utilization");
        }

        credit_line.status = CreditStatus::Closed;
        env.storage().persistent().set(&borrower, &credit_line);

        // Emit CreditLineClosed event
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

    /// Mark a credit line as defaulted (admin).
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

        // Emit CreditLineDefaulted event
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
        env.storage().persistent().get(&borrower)
    }
}

#[cfg(test)]
mod test;
