// SPDX-License-Identifier: MIT
#![no_std]
#![allow(clippy::unused_unit)]

//! Creditra credit contract: credit lines, draw/repay, risk parameters.
//!
//! # Reentrancy
//! Soroban token transfers (e.g. Stellar Asset Contract) do not invoke callbacks back into
//! the caller. This contract uses a reentrancy guard on draw_credit and repay_credit as a
//! defense-in-depth measure; if a token or future integration ever called back, the guard
//! would revert.

mod auth;
mod config;
mod events;
mod lifecycle;
mod query;
mod risk;
mod storage;
pub mod types;

use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, token, Address, Env, Symbol,
};

use events::{
    publish_credit_line_event, publish_drawn_event, publish_repayment_event,
    publish_risk_parameters_updated, CreditLineEvent, DrawnEvent, RepaymentEvent,
    RiskParametersUpdatedEvent,
};
use types::{ContractError, CreditLineData, CreditStatus, RateChangeConfig};

use crate::storage::{clear_reentrancy_guard, set_reentrancy_guard, DataKey};

/// Maximum interest rate in basis points (100%).
const MAX_INTEREST_RATE_BPS: u32 = 10_000;

/// Maximum risk score (0–100 scale).
const MAX_RISK_SCORE: u32 = 100;

/// Instance storage key for reentrancy guard.
fn reentrancy_key(env: &Env) -> Symbol {
    Symbol::new(env, "reentrancy")
}

/// Instance storage key for admin.
pub(crate) fn admin_key(env: &Env) -> Symbol {
    Symbol::new(env, "admin")
}

/// Instance storage key for rate-change config.
fn rate_cfg_key(env: &Env) -> Symbol {
    Symbol::new(env, "rate_cfg")
}

#[contract]
pub struct Credit;

#[contractimpl]
impl Credit {
    /// Initialize the contract with an admin address.
    ///
    /// # Behavior
    /// - Stores `admin` in instance storage under the `"admin"` key exactly once.
    /// - Sets `LiquiditySource` to the contract's own address as a deterministic default.
    /// - Reverts with [`ContractError::AlreadyInitialized`] if called a second time,
    ///   preventing admin takeover via re-initialization.
    ///
    /// # Parameters
    /// - `admin`: The address that will hold admin authority over this contract.
    ///
    /// # Errors
    /// - [`ContractError::AlreadyInitialized`] — contract has already been initialized.
    ///
    /// # Security
    /// - Must be called by the deployer immediately after deployment.
    /// - The admin address is immutable after initialization; see the Admin Rotation
    ///   Proposal in `docs/credit.md` for a safe rotation design.
    pub fn init(env: Env, admin: Address) {
        if env.storage().instance().has(&admin_key(&env)) {
            env.panic_with_error(ContractError::AlreadyInitialized);
        }
        env.storage().instance().set(&admin_key(&env), &admin);
        env.storage()
            .instance()
            .set(&DataKey::LiquiditySource, &env.current_contract_address());
    }

    /// Sets the token contract used for reserve/liquidity checks and draw transfers.
    pub fn set_liquidity_token(env: Env, token_address: Address) {
        require_admin_auth(&env);
        env.storage()
            .instance()
            .set(&DataKey::LiquidityToken, &token_address);
    }

    /// Sets the address that provides liquidity for draw operations.
    pub fn set_liquidity_source(env: Env, reserve_address: Address) {
        require_admin_auth(&env);
        env.storage()
            .instance()
            .set(&DataKey::LiquiditySource, &reserve_address);
    }

