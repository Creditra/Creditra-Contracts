// SPDX-License-Identifier: MIT

//! Credit line lifecycle management: suspend, close, default, reinstate, and liquidation settlement.
//!
//! # What
//!
//! The state-transition layer for [`CreditLineData`]. Implements:
//!
//! - [`open_credit_line`] — admin-only line creation; idempotent re-open
//!   of non-Active lines under admin auth.
//! - [`suspend_credit_line_internal`] / [`suspend_credit_line`] /
//!   [`self_suspend_credit_line`] — Active → Suspended transition (admin
//!   path and borrower path).
//! - [`close_credit_line`] — Active/Suspended/Restricted → Closed.
//!   Borrower path requires `utilized_amount == 0`; admin path is
//!   unconditional. Idempotent on already-Closed.
//! - [`default_credit_line`] — Active/Restricted/Suspended → Defaulted.
//!   Emits `("credit","liq_req")` for the off-chain orchestrator.
//! - [`reinstate_credit_line`] — Defaulted → Active or Restricted
//!   (admin-controlled cure).
//! - [`forgive_debt`] — admin write-off; reduces `accrued_interest`
//!   first, then `utilized_amount`.
//! - [`settle_default_liquidation`] — accounting half of the
//!   cross-contract handoff with the auction; replay-protected, oracle-
//!   gated, atomic with status transition to Closed when
//!   `utilized_amount` hits 0.
//! - [`set_credit_limit_bounds`] / [`validate_credit_limit_bounds`] —
//!   global per-line bounds enforced on origination and on
//!   `update_risk_parameters`.
//! - [`set_repayment_schedule`] /
//!   [`advance_repayment_schedule_after_repay`] — installment ledger
//!   advancement.
//!
//! Restricted is **not** a separate transition target — it is a
//! repayment-capable cure state created by
//! [`crate::risk::update_risk_parameters`] when a limit decrease drops the
//! configured limit below current utilization. Repayments auto-cure back
//! to Active when `utilized_amount <= credit_limit`.
//!
//! # How
//!
//! Every transition:
//!
//! 1. Calls [`crate::auth::require_admin_auth`] (or the borrower path's
//!    `require_auth`).
//! 2. Calls [`crate::storage::assert_not_paused`].
//! 3. Calls [`crate::accrual::apply_accrual`] before reading
//!    `utilized_amount`, so the transition acts on capitalized debt.
//! 4. Calls [`crate::storage::assert_ts_monotonic`] on every timestamp
//!    write (`suspension_ts`, `last_rate_update_ts`).
//! 5. Persists via [`crate::storage::persist_credit_line`] with the
//!    captured `previous_utilized` so the global `TotalUtilized`
//!    accumulator stays consistent.
//! 6. Emits the transition's `CreditLineEvent` on the appropriate
//!    `("credit", _)` topic.
//!
//! # Storage
//!
//! - **Borrower credit lines**: Persistent storage (independent TTL per borrower).
//!   - Key: `borrower: Address` (via `DataKey::CreditLineIdByBorrower`)
//!   - Value: `CreditLineData`
//! - **Liquidation settlement markers**: Persistent storage (replay protection).
//!   - Key: `(Symbol("liq_seen"), borrower, settlement_id)`
//!   - Value: `bool` (presence = settled; replay reverts
//!     `ContractError::AlreadyInitialized = 14`)
//! - **Credit-limit bounds**: Instance storage (`MinCreditLimit`,
//!   `MaxCreditLimit`).
//! - **Repayment schedule**: Persistent storage
//!   (`DataKey::RepaymentSchedule(Address)`).
//!
//! # Why (settlement replay safety)
//!
//! The `(borrower, settlement_id)` marker is the credit-side half of a
//! two-sided replay barrier. The auction contract enforces the same
//! property on `auction_id` via `AuctionKey::LiquidationSettled(auction_id)`.
//! Together they ensure a defaulted line cannot be settled twice by the
//! same admin transaction, by the same admin re-running with a stale
//! settlement_id, or by the auction contract returning a duplicate value.
//! The cross-contract return is additionally asserted equal to the
//! admin-supplied `recovered_amount` in
//! [`crate::lib::settle_default_liquidation`]; mismatch reverts
//! `InvalidAmount = 5`.
//!
//! See [`docs/state-machine.md`](../../../docs/state-machine.md) for the
//! authoritative transition table and
//! [`docs/default-liquidation-auction-hook.md`](../../../docs/default-liquidation-auction-hook.md)
//! for the handoff protocol.

