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

#[soroban_sdk::contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ContractError {
    Unauthorized = 1,
    NotAdmin = 2,
    CreditLineNotFound = 3,
    CreditLineClosed = 4,
    InvalidAmount = 5,
    OverLimit = 6,
    NegativeLimit = 7,
    RateTooHigh = 8,
    ScoreTooHigh = 9,
    UtilizationNotZero = 10,
    Reentrancy = 11,
    Overflow = 12,
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
    /// Ledger timestamp of the last interest-rate update via `update_risk_parameters`.
    /// Zero means no rate update has occurred yet.
    pub last_rate_update_ts: u64,
}

/// Admin-configurable limits on interest-rate changes.
///
/// * `max_rate_change_bps` – Maximum absolute change in `interest_rate_bps`
///   allowed per single `update_risk_parameters` call.
/// * `rate_change_min_interval` – Minimum elapsed seconds between two
///   consecutive rate changes. Set to `0` to disable the time-window check.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RateChangeConfig {
    pub max_rate_change_bps: u32,
    pub rate_change_min_interval: u64,
}
