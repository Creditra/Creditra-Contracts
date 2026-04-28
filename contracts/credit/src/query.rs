// SPDX-License-Identifier: MIT

//! Read-only query helpers for the Credit contract.

use crate::types::CreditLineData;
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
pub fn get_credit_line(env: Env, borrower: Address) -> Option<CreditLineData> {
    env.storage().persistent().get(&borrower)
}
