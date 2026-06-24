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
    publish_default_liquidation_settled_event, CreditLineEvent, DefaultLiquidationSettledEvent,
};
use crate::risk::{MAX_INTEREST_RATE_BPS, MAX_RISK_SCORE};
use crate::storage::{
    assert_not_paused, assert_ts_monotonic, clear_repayment_schedule, persist_credit_line,
    get_repayment_schedule as storage_get_repayment_schedule,
    set_repayment_schedule as storage_set_repayment_schedule,
    get_min_credit_limit, set_min_credit_limit,
    get_max_credit_limit, set_max_credit_limit,
};
use crate::types::{ContractError, CreditLineData, CreditStatus, RepaymentSchedule};
use soroban_sdk::{symbol_short, Address, Env, Symbol};

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

    // Apply interest accrual before any mutation.
    let mut credit_line = crate::accrual::apply_accrual(env, stored_line);

    if credit_line.status != CreditStatus::Active {
        env.panic_with_error(ContractError::CreditLineSuspended);
    }

    credit_line.status = CreditStatus::Suspended;
    let new_ts = env.ledger().timestamp();
    assert_ts_monotonic(env, credit_line.suspension_ts, new_ts);
    credit_line.suspension_ts = new_ts;
    persist_credit_line(env, &borrower, &credit_line, previous_utilized);

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

    let advance_seconds = schedule.period_seconds.saturating_mul(installments_paid);
    schedule.next_due_ts = schedule.next_due_ts.saturating_add(advance_seconds);
    storage_set_repayment_schedule(env, borrower, &schedule);
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
    persist_credit_line(&env, &borrower, &credit_line, previous_utilized);
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

    credit_line.status = CreditStatus::Closed;
    persist_credit_line(&env, &borrower, &credit_line, previous_utilized);
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

    credit_line.status = CreditStatus::Defaulted;
    persist_credit_line(&env, &borrower, &credit_line, previous_utilized);

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

    persist_credit_line(&env, &borrower, &credit_line, previous_utilized);
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
) {
    require_admin_auth(&env);

    if recovered_amount <= 0 {
        env.panic_with_error(ContractError::InvalidAmount);
    }

    let settlement_key = liquidation_settlement_key(&borrower, &settlement_id);
    if env.storage().persistent().has(&settlement_key) {
        env.panic_with_error(ContractError::AlreadyInitialized); // Or a specific LiquidationAlreadyApplied
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

    if recovered_amount > credit_line.utilized_amount {
        env.panic_with_error(ContractError::OverLimit); // Or a specific error
    }

    credit_line.utilized_amount = credit_line
        .utilized_amount
        .checked_sub(recovered_amount)
        .unwrap_or_else(|| env.panic_with_error(ContractError::Overflow));

    if credit_line.utilized_amount == 0 {
        credit_line.status = CreditStatus::Closed;
    }

    persist_credit_line(&env, &borrower, &credit_line, previous_utilized);
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

    credit_line.status = target_status;
    credit_line.suspension_ts = 0;
    persist_credit_line(&env, &borrower, &credit_line, previous_utilized);

    publish_credit_line_event(
        &env,
        (symbol_short!("credit"), symbol_short!("reinstate")),
        CreditLineEvent {
            borrower: borrower.clone(),
            status: target_status,
            credit_limit: credit_line.credit_limit,
            interest_rate_bps: credit_line.interest_rate_bps,
            risk_score: credit_line.risk_score,
        },
    );
}

/// Allow a borrower to voluntarily suspend their own credit line.
///
/// This function enables borrowers to freeze their own line of credit without admin intervention.
/// Only the borrower who owns the credit line can invoke this action.
///
/// # Parameters
/// - `borrower`: The borrower's address (must authorize this call).
///
/// # Authorization
/// - Requires authorization from the `borrower` address.
/// - Admin cannot invoke this function on behalf of a borrower.
///
/// # State Transitions
/// - Valid: `Active` → `Suspended`
/// - Invalid: Any other status (Suspended, Defaulted, Closed) will cause a panic.
///
/// # Post-Suspension Behavior
/// - Draw operations are blocked while the line is self-suspended.
/// - Repayment operations remain allowed.
/// - Admin can reinstate the line to Active status via `reinstate_credit_line`.
/// - Admin can force-close the line via `close_credit_line`.
///
/// # Panics
/// - If no credit line exists for the given borrower.
/// - If the credit line status is not `Active`.
/// - If the caller is not the borrower (authorization failure).
///
/// # Events
/// Emits a `("credit", "selfsus")` [`CreditLineEvent`] with the updated status.
pub fn self_suspend_credit_line(env: Env, borrower: Address) {
    // Require authorization from the borrower (not admin)
    borrower.require_auth();

    let mut credit_line: CreditLineData = env
        .storage()
        .persistent()
        .get(&borrower)
        .expect("Credit line not found");

    // Apply interest accrual before any mutation
    credit_line = crate::accrual::apply_accrual(&env, credit_line);

    // Only allow self-suspension from Active status
    if credit_line.status != CreditStatus::Active {
        panic!("Only active credit lines can be self-suspended");
    }

    // ── helpers ───────────────────────────────────────────────────────────────

    fn setup(env: &Env) -> (TestCreditClient<'_>, Address, Address) {
        env.mock_all_auths();
        let admin = Address::generate(env);
        let contract_id = env.register(TestCredit, ());
        let client = TestCreditClient::new(env, &contract_id);
        client.init(&admin);
        (client, contract_id, admin)
    }

    fn open_line(client: &TestCreditClient<'_>, borrower: &Address) {
        client.open(borrower, &1_000_i128, &300_u32, &70_u32);
    }

    // ── 1. Borrower closes with zero utilization ───────────────────────────────

    #[test]
    fn borrower_can_close_when_utilization_is_zero() {
        let env = Env::default();
        let (client, _cid, _admin) = setup(&env);
        let borrower = Address::generate(&env);
        open_line(&client, &borrower);

        // utilized_amount is 0 at open → borrower can close
        client.close(&borrower, &borrower);

        let line = client.get(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Closed);
        assert_eq!(line.utilized_amount, 0);
    }

    // ── 2. Admin closes with non-zero utilization (force-close) ───────────────

    #[test]
    fn admin_can_force_close_with_non_zero_utilization() {
        let env = Env::default();
        let (client, _cid, admin) = setup(&env);
        let borrower = Address::generate(&env);
        open_line(&client, &borrower);
        client.draw(&borrower, &400_i128);

        assert_eq!(client.get(&borrower).unwrap().utilized_amount, 400);

        client.close(&borrower, &admin);

        let line = client.get(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Closed);
    }

    // ── 3. Admin closes with zero utilization ────────────────────────────────

    #[test]
    fn admin_can_close_with_zero_utilization() {
        let env = Env::default();
        let (client, _cid, admin) = setup(&env);
        let borrower = Address::generate(&env);
        open_line(&client, &borrower);

        client.close(&borrower, &admin);

        assert_eq!(client.get(&borrower).unwrap().status, CreditStatus::Closed);
    }

    // ── 4. Borrower cannot close with outstanding balance ─────────────────────

    #[test]
    #[should_panic(expected = "cannot close: utilized amount not zero")]
    fn borrower_cannot_close_with_non_zero_utilization() {
        let env = Env::default();
        let (client, _cid, _admin) = setup(&env);
        let borrower = Address::generate(&env);
        open_line(&client, &borrower);
        client.draw(&borrower, &1_i128); // any positive draw

        client.close(&borrower, &borrower);
    }

    // ── 5. Third party (neither admin nor borrower) is rejected ───────────────

    #[test]
    #[should_panic(expected = "unauthorized")]
    fn stranger_cannot_close_credit_line() {
        let env = Env::default();
        let (client, _cid, _admin) = setup(&env);
        let borrower = Address::generate(&env);
        let stranger = Address::generate(&env);
        open_line(&client, &borrower);

        client.close(&borrower, &stranger);
    }

    // ── 6. Stranger with zero utilization is still rejected ───────────────────

    #[test]
    #[should_panic(expected = "unauthorized")]
    fn stranger_cannot_close_even_with_zero_utilization() {
        let env = Env::default();
        let (client, _cid, _admin) = setup(&env);
        let borrower = Address::generate(&env);
        let stranger = Address::generate(&env);
        open_line(&client, &borrower);
        // line has zero utilization but closer is neither admin nor borrower
        client.close(&borrower, &stranger);
    }

    // ── 7. Close is idempotent when already Closed ────────────────────────────

    #[test]
    fn close_is_idempotent_when_already_closed() {
        let env = Env::default();
        let (client, _cid, admin) = setup(&env);
        let borrower = Address::generate(&env);
        open_line(&client, &borrower);

        client.close(&borrower, &admin);
        // Second call must not panic
        client.close(&borrower, &admin);

        assert_eq!(client.get(&borrower).unwrap().status, CreditStatus::Closed);
    }

    // ── 8. No draw after close ────────────────────────────────────────────────
    // (draw is tested at the lib.rs level via draw_credit; here we verify that
    //  storage status is Closed so the draw_credit status check will fire.)

    #[test]
    fn closed_line_has_closed_status_preventing_draws() {
        let env = Env::default();
        let (client, _cid, admin) = setup(&env);
        let borrower = Address::generate(&env);
        open_line(&client, &borrower);
        client.close(&borrower, &admin);

        let line = client.get(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Closed);
        // draw_credit in lib.rs checks status == Closed and reverts with CreditLineClosed
    }

    // ── 9. Admin closes a Suspended line ─────────────────────────────────────

    #[test]
    fn admin_can_close_suspended_line() {
        let env = Env::default();
        let (client, _cid, admin) = setup(&env);
        let borrower = Address::generate(&env);
        open_line(&client, &borrower);
        client.suspend(&borrower);

        assert_eq!(
            client.get(&borrower).unwrap().status,
            CreditStatus::Suspended
        );

        client.close(&borrower, &admin);

        assert_eq!(client.get(&borrower).unwrap().status, CreditStatus::Closed);
    }

    // ── 10. Admin closes a Defaulted line ────────────────────────────────────

    #[test]
    fn admin_can_close_defaulted_line() {
        let env = Env::default();
        let (client, _cid, admin) = setup(&env);
        let borrower = Address::generate(&env);
        open_line(&client, &borrower);
        client.default_line(&borrower);

        assert_eq!(
            client.get(&borrower).unwrap().status,
            CreditStatus::Defaulted
        );

        client.close(&borrower, &admin);

        assert_eq!(client.get(&borrower).unwrap().status, CreditStatus::Closed);
    }

    // ── 11. Borrower closes a Suspended line with zero utilization ────────────

    #[test]
    fn borrower_can_close_suspended_line_with_zero_utilization() {
        let env = Env::default();
        let (client, _cid, _admin) = setup(&env);
        let borrower = Address::generate(&env);
        open_line(&client, &borrower);
        client.suspend(&borrower);

        // utilized_amount is still 0 → borrower may close
        client.close(&borrower, &borrower);

        assert_eq!(client.get(&borrower).unwrap().status, CreditStatus::Closed);
    }

    // ── 12. close emits ("credit", "closed") event ────────────────────────────

    #[test]
    fn close_emits_closed_event_with_correct_topics_and_status() {
        let env = Env::default();
        let (client, _cid, admin) = setup(&env);
        let borrower = Address::generate(&env);
        open_line(&client, &borrower);

        client.close(&borrower, &admin);

        let events = env.events().all();
        let (_contract, topics, data) = events.last().unwrap();

        let topic0: Symbol = Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap();
        let topic1: Symbol = Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap();
        assert_eq!(topic0, symbol_short!("credit"));
        assert_eq!(topic1, symbol_short!("closed"));

        let event: CreditLineEvent = data.try_into_val(&env).unwrap();
        assert_eq!(event.status, CreditStatus::Closed);
        assert_eq!(event.borrower, borrower);
    }

    // ── 13. Idempotent close emits no second event ────────────────────────────

    #[test]
    fn idempotent_close_emits_no_additional_event() {
        let env = Env::default();
        let (client, _cid, admin) = setup(&env);
        let borrower = Address::generate(&env);
        open_line(&client, &borrower);

        client.close(&borrower, &admin);

        client.close(&borrower, &admin); // idempotent
        let event_count_after_second = env.events().all().len();

        assert_eq!(
            event_count_after_second, 0,
            "idempotent close must not emit a second event"
        );
    }

    // ── 14. Non-existent credit line reverts ─────────────────────────────────

    #[test]
    #[should_panic(expected = "Credit line not found")]
    fn close_nonexistent_line_reverts() {
        let env = Env::default();
        let (client, _cid, admin) = setup(&env);
        let borrower = Address::generate(&env); // no open_line call

        client.close(&borrower, &admin);
    }

    // ── 15. Closed line status persists; other fields unchanged ───────────────

    #[test]
    fn close_sets_status_to_closed_and_does_not_mutate_other_fields() {
        let env = Env::default();
        let (client, _cid, admin) = setup(&env);
        let borrower = Address::generate(&env);
        open_line(&client, &borrower);
        let before = client.get(&borrower).unwrap();

        client.close(&borrower, &admin);
        let after = client.get(&borrower).unwrap();

        assert_eq!(after.status, CreditStatus::Closed);
        assert_eq!(after.borrower, before.borrower);
        assert_eq!(after.credit_limit, before.credit_limit);
        assert_eq!(after.utilized_amount, before.utilized_amount);
        assert_eq!(after.interest_rate_bps, before.interest_rate_bps);
        assert_eq!(after.risk_score, before.risk_score);
    }

    // ── 16. open_credit_line succeeds after Closed (re-open path) ─────────────

    #[test]
    fn open_credit_line_succeeds_after_close() {
        let env = Env::default();
        let (client, _cid, admin) = setup(&env);
        let borrower = Address::generate(&env);
        open_line(&client, &borrower);
        client.close(&borrower, &admin);

        // Re-opening a Closed line must succeed (status != Active guard)
        client.open(&borrower, &2_000_i128, &400_u32, &60_u32);

        let line = client.get(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Active);
        assert_eq!(line.credit_limit, 2_000);
        assert_eq!(line.utilized_amount, 0);
    }

    // ── 17. Borrower closes with exact-zero boundary ──────────────────────────

    #[test]
    fn borrower_close_at_exact_zero_utilization_boundary() {
        let env = Env::default();
        let (client, _cid, _admin) = setup(&env);
        let borrower = Address::generate(&env);

        // Open with credit_limit == 1 to make the boundary obvious
        client.open(&borrower, &1_i128, &300_u32, &70_u32);
        // Do not draw; utilized_amount == 0 exactly
        client.close(&borrower, &borrower);

        assert_eq!(client.get(&borrower).unwrap().status, CreditStatus::Closed);
    }

    // ── 18. Admin auth is required ────────────────────────────────────────────

    #[test]
    fn close_records_closer_auth_requirement() {
        let env = Env::default();
        let (client, _cid, admin) = setup(&env);
        let borrower = Address::generate(&env);
        open_line(&client, &borrower);

        client.close(&borrower, &admin);

        // Verify that the admin address was required to authenticate
        assert!(
            env.auths().iter().any(|(addr, _)| *addr == admin),
            "close_credit_line must require closer authorization"
        );
    }
}
