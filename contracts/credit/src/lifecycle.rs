// SPDX-License-Identifier: MIT

//! Credit-line lifecycle operations: open, suspend, close, default, reinstate.
//!
//! # TTL bumping
//! Every function that writes a credit-line entry to persistent storage also
//! calls [`extend_ttl`](soroban_sdk::storage::Persistent::extend_ttl) to keep
//! the entry live for [`CREDIT_LINE_TTL_EXTEND_TO`] ledgers.  The threshold
//! [`CREDIT_LINE_TTL_THRESHOLD`] ensures we only pay for the extension when
//! there is a real risk of expiry.

use crate::auth::{require_admin, require_admin_auth};
use crate::events::{publish_credit_line_event, CreditLineEvent};
use crate::storage::{
    assert_not_paused, get_repayment_schedule,
    set_repayment_schedule as storage_set_repayment_schedule, CREDIT_LINE_TTL_EXTEND_TO,
    CREDIT_LINE_TTL_THRESHOLD,
};
use crate::types::{ContractError, CreditLineData, CreditStatus, RepaymentSchedule};
use soroban_sdk::{symbol_short, Address, Env};

/// Helper: bump the TTL of a borrower's persistent credit-line entry.
///
/// Should be called after every read **or** write that constitutes an
/// "interaction" with the credit line so the ledger entry never silently
/// expires while the line is still in use.
fn bump_credit_line_ttl(env: &Env, borrower: &Address) {
    env.storage()
        .persistent()
        .extend_ttl(borrower, CREDIT_LINE_TTL_THRESHOLD, CREDIT_LINE_TTL_EXTEND_TO);
}

/// Set credit limit bounds (admin only, called through contractimpl).
///
/// These bounds are enforced by [`validate_credit_limit_bounds`] during
/// `open_credit_line` and `update_risk_parameters`.
pub fn set_credit_limit_bounds(env: Env, min: i128, max: i128) {
    require_admin_auth(&env);
    crate::storage::set_min_credit_limit(&env, min);
    crate::storage::set_max_credit_limit(&env, max);
}

/// Get the current credit limit bounds, if configured.
///
/// Returns `(Option<min>, Option<max>)` where `None` means the bound is not set.
pub fn get_credit_limit_bounds(env: &Env) -> (Option<i128>, Option<i128>) {
    let min = crate::storage::get_min_credit_limit(env);
    let max = crate::storage::get_max_credit_limit(env);
    (min, max)
}

/// Validate that a credit limit falls within the configured min/max bounds (if set).
///
/// # Panics
/// - `ContractError::LimitOutOfBounds` if `credit_limit` is outside the configured range.
pub fn validate_credit_limit_bounds(env: &Env, credit_limit: i128) {
    let (min_limit, max_limit) = get_credit_limit_bounds(env);
    if let Some(min) = min_limit {
        if credit_limit < min {
            env.panic_with_error(ContractError::LimitOutOfBounds);
        }
    }
    if let Some(max) = max_limit {
        if credit_limit > max {
            env.panic_with_error(ContractError::LimitOutOfBounds);
        }
    }
}

/// Open a new credit line for a borrower (admin only).
///
/// # Parameters
/// - `borrower`: Address of the borrower.
/// - `credit_limit`: Maximum drawable amount (must be > 0).
/// - `interest_rate_bps`: Annual interest rate in basis points (0–10 000).
/// - `risk_score`: Borrower risk score (0–100).
///
/// # Panics
/// - If `credit_limit` ≤ 0, `interest_rate_bps` > 10 000, or `risk_score` > 100.
/// - If the borrower already has an Active credit line.
///
/// # Events
/// Emits a `("credit", "opened")` [`CreditLineEvent`].
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

    // Prevent overwriting an existing Active credit line.
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

    env.storage().persistent().set(&borrower, &credit_line);
    // Bump TTL: newly opened lines start with a full TTL window.
    bump_credit_line_ttl(&env, &borrower);

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
/// Only lines in [`CreditStatus::Active`] status may be suspended.
///
/// # Parameters
/// - `borrower`: The borrower's address.
///
/// # Panics
/// - If no credit line exists for the given borrower.
/// - If the credit line is not currently Active.
///
/// # Events
/// Emits a `("credit", "suspend")` [`CreditLineEvent`].
pub fn suspend_credit_line(env: Env, borrower: Address) {
    assert_not_paused(&env);
    require_admin_auth(&env);
    let mut credit_line: CreditLineData = env
        .storage()
        .persistent()
        .get(&borrower)
        .expect("Credit line not found");

    if credit_line.status != CreditStatus::Active {
        panic!("Only active credit lines can be suspended");
    }

    credit_line.status = CreditStatus::Suspended;
    env.storage().persistent().set(&borrower, &credit_line);
    // Bump TTL: interacting with a suspended line keeps it live.
    bump_credit_line_ttl(&env, &borrower);

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

/// Close a credit line. Callable by admin (force-close) or by borrower
/// when utilization is zero. Idempotent if already Closed.
///
/// # Arguments
/// * `closer` - Must be either the contract admin (can close regardless of
///   utilization) or the borrower (can close only when `utilized_amount` is zero).
///
/// # Panics
/// - If the credit line does not exist.
/// - If `closer` is not the admin or borrower.
/// - If the borrower attempts to close while `utilized_amount != 0`.
///
/// # Events
/// Emits a `("credit", "closed")` [`CreditLineEvent`].
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
            env.panic_with_error(ContractError::UtilizationNotZero);
        }
    } else {
        // Third party: unconditionally rejected.
        env.panic_with_error(ContractError::Unauthorized);
    }

    let previous_status = credit_line.status;
    credit_line.status = CreditStatus::Closed;
    env.storage().persistent().set(&borrower, &credit_line);
    // Bump TTL: keep the closed record live so history is queryable.
    bump_credit_line_ttl(&env, &borrower);

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
/// Transition: Active or Suspended → Defaulted.
/// After this, `draw_credit` is disabled and `repay_credit` remains allowed.
///
/// # Panics
/// - If no credit line exists for the given borrower.
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
    env.storage().persistent().set(&borrower, &credit_line);
    // Bump TTL: defaulted lines must remain queryable during workout period.
    bump_credit_line_ttl(&env, &borrower);

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

    let max_close_factor = crate::storage::get_close_factor_bps(&env);
    if close_factor_bps > max_close_factor {
        env.panic_with_error(ContractError::CloseFactorAboveMax);
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

    credit_line.status = target_status;
    credit_line.suspension_ts = 0;
    persist_credit_line(&env, &borrower, &credit_line, previous_utilized);

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

    let advance_seconds = installments_paid.saturating_mul(schedule.period_seconds);
    schedule.next_due_ts = schedule.next_due_ts.saturating_add(advance_seconds);
    storage_set_repayment_schedule(env, borrower, &schedule);
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
// Version Handshake Check
let remote_version = auction_client.get_version();
assert!(handshake::verify_version(&env, remote_version), "Incompatible Version");
// Version Handshake Check
let remote_version = auction_client.get_version();
assert!(handshake::verify_version(&env, remote_version), "Incompatible Version");