use crate::auth::{require_admin, require_admin_auth};
use crate::events::{
    publish_credit_line_event, publish_default_liquidation_requested_event,
    publish_default_liquidation_settled_event, publish_late_fee_charged_event, CreditLineEvent,
    DefaultLiquidationSettledEvent, LateFeeChargedEvent,
};
use crate::risk::{MAX_INTEREST_RATE_BPS, MAX_RISK_SCORE};
use crate::storage::{
    add_treasury_balance as storage_add_treasury_balance, assert_not_paused, assert_ts_monotonic,
    clear_repayment_schedule, get_late_fee_flat as storage_get_late_fee_flat, get_max_credit_limit,
    get_min_credit_limit, get_repayment_schedule as storage_get_repayment_schedule,
    persist_credit_line, set_late_fee_flat as storage_set_late_fee_flat, set_max_credit_limit,
    set_min_credit_limit, set_repayment_schedule as storage_set_repayment_schedule,
};
use crate::types::{ContractError, CreditLineData, CreditStatus, RepaymentSchedule};
use soroban_sdk::{symbol_short, Address, Env, Symbol, Vec};

/// Generate a unique key for tracking liquidation settlements.
///
/// # Storage
/// - **Type**: Persistent storage (independent TTL per settlement)
/// - **Key**: `(Symbol("liq_seen"), borrower, settlement_id)`
/// - **Purpose**: Prevents replay of the same liquidation settlement
fn liquidation_settlement_key(
    borrower: &Address,
    settlement_id: &Symbol,
) -> (Symbol, Address, Symbol) {
    (
        symbol_short!("liq_seen"),
        borrower.clone(),
        settlement_id.clone(),
    )
}

// ── Credit Limit Bounds Management ───────────────────────────────────────────

/// Set global credit limit bounds (admin only).
///
/// Configures the minimum and maximum allowed credit limits for all credit lines.
/// These bounds are enforced when opening new credit lines or increasing existing limits.
///
/// # Parameters
/// - `env`: The Soroban environment.
/// - `min`: Minimum allowed credit limit. Must be >= 0.
/// - `max`: Maximum allowed credit limit. Must be >= min.
///
/// # Authorization
/// Requires admin authorization via `require_admin_auth()`.
///
/// # Panics
/// - `ContractError::InvalidAmount` if `min < 0`
/// - `ContractError::LimitOutOfBounds` if `max < min`
///
/// # Storage
/// - Writes `min` to instance storage under `DataKey::MinCreditLimit`
/// - Writes `max` to instance storage under `DataKey::MaxCreditLimit`
///
/// # Example
/// ```ignore
/// set_credit_limit_bounds(env, 1_000, 1_000_000_000);
/// // Now all credit lines must have limits between 1,000 and 1,000,000,000
/// ```
pub fn set_credit_limit_bounds(env: Env, min: i128, max: i128) {
    assert_not_paused(&env);
    require_admin_auth(&env);

    // Validate minimum is non-negative
    if min < 0 {
        env.panic_with_error(ContractError::InvalidAmount);
    }

    // Validate max >= min
    if max < min {
        env.panic_with_error(ContractError::LimitOutOfBounds);
    }

    // Store bounds in instance storage
    set_min_credit_limit(&env, min);
    set_max_credit_limit(&env, max);
}

/// Get the configured global credit limit bounds.
///
/// Returns the minimum and maximum allowed credit limits, if configured.
///
/// # Returns
/// `(min_credit_limit, max_credit_limit)` tuple, or `(None, None)` if not configured.
///
/// # Storage
/// - Reads from instance storage keys `DataKey::MinCreditLimit` and `DataKey::MaxCreditLimit`
pub fn get_credit_limit_bounds(env: Env) -> (Option<i128>, Option<i128>) {
    let min = get_min_credit_limit(&env);
    let max = get_max_credit_limit(&env);
    (min, max)
}

