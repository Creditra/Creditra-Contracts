// SPDX-License-Identifier: MIT

//! Read-only query views for the Creditra credit contract.
//!
//! Each function is a pure storage read — no state mutations, no token CPIs,
//! no authentication required. TTL may be bumped by `get_credit_line` via the
//! storage layer when the persistent entry nears expiry.

use crate::storage::{get_borrower_by_credit_line_id, get_credit_line, DataKey, MAX_ENUMERATION_LIMIT};
use crate::types::{
    CreditLineSnapshot, CreditLinesPage, CreditStatus, GracePeriodConfig, ProofOfReserve,
    ProtocolSummaryView, RepaymentSchedule,
};
use soroban_sdk::{Address, Env, Vec};

// ── Protocol-level views ─────────────────────────────────────────────────────

/// Return protocol-level dashboard aggregates including `active_line_count`.
///
/// Reads aggregate instance-storage slots only; does not touch per-borrower
/// records and does not bump persistent-entry TTL.
pub fn get_protocol_summary_view(env: Env) -> ProtocolSummaryView {
    ProtocolSummaryView {
        total_utilized: crate::storage::get_total_utilized(&env),
        total_collateral: crate::storage::get_total_collateral(&env),
        active_line_count: crate::storage::get_active_line_count(&env),
    }
}

/// Return proof-of-reserve balances for the protocol treasury.
///
/// Exposes the accumulated treasury and bounty pool reserves held in the
/// contract as a result of protocol fee collection. A pure storage read —
/// no token CPIs or borrower records are touched.
///
/// Callers can compare `treasury_balance + bounty_balance` against the
/// on-chain token balance of the contract to verify reserve integrity.
pub fn get_proof_of_reserve(env: Env) -> ProofOfReserve {
    ProofOfReserve {
        treasury_balance: crate::storage::get_treasury_balance(&env),
        bounty_balance: crate::storage::get_bounty_balance(&env),
    }
}

/// Return a paginated view of credit lines for off-chain reporting.
///
/// Uses cursor-based pagination where the cursor is the stable numeric ID
/// assigned to each borrower. This allows efficient, stateless navigation
/// through large sets of credit lines without offset-based limitations.
///
/// # Parameters
///
/// - `cursor`: Optional starting cursor (numeric ID). Pass `None` for the first page.
/// - `limit`: Maximum number of credit lines to return. Must be <= `MAX_ENUMERATION_LIMIT`.
///
/// # Returns
///
/// A [`CreditLinesPage`] containing:
/// - `credit_lines`: Vector of credit line data for this page.
/// - `next_cursor`: Cursor for the next page, or `None` if this is the last page.
///
/// # Behavior
///
/// - Starts enumeration from `cursor.unwrap_or(0)`.
/// - Returns at most `limit` credit lines.
/// - Iterates through stable numeric IDs in ascending order.
/// - Skips IDs that have no corresponding borrower (gaps in the sequence).
/// - Bumps TTL for each credit line entry that is loaded.
///
/// # Errors
///
/// - Panics with [`ContractError::Overflow`] if `limit` exceeds `MAX_ENUMERATION_LIMIT`.
///
/// # Example
///
/// ```text
/// // First page
/// let page1 = get_credit_lines_paginated(env, None, 10);
///
/// // Second page
/// if let Some(cursor) = page1.next_cursor {
///     let page2 = get_credit_lines_paginated(env, Some(cursor), 10);
/// }
/// ```
///
/// # Security
///
/// This is a read-only function with no authentication requirement. It only
/// reads storage and does not mutate any state. The TTL bump on loaded entries
/// is a side effect but does not change the logical state of the contract.
pub fn get_credit_lines_paginated(env: Env, cursor: Option<u32>, limit: u32) -> CreditLinesPage {
    // Enforce maximum limit to prevent unbounded gas consumption
    if limit > MAX_ENUMERATION_LIMIT {
        env.panic_with_error(crate::types::ContractError::Overflow);
    }

    let total_count = crate::storage::get_credit_line_count(&env);
    let start_id = cursor.unwrap_or(0);

    // Clamp start_id to valid range
    if start_id >= total_count {
        return CreditLinesPage {
            credit_lines: Vec::new(&env),
            next_cursor: None,
        };
    }

    let mut credit_lines = Vec::new(&env);
    let mut next_cursor: Option<u32> = None;
    let mut current_id = start_id;
    let end_id = total_count.saturating_sub(1);

    // Iterate through IDs until we collect enough results or reach the end
    while credit_lines.len() < limit as u32 && current_id <= end_id {
        if let Some(borrower) = get_borrower_by_credit_line_id(&env, current_id) {
            if let Some(line) = get_credit_line(&env, &borrower) {
                credit_lines.push_back(line);
            }
        }

        // Prepare next cursor if we might have more results
        if credit_lines.len() < limit as u32 && current_id < end_id {
            next_cursor = Some(current_id.saturating_add(1));
        } else if current_id < end_id {
            // We've filled the page but there are more results
            next_cursor = Some(current_id.saturating_add(1));
        }

        current_id = current_id.saturating_add(1);
    }

    // If we didn't fill the page, there are no more results
    if credit_lines.len() < limit as u32 {
        next_cursor = None;
    }

    CreditLinesPage {
        credit_lines,
        next_cursor,
    }
}

