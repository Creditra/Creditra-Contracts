//! Core data types for the Credit contract.

use soroban_sdk::{contracttype, Address};

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreditStatus {
    Active = 0,
    Suspended = 1,
    Defaulted = 2,
    Closed = 3,
}

/// Stored credit line for a borrower.
#[contracttype]
pub struct CreditLineData {
    pub borrower: Address,
    pub credit_limit: i128,
    pub utilized_amount: i128,
    pub interest_rate_bps: u32,
    pub risk_score: u32,
    pub status: CreditStatus,
    /// Ledger timestamp of the last update to this credit line.
    /// Used for interest accrual calculations — records when
    /// utilized_amount last changed.
    pub last_update_timestamp: u64,
}
