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

use crate::storage::grace_period_key;
use crate::types::{CreditLineData, CreditStatus, GracePeriodConfig, RepaymentSchedule};
use soroban_sdk::{Address, Env};

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
