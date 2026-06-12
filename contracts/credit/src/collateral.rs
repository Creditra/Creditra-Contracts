// SPDX-License-Identifier: MIT

//! Collateral deposits and withdrawals.
//!
//! # Trust boundary
//!
//! Both [`deposit_collateral`] and [`withdraw_collateral`] require the
//! borrower's `require_auth` and validate the amount is strictly positive.
//! Withdrawals additionally enforce the configured
//! `MinCollateralRatioBps` floor against the borrower's outstanding
//! utilization, so a withdrawal can never push an active credit line
//! under-collateralized.
//!
//! # Storage
//!
//! Per-borrower collateral balances live in persistent storage under
//! [`crate::storage::DataKey::CollateralBalance`]; the minimum ratio lives
//! under [`crate::storage::DataKey::MinCollateralRatioBps`] in instance
//! storage. See [`docs/storage-layout.md`](../../../docs/storage-layout.md).

use crate::storage::{
    get_collateral_balance, set_collateral_balance, get_min_collateral_ratio_bps,
    get_credit_line, get_collateral_token,
};
use crate::events::{
    publish_collateral_deposited_event, publish_collateral_withdrawn_event,
    CollateralDepositedEvent, CollateralWithdrawnEvent,
};
use crate::types::ContractError;
use soroban_sdk::{Address, Env, token};

/// Deposit collateral tokens from the borrower into the contract.
/// Requires borrower authentication.
pub fn deposit_collateral(env: &Env, borrower: &Address, amount: i128) {
    // Basic validation
    if amount <= 0 {
        env.panic_with_error(ContractError::InvalidAmount);
    }
    borrower.require_auth();

    // Transfer token from borrower to contract address
    let token_addr = get_collateral_token(env).unwrap_or_else(|| {
        env.panic_with_error(ContractError::MissingLiquidityToken);
    });
    let token_client = token::Client::new(env, &token_addr);
    let contract_addr = env.current_contract_address();
    
    // In Soroban token standard, transfer takes (from, to, amount).
    // `borrower.require_auth()` ensures this is authorized by the borrower.
    token_client.transfer(borrower, &contract_addr, &amount);

    // Update stored collateral balance (add amount)
    let cur_balance = get_collateral_balance(env, borrower);
    let new_balance = cur_balance.checked_add(amount).unwrap_or_else(|| {
        env.panic_with_error(ContractError::Overflow);
    });
    set_collateral_balance(env, borrower, new_balance);

    // Publish event
    publish_collateral_deposited_event(env, CollateralDepositedEvent {
        borrower: borrower.clone(),
        amount,
        new_balance,
    });
}

/// Withdraw collateral tokens to the borrower.
/// Requires borrower authentication and ensures collateral ratio remains above minimum.
pub fn withdraw_collateral(env: &Env, borrower: &Address, amount: i128) {
    if amount <= 0 {
        env.panic_with_error(ContractError::InvalidAmount);
    }
    borrower.require_auth();

    // Get current collateral balance
    let cur_balance = get_collateral_balance(env, borrower);
    if amount > cur_balance {
        // We reuse `InsufficientRepaymentBalance` here to avoid expanding the
        // error enum for this niche case; the semantics ("the caller asked to
        // move more tokens than they have available") are close enough that
        // SDK consumers can interpret the code without ambiguity.
        env.panic_with_error(ContractError::InsufficientRepaymentBalance);
    }

    let post_balance = cur_balance - amount;

    // Check if the borrower has an active credit line to enforce ratio
    // If no credit line exists, they can withdraw everything.
    if let Some(credit_line) = get_credit_line(env, borrower) {
        if credit_line.utilized_amount > 0 {
            // Compute required collateral after withdrawal
            let min_ratio_bps = get_min_collateral_ratio_bps(env).unwrap_or(15000);
            let required = (credit_line.utilized_amount as i128)
                .checked_mul(min_ratio_bps as i128)
                .unwrap_or_else(|| env.panic_with_error(ContractError::Overflow))
                / 10_000;
            
            if post_balance < required {
                env.panic_with_error(ContractError::CollateralRatioBelowMinimum);
            }
        }
    }

    // Transfer token from contract to borrower
    let token_addr = get_collateral_token(env).unwrap_or_else(|| {
        env.panic_with_error(ContractError::MissingLiquidityToken);
    });
    let token_client = token::Client::new(env, &token_addr);
    let contract_addr = env.current_contract_address();
    token_client.transfer(&contract_addr, borrower, &amount);

    // Update stored collateral balance (subtract amount)
    set_collateral_balance(env, borrower, post_balance);

    // Publish event
    publish_collateral_withdrawn_event(env, CollateralWithdrawnEvent {
        borrower: borrower.clone(),
        amount,
        new_balance: post_balance,
    });
}

/// Read‑only getter for a borrower's collateral balance.
pub fn get_collateral(env: &Env, borrower: &Address) -> i128 {
    get_collateral_balance(env, borrower)
}
