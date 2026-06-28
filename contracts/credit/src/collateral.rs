// SPDX-License-Identifier: MIT

//! Collateral deposits and withdrawals.
//!
//! # What (the optional collateral floor)
//!
//! Creditra's key differentiator from Aave / Compound is that collateral
//! is an **optional, dial-able floor** rather than the eligibility
//! predicate. The on-chain function:
//!
//! - At deployment, `MinCollateralRatioBps` defaults to 15 000 bps (150 %),
//!   matching Aave's typical floor — i.e. the contract ships in a
//!   conservative collateralized mode.
//! - The admin can dial `MinCollateralRatioBps` down to 0, removing the
//!   ratio check entirely and making the credit line purely
//!   behavior-priced. Or up further, into Maker-style over-collateral
//!   territory.
//!
//! This module enforces the floor on `withdraw_collateral` and on
//! `draw_credit` (step 13 of the draw chain). [`deposit_collateral`] has
//! no ratio check — depositing more collateral is always safe.
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
//!
//! # Error reuse note
//!
//! Over-withdraw reverts with [`ContractError::InsufficientCollateralBalance`]
//! (`= 39`). See [`docs/contract-errors.md`](../../../docs/contract-errors.md)
//! for the full error table.

use crate::events::{
    publish_collateral_deposited_event, publish_collateral_withdrawn_event,
    CollateralDepositedEvent, CollateralWithdrawnEvent,
};
use crate::storage::{
    get_collateral_balance, get_collateral_risk_weight_bps, get_collateral_token,
    get_credit_line, get_min_collateral_ratio_bps, set_collateral_balance,
};
use crate::types::ContractError;
use soroban_sdk::{token, Address, Env};

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
    publish_collateral_deposited_event(
        env,
        CollateralDepositedEvent {
            borrower: borrower.clone(),
            amount,
            new_balance,
        },
    );
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
        env.panic_with_error(ContractError::InsufficientCollateralBalance);
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
    publish_collateral_withdrawn_event(
        env,
        CollateralWithdrawnEvent {
            borrower: borrower.clone(),
            amount,
            new_balance: post_balance,
        },
    );
}

/// Read‑only getter for a borrower's collateral balance.
pub fn get_collateral(env: &Env, borrower: &Address) -> i128 {
    get_collateral_balance(env, borrower)
}

/// Read-only getter for a collateral asset's risk weight, in basis points.
/// Defaults to 10_000 (100%, unweighted) if never explicitly configured.
pub fn get_collateral_risk_weight(env: &Env, asset: &Address) -> u32 {
    get_collateral_risk_weight_bps(env, asset).unwrap_or(10_000)
}

/// Release collateral tokens to the borrower without requiring auth.
///
/// Called internally by atomic repay+release flows. The caller is
/// responsible for computing the correct release amount and ensuring
/// the collateral ratio remains valid.
///
/// Panics with [`ContractError::InsufficientCollateralBalance`] if
/// `amount` exceeds the borrower's stored collateral balance.
pub fn release_collateral(env: &Env, borrower: &Address, amount: i128) {
    if amount < 0 {
        env.panic_with_error(ContractError::InvalidAmount);
    }
    if amount == 0 {
        return;
    }

    let cur_balance = get_collateral_balance(env, borrower);
    if amount > cur_balance {
        env.panic_with_error(ContractError::InsufficientCollateralBalance);
    }

    let post_balance = cur_balance - amount;

    let token_addr = get_collateral_token(env).unwrap_or_else(|| {
        env.panic_with_error(ContractError::MissingLiquidityToken);
    });
    let token_client = token::Client::new(env, &token_addr);
    let contract_addr = env.current_contract_address();
    token_client.transfer(&contract_addr, borrower, &amount);

    set_collateral_balance(env, borrower, post_balance);

    publish_collateral_withdrawn_event(
        env,
        CollateralWithdrawnEvent {
            borrower: borrower.clone(),
            amount,
            new_balance: post_balance,
        },
    );
}
