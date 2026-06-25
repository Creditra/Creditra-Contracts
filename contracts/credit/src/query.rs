// SPDX-License-Identifier: MIT

//! Read-only query helpers for the Credit contract.
//!
//! Every function in this module is side-effect free (modulo TTL bumps in
//! [`crate::storage::get_credit_line`], which write only when the remaining
//! TTL is below `LEDGER_BUMP_THRESHOLD`).
//!
//! These helpers are the primary surface for off-chain indexers: returned
//! structs are designed for stable serialization order (see
//! [`crate::types::CreditLineData`] field ordering note).

use crate::storage::{get_borrower_by_credit_line_id, get_credit_line_count, grace_period_key, MAX_ENUMERATION_LIMIT};
use crate::types::{CreditLineData, CreditStatus, DeveloperBalance, GracePeriodConfig, RepaymentSchedule};
use soroban_sdk::{Address, Env, Vec};

/// Return the credit line for `borrower`, or `None` if no line exists.
///
/// # Authentication
/// No authentication required. This is a pure read — it does not mutate
/// any storage and carries no trust boundary. Any caller (indexer, client,
/// or another contract) may invoke it freely.
///
/// # Stability
/// The returned [`CreditLineData`] struct is stable for integrators.
/// All fields — including `last_rate_update_ts`, `accrued_interest`, and
/// `last_accrual_ts` — are serialized in the order declared in `types.rs`.
/// New fields will only be appended; existing field positions will not change.
///
/// # Note on accrual
/// Interest accrual is lazy: `accrued_interest` and `utilized_amount` reflect
/// the last mutating call (draw, repay, suspend, etc.). Pending interest since
/// the last checkpoint is **not** applied by this query.
#[allow(dead_code)]
pub fn get_credit_line(env: Env, borrower: Address) -> Option<CreditLineData> {
    crate::storage::get_credit_line(&env, &borrower)
}

/// Return the configured installment repayment schedule for `borrower`, if any.
pub fn get_repayment_schedule(env: Env, borrower: Address) -> Option<RepaymentSchedule> {
    env.storage()
        .persistent()
        .get(&crate::storage::DataKey::RepaymentSchedule(borrower))
}

/// Return `true` when the borrower has missed an installment past the grace window.
///
/// Returns `false` for the following short-circuit cases:
/// - The borrower has no credit line.
/// - The line is `Closed` or has zero outstanding principal.
/// - The line has no configured [`RepaymentSchedule`].
///
/// The grace window is determined by the global [`GracePeriodConfig`]. When no
/// config is set, `grace_seconds` defaults to `0`, so any timestamp strictly
/// greater than `next_due_ts` is treated as delinquent. The comparison uses
/// `saturating_add` to ensure timestamps near `u64::MAX` do not wrap.
pub fn is_delinquent(env: Env, borrower: Address) -> bool {
    let Some(line) = get_credit_line(env.clone(), borrower.clone()) else {
        return false;
    };

    if line.status == CreditStatus::Closed || line.utilized_amount <= 0 {
        return false;
    }

    let Some(schedule) = get_repayment_schedule(env.clone(), borrower) else {
        return false;
    };

    let grace_cfg: Option<GracePeriodConfig> = env.storage().instance().get(&grace_period_key(&env));
    let grace_seconds = grace_cfg.map(|cfg| cfg.grace_period_seconds).unwrap_or(0);
    let delinquent_after = schedule.next_due_ts.saturating_add(grace_seconds);

    env.ledger().timestamp() > delinquent_after
}

/// Return a page of developer (borrower) balances in stable insertion order.
///
/// This is the cursor-paginated replacement for a hypothetical unbounded
/// `get_all_developer_balances` scan. Each page is bounded by `limit`
/// (capped at [`MAX_ENUMERATION_LIMIT`]) and anchors on the stable numeric
/// id assigned at credit-line origination so pages remain consistent even
/// when new credit lines are opened between calls.
///
/// # Parameters
/// - `cursor`: Exclusive lower bound. Pass `None` to start from the
///   beginning. Pass the `next_cursor` returned by a previous call to
///   advance to the following page.
/// - `limit`: Maximum number of entries to return. Clamped to
///   [`MAX_ENUMERATION_LIMIT`] (100). Passing `0` returns an empty vec.
///
/// # Returns
/// `(page, next_cursor)` where:
/// - `page` is a `Vec<DeveloperBalance>` of up to `limit` entries in
///   ascending id order.
/// - `next_cursor` is `Some(last_id_in_page + 1)` when more entries may
///   exist, or `None` when the page exhausts the index.
///
/// # Cursor stability guarantees
/// - **New credit lines** added after the first page is read will appear
///   in subsequent pages with ids higher than any id already returned —
///   callers that walk all pages in sequence will not miss them.
/// - **Interleaved credits (draws/repays)** on existing lines do not affect
///   the ordering or the id values, so a cursor obtained mid-walk remains
///   valid.
/// - **No auth required**: pure read — any caller (indexer, client, or
///   another contract) may invoke it freely.
///
/// # Gas / resource cost
/// Cost is proportional to `limit`, not to the total number of credit lines.
/// Set `limit` ≤ 20 for on-chain consumers; 100 is appropriate for
/// off-chain indexers polling via RPC simulation.
///
/// # Example
/// ```ignore
/// // Page 1
/// let (page1, cursor) = client.get_developer_balances_page(&None, &20);
/// // Page 2
/// if let Some(c) = cursor {
///     let (page2, _) = client.get_developer_balances_page(&Some(c), &20);
/// }
/// ```
pub fn get_developer_balances_page(
    env: Env,
    cursor: Option<u32>,
    limit: u32,
) -> (Vec<DeveloperBalance>, Option<u32>) {
    let count = get_credit_line_count(&env);
    let capped_limit = limit.min(MAX_ENUMERATION_LIMIT);
    let mut out: Vec<DeveloperBalance> = Vec::new(&env);

    if capped_limit == 0 || count == 0 {
        return (out, None);
    }

    // cursor is an *exclusive* lower bound on the id, so the first id to
    // inspect is cursor + 1.  None means start from id 0.
    let mut next_id: u32 = match cursor {
        Some(c) => c.saturating_add(1),
        None => 0,
    };

    while next_id < count && out.len() < capped_limit {
        if let Some(borrower) = get_borrower_by_credit_line_id(&env, next_id) {
            if let Some(line) = env
                .storage()
                .persistent()
                .get::<Address, CreditLineData>(&borrower)
            {
                out.push_back(DeveloperBalance {
                    id: next_id,
                    borrower,
                    utilized_amount: line.utilized_amount,
                    credit_limit: line.credit_limit,
                });
            }
        }
        next_id = next_id.saturating_add(1);
    }

    // Emit a next_cursor only when there are more ids left to scan.
    let next_cursor = if next_id < count {
        // The last id we actually included in this page.
        let last_included = next_id.saturating_sub(1);
        Some(last_included)
    } else {
        None
    };

    (out, next_cursor)
}
