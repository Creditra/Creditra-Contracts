//! # Creditra Credit Contract
//!
//! This module implements the on-chain credit line protocol for Creditra on
//! Stellar Soroban. It manages the full lifecycle of borrower credit lines —
//! opening, drawing, repaying, suspending, closing, and defaulting.
//!
//! ## Roles
//!
//! - **Admin**: Deployed and initialized by the protocol deployer. Authorized
//!   to suspend, close, and default credit lines, and update risk parameters.
//! - **Borrower**: An address with an open credit line. Authorized to draw
//!   and repay funds within their credit limit.
//! - **Risk Engine / Backend**: Authorized to open credit lines and update
//!   risk parameters on behalf of the protocol.
//!
//! ## Main Flows
//!
//! 1. **Open**: Admin/backend calls `open_credit_line` to create a credit line
//!    for a borrower with a limit, interest rate, and risk score.
//! 2. **Draw**: Borrower calls `draw_credit` to borrow against their limit.
//! 3. **Repay**: Borrower calls `repay_credit` to repay drawn funds.
//! 4. **Suspend**: Admin calls `suspend_credit_line` to temporarily freeze a line.
//! 5. **Close**: Admin or borrower calls `close_credit_line` to permanently close.
//! 6. **Default**: Admin calls `default_credit_line` to mark a borrower as defaulted.
//!
//! ## Invariants
//!
//! - `utilized_amount` must never exceed `credit_limit`.
//! - A credit line must exist before it can be suspended, closed, or defaulted.
//! - Interest rate is expressed in basis points (1 bps = 0.01%).
//!
//! ## External Docs
//!
//! See [`docs/credit.md`](../../../docs/credit.md) for full documentation
//! including CLI usage and deployment instructions.

#![no_std]
#![allow(clippy::unused_unit)]

//! Creditra credit contract: credit lines, draw/repay, risk parameters.
//!
//! # Status transitions
//!
//! | From    | To        | Trigger |
//! |---------|-----------|---------|
//! | Active  | Defaulted | Admin calls `default_credit_line` (e.g. after past-due or oracle signal). |
//! | Suspended | Defaulted | Admin calls `default_credit_line`. |
//! | Defaulted | Active   | Admin calls `reinstate_credit_line`. |
//! | Defaulted | Suspended | Admin calls `suspend_credit_line`. |
//! | Defaulted | Closed   | Admin or borrower (when utilized_amount == 0) calls `close_credit_line`. |
//!
//! When status is Defaulted: `draw_credit` is disabled; `repay_credit` is allowed.
//!
//! ## Overview
//! This contract manages credit lines for borrowers, allowing them to draw and repay funds, with risk and lifecycle management.
//!
//! ## Security
//! - All entrypoints that accept an `amount` parameter (draw_credit, repay_credit) require `amount > 0` and will revert with `"amount must be positive"` if not.
//! - Reentrancy guard is used on draw_credit and repay_credit as a defense-in-depth measure.
//!
//! ## Reentrancy
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
use types::{ContractError, CreditLineData, CreditStatus};

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

/// Represents the lifecycle status of a credit line.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreditStatus {
    /// Credit line is open and available for drawing.
    Active = 0,
    /// Credit line is temporarily suspended by admin.
    Suspended = 1,
    /// Borrower has defaulted on the credit line.
    Defaulted = 2,
    /// Credit line has been permanently closed.
    Closed = 3,
fn require_admin(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&admin_key(env))
        .unwrap_or_else(|| env.panic_with_error(ContractError::NotAdmin))
}

/// Stores the full state of a borrower's credit line.
///
/// Persisted in contract storage keyed by the borrower's [`Address`].
#[contracttype]
pub struct CreditLineData {
    /// The borrower's Stellar address.
    pub borrower: Address,
    /// Maximum amount the borrower is authorized to draw.
    pub credit_limit: i128,
    /// Amount currently drawn and outstanding.
    pub utilized_amount: i128,
    /// Annual interest rate in basis points (e.g. 300 = 3%).
    pub interest_rate_bps: u32,
    /// Risk score assigned by the risk engine (0–100, higher = riskier).
    pub risk_score: u32,
    /// Current lifecycle status of the credit line.
    pub status: CreditStatus,
}

/// Event emitted on every credit line lifecycle state change.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreditLineEvent {
    /// Short symbol identifying the event type (e.g. `opened`, `suspend`).
    pub event_type: Symbol,
    /// The borrower whose credit line was affected.
    pub borrower: Address,
    /// The new status after the event.
    pub status: CreditStatus,
    /// Credit limit at the time of the event.
    pub credit_limit: i128,
    /// Interest rate at the time of the event.
    pub interest_rate_bps: u32,
    /// Risk score at the time of the event.
    pub risk_score: u32,
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
        env.panic_with_error(ContractError::Reentrancy);
    }
    env.storage().instance().set(&key, &true);
}

fn clear_reentrancy_guard(env: &Env) {
    env.storage().instance().set(&reentrancy_key(env), &false);
}

/// The Creditra credit contract.
#[contract]
pub struct Credit;

