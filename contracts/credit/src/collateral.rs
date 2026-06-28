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
//! [`partial_release_collateral`] is a dedicated borrower-callable entrypoint
//! that applies the same health-factor guard and emits a distinct
//! [`crate::events::CollateralPartialReleasedEvent`] (topic `"col_prel"`),
//! making it easy for indexers to distinguish a deliberate partial release
//! from a generic withdrawal or an atomic repay-and-release.
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
    publish_collateral_deposited_event, publish_collateral_partial_released_event,
    publish_collateral_withdrawn_event, CollateralDepositedEvent, CollateralPartialReleasedEvent,
    CollateralWithdrawnEvent,
};
use crate::storage::{
    get_collateral_balance, get_collateral_token, get_credit_line, get_min_collateral_ratio_bps,
    set_collateral_balance,
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

/// Allow a borrower to release a portion of their collateral while keeping
/// the credit line's health factor above the configured threshold.
///
/// # What
///
/// Transfers `amount` collateral tokens from the contract back to `borrower`,
/// provided the remaining collateral satisfies the minimum collateral ratio:
///
/// ```text
/// post_balance >= utilized_amount * min_ratio_bps / 10_000
/// ```
///
/// When `utilized_amount == 0` the ratio check is skipped and the borrower
/// can release any amount up to their full balance (subject to
/// [`ContractError::InsufficientCollateralBalance`]).
///
/// # Health factor
///
/// After a successful release the function computes:
///
/// ```text
/// health_factor_bps = post_balance * 10_000 / utilized_amount
/// ```
///
/// and embeds it in the emitted [`CollateralPartialReleasedEvent`].
/// When `utilized_amount == 0` the health factor is reported as `u32::MAX`.
///
/// # Authorization
///
/// Requires `borrower.require_auth()` — only the borrower themselves can
/// release their own collateral.
///
/// # Errors
///
/// | Error | Condition |
/// |---|---|
/// | [`ContractError::InvalidAmount`] | `amount` is zero or negative |
/// | [`ContractError::CreditLineNotFound`] — _not raised_; no line is fine | |
/// | [`ContractError::InsufficientCollateralBalance`] | `amount > current_balance` |
/// | [`ContractError::CollateralRatioBelowMinimum`] | post-release balance < required |
/// | [`ContractError::MissingLiquidityToken`] | token address not configured |
/// | [`ContractError::Overflow`] | arithmetic overflow in ratio calculation |
///
/// # Events
///
/// Emits [`CollateralPartialReleasedEvent`] on success with topic
/// `("credit", "col_prel")`.
pub fn partial_release_collateral(env: &Env, borrower: &Address, amount: i128) {
    // ── 1. Input validation ────────────────────────────────────────────────
    if amount <= 0 {
        env.panic_with_error(ContractError::InvalidAmount);
    }

    // ── 2. Authorization ───────────────────────────────────────────────────
    // Only the borrower may release their own collateral.
    borrower.require_auth();

    // ── 3. Balance check ───────────────────────────────────────────────────
    let cur_balance = get_collateral_balance(env, borrower);
    if amount > cur_balance {
        env.panic_with_error(ContractError::InsufficientCollateralBalance);
    }

    // post_balance is the collateral remaining if we proceed.
    let post_balance = cur_balance
        .checked_sub(amount)
        .unwrap_or_else(|| env.panic_with_error(ContractError::Overflow));

    // ── 4. Health-factor guard ─────────────────────────────────────────────
    // Fetch the credit line (if any) and enforce MinCollateralRatioBps.
    let utilized_amount = if let Some(credit_line) = get_credit_line(env, borrower) {
        credit_line.utilized_amount
    } else {
        0_i128
    };

    if utilized_amount > 0 {
        let min_ratio_bps = get_min_collateral_ratio_bps(env).unwrap_or(15_000);

        // required = ceil(utilized * min_ratio_bps / 10_000)
        // We use checked_mul to detect overflow; the division itself cannot overflow.
        let required = utilized_amount
            .checked_mul(min_ratio_bps as i128)
            .unwrap_or_else(|| env.panic_with_error(ContractError::Overflow))
            / 10_000_i128;

        if post_balance < required {
            env.panic_with_error(ContractError::CollateralRatioBelowMinimum);
        }
    }

    // ── 5. Compute reported health factor ──────────────────────────────────
    // health_factor_bps = post_balance * 10_000 / utilized_amount
    // u32::MAX signals "no outstanding debt" (unbounded).
    let health_factor_bps: u32 = if utilized_amount == 0 {
        u32::MAX
    } else {
        // post_balance * 10_000 fits in i128 for any realistic balance.
        let hf = post_balance
            .checked_mul(10_000_i128)
            .unwrap_or(i128::MAX)
            / utilized_amount;
        // Saturate to u32::MAX if the ratio somehow exceeds 4_294_967_295 bps.
        u32::try_from(hf).unwrap_or(u32::MAX)
    };

    // ── 6. Token transfer ──────────────────────────────────────────────────
    let token_addr = get_collateral_token(env).unwrap_or_else(|| {
        env.panic_with_error(ContractError::MissingLiquidityToken)
    });
    let token_client = token::Client::new(env, &token_addr);
    let contract_addr = env.current_contract_address();
    token_client.transfer(&contract_addr, borrower, &amount);

    // ── 7. Persist updated balance ─────────────────────────────────────────
    set_collateral_balance(env, borrower, post_balance);

    // ── 8. Emit event ──────────────────────────────────────────────────────
    publish_collateral_partial_released_event(
        env,
        CollateralPartialReleasedEvent {
            borrower: borrower.clone(),
            amount_released: amount,
            new_balance: post_balance,
            health_factor_bps,
        },
    );
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
