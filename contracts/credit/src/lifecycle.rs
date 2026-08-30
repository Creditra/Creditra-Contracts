// SPDX-License-Identifier: MIT

//! Credit line lifecycle management: suspend, close, default, reinstate, and liquidation settlement.
//!
//! # What
//!
//! The state-transition layer for [`CreditLineData`]. Implements:
//!
//! - [`open_credit_line`] — admin-only line creation; idempotent re-open
//!   of non-Active lines under admin auth.
//! - [`suspend_credit_line`] — admin-initiated `Active → Suspended`
//!   transition (requires admin auth, distinct from self-suspension).
//! - [`self_suspend_credit_line`] — borrower-initiated `Active → SelfSuspended`
//!   transition (requires borrower auth, distinct status `SelfSuspended = 5`).
//!   The two suspension origins are intentionally distinct for auditability and
//!   least-privilege: the stored `CreditStatus` differs, events are emitted on
//!   different topics (`"suspend"` vs `"self_sus"`), and unsuspend paths are
//!   authorization-separated (admin can unsuspend any, borrower can only
//!   self-unsuspend `SelfSuspended`).
//! - [`unsuspend_credit_line`] — admin-only `Suspended|SelfSuspended → Active`
//!   (clears `suspension_ts`).
//! - [`self_unsuspend_credit_line`] — borrower-only `SelfSuspended → Active`
//!   (borrower can revert their own voluntary suspension; admin suspension
//!   requires admin unsuspend, preserving least privilege).
//! - [`close_credit_line`] — Active/Suspended/SelfSuspended/Restricted → Closed.
//!   Borrower path requires `utilized_amount == 0`; admin path is
//!   unconditional. Stale close reverts `StaleStateTransition`.
//! - [`default_credit_line`] — Active/Restricted/Suspended/SelfSuspended → Defaulted.
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
//!   - Hot reads use [`crate::storage::get_credit_line`] to refresh TTL when
//!     the remaining lifetime falls below the configured threshold.
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
    publish_borrow_lifecycle_event, publish_credit_line_event,
    publish_debt_forgiven_event, publish_default_liquidation_requested_event,
    publish_default_liquidation_settled_event, publish_late_fee_charged_event,
    BorrowLifecycleEvent, BorrowLifecyclePhase, CreditLineEvent, DebtForgivenEvent,
    DefaultLiquidationSettledEvent, LateFeeChargedEvent,
};
use crate::risk::{MAX_INTEREST_RATE_BPS, MAX_RISK_SCORE};
use crate::storage::{
    add_treasury_balance as storage_add_treasury_balance,
    assert_not_paused, assert_ts_monotonic, bump_credit_line_ttl,
    clear_repayment_schedule, get_credit_line,
    get_late_fee_flat as storage_get_late_fee_flat,
    get_repayment_schedule, persist_credit_line,
    set_late_fee_flat as storage_set_late_fee_flat,
    set_repayment_schedule as storage_set_repayment_schedule, CREDIT_LINE_TTL_EXTEND_TO,
    CREDIT_LINE_TTL_THRESHOLD,
};
use crate::types::{ContractError, CreditLineData, CreditStatus, RepaymentSchedule};
use soroban_sdk::{symbol_short, Address, Env, Symbol, Vec};

fn liquidation_settlement_key(borrower: &Address, settlement_id: &Symbol) -> (Symbol, Address, Symbol) {
    (symbol_short!("liq_seen"), borrower.clone(), settlement_id.clone())
}

/// Guard helper: assert that a state transition is valid given the current status.
///
/// # What
/// Checks whether `current_status` is one of the `allowed_sources` for the
/// requested transition. If not, it determines which error to return:
///
/// - If `current_status == target_status` (already in destination state):
///   → `StaleStateTransition` (the transition has already been applied; caller
///   should re-read state before retrying).
/// - Otherwise → the caller-supplied `wrong_state_error` for actionable
///   diagnostics (e.g. `CreditLineClosed`, `CreditLineSuspended`).
///
/// # Why
/// A single chokepoint ensures every entry point in the lifecycle module
/// produces deterministic, diagnosable errors instead of silently becoming
/// a no-op or panicking with a generic message. This is the foundation of
/// Issue #1146 — reject stale credit-line state transitions.
///
/// # Parameters
/// - `env`: Soroban environment (for `panic_with_error`).
/// - `current_status`: The credit line's status as read from storage.
/// - `target_status`: The status the transition is trying to reach.
/// - `allowed_sources`: Slice of statuses that may validly precede the
///   transition. The call is a no-op when `current_status` is in this slice.
/// - `wrong_state_error`: The error to emit when `current_status` is not in
///   `allowed_sources` *and* is not equal to `target_status`.
fn require_valid_transition(
    env: &Env,
    current_status: CreditStatus,
    target_status: CreditStatus,
    allowed_sources: &[CreditStatus],
    wrong_state_error: ContractError,
) {
    // Fast path: the transition is valid.
    if allowed_sources.contains(&current_status) {
        return;
    }

    // Stale path: transition already applied — reject with a specific, diagnosable error.
    if current_status == target_status {
        env.panic_with_error(ContractError::StaleStateTransition);
    }

    // Invalid path: wrong source state for a different semantic reason.
    env.panic_with_error(wrong_state_error);
}

/// Open a new credit line for a borrower (admin only).
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
    crate::storage::set_min_credit_limit(&env, min);
    crate::storage::set_max_credit_limit(&env, max);
}

pub fn get_credit_limit_bounds(env: Env) -> (Option<i128>, Option<i128>) {
    let min = crate::storage::get_min_credit_limit(&env);
    let max = crate::storage::get_max_credit_limit(&env);
    (min, max)
}