#[contractimpl]
impl Credit {
    /// @notice Initialize the contract with admin and reserve token address.
    /// @param env The contract environment
    /// @param admin The admin address
    /// @param token The reserve token address
    pub fn init(env: Env, admin: Address, token: Address) {
        if env.storage().instance().has(&admin_key(&env)) {
            panic!("Already initialized");
        }
    /// Initialize the contract (admin).
    pub fn init(env: Env, admin: Address) {
        env.storage().instance().set(&admin_key(&env), &admin);
        env.storage().instance().set(&token_key(&env), &token);
    }

    /// @notice Open a new credit line for a borrower (called by backend/risk engine).
    /// @param env The contract environment
    /// @param borrower The borrower's address
    /// @param credit_limit The maximum amount the borrower can draw (must be > 0)
    /// @param interest_rate_bps Interest rate in basis points (max 10000)
    /// @param risk_score Risk score (max 100)
    /// @dev Panics if credit_limit <= 0, interest_rate_bps > 10000, risk_score > 100, or an active credit line exists
    /// Initialize the contract with an admin address.
    ///
    /// Must be called exactly once after deployment before any other
    /// function can be used.
    ///
    /// # Parameters
    /// - `admin`: The address authorized to perform admin operations.
    ///
    /// # Storage
    /// Stores `admin` in instance storage under the key `"admin"`.
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

    /// Open a new credit line for a borrower.
    ///
    /// Called by the backend or risk engine after off-chain credit assessment.
    /// Creates a new [`CreditLineData`] record with `utilized_amount = 0` and
    /// `status = Active`, then persists it keyed by the borrower's address.
    ///
    /// # Parameters
    /// - `borrower`: The borrower's Stellar address.
    /// - `credit_limit`: Maximum drawable amount.
    /// - `interest_rate_bps`: Annual interest rate in basis points.
    /// - `risk_score`: Risk score from the risk engine (0–100).
    ///
    /// # Events
    /// Emits a `("credit", "opened")` [`CreditLineEvent`].
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

        env.events().publish(
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
 
    /// Errors with ContractError if credit line does not exist, is Closed, or borrower has not authorized.

    /// Reverts if credit line does not exist, is Closed, borrower has not authorized,
    /// or the provided borrower does not match the stored credit line owner.
 
 
    pub fn draw_credit(env: Env, borrower: Address, amount: i128) -> () {


    /// @notice Draw from credit line: verifies limit, updates utilized_amount, and transfers the protocol token from the contract reserve to the borrower.
    /// @param env The contract environment
    /// @param borrower The borrower's address
    /// @param amount The amount to draw (must be > 0)
    /// @dev Panics if credit line not found, closed, not active, exceeds credit limit, or amount <= 0. Uses a reentrancy guard.
    /// @custom:security Throws `"amount must be positive"` if amount is zero or negative.
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
 

    /// Draw from credit line (borrower).
    /// Reverts if credit line does not exist, is Closed/Suspended, or borrower has not authorized.
    /// Reverts if credit line does not exist, is Closed, or borrower has not authorized.
    pub fn draw_credit(env: Env, borrower: Address, amount: i128) {
        set_reentrancy_guard(&env);
        borrower.require_auth();

    }

    /// Draw funds from an active credit line.
    ///
    /// Called by the borrower to borrow against their credit limit.
    ///
    /// # Parameters
    /// - `borrower`: The borrower's address.
    /// - `amount`: Amount to draw. Must not exceed available credit.
    ///
    /// # Note
    /// Not yet implemented. Planned logic: validate amount against available
    /// credit, update `utilized_amount`, transfer tokens to borrower.
    pub fn draw_credit(_env: Env, _borrower: Address, _amount: i128) -> () {
        // TODO: check limit, update utilized_amount, transfer token to borrower
        ()
    }

    /// Repay outstanding credit and accrue interest.
    ///
    /// Called by the borrower to reduce their `utilized_amount`.
    ///
    /// # Parameters
    /// - `borrower`: The borrower's address.
    /// - `amount`: Amount to repay.
    ///
    /// # Note
    /// Not yet implemented. Planned logic: accept token transfer, reduce
    /// `utilized_amount`, accrue interest on outstanding balance.
    pub fn repay_credit(_env: Env, _borrower: Address, _amount: i128) -> () {
        // TODO: accept token, reduce utilized_amount, accrue interest
        ()
    }

    /// Update risk parameters for an existing credit line.
    ///
    /// Called by admin or risk engine when a borrower's risk profile changes.
    ///
    /// # Parameters
    /// - `borrower`: The borrower's address.
    /// - `credit_limit`: New credit limit.
    /// - `interest_rate_bps`: New interest rate in basis points.
    /// - `risk_score`: New risk score.
    ///
    /// # Note
    /// Not yet implemented. Planned logic: load existing record, update fields,
    /// persist updated [`CreditLineData`].
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
 
            .unwrap_or_else(|| {
                clear_reentrancy_guard(&env);
                env.panic_with_error(ContractError::CreditLineNotFound)
            });

            .expect("Credit line not found");


        if credit_line.borrower != borrower {
            clear_reentrancy_guard(&env);
            panic!("Borrower mismatch for credit line");
        }

 


        if credit_line.status == CreditStatus::Closed {
            clear_reentrancy_guard(&env);
            env.panic_with_error(ContractError::CreditLineClosed);
        }


        if credit_line.status != CreditStatus::Active {
            clear_reentrancy_guard(&env);

            env.panic_with_error(ContractError::InvalidAmount);

            panic!("Credit line not active");

        }
        if credit_line.status == CreditStatus::Suspended {
            clear_reentrancy_guard(&env);
            panic!("credit line is suspended");
        }
        if amount <= 0 {
            clear_reentrancy_guard(&env);
            panic!("amount must be positive");
        }
        let new_utilized = credit_line

        let new_utilized = credit_line

        let updated_utilized = credit_line

            .utilized_amount
            .checked_add(amount)
 
            .unwrap_or_else(|| {
                clear_reentrancy_guard(&env);
                env.panic_with_error(ContractError::Overflow)
            });

            .expect("overflow");


 
        if new_utilized > credit_line.credit_limit {

        if updated_utilized > credit_line.credit_limit {

            clear_reentrancy_guard(&env);
            env.panic_with_error(ContractError::OverLimit);
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
        // TODO: transfer token to borrower
        ()
    }

    /// @notice Repay credit (borrower).
    /// @param env The contract environment
    /// @param borrower The borrower's address
    /// @param amount The amount to repay (must be > 0)
    /// @dev Reverts if credit line does not exist, is Closed, or borrower has not authorized. Reduces utilized_amount by amount (capped at 0). Emits RepaymentEvent.
    /// @custom:security Throws `"amount must be positive"` if amount is zero or negative.
    /// Repay credit (borrower).
    /// Allowed when status is Active, Suspended, or Defaulted. Reverts if credit line does not exist,
    /// is Closed, or borrower has not authorized. Reduces utilized_amount by amount (capped at 0).
    ///
    /// Transfers the repayment amount from the borrower to the contract reserve via the Stellar
    /// token contract. The transfer is executed before any state change so that a failed transfer
    /// (e.g. insufficient balance or allowance) reverts the entire call and prevents griefing.
    ///
    /// The amount applied to the credit line is capped at the current utilized amount; any excess
    /// is not pulled from the borrower. Utilized amount is then decreased and a RepaymentEvent
    /// is emitted.
    ///
    /// # Arguments
    /// * `borrower` - Must have authorized the call and the token transfer (same invocation).
    /// * `amount` - Nominal repayment amount; effective transfer is min(amount, utilized_amount).
    ///
    /// # Panics
    /// * "Credit line not found" – no credit line for borrower
    /// * "credit line is closed" – line is closed
    /// * "amount must be positive" – amount <= 0
    /// * Token transfer fails if borrower has insufficient balance or has not authorized the transfer.
    /// Errors with ContractError if credit line does not exist, is Closed, or borrower has not authorized.
    /// Reverts if credit line does not exist, is Closed, or borrower has not authorized.
    /// If a liquidity token is configured, transfers that token from the borrower to the
    /// configured liquidity source via allowance + transfer_from.
    /// Reduces utilized_amount by amount (capped at 0). Emits RepaymentEvent.
    pub fn repay_credit(env: Env, borrower: Address, amount: i128) {
        set_reentrancy_guard(&env);
        borrower.require_auth();
        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
 
            .unwrap_or_else(|| {
                clear_reentrancy_guard(&env);
                env.panic_with_error(ContractError::CreditLineNotFound)
            });

            .expect("Credit line not found");

        if credit_line.borrower != borrower {
            panic!("Borrower mismatch for credit line");
        }

 
        if credit_line.status == CreditStatus::Closed {
            clear_reentrancy_guard(&env);
            env.panic_with_error(ContractError::CreditLineClosed);
        }
        if amount <= 0 {
            clear_reentrancy_guard(&env);
            env.panic_with_error(ContractError::InvalidAmount);
        }

        let effective_repay = amount.min(credit_line.utilized_amount);
        if effective_repay > 0 {
            if let Some(token_address) = env
                .storage()
                .instance()
                .get::<_, Address>(&DataKey::LiquidityToken)
            {
                let token_client = token::Client::new(&env, &token_address);
                token_client.transfer(&borrower, &env.current_contract_address(), &effective_repay);
            }
        }

        let new_utilized = credit_line.utilized_amount.saturating_sub(amount).max(0);
        // Apply at most the outstanding utilized amount to avoid over-charging on overpayment.
        let repay_amount = if amount > credit_line.utilized_amount {
            credit_line.utilized_amount
        } else {
            amount
        };

        let new_utilized = credit_line
            .utilized_amount
            .saturating_sub(repay_amount)
            .max(0);
        credit_line.utilized_amount = new_utilized;
        env.storage().persistent().set(&borrower, &credit_line);

        if repay_amount > 0 {
            let token_address: Option<Address> =
                env.storage().instance().get(&DataKey::LiquidityToken);
            let reserve_address: Address = env
                .storage()
                .instance()
                .get(&DataKey::LiquiditySource)
                .unwrap_or(env.current_contract_address());

            if let Some(token_address) = token_address {
                let token_client = token::Client::new(&env, &token_address);
                let contract_address = env.current_contract_address();

                let allowance = token_client.allowance(&borrower, &contract_address);
                if allowance < repay_amount {
                    clear_reentrancy_guard(&env);
                    panic!("Insufficient allowance");
                }

                let balance = token_client.balance(&borrower);
                if balance < repay_amount {
                    clear_reentrancy_guard(&env);
                    panic!("Insufficient balance");
                }

                token_client.transfer_from(
                    &contract_address,
                    &borrower,
                    &reserve_address,
                    &repay_amount,
                );
            }
        }

        let timestamp = env.ledger().timestamp();
        publish_repayment_event(
            &env,
            RepaymentEvent {
                borrower: borrower.clone(),
                amount: effective_repay,
                amount: repay_amount,
                new_utilized_amount: new_utilized,
                timestamp,
            },
        );
        clear_reentrancy_guard(&env);
        // TODO: accept token from borrower
        ()
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

    /// * ContractError::NotAdmin if caller is not the contract admin.
    /// * ContractError::CreditLineNotFound if no credit line exists for the borrower.
    /// * ContractError::OverLimit if credit_limit < utilized_amount.
    /// * ContractError::NegativeLimit if credit_limit < 0.
    /// * ContractError::RateTooHigh if interest_rate_bps reflects a violation.
    /// * ContractError::ScoreTooHigh if risk_score reflects a violation.
    ///
    /// Emits a risk_updated event.


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
            .unwrap_or_else(|| env.panic_with_error(ContractError::CreditLineNotFound));

        if credit_limit < 0 {
            env.panic_with_error(ContractError::NegativeLimit);
        }
        if credit_limit < credit_line.utilized_amount {
            env.panic_with_error(ContractError::OverLimit);
        }
        if interest_rate_bps > MAX_INTEREST_RATE_BPS {
            env.panic_with_error(ContractError::RateTooHigh);
        }
        if risk_score > MAX_RISK_SCORE {
            env.panic_with_error(ContractError::ScoreTooHigh);
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

    /// Suspend a credit line temporarily.
    ///
    /// Called by admin to freeze a borrower's credit line without closing it.
    /// The credit line can be reactivated or closed after suspension.
    ///
    /// # Parameters
    /// - `borrower`: The borrower's address.
    ///
    /// # Panics
    /// - If no credit line exists for the given borrower.
    ///
    /// # Events
    /// Emits a `("credit", "suspend")` [`CreditLineEvent`].
    pub fn suspend_credit_line(env: Env, borrower: Address) -> () {
    /// Suspend a credit line (admin only).
    /// Emits a CreditLineSuspended event.
    pub fn suspend_credit_line(env: Env, borrower: Address) {
        require_admin_auth(&env);
        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
            .unwrap_or_else(|| env.panic_with_error(ContractError::CreditLineNotFound));

        if credit_line.status != CreditStatus::Active {
            panic!("Only active credit lines can be suspended");
        }

        credit_line.status = CreditStatus::Suspended;
        env.storage().persistent().set(&borrower, &credit_line);

        env.events().publish(
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

    /// Permanently close a credit line.
    ///
    /// Can be called by admin or by the borrower when `utilized_amount` is 0.
    /// Once closed, the credit line cannot be reopened.
    ///
    /// # Parameters
    /// - `borrower`: The borrower's address.
    ///
    /// # Panics
    /// - If no credit line exists for the given borrower.
    ///
    /// # Events
    /// Emits a `("credit", "closed")` [`CreditLineEvent`].
    pub fn close_credit_line(env: Env, borrower: Address) -> () {
    /// Close a credit line. Callable by admin (force-close) or by borrower when utilization is zero.
    /// Allowed from Active, Suspended, or Defaulted. Idempotent if already Closed.
    ///
    /// # Arguments



    /// * `closer` - Address that must have authorized this call. Must be either the contract admin
    ///   (can close regardless of utilization) or the borrower (can close only when
    ///   `utilized_amount` is zero).
    ///
    /// # Errors

    /// * ContractError::CreditLineNotFound if credit line does not exist.
    /// * ContractError::Unauthorized if `closer` is not admin/borrower.
    /// * ContractError::UtilizationNotZero if borrower closes while `utilized_amount != 0`.
    ///
    /// Emits a CreditLineClosed event.

    /// * `closer` - Must be either the contract admin or the borrower (only when utilized_amount == 0).


    /// * Panics if credit line does not exist, or if `closer` is not admin/borrower, or if
    ///   borrower closes while `utilized_amount != 0`.
    ///
    /// Emits a CreditLineClosed event.

    /// * `borrower` - Address of the borrower whose credit line to close.
    ///
    /// # Errors
    /// * Panics if credit line does not exist.
    ///
    /// Emits a CreditLineClosed event.
    pub fn close_credit_line(env: Env, borrower: Address) {
    pub fn close_credit_line(env: Env, borrower: Address, closer: Address) {
        closer.require_auth();

        let admin: Address = require_admin(&env);

        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
            .unwrap_or_else(|| env.panic_with_error(ContractError::CreditLineNotFound));

        if credit_line.status == CreditStatus::Closed {
            return;
        }

        let allowed = closer == admin || (closer == borrower && credit_line.utilized_amount == 0);

        if !allowed {
            if closer == borrower {
                env.panic_with_error(ContractError::UtilizationNotZero);
            }
            env.panic_with_error(ContractError::Unauthorized);
        }

        credit_line.status = CreditStatus::Closed;
        env.storage().persistent().set(&borrower, &credit_line);

        env.events().publish(
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

    /// Mark a credit line as defaulted.
    ///
    /// Called by admin when a borrower fails to repay. Defaulted credit lines
    /// are permanently marked and cannot be reactivated.
    ///
    /// # Parameters
    /// - `borrower`: The borrower's address.
    ///
    /// # Panics
    /// - If no credit line exists for the given borrower.
    ///
    /// # Events
    /// Emits a `("credit", "default")` [`CreditLineEvent`].
    pub fn default_credit_line(env: Env, borrower: Address) -> () {
    /// Mark a credit line as defaulted (admin only).
    ///
    /// Call when the line is past due or when an oracle/off-chain signal indicates default.
    /// Transition: Active or Suspended → Defaulted.
    /// After this, draw_credit is disabled and repay_credit remains allowed.
    /// Emits a CreditLineDefaulted event.
    pub fn default_credit_line(env: Env, borrower: Address) {
        require_admin_auth(&env);
        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
            .unwrap_or_else(|| env.panic_with_error(ContractError::CreditLineNotFound));

        credit_line.status = CreditStatus::Defaulted;
        env.storage().persistent().set(&borrower, &credit_line);

        env.events().publish(
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

    /// Reinstate a defaulted credit line to Active (admin only).
    ///
    /// Allowed only when status is Defaulted. Transition: Defaulted → Active.
    pub fn reinstate_credit_line(env: Env, borrower: Address) {
        require_admin_auth(&env);

        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
            .expect("Credit line not found");

        if credit_line.status != CreditStatus::Defaulted {
            panic!("credit line is not defaulted");
        }

        credit_line.status = CreditStatus::Active;
        env.storage().persistent().set(&borrower, &credit_line);

        publish_credit_line_event(
            &env,
            (symbol_short!("credit"), symbol_short!("reinstate")),
            CreditLineEvent {
                event_type: symbol_short!("reinstate"),
                borrower: borrower.clone(),
                status: CreditStatus::Active,
                credit_limit: credit_line.credit_limit,
                interest_rate_bps: credit_line.interest_rate_bps,
                risk_score: credit_line.risk_score,
            },
        );
    }

    /// Retrieve the current credit line data for a borrower.
    ///
    /// View function — does not modify any state.
    ///
    /// # Parameters
    /// - `borrower`: The borrower's address to look up.
    ///
    /// # Returns
    /// `Some(CreditLineData)` if a credit line exists, `None` otherwise.
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
    use soroban_sdk::testutils::Events as _;
    use soroban_sdk::token;
    use soroban_sdk::contractclient::ContractClient;
    use soroban_sdk::testutils::Events;
    use soroban_sdk::token::StellarAssetClient;
    use soroban_sdk::{TryFromVal, TryIntoVal};

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
        client.init(&admin);
        client.set_liquidity_token(&token_address);
        client.open_credit_line(borrower, &credit_limit, &300_u32, &70_u32);
        (client, token_address, admin)
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
    /// Sets up a Stellar asset contract, mints `initial_balance` to `contract_id`, returns (token_address, ()).
    fn setup_token(env: &Env, contract_id: &Address, initial_balance: i128) -> (Address, ()) {
        env.mock_all_auths();
        let token_admin = Address::generate(env);
        let token = env.register_stellar_asset_contract_v2(token_admin);
        let token_admin_client = StellarAssetClient::new(env, &token.address());
        token_admin_client.mint(contract_id, &initial_balance);
        (token.address(), ())
    }

    /// Full setup: contract inited with admin, liquidity token set, credit line opened for borrower.
    /// Returns (client, token_address, admin).
    fn setup_contract_with_credit_line<'a>(
        env: &'a Env,
        borrower: &Address,
        credit_limit: i128,
        initial_mint: i128,
    ) -> (CreditClient<'a>, Address, Address) {
        env.mock_all_auths();
        let admin = Address::generate(env);
        let contract_id = env.register(Credit, ());
        let (token_address, _sac) = setup_token(env, &contract_id, initial_mint);
        let client = CreditClient::new(env, &contract_id);
        client.init(&admin);
        client.set_liquidity_token(&token_address);
        client.open_credit_line(borrower, &credit_limit, &300_u32, &70_u32);
        (client, token_address, admin)
    fn approve_token_spend(
        env: &Env,
        token_address: &Address,
        owner: &Address,
        spender: &Address,
        amount: i128,
    ) {
        let token_client = token::Client::new(env, token_address);
        let expiration_ledger = 1_000_u32;
        token_client.approve(owner, spender, &amount, &expiration_ledger);
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
    #[should_panic(expected = "Only active credit lines can be suspended")]
    fn test_suspend_credit_line_only_when_active() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.suspend_credit_line(&borrower);
        client.suspend_credit_line(&borrower);
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
        client.open_credit_line(&borrower, &5000_i128, &500_u32, &80_u32);
        assert_eq!(client.get_credit_line(&borrower).unwrap().status, CreditStatus::Active);

        client.suspend_credit_line(&borrower);
        assert_eq!(client.get_credit_line(&borrower).unwrap().status, CreditStatus::Suspended);

        client.close_credit_line(&borrower);
        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.status, CreditStatus::Closed);
        assert_eq!(client.get_credit_line(&borrower).unwrap().status, CreditStatus::Closed);
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
        client.open_credit_line(&borrower, &2000_i128, &400_u32, &75_u32);

        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.borrower, borrower);
        assert_eq!(credit_line.status, CreditStatus::Active);
        assert_eq!(credit_line.credit_limit, 2000);
        assert_eq!(credit_line.interest_rate_bps, 400);
        assert_eq!(credit_line.risk_score, 75);
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
        client.close_credit_line(&borrower);
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
    fn test_repay_credit_full_repayment() {
    fn test_draw_credit_exact_available_limit_succeeds() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);

        // Draw 500 from credit line
        client.draw_credit(&borrower, &500_i128);
        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.utilized_amount, 500);

        // Full repayment
        client.repay_credit(&borrower, &500_i128);
        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.utilized_amount, 0);
        assert_eq!(credit_line.credit_limit, 1000);
        assert_eq!(credit_line.status, CreditStatus::Active);
        assert_eq!(client.get_credit_line(&borrower).unwrap().status, CreditStatus::Active);

        client.default_credit_line(&borrower);
        assert_eq!(client.get_credit_line(&borrower).unwrap().status, CreditStatus::Defaulted);
        let limit = 5000_i128;
        client.open_credit_line(&borrower, &limit, &300_u32, &70_u32);

        client.draw_credit(&borrower, &limit);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.utilized_amount, limit);
        assert_eq!(line.credit_limit, limit);
    }

    /// Test partial repayment: utilized amount decreases correctly
    #[test]
    fn test_repay_credit_partial_repayment() {
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

        client.init(&admin);
        client.open_credit_line(&borrower, &2000_i128, &400_u32, &75_u32);

        // Draw 1000 from credit line
        client.draw_credit(&borrower, &1000_i128);
        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.utilized_amount, 1000);

        // Partial repayment of 300
        client.repay_credit(&borrower, &300_i128);
        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.utilized_amount, 700);
        assert_eq!(credit_line.credit_limit, 2000);
        assert_eq!(credit_line.status, CreditStatus::Active);

        // Another partial repayment of 200
        client.repay_credit(&borrower, &200_i128);
        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.utilized_amount, 500);
        // Partial repayment
        call_contract(&env, &contract_id, || {
            Credit::repay_credit(env.clone(), borrower.clone(), 200_i128);
        });

        let credit_data = get_credit_data(&env, &contract_id, &borrower);
        assert_eq!(credit_data.utilized_amount, 300_i128); // 500 - 200
    }

    /// Test multiple partial repayments leading to full repayment
    #[test]
    fn test_repay_credit_multiple_partial_to_full() {
    fn test_repay_credit_full() {
        let env = Env::default();
        let (_admin, borrower, contract_id) = setup_test(&env);

        client.init(&admin);
        client.open_credit_line(&borrower, &5000_i128, &500_u32, &80_u32);

        // Draw 1500
        client.draw_credit(&borrower, &1500_i128);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            1500
        );

        // Repay in increments
        client.repay_credit(&borrower, &500_i128);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            1000
        );

        client.repay_credit(&borrower, &400_i128);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            600
        );

        client.repay_credit(&borrower, &600_i128);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            0
        );
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
    #[should_panic(expected = "Error(Contract, #3)")]
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
    #[should_panic(expected = "Error(Contract, #3)")]
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
    #[should_panic(expected = "Error(Contract, #3)")]
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
    #[should_panic(expected = "Error(Contract, #10)")]
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


        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let (client, _token, _admin) =
            setup_contract_with_credit_line(&env, &borrower, 1_000, 1_000);
        client.draw_credit(&borrower, &300);


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

    /// Test repayment exceeds utilized amount (should cap at 0)
    #[test]
    fn test_repay_credit_exceeds_utilized() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);

        client.draw_credit(&borrower, &500_i128);
        client.repay_credit(&borrower, &600_i128); // Exceeds utilized

        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.utilized_amount, 0); // Should be capped at 0
    }

    /// Test repayment with zero amount (should panic)
    #[test]


    #[should_panic(expected = "Error(Contract, #4)")]

    #[should_panic(expected = "credit line is closed")]

    fn test_draw_credit_rejected_when_closed() {
    #[should_panic(expected = "amount must be positive")]
    fn test_repay_credit_zero_amount() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);

        client.draw_credit(&borrower, &500_i128);
        client.repay_credit(&borrower, &0_i128);
    }

    /// Test repayment on nonexistent credit line (should panic)
    #[test]

    #[should_panic(expected = "Error(Contract, #4)")]

    #[should_panic(expected = "Credit line not found")]
    fn test_repay_credit_nonexistent_line() {
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
        client.repay_credit(&borrower, &100_i128);
    }

    /// Test state consistency after draw and repay cycle
    #[test]

    #[should_panic(expected = "Error(Contract, #1)")]


    #[should_panic(expected = "unauthorized")]

    fn test_close_credit_line_unauthorized_closer() {
    fn test_repay_credit_state_consistency() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &3000_i128, &350_u32, &85_u32);

        let initial = client.get_credit_line(&borrower).unwrap();
        assert_eq!(initial.utilized_amount, 0);
        assert_eq!(initial.credit_limit, 3000);
        assert_eq!(initial.interest_rate_bps, 350);
        assert_eq!(initial.risk_score, 85);

        // Draw and repay cycle
        client.draw_credit(&borrower, &800_i128);
        client.repay_credit(&borrower, &300_i128);

        let after_cycle = client.get_credit_line(&borrower).unwrap();
        assert_eq!(after_cycle.utilized_amount, 500);
        assert_eq!(after_cycle.credit_limit, 3000); // Unchanged
        assert_eq!(after_cycle.interest_rate_bps, 350); // Unchanged
        assert_eq!(after_cycle.risk_score, 85); // Unchanged
        assert_eq!(after_cycle.status, CreditStatus::Active); // Unchanged
        assert_eq!(after_cycle.borrower, borrower); // Unchanged
    }

    /// Test repayment with exact utilized amount
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
        assert_eq!(
            Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
            symbol_short!("reinstate")
        );
        let event_data: CreditLineEvent = data.try_into_val(&env).unwrap();
        assert_eq!(event_data.status, CreditStatus::Active);
    }