    /// Open a new credit line for a borrower (admin only).
    pub fn open_credit_line(
        env: Env,
        borrower: Address,
        credit_limit: i128,
        interest_rate_bps: u32,
        risk_score: u32,
    ) {
        assert!(credit_limit > 0, "credit_limit must be greater than zero");
        if interest_rate_bps > MAX_INTEREST_RATE_BPS {
            env.panic_with_error(ContractError::RateTooHigh);
        }
        if risk_score > MAX_RISK_SCORE {
            env.panic_with_error(ContractError::ScoreTooHigh);
        }

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
            accrued_interest: 0,
            last_accrual_ts: 0,
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

    /// Draw credit by transferring liquidity tokens to the borrower.
    ///
    /// Enforces status/limit/liquidity checks and uses a reentrancy guard.
    pub fn draw_credit(env: Env, borrower: Address, amount: i128) {
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

        if credit_line.borrower != borrower {
            clear_reentrancy_guard(&env);
            panic!("Borrower mismatch for credit line");
        }

        if credit_line.status == CreditStatus::Closed {
            clear_reentrancy_guard(&env);
            env.panic_with_error(ContractError::CreditLineClosed);
        }

        if credit_line.status == CreditStatus::Suspended {
            clear_reentrancy_guard(&env);
            panic!("credit line is suspended");
        }

        if credit_line.status == CreditStatus::Defaulted {
            clear_reentrancy_guard(&env);
            panic!("credit line is defaulted");
        }

        if credit_line.status != CreditStatus::Active {
            clear_reentrancy_guard(&env);
            env.panic_with_error(ContractError::InvalidAmount);
        }

        let updated_utilized = credit_line
            .utilized_amount
            .checked_add(amount)
            .unwrap_or_else(|| {
                clear_reentrancy_guard(&env);
                env.panic_with_error(ContractError::Overflow)
            });

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
    }

    /// Repay credit (borrower).
    ///
    /// Reverts if credit line does not exist, is Closed, or borrower has not authorized.
    /// Reduces utilized_amount by amount (capped at 0). Emits RepaymentEvent.
    ///
    /// # Reentrancy Protection
    /// This function uses a reentrancy guard to prevent re-entrant calls during
    /// token transfers. If a token contract were to call back into this contract
    /// during transfer, the guard would revert the transaction.
    ///
    /// # Security Notes
    /// - Soroban token transfers (e.g. Stellar Asset Contract) do not invoke callbacks
    /// - This guard is defense-in-depth for future token integrations
    /// - Guard is cleared on all success and failure paths
    pub fn repay_credit(env: Env, borrower: Address, amount: i128) {
        set_reentrancy_guard(&env);
        borrower.require_auth();

        if amount <= 0 {
            clear_reentrancy_guard(&env);
            env.panic_with_error(ContractError::InvalidAmount);
        }

        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
            .unwrap_or_else(|| {
                clear_reentrancy_guard(&env);
                env.panic_with_error(ContractError::CreditLineNotFound)
            });

        if credit_line.status == CreditStatus::Closed {
            clear_reentrancy_guard(&env);
            env.panic_with_error(ContractError::CreditLineClosed);
        }

        let effective_repay = if amount > credit_line.utilized_amount {
            credit_line.utilized_amount
        } else {
            amount
        };

        if effective_repay > 0 {
            let token_address: Option<Address> =
                env.storage().instance().get(&DataKey::LiquidityToken);

            if let Some(token_address) = token_address {
                let reserve_address: Address = env
                    .storage()
                    .instance()
                    .get(&DataKey::LiquiditySource)
                    .unwrap_or_else(|| env.current_contract_address());

                let token_client = token::Client::new(&env, &token_address);
                let contract_address = env.current_contract_address();

                let allowance = token_client.allowance(&borrower, &contract_address);
                if allowance < effective_repay {
                    clear_reentrancy_guard(&env);
                    panic!("Insufficient allowance");
                }

                let balance = token_client.balance(&borrower);
                if balance < effective_repay {
                    clear_reentrancy_guard(&env);
                    panic!("Insufficient balance");
                }

                token_client.transfer_from(
                    &contract_address,
                    &borrower,
                    &reserve_address,
                    &effective_repay,
                );
            }
        }

        let new_utilized = credit_line
            .utilized_amount
            .saturating_sub(effective_repay)
            .max(0);
        credit_line.utilized_amount = new_utilized;
        env.storage().persistent().set(&borrower, &credit_line);

        let timestamp = env.ledger().timestamp();
        publish_repayment_event(
            &env,
            RepaymentEvent {
                borrower,
                amount: effective_repay,
                new_utilized_amount: new_utilized,
                timestamp,
            },
        );

        clear_reentrancy_guard(&env);
    }

    /// Update risk parameters for an existing credit line (admin only).
    ///
    /// # Arguments
    /// * `borrower` - Borrower whose credit line to update.
    /// * `credit_limit` - New credit limit (must be >= current utilized_amount and >= 0).
    /// * `interest_rate_bps` - New interest rate in basis points (0 ..= 10000).
    /// * `risk_score` - New risk score (0 ..= 100).
    ///
    /// # Rate-change limits
    /// When a `RateChangeConfig` has been set via `set_rate_change_limits`, the
    /// following additional checks are enforced whenever the interest rate is
    /// actually changing:
    /// * The absolute delta `|new_rate - old_rate|` must be ≤ `max_rate_change_bps`.
    /// * If a minimum interval is configured and a previous rate change
    ///   timestamp exists, the elapsed time since the last change must be ≥
    ///   `rate_change_min_interval`.
    pub fn update_risk_parameters(
        env: Env,
        borrower: Address,
        credit_limit: i128,
        interest_rate_bps: u32,
        risk_score: u32,
    ) {
        risk::update_risk_parameters(env, borrower, credit_limit, interest_rate_bps, risk_score)
    }

    /// Set rate-change limits (admin only).
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

    /// Suspend an active credit line (admin only).
    ///
    /// # State transition
    /// `Active → Suspended`
    pub fn suspend_credit_line(env: Env, borrower: Address) {
        lifecycle::suspend_credit_line(env, borrower)
    }

    /// Close a credit line (admin force-close or borrower self-close with zero utilization).
    pub fn close_credit_line(env: Env, borrower: Address, closer: Address) {
        lifecycle::close_credit_line(env, borrower, closer)
    }

    /// Mark a credit line as defaulted (admin only).
    ///
    /// # State transition
    /// `Active | Suspended → Defaulted`
    pub fn default_credit_line(env: Env, borrower: Address) {
        lifecycle::default_credit_line(env, borrower)
    }

    /// Reinstate a `Defaulted` credit line to `Active` or `Suspended` (admin only).
    ///
    /// # State transition
    /// `Defaulted → Active`   (when `target_status == CreditStatus::Active`)
    /// `Defaulted → Suspended` (when `target_status == CreditStatus::Suspended`)
    ///
    /// This is the only valid exit from `Defaulted` other than a force-close.
    ///
    /// # Post-reinstatement invariants
    /// - `utilized_amount` is preserved; it continues to satisfy
    ///   `0 ≤ utilized_amount ≤ credit_limit`.
    /// - `interest_rate_bps`, `risk_score`, and `credit_limit` are unchanged.
    /// - If reinstated to `Active`, draws are immediately permitted.
    /// - If reinstated to `Suspended`, draws remain blocked.
    /// - A `"reinstate"` event is emitted.
    ///
    /// # Parameters
    /// - `borrower` — The borrower whose line is being reinstated.
    /// - `target_status` — Must be [`CreditStatus::Active`] or [`CreditStatus::Suspended`].
    ///
    /// # Errors
    /// - [`ContractError::NotAdmin`] — caller is not the contract admin.
    /// - Panics `"credit line is not defaulted"` if not in `Defaulted` state.
    /// - Panics `"invalid target status: must be Active or Suspended"` for invalid targets.
    /// - Panics `"Credit line not found"` if borrower has no credit line.
    pub fn reinstate_credit_line(env: Env, borrower: Address, target_status: CreditStatus) {
        lifecycle::reinstate_credit_line(env, borrower, target_status)
    }

    /// Get credit line data for a borrower (view function).
    pub fn get_credit_line(env: Env, borrower: Address) -> Option<CreditLineData> {
        env.storage().persistent().get(&borrower)
    }
}

fn require_admin_auth(env: &Env) {
    let admin: Address = env
        .storage()
        .instance()
        .get(&admin_key(env))
        .unwrap_or_else(|| env.panic_with_error(ContractError::NotAdmin));
    admin.require_auth();
}

fn rate_cfg_key(env: &Env) -> Symbol {
    Symbol::new(env, "rate_cfg")
}