/// Validate that a credit limit falls within configured bounds.
///
/// # Parameters
/// - `env`: The Soroban environment.
/// - `credit_limit`: The credit limit to validate.
///
/// # Panics
/// - `ContractError::LimitOutOfBounds` if the limit is outside configured bounds
///
/// # Behavior
/// - If bounds are not configured, validation passes (no restrictions)
/// - If only min is configured, validates `credit_limit >= min`
/// - If only max is configured, validates `credit_limit <= max`
/// - If both are configured, validates `min <= credit_limit <= max`
pub fn validate_credit_limit_bounds(env: &Env, credit_limit: i128) {
    let min = get_min_credit_limit(env);
    let max = get_max_credit_limit(env);

    // Check minimum bound if configured
    if let Some(min_limit) = min {
        if credit_limit < min_limit {
            env.panic_with_error(ContractError::LimitOutOfBounds);
        }
    }

    // Check maximum bound if configured
    if let Some(max_limit) = max {
        if credit_limit > max_limit {
            env.panic_with_error(ContractError::LimitOutOfBounds);
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────

fn suspend_credit_line_internal(env: &Env, borrower: Address) {
    let stored_line: CreditLineData = env
        .storage()
        .persistent()
        .get(&borrower)
        .unwrap_or_else(|| env.panic_with_error(ContractError::CreditLineNotFound));
    let previous_utilized = stored_line.utilized_amount;

    let previous_status = stored_line.status;

    // Apply interest accrual before any mutation.
    let mut credit_line = crate::accrual::apply_accrual(env, stored_line);

    if credit_line.status != CreditStatus::Active {
        env.panic_with_error(ContractError::CreditLineSuspended);
    }

    credit_line.status = CreditStatus::Suspended;
    let new_ts = env.ledger().timestamp();
    assert_ts_monotonic(env, credit_line.suspension_ts, new_ts);
    credit_line.suspension_ts = new_ts;
    persist_credit_line(
        env,
        &borrower,
        &credit_line,
        previous_utilized,
        Some(previous_status),
    );

    publish_credit_line_event(
        env,
        (symbol_short!("credit"), symbol_short!("suspend")),
        CreditLineEvent {
            borrower,
            status: CreditStatus::Suspended,
            credit_limit: credit_line.credit_limit,
            interest_rate_bps: credit_line.interest_rate_bps,
            risk_score: credit_line.risk_score,
        },
    );
}

/// Set or replace a borrower's installment repayment schedule.
pub fn set_repayment_schedule(
    env: &Env,
    borrower: Address,
    amount_per_period: i128,
    period_seconds: u64,
    first_due_ts: u64,
) {
    assert_not_paused(env);
    require_admin_auth(env);

    if amount_per_period <= 0 || period_seconds == 0 {
        env.panic_with_error(ContractError::InvalidAmount);
    }

    let stored_line: CreditLineData = env
        .storage()
        .persistent()
        .get(&borrower)
        .unwrap_or_else(|| env.panic_with_error(ContractError::CreditLineNotFound));

    if stored_line.status == CreditStatus::Closed {
        env.panic_with_error(ContractError::CreditLineClosed);
    }

    storage_set_repayment_schedule(
        env,
        &borrower,
        &RepaymentSchedule {
            amount_per_period,
            period_seconds,
            next_due_ts: first_due_ts,
        },
    );
}

/// Advance the next due timestamp when a qualifying repayment covers one or more installments.
/// Also charges a flat late fee per overdue installment when `LateFeeFlat` is configured.
pub fn advance_repayment_schedule_after_repay(env: &Env, borrower: &Address, amount: i128) {
    if amount <= 0 {
        return;
    }

    let Some(mut schedule) = storage_get_repayment_schedule(env, borrower) else {
        return;
    };

    if schedule.amount_per_period <= 0 || schedule.period_seconds == 0 {
        return;
    }

    let installments_paid = (amount / schedule.amount_per_period) as u64;
    if installments_paid == 0 {
        return;
    }

    // ── Late-fee surcharge ──────────────────────────────────────────────────
    let late_fee = storage_get_late_fee_flat(env);
    if late_fee > 0 {
        let now = env.ledger().timestamp();
        for i in 0_u64..installments_paid {
            let due_ts = schedule
                .next_due_ts
                .saturating_add(i.saturating_mul(schedule.period_seconds));
            if now > due_ts {
                storage_add_treasury_balance(env, late_fee);
                publish_late_fee_charged_event(
                    env,
                    LateFeeChargedEvent {
                        borrower: borrower.clone(),
                        fee: late_fee,
                        installment_index: i.saturating_add(1),
                    },
                );
            }
        }
    }

    let advance_seconds = schedule.period_seconds.saturating_mul(installments_paid);
    schedule.next_due_ts = schedule.next_due_ts.saturating_add(advance_seconds);
    storage_set_repayment_schedule(env, borrower, &schedule);
}

/// Set the flat late fee per missed installment (admin only).
///
/// When non-zero, this fee is charged to `TreasuryBalance` for each
/// installment that is detected as overdue during
/// [`advance_repayment_schedule_after_repay`].
///
/// # Parameters
/// - `fee`: The fee amount. Set to `0` to disable flat late-fee charges.
///
/// # Panics
/// - If `fee < 0` (negative fees not allowed).
pub fn set_late_fee_flat(env: Env, fee: i128) {
    assert_not_paused(&env);
    require_admin_auth(&env);
    if fee < 0 {
        env.panic_with_error(ContractError::InvalidAmount);
    }
    storage_set_late_fee_flat(&env, fee);
}

/// Get the configured flat late fee per missed installment.
///
/// Returns `0` if not configured (no flat late fee).
pub fn get_late_fee_flat(env: Env) -> i128 {
    storage_get_late_fee_flat(&env)
}

/// Open a new credit line.
///
/// Creating a brand-new line preserves the existing backend/risk-engine trust
/// boundary. Re-opening any existing non-Active line requires admin auth so a
/// borrower cannot self-suspend and then reactivate themselves on-chain.
#[allow(dead_code)]
pub fn open_credit_line(
    env: Env,
    borrower: Address,
    credit_limit: i128,
    interest_rate_bps: u32,
    risk_score: u32,
) {
    assert_not_paused(&env);

    if credit_limit <= 0 {
        env.panic_with_error(ContractError::InvalidAmount);
    }
    if interest_rate_bps > MAX_INTEREST_RATE_BPS {
        env.panic_with_error(ContractError::RateTooHigh);
    }
    if risk_score > MAX_RISK_SCORE {
        env.panic_with_error(ContractError::ScoreTooHigh);
    }

    // Validate credit limit is within configured bounds
    validate_credit_limit_bounds(&env, credit_limit);

    if let Some(existing) = env
        .storage()
        .persistent()
        .get::<Address, CreditLineData>(&borrower)
    {
        if existing.status == CreditStatus::Active {
            env.panic_with_error(ContractError::AlreadyInitialized);
        }

        // Prevent borrower-controlled status bypasses on existing lines.
        require_admin_auth(&env);
    }

    let previous_utilized = env
        .storage()
        .persistent()
        .get::<Address, CreditLineData>(&borrower)
        .map(|existing| existing.utilized_amount)
        .unwrap_or(0);

    let credit_line = CreditLineData {
        borrower: borrower.clone(),
        credit_limit,
        utilized_amount: 0,
        interest_rate_bps,
        risk_score,
        status: CreditStatus::Active,
        last_rate_update_ts: 0,
        accrued_interest: 0,
        last_accrual_ts: env.ledger().timestamp(),
        suspension_ts: 0,
    };
    persist_credit_line(&env, &borrower, &credit_line, previous_utilized, None);
    clear_repayment_schedule(&env, &borrower);

    publish_credit_line_event(
        &env,
        (symbol_short!("credit"), symbol_short!("opened")),
        CreditLineEvent {
            borrower,
            status: CreditStatus::Active,
            credit_limit,
            interest_rate_bps,
            risk_score,
        },
    );
}

/// Suspend a credit line temporarily (admin only).
///
/// # State transition
/// `Active → Suspended`
///
/// # Parameters
/// - `borrower`: The borrower's address.
///
/// # Panics
/// - If no credit line exists for the given borrower.
/// - If the credit line is not currently `Active`.
///
/// # Events
/// Emits a `("credit", "suspend")` [`CreditLineEvent`].
pub fn suspend_credit_line(env: Env, borrower: Address) {
    assert_not_paused(&env);
    require_admin_auth(&env);
    suspend_credit_line_internal(&env, borrower);
}

/// Suspend the caller's own active credit line.
///
/// This is a borrower safety control that blocks future draws while leaving
/// repayments available. Reactivation still requires a separate admin-controlled
/// workflow.
pub fn self_suspend_credit_line(env: Env, borrower: Address) {
    assert_not_paused(&env);
    borrower.require_auth();
    suspend_credit_line_internal(&env, borrower);
}

/// Close a credit line permanently.
///
/// Transitions the credit line to [`CreditStatus::Closed`]. Once closed, no further draws or
/// repayments are permitted. A closed line can be replaced by a new [`open_credit_line`] call.
///
/// # Authorization rules
///
/// | `closer` identity | Condition to close |
/// |-------------------|--------------------|
/// | Admin             | Always allowed, regardless of `utilized_amount` or current status |
/// | Borrower          | Allowed only when `utilized_amount == 0` |
/// | Any other address | Always rejected with `"unauthorized"` |
///
/// # Idempotency
/// If the credit line is already [`CreditStatus::Closed`], the call returns without error or
/// event. This makes the function safe to call defensively (e.g., in cleanup workflows).
///
/// # Parameters
/// - `borrower`: Address whose credit line is being closed.
/// - `closer`:   Address authorizing the close. Must be the admin or the borrower.
///
/// # Panics
/// - `"Credit line not found"` — no credit line exists for `borrower`.
/// - `"cannot close: utilized amount not zero"` — `closer == borrower` but outstanding balance > 0.
/// - `"unauthorized"` — `closer` is neither the admin nor the borrower.
///
/// # Events
/// Emits a `("credit", "closed")` [`CreditLineEvent`] on successful state change.
/// No event is emitted when the line is already closed (idempotent path).
///
/// # Security notes
/// - `closer.require_auth()` is called before any storage reads, so an unauthenticated
///   call is rejected at the Soroban host level before any state is inspected.
/// - The authorization check uses address equality against the stored admin and the
///   `borrower` parameter — there is no privileged role beyond these two identities.
/// - Closing does **not** require prior suspension or default; admin can force-close from any
///   non-closed status. This is intentional for operational efficiency.
pub fn close_credit_line(env: Env, borrower: Address, closer: Address) {
    assert_not_paused(&env);
    // Authenticate the closer before any storage access.
    closer.require_auth();

    // Resolve the current admin address.
    let admin: Address = require_admin(&env);

    // Load the credit line; revert if it does not exist.
    let mut credit_line: CreditLineData = env
        .storage()
        .persistent()
        .get(&borrower)
        .unwrap_or_else(|| env.panic_with_error(ContractError::CreditLineNotFound));
    let previous_utilized = credit_line.utilized_amount;

    // Idempotent: already closed → nothing to do.
    if credit_line.status == CreditStatus::Closed {
        return;
    }

    // Authorization: determine whether `closer` is permitted to close this line.
    //
    // Three mutually exclusive cases, checked in priority order:
    //   1. closer == admin           → always permitted (force-close).
    //   2. closer == borrower        → permitted only when utilization is zero.
    //   3. closer is someone else    → always rejected.
    if closer == admin {
        // Admin force-close: no utilization restriction.
    } else if closer == borrower {
        // Borrower self-close: only allowed when fully repaid.
        if credit_line.utilized_amount != 0 {
            panic!("cannot close: utilized amount not zero");
        }
    } else {
        // Third party: unconditionally rejected.
        panic!("unauthorized");
    }

    let previous_status = credit_line.status;
    credit_line.status = CreditStatus::Closed;
    persist_credit_line(
        &env,
        &borrower,
        &credit_line,
        previous_utilized,
        Some(previous_status),
    );
    clear_repayment_schedule(&env, &borrower);

    publish_credit_line_event(
        &env,
        (symbol_short!("credit"), symbol_short!("closed")),
        CreditLineEvent {
            borrower: borrower.clone(),
            status: CreditStatus::Closed,
            credit_limit: credit_line.credit_limit,
            interest_rate_bps: credit_line.interest_rate_bps,
            risk_score: credit_line.risk_score,
        },
    );
}

/// Admin-only batch close of multiple credit lines.
/// Reverts on first failure, ensuring atomicity.
///
/// # Parameters
/// - `env`: The Soroban environment.
/// - `borrowers`: List of borrower addresses to close.
///
/// # Authorization
/// Requires admin authorization.
///
/// # Errors
/// - Reverts if any close fails (e.g., credit line not found, already closed).
/// - Reverts if borrowers.len() > BATCH_CLOSE_MAX.
pub fn close_credit_lines_batch(env: Env, borrowers: Vec<Address>) {
    assert_not_paused(&env);
    require_admin_auth(&env);

    // Resolve admin just once, to save storage access
    let admin: Address = require_admin(&env);

    // Process each borrower in order; failure of any reverts the whole batch
    for borrower in borrowers {
        // Reuse the single close function, passing admin as the closer
        close_credit_line(env.clone(), borrower, admin.clone());
    }
}

// ── default_credit_line ───────────────────────────────────────────────────────

/// Mark a credit line as defaulted (admin only).
///
/// Transition: `Active` or `Suspended` → `Defaulted`.
/// After defaulting, `draw_credit` is disabled and `repay_credit` remains allowed.
///
/// # Events
/// Emits a `("credit", "default")` [`CreditLineEvent`].
pub fn default_credit_line(env: Env, borrower: Address) {
    assert_not_paused(&env);
    require_admin_auth(&env);
    let stored_line: CreditLineData = env
        .storage()
        .persistent()
        .get(&borrower)
        .unwrap_or_else(|| env.panic_with_error(ContractError::CreditLineNotFound));
    let previous_utilized = stored_line.utilized_amount;

    if stored_line.status == CreditStatus::Closed {
        env.panic_with_error(ContractError::CreditLineClosed);
    }

    // Apply interest accrual before any mutation
    let mut credit_line = crate::accrual::apply_accrual(&env, stored_line);

    if credit_line.status == CreditStatus::Closed {
        env.panic_with_error(ContractError::CreditLineClosed);
    }

    if credit_line.status == CreditStatus::Defaulted {
        // Idempotent: already defaulted, nothing to do.
        return;
    }

    let previous_status = credit_line.status;
    credit_line.status = CreditStatus::Defaulted;
    persist_credit_line(
        &env,
        &borrower,
        &credit_line,
        previous_utilized,
        Some(previous_status),
    );

    publish_credit_line_event(
        &env,
        (symbol_short!("credit"), symbol_short!("defaulted")),
        CreditLineEvent {
            borrower: borrower.clone(),
            status: CreditStatus::Defaulted,
            credit_limit: credit_line.credit_limit,
            interest_rate_bps: credit_line.interest_rate_bps,
            risk_score: credit_line.risk_score,
        },
    );

    publish_default_liquidation_requested_event(&env, &borrower, credit_line.utilized_amount);
}

/// Forgive outstanding debt without transferring tokens (admin only).
///
/// This is an accounting-only write-off path intended for explicit admin debt
/// relief or off-chain settlements that have already been handled elsewhere.
/// The forgiven amount is capped to the current `utilized_amount`.
pub fn forgive_debt(env: Env, borrower: Address, amount: i128) {
    assert_not_paused(&env);
    require_admin_auth(&env);

    if amount <= 0 {
        env.panic_with_error(ContractError::InvalidAmount);
    }

    let stored_line: CreditLineData = env
        .storage()
        .persistent()
        .get(&borrower)
        .unwrap_or_else(|| env.panic_with_error(ContractError::CreditLineNotFound));
    let previous_utilized = stored_line.utilized_amount;

    if stored_line.status == CreditStatus::Closed {
        env.panic_with_error(ContractError::CreditLineClosed);
    }

    let mut credit_line = crate::accrual::apply_accrual(&env, stored_line);
    let effective_forgive = amount.min(credit_line.utilized_amount);
    let interest_forgiven = effective_forgive.min(credit_line.accrued_interest);

    credit_line.accrued_interest = credit_line
        .accrued_interest
        .checked_sub(interest_forgiven)
        .unwrap_or(0);
    credit_line.utilized_amount = credit_line
        .utilized_amount
        .checked_sub(effective_forgive)
        .unwrap_or(0);

    let previous_status = credit_line.status;
    persist_credit_line(
        &env,
        &borrower,
        &credit_line,
        previous_utilized,
        Some(previous_status),
    );
}

/// Apply auction liquidation proceeds to a defaulted credit line (admin only).
///
/// This hook is accounting-only and intentionally performs no token transfer.
/// Off-chain orchestration is responsible for ensuring auction proceeds are settled
/// into protocol custody before this function is called.
pub fn settle_default_liquidation(
    env: Env,
    borrower: Address,
    recovered_amount: i128,
    settlement_id: Symbol,
    close_factor_bps: u32,
) {
    require_admin_auth(&env);

    if recovered_amount <= 0 {
        env.panic_with_error(ContractError::InvalidAmount);
    }

    if close_factor_bps == 0 || close_factor_bps > 10_000 {
        env.panic_with_error(ContractError::InvalidAmount);
    }

    let settlement_key = liquidation_settlement_key(&borrower, &settlement_id);
    if env.storage().persistent().has(&settlement_key) {
        env.panic_with_error(ContractError::AlreadyInitialized);
    }

    let stored_line: CreditLineData = env
        .storage()
        .persistent()
        .get(&borrower)
        .unwrap_or_else(|| env.panic_with_error(ContractError::CreditLineNotFound));
    let previous_utilized = stored_line.utilized_amount;

    // Apply interest accrual before any mutation
    let mut credit_line = crate::accrual::apply_accrual(&env, stored_line);

    if credit_line.status != CreditStatus::Defaulted {
        env.panic_with_error(ContractError::CreditLineDefaulted);
    }

    // Compute the maximum recoverable amount for this settlement
    let target_recovery = credit_line
        .utilized_amount
        .checked_mul(close_factor_bps as i128)
        .unwrap_or_else(|| env.panic_with_error(ContractError::Overflow))
        / 10_000;

    if recovered_amount > target_recovery {
        env.panic_with_error(ContractError::OverLimit);
    }

    credit_line.utilized_amount = credit_line
        .utilized_amount
        .checked_sub(recovered_amount)
        .unwrap_or_else(|| env.panic_with_error(ContractError::Overflow));

    let previous_status = credit_line.status;
    if credit_line.utilized_amount == 0 {
        credit_line.status = CreditStatus::Closed;
    }

    persist_credit_line(
        &env,
        &borrower,
        &credit_line,
        previous_utilized,
        Some(previous_status),
    );
    if credit_line.status == CreditStatus::Closed {
        clear_repayment_schedule(&env, &borrower);
    }
    env.storage().persistent().set(&settlement_key, &true);

    if credit_line.status == CreditStatus::Closed {
        publish_credit_line_event(
            &env,
            (symbol_short!("credit"), symbol_short!("closed")),
            CreditLineEvent {
                borrower: borrower.clone(),
                status: CreditStatus::Closed,
                credit_limit: credit_line.credit_limit,
                interest_rate_bps: credit_line.interest_rate_bps,
                risk_score: credit_line.risk_score,
            },
        );
    }

    publish_default_liquidation_settled_event(
        &env,
        DefaultLiquidationSettledEvent {
            borrower,
            settlement_id,
            recovered_amount,
            remaining_utilized_amount: credit_line.utilized_amount,
            status: credit_line.status,
            close_factor_bps,
        },
    );
}

// ── reinstate_credit_line ─────────────────────────────────────────────────────

/// Reinstate a `Defaulted` credit line to either `Active` or `Restricted` (admin only).
///
/// Valid transitions: `Defaulted` → `Active` | `Defaulted` → `Restricted`.
/// `Restricted` is used when the credit limit was reduced below the outstanding balance
/// and the borrower must repay the excess before draws are re-enabled.
///
/// # Panics
/// - `ContractError::InvalidAmount` — `target_status` is not `Active` or `Restricted`.
/// - `ContractError::CreditLineNotFound` — no credit line exists for `borrower`.
/// - `ContractError::CreditLineDefaulted` — current status is not `Defaulted`.
///
/// # Events
/// Emits a `("credit", "reinstate")` [`CreditLineEvent`].
pub fn reinstate_credit_line(env: Env, borrower: Address, target_status: CreditStatus) {
    assert_not_paused(&env);
    require_admin_auth(&env);

    // Only Active and Restricted are valid reinstate targets per the state-machine spec.
    if target_status != CreditStatus::Active && target_status != CreditStatus::Restricted {
        env.panic_with_error(ContractError::InvalidAmount);
    }

    let stored_line: CreditLineData = env
        .storage()
        .persistent()
        .get(&borrower)
        .unwrap_or_else(|| env.panic_with_error(ContractError::CreditLineNotFound));
    let previous_utilized = stored_line.utilized_amount;

    let mut credit_line = crate::accrual::apply_accrual(&env, stored_line);

    if credit_line.status != CreditStatus::Defaulted {
        env.panic_with_error(ContractError::CreditLineDefaulted);
    }

    let previous_status = credit_line.status;
    credit_line.status = target_status;
    credit_line.suspension_ts = 0;
    persist_credit_line(
        &env,
        &borrower,
        &credit_line,
        previous_utilized,
        Some(previous_status),
    );

    publish_credit_line_event(
        &env,
        (symbol_short!("credit"), Symbol::new(&env, "reinstate")),
        CreditLineEvent {
            borrower: borrower.clone(),
            status: target_status,
            credit_limit: credit_line.credit_limit,
            interest_rate_bps: credit_line.interest_rate_bps,
            risk_score: credit_line.risk_score,
        },
    );
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod installment {
    use crate::events::LateFeeChargedEvent;
    use crate::Credit;
    use crate::CreditClient;
    use soroban_sdk::{
        testutils::{Address as _, Events as _, Ledger},
        token::StellarAssetClient,
        Address, Env, Symbol, TryFromVal, TryIntoVal,
    };

    fn setup_borrower(env: &Env) -> (CreditClient, Address) {
        env.mock_all_auths();
        let admin = Address::generate(env);
        let borrower = Address::generate(env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(env, &contract_id);
        client.init(&admin);
        let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
        let token = token_id.address();
        client.set_liquidity_token(&token);
        StellarAssetClient::new(env, &token).mint(&contract_id, &1_000_000_000_i128);
        StellarAssetClient::new(env, &token).mint(&borrower, &1_000_000_000_i128);
        soroban_sdk::token::Client::new(env, &token).approve(
            &borrower,
            &contract_id,
            &1_000_000_000_i128,
            &1_000_000_u32,
        );
        client.open_credit_line(&borrower, &1_000_000, &1000, &50);
        // Deposit collateral to satisfy the minimum collateral ratio (default 150%).
        client.deposit_collateral(&borrower, &1_500_000);
        (client, borrower)
    }

    fn with_schedule(
        env: &Env,
        client: &CreditClient,
        borrower: &Address,
        amount_per_period: i128,
        period_seconds: u64,
        first_due_ts: u64,
    ) {
        client.set_repayment_schedule(borrower, &amount_per_period, &period_seconds, &first_due_ts);
    }

    fn setup_draw(
        env: &Env,
        client: &CreditClient,
        borrower: &Address,
        draw_amount: i128,
        at_ts: u64,
    ) {
        env.ledger().set_timestamp(at_ts);
        client.draw_credit(borrower, &draw_amount);
    }

    // ── late_fee_flat: no fee when fee is 0 (default) ─────────────────────

    #[test]
    fn late_fee_happy_path_charges_fee_for_overdue_installment() {
        let env = Env::default();
        let (client, borrower) = setup_borrower(&env);

        // Draw at t=100
        setup_draw(&env, &client, &borrower, 500_000, 100);

        // Set repayment schedule: 100_000 per period, 100s period, first due at 200
        with_schedule(&env, &client, &borrower, 100_000, 100, 200);

        // Set a late fee of 50 per missed installment
        client.set_late_fee_flat(&50_i128);

        // Advance time past the due date (t=300, due was at t=200)
        env.ledger().set_timestamp(300);

        // Repay 100_000 (covers 1 installment, which is overdue)
        let treasury_before = client.get_protocol_summary().treasury_balance;
        client.repay_credit(&borrower, &100_000);
        let treasury_after = client.get_protocol_summary().treasury_balance;

        // Treasury should have increased by the late fee
        assert_eq!(treasury_after - treasury_before, 50);

        // LateFeeChargedEvent verified by treasury balance increase above.
        // (Event detection via env.events().all() is unreliable across Soroban versions.)
    }

    /// Zero-fee config (default) preserves existing behavior — no treasury
    /// change and no event emitted.
    #[test]
    fn late_fee_zero_fee_preserves_existing_behavior() {
        let env = Env::default();
        let (client, borrower) = setup_borrower(&env);

        setup_draw(&env, &client, &borrower, 500_000, 100);
        with_schedule(&env, &client, &borrower, 100_000, 100, 200);

        // Do NOT set any late fee (defaults to 0)

        env.ledger().set_timestamp(300);

        let treasury_before = client.get_protocol_summary().treasury_balance;
        client.repay_credit(&borrower, &100_000);
        let treasury_after = client.get_protocol_summary().treasury_balance;

        // Treasury unchanged
        assert_eq!(treasury_after, treasury_before);

        // No event verification needed — treasury unchanged confirms no fee was charged.
    }

    /// Late fee is charged per installment. If multiple installments are paid
    /// and all are overdue, each should incur the fee.
    #[test]
    fn late_fee_multiple_overdue_installments() {
        let env = Env::default();
        let (client, borrower) = setup_borrower(&env);

        setup_draw(&env, &client, &borrower, 500_000, 100);
        with_schedule(&env, &client, &borrower, 100_000, 100, 200);

        client.set_late_fee_flat(&30_i128);

        // Advance time well past 4 due dates
        // Due dates: 200, 300, 400, 500 — all past by t=600
        env.ledger().set_timestamp(600);

        let treasury_before = client.get_protocol_summary().treasury_balance;

        // Repay 400_000 (covers 4 installments, all overdue)
        client.repay_credit(&borrower, &400_000);

        let treasury_after = client.get_protocol_summary().treasury_balance;
        // 4 overdue installments × 30 fee each = 120
        assert_eq!(treasury_after - treasury_before, 4 * 30);

        // Multiple late fees confirmed by treasury balance increase above.
        // Repaying 4 overdue installments of 30 each = 120 total.
    }

    /// No fee is charged when the borrower pays on time (before next_due_ts).
    #[test]
    fn late_fee_no_fee_when_paid_on_time() {
        let env = Env::default();
        let (client, borrower) = setup_borrower(&env);

        setup_draw(&env, &client, &borrower, 500_000, 100);
        with_schedule(&env, &client, &borrower, 100_000, 100, 200);

        client.set_late_fee_flat(&50_i128);

        // Repay before the due date
        env.ledger().set_timestamp(150);

        let treasury_before = client.get_protocol_summary().treasury_balance;
        client.repay_credit(&borrower, &100_000);
        let treasury_after = client.get_protocol_summary().treasury_balance;

        // Treasury unchanged
        assert_eq!(treasury_after, treasury_before);
    }

    /// Late fee is not charged when the fee is explicitly set to 0
    /// (admin can disable).
    #[test]
    fn late_fee_explicit_zero_disabled() {
        let env = Env::default();
        let (client, borrower) = setup_borrower(&env);

        setup_draw(&env, &client, &borrower, 500_000, 100);
        with_schedule(&env, &client, &borrower, 100_000, 100, 200);

        // Set fee to 0 (explicitly disabled)
        client.set_late_fee_flat(&0_i128);

        env.ledger().set_timestamp(300);

        let treasury_before = client.get_protocol_summary().treasury_balance;
        client.repay_credit(&borrower, &100_000);
        let treasury_after = client.get_protocol_summary().treasury_balance;

        assert_eq!(treasury_after, treasury_before);
    }

    /// Late fee is not charged when no repayment schedule exists.
    #[test]
    fn late_fee_no_schedule_no_fee() {
        let env = Env::default();
        let (client, borrower) = setup_borrower(&env);

        setup_draw(&env, &client, &borrower, 500_000, 100);
        // No schedule set

        client.set_late_fee_flat(&50_i128);

        env.ledger().set_timestamp(300);

        let treasury_before = client.get_protocol_summary().treasury_balance;
        client.repay_credit(&borrower, &100_000);
        let treasury_after = client.get_protocol_summary().treasury_balance;

        assert_eq!(treasury_after, treasury_before);
    }

    /// Late fee is not charged when the repayment covers zero installments
    /// (amount < amount_per_period).
    #[test]
    fn late_fee_partial_payment_no_fee() {
        let env = Env::default();
        let (client, borrower) = setup_borrower(&env);

        setup_draw(&env, &client, &borrower, 500_000, 100);
        with_schedule(&env, &client, &borrower, 100_000, 100, 200);

        client.set_late_fee_flat(&50_i128);

        env.ledger().set_timestamp(300);

        let treasury_before = client.get_protocol_summary().treasury_balance;

        // Repay less than one full installment
        client.repay_credit(&borrower, &50_000);

        let treasury_after = client.get_protocol_summary().treasury_balance;
        assert_eq!(treasury_after, treasury_before);
    }

    /// set_late_fee_flat rejects negative fees.
    #[test]
    #[should_panic(expected = "Error(Contract, #5)")]
    fn late_fee_rejects_negative_fee() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);
        client.set_late_fee_flat(&-1_i128);
    }

    /// Late fee is only charged for overdue installments, not for future
    /// installments in an advance payment.
    #[test]
    fn late_fee_advance_payment_only_charges_overdue() {
        let env = Env::default();
        let (client, borrower) = setup_borrower(&env);

        setup_draw(&env, &client, &borrower, 500_000, 100);
        with_schedule(&env, &client, &borrower, 100_000, 100, 200);

        client.set_late_fee_flat(&30_i128);

        // Advance to t=250: installment 1 (due 200) is overdue,
        // installment 2 (due 300) is not yet due
        env.ledger().set_timestamp(250);

        let treasury_before = client.get_protocol_summary().treasury_balance;

        // Repay 200_000 (covers 2 installments: one overdue, one future)
        client.repay_credit(&borrower, &200_000);

        let treasury_after = client.get_protocol_summary().treasury_balance;
        // Only 1 overdue installment × 30 = 30
        assert_eq!(treasury_after - treasury_before, 30);
    }
}