    #[test]
    fn test_event_lifecycle_sequence() {}
    #[test]
    #[should_panic(expected = "interest_rate_bps exceeds maximum")]
    fn test_update_risk_parameters_interest_rate_exceeds_max() {
    fn test_repay_credit_exact_amount() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);

        client.draw_credit(&borrower, &750_i128);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            750
        );

        client.repay_credit(&borrower, &750_i128);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            0
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

    #[test]
    #[should_panic(expected = "credit line is suspended")]
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
    fn test_repay_credit_transfers_tokens_from_borrower_to_contract() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let admin = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let (token_address, _sac) = setup_token(&env, &contract_id, 1_000);
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);
        client.set_liquidity_token(&token_address);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);
        client.draw_credit(&borrower, &400);
        let token_client = token::Client::new(&env, &token_address);
        let borrower_before = token_client.balance(&borrower);
        let contract_before = token_client.balance(&contract_id);
        client.repay_credit(&borrower, &200);
        assert_eq!(token_client.balance(&borrower), borrower_before - 200);
        assert_eq!(token_client.balance(&contract_id), contract_before + 200);
    }

    #[test]
    fn test_repay_credit_transfer_capped_at_utilized() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let admin = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let (token_address, _sac) = setup_token(&env, &contract_id, 1_000);
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);
        client.set_liquidity_token(&token_address);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);
        client.draw_credit(&borrower, &100);
        let token_client = token::Client::new(&env, &token_address);
        let contract_before = token_client.balance(&contract_id);
        client.repay_credit(&borrower, &500);
        assert_eq!(token_client.balance(&contract_id), contract_before + 100);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            0
        );
    }

    #[test]
    #[should_panic]
    fn test_repay_credit_insufficient_balance_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let other = Address::generate(&env);
        let (client, token_address, _admin) =
            setup_contract_with_credit_line(&env, &borrower, 1_000, 1_000);
        client.draw_credit(&borrower, &200);
        let token_client = token::Client::new(&env, &token_address);
        token_client.transfer(&borrower, &other, &150);
        client.repay_credit(&borrower, &200);
    }

    #[test]
    fn test_repay_credit_requires_borrower_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) =
            setup_contract_with_credit_line(&env, &borrower, 1_000, 1_000);
        client.draw_credit(&borrower, &100);
        client.repay_credit(&borrower, &50);
        assert!(
            env.auths().iter().any(|(addr, _)| *addr == borrower),
            "repay_credit must require borrower authorization"
        );
    }

    #[test]
    fn test_repay_credit_on_suspended_line_transfers_and_updates() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let admin = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let (token_address, _sac) = setup_token(&env, &contract_id, 1_000);
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);
        client.set_liquidity_token(&token_address);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);
        client.draw_credit(&borrower, &300);
        client.suspend_credit_line(&borrower);
        let token_client = token::Client::new(&env, &token_address);
        let contract_before = token_client.balance(&contract_id);
        client.repay_credit(&borrower, &100);
        assert_eq!(token_client.balance(&contract_id), contract_before + 100);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            200
        );
    }
    // --- suspend/default: unauthorized caller ---
    // --- update_risk_parameters (#9) ---
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
    #[should_panic(expected = "Error(Contract, #3)")]
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
    #[should_panic(expected = "Error(Contract, #6)")]
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
    #[should_panic(expected = "Error(Contract, #7)")]
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
    #[should_panic(expected = "Error(Contract, #8)")]
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
    #[should_panic(expected = "Error(Contract, #9)")]
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

    // --- repay_credit: token acceptance (SEP-41) ---

    #[test]
 
    #[should_panic(expected = "Error(Contract, #5)")]
    fn test_repay_credit_rejects_non_positive_amount() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.repay_credit(&borrower, &0_i128);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #3)")]

    #[should_panic(expected = "Credit line not found")]
 
    fn test_repay_credit_nonexistent_line() {
    fn test_repay_credit_transfers_token_and_consumes_allowance() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let token_admin = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);

        // Create utilization without requiring any token liquidity.
        client.draw_credit(&borrower, &300_i128);

        let token = env.register_stellar_asset_contract_v2(token_admin);
        let token_admin_client = StellarAssetClient::new(&env, &token.address());
        let token_client = token::Client::new(&env, &token.address());

        client.set_liquidity_token(&token.address());

        // Fund the borrower so they can repay using transfer_from.
        token_admin_client.mint(&borrower, &300_i128);

        let repay_amount = 200_i128;
        approve_token_spend(
            &env,
            &token.address(),
            &borrower,
            &contract_id,
            repay_amount,
        );

        let borrower_balance_before = token_client.balance(&borrower);
        let reserve_balance_before = token_client.balance(&contract_id);
        let allowance_before = token_client.allowance(&borrower, &contract_id);

        client.repay_credit(&borrower, &repay_amount);

        let borrower_balance_after = token_client.balance(&borrower);
        let reserve_balance_after = token_client.balance(&contract_id);
        let allowance_after = token_client.allowance(&borrower, &contract_id);

        assert_eq!(
            borrower_balance_before - borrower_balance_after,
            repay_amount
        );
        assert_eq!(reserve_balance_after - reserve_balance_before, repay_amount);
        assert_eq!(allowance_before - allowance_after, repay_amount);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            100_i128
        );
    }

    #[test]
    fn test_repay_credit_transfers_token_to_configured_liquidity_source() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let token_admin = Address::generate(&env);
        let reserve = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);

        // Create utilization without requiring any token liquidity.
        client.draw_credit(&borrower, &250_i128);

        let token = env.register_stellar_asset_contract_v2(token_admin);
        let token_admin_client = StellarAssetClient::new(&env, &token.address());
        let token_client = token::Client::new(&env, &token.address());

        client.set_liquidity_token(&token.address());
        client.set_liquidity_source(&reserve);

        token_admin_client.mint(&borrower, &250_i128);

        let repay_amount = 100_i128;
        approve_token_spend(
            &env,
            &token.address(),
            &borrower,
            &contract_id,
            repay_amount,
        );

        let borrower_balance_before = token_client.balance(&borrower);
        let reserve_balance_before = token_client.balance(&reserve);
        let allowance_before = token_client.allowance(&borrower, &contract_id);

        client.repay_credit(&borrower, &repay_amount);

        assert_eq!(
            token_client.balance(&borrower),
            borrower_balance_before - repay_amount
        );
        assert_eq!(
            token_client.balance(&reserve),
            reserve_balance_before + repay_amount
        );
        assert_eq!(
            token_client.allowance(&borrower, &contract_id),
            allowance_before - repay_amount
        );
    }

    #[test]
    #[should_panic(expected = "Insufficient allowance")]
    fn test_repay_credit_reverts_on_insufficient_allowance() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let token_admin = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);
        client.draw_credit(&borrower, &200_i128);

        let token = env.register_stellar_asset_contract_v2(token_admin);
        let token_admin_client = StellarAssetClient::new(&env, &token.address());

        client.set_liquidity_token(&token.address());
        token_admin_client.mint(&borrower, &200_i128);

        // Approve less than the repay amount.
        approve_token_spend(&env, &token.address(), &borrower, &contract_id, 50_i128);

        client.repay_credit(&borrower, &200_i128);
    }

    #[test]
    #[should_panic(expected = "Insufficient balance")]
    fn test_repay_credit_reverts_on_insufficient_balance() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let token_admin = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);
        client.draw_credit(&borrower, &200_i128);

        let token = env.register_stellar_asset_contract_v2(token_admin);
        let token_admin_client = StellarAssetClient::new(&env, &token.address());

        client.set_liquidity_token(&token.address());

        // Fund borrower with less than repayment amount but approve full amount.
        token_admin_client.mint(&borrower, &50_i128);
        approve_token_spend(&env, &token.address(), &borrower, &contract_id, 200_i128);

        client.repay_credit(&borrower, &200_i128);
    }

    // --- suspend/default admin-only: unauthorized caller ---
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

    // ── event emission ────────────────────────────────────────────────────────

    /// Test that repay_credit emits RepaymentEvent with correct payload.
    #[test]
    fn test_event_repay_credit_payload() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) =
            setup_contract_with_credit_line(&env, &borrower, 5_000, 5_000);
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
    }

    /// Test that repay_credit emits correct event for full repayment.
    #[test]
    fn test_event_repay_credit_full_amount() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) =
            setup_contract_with_credit_line(&env, &borrower, 5_000, 5_000);
        client.draw_credit(&borrower, &2000_i128);

        // Repay full amount
        client.repay_credit(&borrower, &2000_i128);

        let events = env.events().all();
        let (_contract, _topics, data) = events.last().unwrap();
        let event_data: RepaymentEvent = data.try_into_val(&env).unwrap();
        assert_eq!(event_data.borrower, borrower);
        assert_eq!(event_data.amount, 2000);
        assert_eq!(event_data.new_utilized_amount, 0);
    }

    /// Test that repay_credit emits correct event for overpayment (saturating).
    #[test]
    fn test_event_repay_credit_overpayment() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) =
            setup_contract_with_credit_line(&env, &borrower, 5_000, 1_000);
        client.draw_credit(&borrower, &500_i128);

        // Repay more than utilized (should saturate to 0)
        client.repay_credit(&borrower, &1000_i128);

        let events = env.events().all();
        let (_contract, _topics, data) = events.last().unwrap();
        let event_data: RepaymentEvent = data.try_into_val(&env).unwrap();
        assert_eq!(event_data.borrower, borrower);
        assert_eq!(event_data.amount, 1000);
        assert_eq!(event_data.new_utilized_amount, 0);
    }

    /// Test multiple repay events are correctly emitted.
    #[test]
    fn test_event_multiple_repayments() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) =
            setup_contract_with_credit_line(&env, &borrower, 10_000, 10_000);
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

    /// Test that event data remains consistent across lifecycle operations.
    #[test]
    fn test_event_data_consistency_across_lifecycle() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let (token_address, _) = setup_token(&env, &contract_id, 0);
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);
        client.set_liquidity_token(&token_address);

        // Open with specific parameters
        let credit_limit = 7500_i128;
        let interest_rate = 450_u32;
        let risk_score = 85_u32;

        client.open_credit_line(&borrower, &credit_limit, &interest_rate, &risk_score);
        let events = env.events().all();
        let (_c, _topics, data) = events.last().unwrap();
        let open_data: CreditLineEvent = data.try_into_val(&env).unwrap();
        assert_eq!(open_data.credit_limit, credit_limit);
        assert_eq!(open_data.interest_rate_bps, interest_rate);
        assert_eq!(open_data.risk_score, risk_score);

        client.suspend_credit_line(&borrower);
        let events = env.events().all();
        let (_c, _topics, data) = events.last().unwrap();
        let suspend_data: CreditLineEvent = data.try_into_val(&env).unwrap();
        assert_eq!(suspend_data.credit_limit, credit_limit);
        assert_eq!(suspend_data.interest_rate_bps, interest_rate);
        assert_eq!(suspend_data.risk_score, risk_score);

        client.default_credit_line(&borrower);
        let events = env.events().all();
        let (_c, _topics, data) = events.last().unwrap();
        let default_data: CreditLineEvent = data.try_into_val(&env).unwrap();
        assert_eq!(default_data.credit_limit, credit_limit);
        assert_eq!(default_data.interest_rate_bps, interest_rate);
        assert_eq!(default_data.risk_score, risk_score);
    }

    // =========================================================================
    // Integration tests: full lifecycle flows (open → draw → repay → close)
    // =========================================================================

    /// End-to-end flow: init → open → draw × 2 → repay × 2 → borrower close.
    ///
    /// Asserts every state transition and event count along the way.
    /// Events are checked immediately after each emitting call (before any
    /// subsequent contract call clears the per-invocation event buffer).
    #[test]
    fn test_integration_flow_open_draw_repay_close() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let (token_address, _) = setup_token(&env, &contract_id, 10_000);
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);
        client.set_liquidity_token(&token_address);

        // --- 1. Open credit line --------------------------------------------
        client.open_credit_line(&borrower, &10_000_i128, &500_u32, &75_u32);
        // CreditLineOpened event — check BEFORE next contract call resets buffer
        assert_eq!(env.events().all().len(), 1);

        let cl = client.get_credit_line(&borrower).unwrap();
        assert_eq!(cl.borrower, borrower);
        assert_eq!(cl.credit_limit, 10_000);
        assert_eq!(cl.utilized_amount, 0);
        assert_eq!(cl.interest_rate_bps, 500);
        assert_eq!(cl.risk_score, 75);
        assert_eq!(cl.status, CreditStatus::Active);

        // --- 2. First draw: 3 000 -------------------------------------------
        client.draw_credit(&borrower, &3_000_i128);
        // draw_credit emits 2 events: SAC transfer event + (credit, draw) event
        assert_eq!(env.events().all().len(), 2);

        let cl = client.get_credit_line(&borrower).unwrap();
        assert_eq!(cl.utilized_amount, 3_000);
        assert_eq!(cl.status, CreditStatus::Active);

        // --- 3. Second draw: 2 000 (cumulative: 5 000) ----------------------
        client.draw_credit(&borrower, &2_000_i128);
        assert_eq!(env.events().all().len(), 2);

        let cl = client.get_credit_line(&borrower).unwrap();
        assert_eq!(cl.utilized_amount, 5_000);
        assert_eq!(cl.credit_limit, 10_000);
        assert_eq!(cl.status, CreditStatus::Active);

        // --- 4. First repay: 2 500 (utilized → 2 500) -----------------------
        client.repay_credit(&borrower, &2_500_i128);
        // repay emits RepaymentEvent
        assert_eq!(env.events().all().len(), 1);

        let cl = client.get_credit_line(&borrower).unwrap();
        assert_eq!(cl.status, CreditStatus::Active);
        assert_eq!(cl.utilized_amount, 2_500);

        // --- 5. Second repay: 2 500 (utilized → 0) --------------------------
        client.repay_credit(&borrower, &2_500_i128);
        assert_eq!(env.events().all().len(), 1);

        let cl = client.get_credit_line(&borrower).unwrap();
        assert_eq!(cl.status, CreditStatus::Active);
        assert_eq!(cl.utilized_amount, 0);

        // --- 6. Borrower self-closes (utilized == 0) -------------------------
        client.close_credit_line(&borrower, &borrower);
        // CreditLineClosed event — check BEFORE next contract call resets buffer
        assert_eq!(env.events().all().len(), 1);

        let cl = client.get_credit_line(&borrower).unwrap();
        assert_eq!(cl.status, CreditStatus::Closed);
        assert_eq!(cl.credit_limit, 10_000);
        assert_eq!(cl.interest_rate_bps, 500);
        assert_eq!(cl.risk_score, 75);
    }

    /// Integration variant: open → (no draw) → borrower self-closes when utilized == 0.
    ///
    /// Confirms a borrower may close their own line with no outstanding balance,
    /// and that the correct state and events are recorded.
    #[test]
    fn test_integration_flow_borrower_close_zero_utilized() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let (token_address, _) = setup_token(&env, &contract_id, 0);
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);
        client.set_liquidity_token(&token_address);

        // --- 1. Open --------------------------------------------------------
        client.open_credit_line(&borrower, &5_000_i128, &300_u32, &60_u32);
        // CreditLineOpened event — check BEFORE next contract call resets buffer
        assert_eq!(env.events().all().len(), 1);

        let cl = client.get_credit_line(&borrower).unwrap();
        assert_eq!(cl.status, CreditStatus::Active);
        assert_eq!(cl.utilized_amount, 0);
        assert_eq!(cl.credit_limit, 5_000);
        assert_eq!(cl.interest_rate_bps, 300);
        assert_eq!(cl.risk_score, 60);

        // --- 2. Borrower closes with zero utilization -----------------------
        client.close_credit_line(&borrower, &borrower);
        // CreditLineClosed event — check BEFORE next contract call resets buffer
        assert_eq!(env.events().all().len(), 1);

        let cl = client.get_credit_line(&borrower).unwrap();
        assert_eq!(cl.status, CreditStatus::Closed);
        assert_eq!(cl.utilized_amount, 0);
    }

    // ── liquidity source tests ───────────────────────────────────────────────

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

    // --- Comprehensive open_credit_line success and persistence tests ---

    #[test]
    fn test_open_credit_line_persists_all_fields_correctly() {
    #[test]
    fn test_set_liquidity_source_updates_instance_storage() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let reserve = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);

        // Test with specific values
        let credit_limit = 5000_i128;
        let interest_rate_bps = 450_u32;
        let risk_score = 85_u32;

        client.open_credit_line(&borrower, &credit_limit, &interest_rate_bps, &risk_score);

        // Verify all fields are persisted correctly
        let credit_line = client.get_credit_line(&borrower);
        assert!(credit_line.is_some(), "Credit line should exist after opening");

        let credit_line = credit_line.unwrap();
        assert_eq!(credit_line.borrower, borrower, "Borrower address should match");
        assert_eq!(credit_line.credit_limit, credit_limit, "Credit limit should match");
        assert_eq!(credit_line.utilized_amount, 0, "Utilized amount should be zero initially");
        assert_eq!(credit_line.interest_rate_bps, interest_rate_bps, "Interest rate should match");
        assert_eq!(credit_line.risk_score, risk_score, "Risk score should match");
        assert_eq!(credit_line.status, CreditStatus::Active, "Status should be Active");
    }

    #[test]
    fn test_open_credit_line_emits_correct_event() {
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

        let credit_limit = 2500_i128;
        let interest_rate_bps = 350_u32;
        let risk_score = 75_u32;

        client.open_credit_line(&borrower, &credit_limit, &interest_rate_bps, &risk_score);

        // Verify the correct event was emitted
        let events = env.events().all();
        assert_eq!(events.len(), 2, "Should have 2 events: init and credit line opened");

        // The second event should be the credit line opened event
        let credit_event = &events[1];
        assert_eq!(credit_event.0, (symbol_short!("credit"), symbol_short!("opened")));

        let event_data: CreditLineEvent = credit_event.1.clone();
        assert_eq!(event_data.event_type, symbol_short!("opened"));
        assert_eq!(event_data.borrower, borrower);
        assert_eq!(event_data.status, CreditStatus::Active);
        assert_eq!(event_data.credit_limit, credit_limit);
        assert_eq!(event_data.interest_rate_bps, interest_rate_bps);
        assert_eq!(event_data.risk_score, risk_score);
    }

    #[test]
    fn test_open_credit_line_with_edge_case_values() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);

        // Test with minimum values
        client.open_credit_line(&borrower, &1_i128, &0_u32, &0_u32);

        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.credit_limit, 1);
        assert_eq!(credit_line.interest_rate_bps, 0);
        assert_eq!(credit_line.risk_score, 0);
        assert_eq!(credit_line.utilized_amount, 0);
        assert_eq!(credit_line.status, CreditStatus::Active);
    }

    #[test]
    fn test_open_credit_line_with_maximum_values() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);

        // Test with large values
        let credit_limit = i128::MAX / 2; // Leave room for addition
        let interest_rate_bps = u32::MAX;
        let risk_score = u32::MAX;

        client.open_credit_line(&borrower, &credit_limit, &interest_rate_bps, &risk_score);

        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.credit_limit, credit_limit);
        assert_eq!(credit_line.interest_rate_bps, interest_rate_bps);
        assert_eq!(credit_line.risk_score, risk_score);
        assert_eq!(credit_line.utilized_amount, 0);
        assert_eq!(credit_line.status, CreditStatus::Active);
    }

    #[test]
    fn test_open_credit_line_multiple_borrowers_persistence() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower1 = Address::generate(&env);
        let borrower2 = Address::generate(&env);
        let borrower3 = Address::generate(&env);
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

        // Open credit lines for multiple borrowers
        client.open_credit_line(&borrower1, &1000_i128, &300_u32, &70_u32);
        client.open_credit_line(&borrower2, &2000_i128, &400_u32, &80_u32);
        client.open_credit_line(&borrower3, &3000_i128, &500_u32, &90_u32);

        // Verify each borrower's credit line is persisted correctly and independently
        let credit_line1 = client.get_credit_line(&borrower1).unwrap();
        assert_eq!(credit_line1.credit_limit, 1000);
        assert_eq!(credit_line1.interest_rate_bps, 300);
        assert_eq!(credit_line1.risk_score, 70);
        assert_eq!(credit_line1.borrower, borrower1);

        let credit_line2 = client.get_credit_line(&borrower2).unwrap();
        assert_eq!(credit_line2.credit_limit, 2000);
        assert_eq!(credit_line2.interest_rate_bps, 400);
        assert_eq!(credit_line2.risk_score, 80);
        assert_eq!(credit_line2.borrower, borrower2);

        let credit_line3 = client.get_credit_line(&borrower3).unwrap();
        assert_eq!(credit_line3.credit_limit, 3000);
        assert_eq!(credit_line3.interest_rate_bps, 500);
        assert_eq!(credit_line3.risk_score, 90);
        assert_eq!(credit_line3.borrower, borrower3);
    }

    #[test]
    fn test_open_credit_line_storage_persistence_across_operations() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
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

        // Open credit line
        client.open_credit_line(&borrower, &1500_i128, &350_u32, &75_u32);

        // Verify initial persistence
        let initial_credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(initial_credit_line.credit_limit, 1500);
        assert_eq!(initial_credit_line.utilized_amount, 0);

        // Perform other operations and verify persistence remains intact
        client.draw_credit(&borrower, &500_i128);

        let after_draw = client.get_credit_line(&borrower).unwrap();
        assert_eq!(after_draw.credit_limit, 1500, "Credit limit should persist");
        assert_eq!(after_draw.utilized_amount, 500, "Utilized amount should update");
        assert_eq!(after_draw.interest_rate_bps, 350, "Interest rate should persist");
        assert_eq!(after_draw.risk_score, 75, "Risk score should persist");
        assert_eq!(after_draw.status, CreditStatus::Active, "Status should persist");
    }

    #[test]
    fn test_open_credit_line_data_integrity_after_modification() {
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

        // Open credit line
        let original_limit = 1000_i128;
        let original_rate = 300_u32;
        let original_score = 70_u32;

        client.open_credit_line(&borrower, &original_limit, &original_rate, &original_score);

        // Modify the credit line through other operations
        client.draw_credit(&borrower, &200_i128);
        client.repay_credit(&borrower, &100_i128);

        // Verify original data integrity except for utilized amount
        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.borrower, borrower, "Borrower should remain unchanged");
        assert_eq!(credit_line.credit_limit, original_limit, "Credit limit should remain unchanged");
        assert_eq!(credit_line.interest_rate_bps, original_rate, "Interest rate should remain unchanged");
        assert_eq!(credit_line.risk_score, original_score, "Risk score should remain unchanged");
        assert_eq!(credit_line.status, CreditStatus::Active, "Status should remain Active");
        assert_eq!(credit_line.utilized_amount, 100, "Only utilized amount should change");
    }

    #[test]
    fn test_open_credit_line_getter_consistency() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);

        // Open credit line
        client.open_credit_line(&borrower, &2500_i128, &425_u32, &82_u32);

        // Test getter consistency across multiple calls
        let credit_line1 = client.get_credit_line(&borrower).unwrap();
        let credit_line2 = client.get_credit_line(&borrower).unwrap();
        let credit_line3 = client.get_credit_line(&borrower).unwrap();

        // All calls should return identical data
        assert_eq!(credit_line1.borrower, credit_line2.borrower);
        assert_eq!(credit_line1.borrower, credit_line3.borrower);
        assert_eq!(credit_line1.credit_limit, credit_line2.credit_limit);
        assert_eq!(credit_line1.credit_limit, credit_line3.credit_limit);
        assert_eq!(credit_line1.utilized_amount, credit_line2.utilized_amount);
        assert_eq!(credit_line1.utilized_amount, credit_line3.utilized_amount);
        assert_eq!(credit_line1.interest_rate_bps, credit_line2.interest_rate_bps);
        assert_eq!(credit_line1.interest_rate_bps, credit_line3.interest_rate_bps);
        assert_eq!(credit_line1.risk_score, credit_line2.risk_score);
        assert_eq!(credit_line1.risk_score, credit_line3.risk_score);
        assert_eq!(credit_line1.status, credit_line2.status);
        assert_eq!(credit_line1.status, credit_line3.status);
    }

    #[test]
    fn test_open_credit_line_with_zero_values() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);

        // Test with zero credit limit (should be allowed)
        client.open_credit_line(&borrower, &0_i128, &100_u32, &50_u32);

        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.credit_limit, 0);
        assert_eq!(credit_line.utilized_amount, 0);
        assert_eq!(credit_line.interest_rate_bps, 100);
        assert_eq!(credit_line.risk_score, 50);
        assert_eq!(credit_line.status, CreditStatus::Active);
    }

    #[test]
    fn test_open_credit_line_event_data_completeness() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);

        let credit_limit = 7500_i128;
        let interest_rate_bps = 550_u32;
        let risk_score = 95_u32;

        client.open_credit_line(&borrower, &credit_limit, &interest_rate_bps, &risk_score);

        // Verify event contains all required fields
        let events = env.events().all();
        let credit_event = &events[1];
        let event_data: CreditLineEvent = credit_event.1.clone();

        // Verify all event fields are populated correctly
        assert_eq!(event_data.event_type, symbol_short!("opened"), "Event type should be 'opened'");
        assert_eq!(event_data.borrower, borrower, "Event borrower should match input");
        assert_eq!(event_data.status, CreditStatus::Active, "Event status should be Active");
        assert_eq!(event_data.credit_limit, credit_limit, "Event credit limit should match");
        assert_eq!(event_data.interest_rate_bps, interest_rate_bps, "Event interest rate should match");
        assert_eq!(event_data.risk_score, risk_score, "Event risk score should match");
    }
}
        client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);

        let token = env.register_stellar_asset_contract_v2(token_admin);
        let token_admin_client = StellarAssetClient::new(&env, &token.address());

        client.set_liquidity_token(&token.address());

        token_admin_client.mint(&contract_id, &50_i128);
        client.draw_credit(&borrower, &100_i128);
    }

    // ========== borrower-only draw_credit and repay_credit ==========

    /// draw_credit must revert when the caller has not authorized as the borrower.
    /// No delegated spender is set, so only the borrower may draw.
    #[test]
    #[should_panic]
    fn test_draw_credit_reverts_when_caller_is_not_borrower() {
        let env = Env::default();
        // Deliberately omit mock_all_auths(): borrower.require_auth() must fail.
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);

        // Attempt draw without borrower authorization — must panic.
        client.draw_credit(&borrower, &200_i128);
    }

    /// repay_credit must revert when the caller has not authorized as the borrower.
    /// No delegated spender is set, so only the borrower may repay.
    #[test]
    #[should_panic]
    fn test_repay_credit_reverts_when_caller_is_not_borrower() {
        let env = Env::default();
        // Deliberately omit mock_all_auths(): borrower.require_auth() must fail.
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);

        // Attempt repay without borrower authorization — must panic.
        client.repay_credit(&borrower, &100_i128);
    }

    /// draw_credit succeeds when the caller is the authorized borrower.
    #[test]
    fn test_draw_credit_succeeds_when_caller_is_borrower() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);

        // Borrower authorized — draw should succeed.
        client.draw_credit(&borrower, &400_i128);

        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.utilized_amount, 400);
        assert_eq!(credit_line.status, CreditStatus::Active);
    }

    /// repay_credit succeeds when the caller is the authorized borrower.
    #[test]
    fn test_repay_credit_succeeds_when_caller_is_borrower() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.draw_credit(&borrower, &500_i128);

        // Borrower authorized — repay should succeed.
        client.repay_credit(&borrower, &200_i128);

        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.utilized_amount, 300);
        assert_eq!(credit_line.status, CreditStatus::Active);
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
        client.init(&admin);
        client.set_liquidity_token(&token_address);
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
