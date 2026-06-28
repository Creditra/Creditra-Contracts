// SPDX-License-Identifier: MIT

//! Read-only query views for specialized campaign indexing.
//!
//! Provides the protocol summary view requested for the GrantFox campaign
//! and the proof-of-reserve view for protocol treasury transparency.

use crate::storage::{get_borrower_by_credit_line_id, get_credit_line, MAX_ENUMERATION_LIMIT};
use crate::types::{CreditLinesPage, ProofOfReserve, ProtocolSummaryView};
use soroban_sdk::{Env, Vec};

/// Return protocol-level dashboard aggregates including ActiveLineCount.
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
/// contract as a result of protocol fee collection. This is a pure
/// storage read — no token CPIs or borrower records are touched.
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
