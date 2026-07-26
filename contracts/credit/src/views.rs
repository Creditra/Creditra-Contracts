// SPDX-License-Identifier: MIT

//! Read-only query views for specialized campaign indexing.
//!
//! Provides the protocol summary view requested for the GrantFox campaign.

use crate::types::ProtocolSummaryView;
use soroban_sdk::Env;

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
