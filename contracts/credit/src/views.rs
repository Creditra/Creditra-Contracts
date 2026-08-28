// SPDX-License-Identifier: MIT

//! Read-only query views for specialized campaign indexing.
//!
//! Provides the protocol summary view requested for the GrantFox campaign.

use crate::storage::{
    get_borrower_by_credit_line_id, get_credit_line, is_borrower_blocked, is_borrower_frozen,
    is_paused, MAX_ENUMERATION_LIMIT,
};
use crate::types::{
    AccrualCapabilities, BorrowCapabilities, BorrowStateSnapshot, CollateralCapabilities,
    CreditLineSnapshot, CreditLinesPage, ProofOfReserve, ProtocolSummaryView,
};
use soroban_sdk::{Address, Env, Vec};

// ── Borrow capabilities view ─────────────────────────────────────────────────

/// Return a borrower's current capabilities bitmap.
///
/// This is a read-only, no-auth view that reports which operations are
/// currently permitted for a given borrower. It evaluates the same
/// pre-flight checks that `draw_credit`, `repay_credit`, and
/// `self_suspend_credit_line` perform, EXCEPT for amount-dependent
/// checks (credit limit, collateral ratio, cooldown, exposure caps)
/// because this view does not know the intended draw/repay amount.
///
/// # Parameters
/// - `borrower`: The borrower address to query.
///
/// # Returns
/// A [`BorrowCapabilities`] struct with three bool fields:
/// - `can_draw` — draw pre-flight checks pass
/// - `can_repay` — repay pre-flight checks pass
/// - `can_self_suspend` — self-suspend pre-flight checks pass
///
/// # Security
/// This is a pure read-only query. It does not require authentication
/// and does not mutate any state.
pub fn borrow_capabilities(env: Env, borrower: Address) -> BorrowCapabilities {
    let credit_line = get_credit_line(&env, &borrower);

    let can_draw = credit_line
        .as_ref()
        .map(|line| {
            crate::borrow::draw_status_error(line.status).is_none()
                && !is_paused(&env)
                && !crate::freeze::is_draws_frozen(&env)
                && !is_borrower_blocked(&env, &borrower)
                && !is_borrower_frozen(&env, &borrower)
                && !crate::freeze::is_credit_line_frozen(&env, &borrower)
        })
        .unwrap_or(false);

    let can_repay = credit_line
        .as_ref()
        .map(|line| line.status != crate::types::CreditStatus::Closed)
        .unwrap_or(false);

    let can_self_suspend = credit_line
        .as_ref()
        .map(|line| line.status == crate::types::CreditStatus::Active)
        .unwrap_or(false);

    BorrowCapabilities {
        can_draw,
        can_repay,
        can_self_suspend,
    }
}

/// Return a borrower's current collateral capabilities bitmap.
///
/// This is a read-only, no-auth view that reports whether the borrower can
/// currently attempt collateral deposit, withdrawal, or partial release.
/// The view uses the same prerequisite checks that the entrypoints rely on:
/// an explicitly configured collateral token and a positive collateral balance
/// for withdraw/release operations.
pub fn capabilities(env: Env, borrower: Address) -> CollateralCapabilities {
    let token_configured = crate::storage::get_collateral_token(&env).is_some();
    let has_balance = crate::storage::get_collateral_balance(&env, &borrower) > 0;

    CollateralCapabilities {
        can_deposit: token_configured,
        can_withdraw: token_configured && has_balance,
        can_partial_release: token_configured && has_balance,
    }
}

// ── Protocol-level views ─────────────────────────────────────────────────────

/// Assemble a full read-only snapshot of `borrower`'s credit line.
///
/// Returns `None` when no credit line has been opened for `borrower`.
/// See [`CreditLineSnapshot`] for the aggregated fields.
pub fn get_credit_line_snapshot(env: Env, borrower: Address) -> Option<CreditLineSnapshot> {
    let line = get_credit_line(&env, &borrower)?;
    let collateral_balance = crate::collateral::get_collateral(&env, &borrower);
    let health_factor_bps = crate::query::get_health_factor(env.clone(), borrower.clone());
    let mut repayment_schedule = Vec::new(&env);
    if let Some(schedule) = crate::query::get_repayment_schedule(env.clone(), borrower.clone()) {
        repayment_schedule.push_back(schedule);
    }
    let is_delinquent = crate::query::is_delinquent(env.clone(), borrower);

    Some(CreditLineSnapshot {
        line,
        collateral_balance,
        health_factor_bps,
        repayment_schedule,
        is_delinquent,
    })
}