// ── Per-borrower snapshot view ────────────────────────────────────────────────

/// Return a full snapshot of `borrower`'s credit line, or `None` if no line exists.
///
/// Assembles [`CreditLineSnapshot`] in a single entrypoint call, avoiding the
/// multiple round-trips that callers would otherwise need to issue for
/// `get_credit_line` + `get_collateral` + `get_health_factor` +
/// `get_repayment_schedule` + `is_delinquent`.
///
/// # Authentication
/// None — this is a pure read with no state mutations or trust boundary.
///
/// # Laziness
/// Interest accrual is lazy: `line.accrued_interest` and `line.utilized_amount`
/// reflect the last mutating checkpoint, not the current ledger timestamp.
///
/// # Collateral health
/// `health_factor_bps` is `u32::MAX` when `utilized_amount == 0`. A value
/// below `10_000` signals the line is eligible for liquidation via
/// `default_credit_line`.
///
/// # Delinquency
/// `is_delinquent` is always `false` when `repayment_schedule` is `None` or
/// when `utilized_amount == 0` or status is `Closed`.
pub fn get_credit_line_snapshot(env: Env, borrower: Address) -> Option<CreditLineSnapshot> {
    // A single storage read; returns None immediately for unknown borrowers.
    let line = crate::storage::get_credit_line(&env, &borrower)?;

    let collateral_balance = crate::storage::get_collateral_balance(&env, &borrower);

    let health_factor_bps = compute_health_factor_bps(&env, &borrower, line.utilized_amount);

    let repayment_schedule: Option<RepaymentSchedule> = env
        .storage()
        .persistent()
        .get(&DataKey::RepaymentSchedule(borrower.clone()));

    let is_delinquent =
        check_is_delinquent(&env, &line.status, line.utilized_amount, &repayment_schedule);

    let has_repayment_schedule = repayment_schedule.is_some();
    let schedule = repayment_schedule.unwrap_or(RepaymentSchedule {
        amount_per_period: 0,
        period_seconds: 0,
        next_due_ts: 0,
    });

    Some(CreditLineSnapshot {
        line,
        collateral_balance,
        health_factor_bps,
        has_repayment_schedule,
        repayment_amount_per_period: schedule.amount_per_period,
        repayment_period_seconds: schedule.period_seconds,
        repayment_next_due_ts: schedule.next_due_ts,
        is_delinquent,
    })
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Compute the collateral health factor in basis points for a borrower.
///
/// Returns `u32::MAX` when `utilized_amount <= 0` (no debt).
/// Formula: `collateral * 100_000_000 / (utilized * min_ratio_bps)`.
fn compute_health_factor_bps(env: &Env, borrower: &Address, utilized_amount: i128) -> u32 {
    if utilized_amount <= 0 {
        return u32::MAX;
    }

    let collateral = crate::storage::get_collateral_balance(env, borrower);
    let min_ratio_bps = crate::storage::get_min_collateral_ratio_bps(env).unwrap_or(15_000);

    let collateral_u128 = collateral.max(0) as u128;
    let utilized_u128 = utilized_amount.max(0) as u128;
    let min_ratio_u128 = min_ratio_bps as u128;

    let numerator = collateral_u128
        .checked_mul(100_000_000)
        .unwrap_or(u128::MAX);
    let denominator = utilized_u128
        .checked_mul(min_ratio_u128)
        .unwrap_or(u128::MAX);

    u32::try_from(numerator / denominator).unwrap_or(u32::MAX)
}

/// Determine whether the borrower is past an installment due date.
///
/// Returns `false` when:
/// - The line is `Closed` or `utilized_amount <= 0`.
/// - No repayment schedule is configured.
/// - The current timestamp is within the grace window.
fn check_is_delinquent(
    env: &Env,
    status: &CreditStatus,
    utilized_amount: i128,
    schedule: &Option<RepaymentSchedule>,
) -> bool {
    if *status == CreditStatus::Closed || utilized_amount <= 0 {
        return false;
    }
    let Some(sched) = schedule else {
        return false;
    };

    let grace_cfg: Option<GracePeriodConfig> = env
        .storage()
        .instance()
        .get(&crate::storage::grace_period_key(env));
    let grace_seconds = grace_cfg
        .map(|cfg| cfg.grace_period_seconds)
        .unwrap_or(0);
    let delinquent_after = sched.next_due_ts.saturating_add(grace_seconds);

    env.ledger().timestamp() > delinquent_after
}