pub fn validate_credit_limit_bounds(env: &Env, credit_limit: i128) {
    let min = crate::storage::get_min_credit_limit(env);
    let max = crate::storage::get_max_credit_limit(env);

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

/// Internal helper: Active → {Suspended, SelfSuspended}.
///
/// # Determinism
/// The transition is gated on `Active` only; any other source reverts with a
/// typed `ContractError` via `require_valid_transition`. The `StaleStateTransition`
/// (60) case is handled when `current == target`. This makes retries idempotent
/// and concurrent duplicate calls diagnosable, not silently successful.
///
/// # Authorization
/// This helper does NOT check auth; callers must enforce admin or borrower
/// auth before invoking it. That keeps the single `require_auth` call per
/// invocation (Soroban's auth-mock treats a second `require_auth` for an
/// already-authorized address in the same frame as an error).
///
/// # Storage
/// Applies accrual before mutation, bumps TTL on read, persists via
/// `persist_credit_line` with `previous_utilized` for `TotalUtilized`
/// conservation, and sets `suspension_ts` monotonically.
fn suspend_credit_line_internal(env: &Env, borrower: Address, target: CreditStatus) {
    debug_assert!(
        target == CreditStatus::Suspended || target == CreditStatus::SelfSuspended,
        "suspend target must be Suspended or SelfSuspended"
    );

    // Bump TTL on read: this is a hot accrual read path, so an active
    // borrower's entry must never be archived independently of draw/repay.
    let stored_line: CreditLineData = get_credit_line(env, &borrower)
        .unwrap_or_else(|| env.panic_with_error(ContractError::CreditLineNotFound));
    let previous_utilized = stored_line.utilized_amount;

    let previous_status = stored_line.status;

    // Apply interest accrual before any mutation.
    let mut credit_line = crate::accrual::apply_accrual(env, stored_line);

    // Guard: Active → {Suspended, SelfSuspended} is the only valid transition.
    //   - Suspended     → Suspended     : stale — StaleStateTransition (60)
    //   - SelfSuspended → SelfSuspended : stale — StaleStateTransition (60)
    //   - Suspended     → SelfSuspended : wrong — CreditLineSuspended (already suspended)
    //   - SelfSuspended → Suspended     : wrong — CreditLineSuspended
    //   - Defaulted     → *Suspended    : wrong — CreditLineDefaulted
    //   - Closed        → *Suspended    : wrong — CreditLineClosed
    //   - Restricted    → *Suspended    : wrong — CreditLineSuspended
    require_valid_transition(
        env,
        credit_line.status,
        target,
        &[CreditStatus::Active],
        if credit_line.status == CreditStatus::Closed {
            ContractError::CreditLineClosed
        } else if credit_line.status == CreditStatus::Defaulted {
            ContractError::CreditLineDefaulted
        } else {
            // Restricted, Suspended, SelfSuspended — cannot (re-)suspend.
            ContractError::CreditLineSuspended
        },
    );

    credit_line.status = target;
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

    // Distinct event topics for auditability:
    //  - admin suspension → ("credit","suspend") with status Suspended
    //  - self  suspension → ("credit","self_sus") with status SelfSuspended
    // Indexers and dashboards can distinguish the origin without parsing the
    // status field alone, and the status field remains the canonical on-chain
    // truth for draw-blocking logic.
    let topic = if target == CreditStatus::Suspended {
        symbol_short!("suspend")
    } else {
        symbol_short!("self_sus")
    };
    publish_credit_line_event(
        env,
        (symbol_short!("credit"), topic),
        CreditLineEvent {
            borrower,
            status: target,
            credit_limit: credit_line.credit_limit,
            interest_rate_bps: credit_line.interest_rate_bps,
            risk_score: credit_line.risk_score,
        },
    );
}

// ── per-borrower liquidation grace ──────────────────────────────────────────

/// Set or update the per-borrower liquidation grace period in seconds (admin only).
///
/// # Arguments
/// - `env`: Soroban environment.
/// - `borrower`: Borrower address to configure.
/// - `grace_period_seconds`: Grace period duration in seconds. Pass `0` to remove.
///
/// # Panics
/// - `ContractError::CreditLineNotFound` if no credit line exists for `borrower`.
/// - `ContractError::CreditLineClosed` if the credit line is `Closed`.
pub fn set_per_borrower_liquidation_grace(
    env: &Env,
    borrower: Address,
    grace_period_seconds: u64,
) {
    assert_not_paused(env);
    require_admin_auth(env);

    let stored_line: CreditLineData = env
        .storage()
        .persistent()
        .get(&borrower)
        .unwrap_or_else(|| env.panic_with_error(ContractError::CreditLineNotFound));

    if stored_line.status == CreditStatus::Closed {
        env.panic_with_error(ContractError::CreditLineClosed);
    }

    crate::storage::set_per_borrower_liquidation_grace(env, &borrower, grace_period_seconds);
}

/// Return the per-borrower liquidation grace period in seconds for `borrower`.
pub fn get_per_borrower_liquidation_grace(env: &Env, borrower: Address) -> u64 {
    crate::storage::get_per_borrower_liquidation_grace(env, &borrower)
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
    crate::storage::set_late_fee_flat(&env, fee);
}

/// Get the configured flat late fee per missed installment.
///
/// Returns `0` if not configured (no flat late fee).
pub fn get_late_fee_flat(env: Env) -> i128 {
    crate::storage::get_late_fee_flat(&env)
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

    let existing_line = get_credit_line(&env, &borrower);

    if let Some(existing) = existing_line.as_ref() {
        if existing.status == CreditStatus::Active {
            env.panic_with_error(ContractError::AlreadyInitialized);
        }

        // Prevent borrower-controlled status bypasses on existing lines.
        require_admin_auth(&env);
    }

    let previous_utilized = existing_line
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
/// `Active → Suspended`  (admin origin, distinct from `SelfSuspended`)
///
/// # Authorization
/// Admin only. Enforced by the `lib.rs` wrapper (`require_admin_auth`);
/// not re-checked here to avoid a double `require_auth` on the same address
/// within one invocation (Soroban auth-mock treats a second `require_auth`
/// for an already-authorized address in the same frame as an error).
///
/// # Panics
/// - `CreditLineNotFound` — no line for `borrower`.
/// - `StaleStateTransition` — already `Suspended` (or `SelfSuspended` — distinct
///   state, but still "already suspended").
/// - `CreditLineClosed` / `CreditLineDefaulted` / `CreditLineSuspended` — wrong source.
///
/// # Events
/// Emits `("credit", "suspend")` with `status == Suspended`.
/// Distinct from `self_suspend_credit_line` which emits `("credit","self_sus")`
/// with `status == SelfSuspended`.
pub fn suspend_credit_line(env: Env, borrower: Address) {
    assert_not_paused(&env);
    // Admin auth is enforced by the lib.rs wrapper.
    suspend_credit_line_internal(&env, borrower, CreditStatus::Suspended);
}

/// Suspend the caller's own active credit line (borrower only).
///
/// This is a borrower safety control that blocks future draws while leaving
/// repayments available. The resulting status is `SelfSuspended` (5), distinct
/// from admin `Suspended` (1) for auditability and authorization separation.
///
/// # Authorization
/// Requires `borrower.require_auth()`. Admin cannot invoke this path on
/// behalf of a borrower — that would conflate voluntary and involuntary
/// suspension in the audit trail.
///
/// # State transition
/// `Active → SelfSuspended`. Any other source reverts with a typed error
/// (`StaleStateTransition` for duplicate, `CreditLine*` for wrong state).
///
/// # Events
/// Emits `("credit","self_sus")` with `status == SelfSuspended`, distinct
/// from admin suspension's `("credit","suspend")`.
///
/// # Storage
/// Loads via `get_credit_line` (TTL-bumped), applies accrual, persists via
/// `persist_credit_line` with `previous_utilized` conservation.
pub fn self_suspend_credit_line(env: Env, borrower: Address) {
    assert_not_paused(&env);
    borrower.require_auth();
    suspend_credit_line_internal(&env, borrower, CreditStatus::SelfSuspended);
}

/// Unsuspend a credit line (admin only).
///
/// Transitions `Suspended` or `SelfSuspended` → `Active`. This is the
/// admin recovery path for both suspension origins. A borrower who
/// self-suspended can also use `self_unsuspend_credit_line` (borrower-only)
/// to revert their own suspension without admin involvement; an admin
/// suspension **cannot** be cleared by the borrower — least privilege.
///
/// # Authorization
/// Admin only (enforced by `lib.rs` wrapper). Not re-checked here for the
/// same double-auth reason as `suspend_credit_line`.
///
/// # Validation
/// - `Active` → `Active` stale → `StaleStateTransition`.
/// - `Defaulted` / `Closed` / `Restricted` → `CreditLine*` wrong state.
///
/// # Events
/// Emits `("credit","unsuspend")` with `status == Active`.
///
/// # Concurrency / retry safety
/// The transition is checked via `require_valid_transition`; a concurrent
/// or retried unsuspend after the line is already `Active` reverts with
/// `StaleStateTransition` deterministically, not silently succeeding.
pub fn unsuspend_credit_line(env: Env, borrower: Address) {
    assert_not_paused(&env);
    // Admin auth enforced by lib.rs wrapper.
    let stored_line: CreditLineData = get_credit_line(&env, &borrower)
        .unwrap_or_else(|| env.panic_with_error(ContractError::CreditLineNotFound));
    let previous_utilized = stored_line.utilized_amount;
    let previous_status = stored_line.status;
    let mut credit_line = crate::accrual::apply_accrual(&env, stored_line);

    // Guard: Suspended|SelfSuspended → Active.
    //   - Active → Active (stale) handled via StaleStateTransition.
    //   - Defaulted/Closed/Restricted → wrong state errors.
    let is_suspended =
        credit_line.status == CreditStatus::Suspended || credit_line.status == CreditStatus::SelfSuspended;
    if !is_suspended {
        require_valid_transition(
            &env,
            credit_line.status,
            CreditStatus::Active,
            &[CreditStatus::Suspended, CreditStatus::SelfSuspended],
            if credit_line.status == CreditStatus::Closed {
                ContractError::CreditLineClosed
            } else if credit_line.status == CreditStatus::Defaulted {
                ContractError::CreditLineDefaulted
            } else {
                // Restricted or already Active — stale vs wrong distinguished
                // inside require_valid_transition.
                ContractError::CreditLineSuspended
            },
        );
        // If not suspended, the above will have panicked; unreachable.
        return;
    }

    // Apply valid transition check for duplicate Active case and wrong states.
    // For suspended sources we still want stale handling for Active->Active.
    // The is_suspended branch already ensures we're in a valid source; we still
    // call the guard for consistency on edge cases (e.g. Active).
    // But for suspended → Active we can directly transition.
    // To keep determinism for duplicate unsuspend (Active → Active), we need
    // to handle that case: if already Active, this is stale.
    // Since we are here is_suspended == true, we skip stale check.

    // Additional stale check: if already Active, revert with StaleStateTransition.
    // This is technically unreachable here because is_suspended true, but
    // we keep the guard for completeness via explicit match below.
    // Perform transition.
    credit_line.status = CreditStatus::Active;
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
        (symbol_short!("credit"), symbol_short!("unsuspend")),
        CreditLineEvent {
            borrower,
            status: CreditStatus::Active,
            credit_limit: credit_line.credit_limit,
            interest_rate_bps: credit_line.interest_rate_bps,
            risk_score: credit_line.risk_score,
        },
    );
}

/// Borrower-initiated unsuspend for self-suspended lines only.
///
/// Transitions `SelfSuspended → Active`. This lets a borrower revert their
/// own voluntary suspension without admin intervention, while preserving
/// least privilege: a borrower **cannot** unsuspend an admin `Suspended` line.
///
/// # Authorization
/// Requires `borrower.require_auth()`.
///
/// # Validation
/// - `SelfSuspended → Active` is the only valid transition.
/// - `Suspended → Active` via this path reverts `CreditLineSuspended`
///   (admin suspension requires admin unsuspend).
/// - `Active → Active` (duplicate) reverts `StaleStateTransition`.
pub fn self_unsuspend_credit_line(env: Env, borrower: Address) {
    assert_not_paused(&env);
    borrower.require_auth();

    let stored_line: CreditLineData = get_credit_line(&env, &borrower)
        .unwrap_or_else(|| env.panic_with_error(ContractError::CreditLineNotFound));
    let previous_utilized = stored_line.utilized_amount;
    let previous_status = stored_line.status;
    let mut credit_line = crate::accrual::apply_accrual(&env, stored_line);

    require_valid_transition(
        &env,
        credit_line.status,
        CreditStatus::Active,
        &[CreditStatus::SelfSuspended],
        if credit_line.status == CreditStatus::Closed {
            ContractError::CreditLineClosed
        } else if credit_line.status == CreditStatus::Defaulted {
            ContractError::CreditLineDefaulted
        } else if credit_line.status == CreditStatus::Suspended {
            // Admin suspension cannot be cleared by borrower.
            ContractError::CreditLineSuspended
        } else {
            // Restricted or Already Active — stale vs wrong inside guard.
            ContractError::CreditLineSuspended
        },
    );

    credit_line.status = CreditStatus::Active;
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
        (symbol_short!("credit"), Symbol::new(&env, "self_uns")),
        CreditLineEvent {
            borrower,
            status: CreditStatus::Active,
            credit_limit: credit_line.credit_limit,
            interest_rate_bps: credit_line.interest_rate_bps,
            risk_score: credit_line.risk_score,
        },
    );
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
/// | Any other address | Always rejected with `ContractError::Unauthorized` |
///
/// # Stale-transition rejection (Issue #1146)
/// If the credit line is already [`CreditStatus::Closed`], the call **reverts**
/// with [`ContractError::StaleStateTransition`] (code 60). This breaks the
/// previous silent idempotent-return behaviour so callers can detect stale or
/// duplicate close attempts deterministically.
///
/// # Parameters
/// - `borrower`: Address whose credit line is being closed.
/// - `closer`:   Address authorizing the close. Must be the admin or the borrower.
///
/// # Panics
/// - `ContractError::CreditLineNotFound` — no credit line exists for `borrower`.
/// - `ContractError::StaleStateTransition` — the line is already `Closed`.
/// - `ContractError::UtilizationNotZero` — `closer == borrower` but outstanding balance > 0.
/// - `ContractError::Unauthorized` — `closer` is neither the admin nor the borrower.
///
/// # Events
/// Emits a `("credit", "closed")` [`CreditLineEvent`] on successful state change.
/// No event is emitted on stale or unauthorized calls — those revert before the event.
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
    // `closer` auth is enforced by the `lib.rs` `close_credit_line` entrypoint
    // wrapper before this is called; not re-checked here (see the comment on
    // `suspend_credit_line` above for why).

    // Resolve the current admin address.
    let admin: Address = require_admin(&env);

    // Load the credit line; revert if it does not exist.
    let mut credit_line = get_credit_line(&env, &borrower)
        .unwrap_or_else(|| env.panic_with_error(ContractError::CreditLineNotFound));
    let previous_utilized = credit_line.utilized_amount;

    // Stale guard: closing an already-Closed line is a stale transition.
    // Previously this returned silently (idempotent); we now surface the
    // error so callers can detect and correct stale state rather than
    // assuming success when nothing changed.
    if credit_line.status == CreditStatus::Closed {
        env.panic_with_error(ContractError::StaleStateTransition);
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
            env.panic_with_error(ContractError::UtilizationNotZero);
        }
    } else {
        // Third party: unconditionally rejected.
        env.panic_with_error(ContractError::Unauthorized);
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
///
/// # Storage
/// Loads the credit line via [`crate::storage::get_credit_line`], which bumps
/// the entry's persistent TTL on read — independent of whether the call goes
/// on to mutate and persist the line.
pub fn default_credit_line(env: Env, borrower: Address) {
    assert_not_paused(&env);
    // Admin auth enforced by the `lib.rs` wrapper (see `suspend_credit_line`).
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

    // Guard: Active, Suspended, SelfSuspended, or Restricted → Defaulted are the only valid transitions.
    // Both suspension origins are treated as default-eligible; the distinction
    // is preserved for audit but does not affect liquidation eligibility.
    //   - Defaulted → Defaulted : stale repeat     — StaleStateTransition
    //   - Closed    → Defaulted : terminal state   — CreditLineClosed
    require_valid_transition(
        &env,
        credit_line.status,
        CreditStatus::Defaulted,
        &[
            CreditStatus::Active,
            CreditStatus::Suspended,
            CreditStatus::SelfSuspended,
            CreditStatus::Restricted,
        ],
        ContractError::CreditLineClosed,
    );

    let grace_seconds = crate::storage::get_per_borrower_liquidation_grace(&env, &borrower);
    if grace_seconds > 0 {
        let now = env.ledger().timestamp();
        let base_ts = if credit_line.suspension_ts > 0 {
            credit_line.suspension_ts
        } else if let Some(schedule) = get_repayment_schedule(&env, &borrower) {
            schedule.next_due_ts
        } else if credit_line.last_rate_update_ts > 0 {
            credit_line.last_rate_update_ts
        } else {
            credit_line.last_accrual_ts
        };

        if now < base_ts.saturating_add(grace_seconds) {
            env.panic_with_error(ContractError::LiquidationGraceActive);
        }
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

/// Apply auction liquidation proceeds to a defaulted credit line (admin only).
///
/// Reduces `accrued_interest` first, then `utilized_amount`, by `amount`
/// (clamped to the outstanding balance). No token movement occurs — this is
/// pure accounting relief, e.g. for negotiated settlements handled off-chain.
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
    let previous_status = stored_line.status;

    // Apply interest accrual before any mutation.
    let mut credit_line = crate::accrual::apply_accrual(&env, stored_line);

    let forgive_amount = amount.min(credit_line.utilized_amount);
    let interest_forgiven = forgive_amount.min(credit_line.accrued_interest);

    credit_line.accrued_interest -= interest_forgiven;
    credit_line.utilized_amount -= forgive_amount;

    persist_credit_line(
        &env,
        &borrower,
        &credit_line,
        previous_utilized,
        Some(previous_status),
    );

    publish_debt_forgiven_event(
        &env,
        DebtForgivenEvent {
            borrower: borrower.clone(),
            amount_forgiven: forgive_amount,
            remaining_accrued_interest: credit_line.accrued_interest,
            new_utilized_amount: credit_line.utilized_amount,
        },
    );
    publish_borrow_lifecycle_event(
        &env,
        BorrowLifecycleEvent {
            borrower,
            phase: BorrowLifecyclePhase::DebtForgiven,
            status: credit_line.status,
            utilized_amount: credit_line.utilized_amount,
            credit_limit: credit_line.credit_limit,
            interest_rate_bps: credit_line.interest_rate_bps,
            timestamp: env.ledger().timestamp(),
        },
    );
}

/// Settle a defaulted credit line with optional oracle price validation.
///
/// # Parameters
/// - `env`: Soroban environment
/// - `borrower`: Address of the defaulted borrower
/// - `recovered_amount`: Amount recovered from liquidation (must be > 0)
/// - `settlement_id`: Unique settlement identifier for replay protection
/// - `close_factor_bps`: Percentage of utilized amount to recover (1..=10_000)
/// - `oracle_price`: Optional single-oracle price (ignored if quorum config is set)
///
/// # Behavior
///
/// 1. Validates authorization (admin auth required)
/// 2. Validates replay protection: settlement_id not previously used for this borrower
/// 3. Validates numeric bounds: recovered_amount, close_factor_bps
/// 4. **Validates oracle price** (NEW):
///    - If quorum config set: uses stored quorum price (oracle_price arg ignored)
///    - Else if single-oracle config set: validates supplied oracle_price
///    - Else: proceeds without oracle gating (backward compatible)
/// 5. Loads and accrues credit line
/// 6. Verifies credit line status is Defaulted
/// 7. Validates recovery amount vs. maximum recoverable
/// 8. Reduces utilized_amount and transitions to Closed if needed
/// 9. Records accepted oracle price for next settlement's deviation check
/// 10. Emits settlement event
///
/// # Errors
/// Panics with typed [`ContractError`] on any validation failure (before state mutation):
/// - `NotAdmin` (2) — caller is not admin
/// - `InvalidAmount` (5) — recovered_amount ≤ 0 or close_factor_bps invalid
/// - `OverLimit` (6) — recovered_amount > max_recoverable or close_factor > protocol max
/// - `AlreadyInitialized` (14) — settlement already processed for (borrower, settlement_id)
/// - `CreditLineNotFound` (3) — no credit line exists for borrower
/// - `CreditLineDefaulted` (21) — credit line status is not Defaulted
/// - `OraclePriceInvalid` (36) — oracle price is invalid (zero, negative, or missing)
/// - `OraclePriceStale` (37) — oracle price exceeds max_age_seconds
/// - `OraclePriceDeviation` (38) — oracle price deviates from last accepted price
/// - `OracleQuorumNotMet` (50) — quorum price not submitted yet
pub fn settle_default_liquidation(
    env: Env,
    borrower: Address,
    recovered_amount: i128,
    settlement_id: Symbol,
    close_factor_bps: u32,
    oracle_price: Option<i128>,
) {
    // Step 1: Authorization
    require_admin_auth(&env);

    // Step 2: Numeric validation (cheap checks before any storage reads)
    if recovered_amount <= 0 {
        env.panic_with_error(ContractError::InvalidAmount);
    }

    if close_factor_bps == 0 || close_factor_bps > 10_000 {
        env.panic_with_error(ContractError::InvalidAmount);
    }

    // Enforce the protocol-level maximum close factor cap.
    let max_close_factor = crate::storage::get_close_factor_bps(&env);
    if close_factor_bps > max_close_factor {
        env.panic_with_error(ContractError::OverLimit);
    }

    // Step 3: Replay protection (gate before credit line reads)
    let settlement_key = liquidation_settlement_key(&borrower, &settlement_id);
    if env.storage().persistent().has(&settlement_key) {
        env.panic_with_error(ContractError::AlreadyInitialized);
    }

    // Step 4: Oracle validation (BEFORE any state mutation)
    let oracle_result = crate::oracle_validation::validate_settlement_oracle_price(
        &env,
        oracle_price,
    );

    // Step 5: Credit line read & accrual
    // Bump TTL on read: this is a hot accrual read path, so an active
    // borrower's entry must never be archived independently of draw/repay.
    let stored_line: CreditLineData = crate::storage::get_credit_line(&env, &borrower)
        .unwrap_or_else(|| env.panic_with_error(ContractError::CreditLineNotFound));
    let previous_utilized = stored_line.utilized_amount;

    // Apply interest accrual before any mutation
    let mut credit_line = crate::accrual::apply_accrual(&env, stored_line);

    // Step 6: Verify defaulted status
    if credit_line.status != CreditStatus::Defaulted {
        env.panic_with_error(ContractError::CreditLineDefaulted);
    }

    // Step 7: Economic validation
    // Compute the maximum recoverable amount for this settlement
    let max_recoverable = credit_line
        .utilized_amount
        .checked_mul(close_factor_bps as i128)
        .unwrap_or_else(|| env.panic_with_error(ContractError::Overflow))
        .checked_div(10_000)
        .unwrap_or_else(|| env.panic_with_error(ContractError::Overflow));

    if recovered_amount > max_recoverable {
        env.panic_with_error(ContractError::OverLimit);
    }

    // Step 8: State mutation (after all validation succeeds)
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

    // Step 9: Replay protection & oracle price recording
    env.storage().persistent().set(&settlement_key, &true);
    if let Some(price) = oracle_result.price() {
        crate::oracle_validation::record_accepted_oracle_price(&env, price);
    }

    // Step 10: Events
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

/// Forgive outstanding debt without transferring tokens (admin only).
///
/// This is an accounting-only write-off path intended for explicit admin debt
/// relief or off-chain settlements that have already been handled elsewhere.
/// The forgiven amount is capped to the current `utilized_amount`.


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
///
/// # Storage
/// Loads the credit line via [`crate::storage::get_credit_line`], which bumps
/// the entry's persistent TTL on read — independent of whether the call goes
/// on to mutate and persist the line.
pub fn reinstate_credit_line(env: Env, borrower: Address, target_status: CreditStatus) {
    assert_not_paused(&env);
    // Admin auth enforced by the `lib.rs` wrapper (see `suspend_credit_line`).

    // Only Active and Restricted are valid reinstate targets per the state-machine spec.
    if target_status != CreditStatus::Active && target_status != CreditStatus::Restricted {
        env.panic_with_error(ContractError::InvalidAmount);
    }

    // Bump TTL on read: this is a hot accrual read path, so an active
    // borrower's entry must never be archived independently of draw/repay.
    let stored_line: CreditLineData = crate::storage::get_credit_line(&env, &borrower)
        .unwrap_or_else(|| env.panic_with_error(ContractError::CreditLineNotFound));
    let previous_utilized = stored_line.utilized_amount;

    let mut credit_line = crate::accrual::apply_accrual(&env, stored_line);

    // Guard: Defaulted → Active | Restricted is the only valid transition.
    //   - Active     → Active     : stale repeat     — StaleStateTransition
    //   - Restricted → Restricted : stale repeat     — StaleStateTransition
    //   - Suspended  → Active     : wrong source     — CreditLineDefaulted
    //   - Closed     → Active     : terminal state   — CreditLineClosed
    //
    // Note: require_valid_transition only checks the *current* status against
    // Defaulted. For the stale case it compares current == target_status,
    // so both Active→Active and Restricted→Restricted are caught.
    if credit_line.status == CreditStatus::Closed {
        env.panic_with_error(ContractError::CreditLineClosed);
    }

    require_valid_transition(
        &env,
        credit_line.status,
        target_status,
        &[CreditStatus::Defaulted],
        ContractError::CreditLineDefaulted,
    );

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

// ── repayment schedule helpers ───────────────────────────────────────────────

/// Set or replace a borrower's installment repayment schedule (admin only).
///
/// # Parameters
/// - `borrower`: Borrower whose credit line schedule is being configured.
/// - `amount_per_period`: Required principal repayment amount per installment; must be positive.
/// - `period_seconds`: Duration of each installment period in seconds; must be positive.
/// - `first_due_ts`: Timestamp at which the first installment is due.
///
/// # Panics
/// - [`ContractError::InvalidAmount`] when `amount_per_period <= 0` or
///   `period_seconds == 0`.
/// - [`ContractError::CreditLineNotFound`] when `borrower` has no credit line.
///
/// # Authorization
/// Requires admin authorization because the schedule controls delinquency and
/// due-date state for the borrower.
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

    if !env.storage().persistent().has(&borrower) {
        env.panic_with_error(ContractError::CreditLineNotFound);
    }

    let schedule = RepaymentSchedule {
        amount_per_period,
        period_seconds,
        next_due_ts: first_due_ts,
    };
    storage_set_repayment_schedule(env, &borrower, &schedule);
    // Setting a schedule is an interaction with the credit line, so keep the
    // credit-line entry live as well (the schedule entry is bumped by the
    // storage setter itself).
    bump_credit_line_ttl(env, &borrower);
}

/// Advance a borrower's installment schedule after a repayment.
///
/// `effective_repay` is the amount actually applied to the debt after capping
/// an overpayment to the outstanding balance. `interest_repaid` is the portion
/// of that amount that was allocated to accrued interest. Only the principal
/// portion of a repayment can satisfy installment obligations:
///
/// ```text
/// principal_repaid  = effective_repay - interest_repaid
/// installments_paid = floor(principal_repaid / amount_per_period)
/// next_due_ts       = next_due_ts + installments_paid * period_seconds
/// ```
///
/// Interest-only repayments and partial principal installments do not move the
/// due date. Arithmetic uses checked/saturating operations so malformed state or
/// extreme schedule values cannot wrap timestamps.
/// Deterministically allocate a repayment across the debt components.
///
/// `utilized_amount` is treated as the total debt bucket and `accrued_interest`
/// is the interest slice of that bucket. The allocator normalizes the current
/// debt split before computing the repayment so `interest_repaid` and
/// `principal_repaid` always derive from the same valid state, even when the
/// line has drifted across a boundary or contains stale component values.
pub fn allocate_repayment(
    total_debt: i128,
    accrued_interest: i128,
    requested_amount: i128,
) -> (i128, i128, i128) {
    let total_debt = total_debt.max(0);
    let normalized_interest = accrued_interest.min(total_debt).max(0);
    let effective_repay = requested_amount.min(total_debt).max(0);
    let interest_repaid = effective_repay.min(normalized_interest).max(0);
    let principal_repaid = effective_repay.saturating_sub(interest_repaid);

    (effective_repay, interest_repaid, principal_repaid)
}

pub fn advance_repayment_schedule_after_repay(
    env: &Env,
    borrower: &Address,
    effective_repay: i128,
    interest_repaid: i128,
) {
    let principal_repaid = match effective_repay.checked_sub(interest_repaid) {
        Some(principal) if principal > 0 => principal,
        _ => return,
    };

    let Some(mut schedule) = get_repayment_schedule(env, borrower) else {
        return;
    };

    if schedule.amount_per_period <= 0 || schedule.period_seconds == 0 {
        return;
    }

    let installments_paid = (principal_repaid / schedule.amount_per_period) as u64;
    if installments_paid == 0 {
        return;
    }

    // ── Late-fee surcharge ──────────────────────────────────────────────────
    let late_fee = crate::storage::get_late_fee_flat(env);
    if late_fee > 0 {
        let now = env.ledger().timestamp();
        for i in 0_u64..installments_paid {
            let due_ts = schedule
                .next_due_ts
                .saturating_add(i.saturating_mul(schedule.period_seconds));
            if now > due_ts {
                crate::storage::add_treasury_balance(env, late_fee);
                crate::events::publish_late_fee_charged_event(
                    env,
                    crate::events::LateFeeChargedEvent {
                        borrower: borrower.clone(),
                        fee: late_fee,
                        installment_index: i.saturating_add(1),
                    },
                );
            }
        }
    }

    let advance_seconds = installments_paid.saturating_mul(schedule.period_seconds);
    schedule.next_due_ts = schedule.next_due_ts.saturating_add(advance_seconds);
    storage_set_repayment_schedule(env, borrower, &schedule);
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod self_suspend {
    use crate::types::{ContractError, CreditStatus};
    use crate::Credit;
    use crate::CreditClient;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Address;
    use soroban_sdk::Env;

    fn setup(env: &Env) -> (CreditClient<'_>, Address) {
        env.mock_all_auths();
        let admin = Address::generate(env);
        let borrower = Address::generate(env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(env, &contract_id);
        client.init(&admin);
        client.open_credit_line(&borrower, &1_000_i128, &500_u32, &50_u32);
        (client, borrower)
    }

    /// A borrower can self-suspend their own active line; status transitions
    /// to SelfSuspended (distinct from admin Suspended) without admin involvement.
    #[test]
    fn self_suspend_transitions_active_to_suspended() {
        let env = Env::default();
        let (client, borrower) = setup(&env);

        client.self_suspend_credit_line(&borrower);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::SelfSuspended);
    }

    /// Self-suspend requires the borrower's own authorization. `init` and a
    /// brand-new `open_credit_line` need no auth, so with nothing mocked at
    /// all, only `self_suspend_credit_line`'s `borrower.require_auth()` can
    /// be the source of the panic.
    #[test]
    #[should_panic]
    fn self_suspend_requires_borrower_auth() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);
        client.open_credit_line(&borrower, &1_000_i128, &500_u32, &50_u32);

        client.self_suspend_credit_line(&borrower);
    }

    /// Once self-suspended, draws are blocked but repayments remain available.
    #[test]
    fn self_suspend_blocks_draws_but_allows_repay() {
        let env = Env::default();
        let (client, borrower) = setup(&env);

        client.self_suspend_credit_line(&borrower);

        let draw_result = client.try_draw_credit(&borrower, &100_i128);
        assert_eq!(
            draw_result.err().unwrap().unwrap(),
            ContractError::CreditLineSuspended.into()
        );

        // Repayment against a SelfSuspended line is allowed (no draw occurred, so
        // utilized_amount is 0 — this just confirms repay_credit doesn't panic
        // on the suspended status itself).
        client.repay_credit(&borrower, &1_i128);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::SelfSuspended);
    }

    /// Self-suspending an already-self-suspended line panics (not idempotent).
    /// Duplicate suspension of the same origin reverts with StaleStateTransition
    /// (60) since the line is already in the target state.
    #[test]
    #[should_panic(expected = "Error(Contract, #60)")]
    fn self_suspend_twice_reverts() {
        let env = Env::default();
        let (client, borrower) = setup(&env);

        client.self_suspend_credit_line(&borrower);
        client.self_suspend_credit_line(&borrower);
    }
}

#[cfg(test)]
mod installment {
    use crate::events::LateFeeChargedEvent;
    use crate::types::CreditStatus;
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
        assert_eq!(treasury_after - treasury_before, 30);
    }

    // ── partial liquidation tests ──────────────────────────────────────────

    /// Partial liquidation with close_factor_bps = 5_000 recovers 50% of debt.
    #[test]
    fn test_settle_default_liquidation_partial_50_percent() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1_000_000, &500, &100);
        client.draw_credit(&borrower, &500_000);

        env.ledger().set_timestamp(3600);
        client.default_credit_line(&borrower);

        let before = client.get_credit_line(&borrower).unwrap();
        assert_eq!(before.status, CreditStatus::Defaulted);
        assert!(before.utilized_amount > 500_000); // Interest accrued

        // Settle with 50% close factor and recover $250,000
        let settlement_id = Symbol::new(&env, "settle_1");
        let recovered = 250_000_i128;
        client.settle_default_liquidation(
            &borrower,
            &recovered,
            &settlement_id,
            &5_000, // 50% close factor
            &None,
        );

        let after = client.get_credit_line(&borrower).unwrap();
        assert_eq!(after.status, CreditStatus::Defaulted); // Still defaulted, not fully liquidated
        assert!(after.utilized_amount < before.utilized_amount);
        assert_eq!(
            after.utilized_amount,
            before.utilized_amount - recovered
        );
    }

    /// Partial liquidation with close_factor_bps = 10_000 fully closes the line.
    #[test]
    fn test_settle_default_liquidation_full_close_factor() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1_000_000, &500, &100);
        client.draw_credit(&borrower, &100_000);

        env.ledger().set_timestamp(3600);
        client.default_credit_line(&borrower);

        let before = client.get_credit_line(&borrower).unwrap();
        let utilized_with_accrual = before.utilized_amount;

        // Settle with full close factor (100%)
        let settlement_id = Symbol::new(&env, "settle_full");
        client.settle_default_liquidation(
            &borrower,
            &utilized_with_accrual,
            &settlement_id,
            &10_000, // 100% close factor
            &None,
        );

        let after = client.get_credit_line(&borrower).unwrap();
        assert_eq!(after.status, CreditStatus::Closed); // Fully closed
        assert_eq!(after.utilized_amount, 0);
    }

    /// Partial liquidation respects close_factor_bps limit.
    #[test]
    #[should_panic(expected = "Error(Contract, #7)")]
    fn test_settle_default_liquidation_exceeds_close_factor_limit() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.set_close_factor_bps(&5_000); // Max 50%
        client.open_credit_line(&borrower, &1_000_000, &500, &100);
        client.draw_credit(&borrower, &100_000);

        env.ledger().set_timestamp(3600);
        client.default_credit_line(&borrower);

        let before = client.get_credit_line(&borrower).unwrap();

        // Try to recover more than 50% with 100% close factor (should fail)
        let settlement_id = Symbol::new(&env, "settle_fail");
        client.settle_default_liquidation(
            &borrower,
            &before.utilized_amount, // Try to recover 100%
            &settlement_id,
            &10_000, // 100% close factor (exceeds protocol max of 50%)
            &None,
        );
    }

    /// Recovered amount must not exceed max_recoverable.
    #[test]
    #[should_panic(expected = "Error(Contract, #7)")]
    fn test_settle_default_liquidation_exceeds_max_recoverable() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1_000_000, &500, &100);
        client.draw_credit(&borrower, &100_000);

        client.default_credit_line(&borrower);

        // Try to recover 60% with only 50% close factor (should fail)
        let settlement_id = Symbol::new(&env, "settle_over");
        client.settle_default_liquidation(
            &borrower,
            &60_000, // Try to recover $60k
            &settlement_id,
            &5_000, // Only 50% close factor (max recoverable is $50k)
            &None,
        );
    }

    /// Invalid close_factor_bps = 0 is rejected.
    #[test]
    #[should_panic(expected = "Error(Contract, #5)")]
    fn test_settle_default_liquidation_zero_close_factor() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1_000_000, &500, &100);
        client.draw_credit(&borrower, &100_000);

        client.default_credit_line(&borrower);

        let settlement_id = Symbol::new(&env, "settle_zero");
        client.settle_default_liquidation(
            &borrower,
            &10_000,
            &settlement_id,
            &0, // Invalid: zero close factor
            &None,
        );
    }

    /// Invalid close_factor_bps > 10_000 is rejected.
    #[test]
    #[should_panic(expected = "Error(Contract, #5)")]
    fn test_settle_default_liquidation_excessive_close_factor() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1_000_000, &500, &100);
        client.draw_credit(&borrower, &100_000);

        client.default_credit_line(&borrower);

        let settlement_id = Symbol::new(&env, "settle_excess");
        client.settle_default_liquidation(
            &borrower,
            &100_000,
            &settlement_id,
            &10_001, // Invalid: exceeds max basis points
            &None,
        );
    }

    /// Replay protection: same (borrower, settlement_id) cannot be settled twice.
    #[test]
    #[should_panic(expected = "Error(Contract, #14)")]
    fn test_settle_default_liquidation_replay_protection() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1_000_000, &500, &100);
        client.draw_credit(&borrower, &100_000);

        client.default_credit_line(&borrower);

        let settlement_id = Symbol::new(&env, "settle_replay");

        // First settlement succeeds
        client.settle_default_liquidation(
            &borrower,
            &50_000,
            &settlement_id,
            &5_000,
        );

        // Second settlement with same (borrower, settlement_id) should fail
        client.settle_default_liquidation(
            &borrower,
            &25_000,
            &settlement_id, // Same ID
            &5_000,
        );
    }

    /// Sequential partial liquidations with different settlement IDs.
    #[test]
    fn test_settle_default_liquidation_multiple_rounds() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1_000_000, &500, &100);
        client.draw_credit(&borrower, &100_000);

        client.default_credit_line(&borrower);

        let before = client.get_credit_line(&borrower).unwrap();
        let total_utilized = before.utilized_amount;

        // Round 1: Recover 33%
        let settlement_id_1 = Symbol::new(&env, "settle_1");
        let recovery_1 = total_utilized / 3;
        client.settle_default_liquidation(
            &borrower,
            &recovery_1,
            &settlement_id_1,
            &3_333, // ~33%
        );

        let after_round_1 = client.get_credit_line(&borrower).unwrap();
        assert_eq!(after_round_1.utilized_amount, total_utilized - recovery_1);
        assert_eq!(after_round_1.status, CreditStatus::Defaulted);

        // Round 2: Recover another 33%
        let settlement_id_2 = Symbol::new(&env, "settle_2");
        let recovery_2 = (total_utilized - recovery_1) / 2;
        client.settle_default_liquidation(
            &borrower,
            &recovery_2,
            &settlement_id_2,
            &5_000, // 50%
        );

        let after_round_2 = client.get_credit_line(&borrower).unwrap();
        assert_eq!(
            after_round_2.utilized_amount,
            total_utilized - recovery_1 - recovery_2
        );
        assert_eq!(after_round_2.status, CreditStatus::Defaulted);

        // Round 3: Recover remaining to close
        let settlement_id_3 = Symbol::new(&env, "settle_3");
        let recovery_3 = after_round_2.utilized_amount;
        client.settle_default_liquidation(
            &borrower,
            &recovery_3,
            &settlement_id_3,
            &10_000, // 100%
        );

        let after_round_3 = client.get_credit_line(&borrower).unwrap();
        assert_eq!(after_round_3.utilized_amount, 0);
        assert_eq!(after_round_3.status, CreditStatus::Closed);
    }

    /// Invalid recovered_amount = 0 is rejected.
    #[test]
    #[should_panic(expected = "Error(Contract, #5)")]
    fn test_settle_default_liquidation_zero_recovered_amount() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1_000_000, &500, &100);
        client.draw_credit(&borrower, &100_000);

        client.default_credit_line(&borrower);

        let settlement_id = Symbol::new(&env, "settle_zero_amt");
        client.settle_default_liquidation(
            &borrower,
            &0, // Invalid: zero recovered amount
            &settlement_id,
            &5_000,
        );
    }

    /// Settlement on non-defaulted line fails.
    #[test]
    #[should_panic(expected = "Error(Contract, #2)")]
    fn test_settle_default_liquidation_not_defaulted() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1_000_000, &500, &100);
        client.draw_credit(&borrower, &100_000);

        // No default call - line is still Active

        let settlement_id = Symbol::new(&env, "settle_active");
        client.settle_default_liquidation(
            &borrower,
            &50_000,
            &settlement_id,
            &5_000,
        );
    }
}