// ── Borrow capabilities view ─────────────────────────────────────────────────

/// Return a borrower's current capabilities bitmap.
///
/// This reads aggregate storage slots to return TotalUtilized, TotalCollateral,
/// and ActiveLineCount without iterating through individual borrower records.
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
            lines: Vec::new(&env),
            next_cursor: None,
            has_more: false,
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

    let has_more = next_cursor.is_some();
    CreditLinesPage {
        lines: credit_lines,
        next_cursor,
        has_more,
    }
}

// ── Borrow state snapshot view ───────────────────────────────────────────────

/// Return a full state snapshot for a borrower's credit line.
///
/// This is a read-only, no-auth view that returns a comprehensive snapshot
/// of the borrower's current state including credit line data, collateral
/// balance, and borrow capabilities. This is useful for off-chain monitoring,
/// risk dashboards, and debugging.
///
/// # Parameters
///
/// - `borrower`: The borrower address to query.
///
/// # Returns
///
/// A [`BorrowStateSnapshot`] struct containing:
/// - `credit_line`: The full [`CreditLineData`] if it exists, or `None`.
/// - `collateral_balance`: The borrower's collateral balance.
/// - `capabilities`: The borrower's current [`BorrowCapabilities`].
///
/// # Security
///
/// This is a pure read-only query. It does not require authentication
/// and does not mutate any state. TTL may be bumped if the borrower's
/// persistent entry is near expiry, but this does not change logical state.
pub fn get_borrow_state(env: Env, borrower: Address) -> BorrowStateSnapshot {
    let mut credit_line = soroban_sdk::Vec::new(&env);
    if let Some(line) = get_credit_line(&env, &borrower) {
        credit_line.push_back(line);
    }
    let collateral_balance = crate::storage::get_collateral_balance(&env, &borrower);
    let capabilities = borrow_capabilities(env.clone(), borrower.clone());

    BorrowStateSnapshot {
        credit_line,
        collateral_balance,
        capabilities,
    }
}
/// operations are currently available for a given borrower, without
/// executing any state-mutating logic.
///
/// # Parameters
/// - `borrower`: The borrower address to query.
///
/// # Returns
/// An [`AccrualCapabilities`] struct with four bool fields:
/// - `can_accrue` — `accrue_batch` will process this borrower
/// - `batch_open` — protocol accepts new `accrue_batch` submissions
/// - `penalty_rate_active` — borrower is delinquent and a surcharge is configured
/// - `grace_waiver_active` — borrower is within their suspension grace window
///
/// # Security
/// Pure read-only query. No authentication required. No state mutations occur.
pub fn accrual_capabilities(env: Env, borrower: Address) -> AccrualCapabilities {
    let paused = is_paused(&env);

    // batch_open: protocol not paused (batch size check is per-call; not evaluated here)
    let batch_open = !paused;

    let credit_line = get_credit_line(&env, &borrower);

    // can_accrue: line exists, Active, has utilization, protocol not paused
    let can_accrue = credit_line
        .as_ref()
        .map(|line| !paused && line.status == crate::types::CreditStatus::Active && line.utilized_amount > 0)
        .unwrap_or(false);

    // penalty_rate_active: surcharge configured AND borrower is delinquent
    let penalty_surcharge_bps = crate::storage::get_penalty_surcharge_bps(&env);
    let penalty_rate_active = if penalty_surcharge_bps > 0 {
        crate::query::is_delinquent(env.clone(), borrower.clone())
    } else {
        false
    };

    // grace_waiver_active: line is Suspended, a grace config exists, and now <= grace_end
    let grace_waiver_active = credit_line
        .as_ref()
        .map(|line| {
            if line.status != crate::types::CreditStatus::Suspended {
                return false;
            }
            let grace_cfg = crate::storage::get_grace_period_config(&env);
            match grace_cfg {
                Some(cfg) if cfg.grace_period_seconds > 0 => {
                    let now = env.ledger().timestamp();
                    let grace_end = line.suspension_ts.saturating_add(cfg.grace_period_seconds);
                    now <= grace_end
                }
                _ => false,
            }
        })
        .unwrap_or(false);

    AccrualCapabilities {
        can_accrue,
        batch_open,
        penalty_rate_active,
        grace_waiver_active,
    }
}
