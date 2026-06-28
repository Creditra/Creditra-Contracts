// SPDX-License-Identifier: MIT

//! Read-only query views for specialized campaign indexing.
//!
//! Provides the protocol summary view requested for the GrantFox campaign
//! and the proof-of-reserve view for protocol treasury transparency.

use crate::types::{ProofOfReserve, ProtocolSummaryView};
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
