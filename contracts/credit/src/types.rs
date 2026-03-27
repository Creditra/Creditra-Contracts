//! Core data types for the Credit contract.

use soroban_sdk::{contracttype, Address};

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreditStatus {
    Active = 0,
    Suspended = 1,
    Defaulted = 2,
    Closed = 3,
    /// Credit limit was decreased below utilized amount; excess must be repaid.
    Restricted = 4,
}

#[soroban_sdk::contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ContractError {
    /// Caller is not authorized to perform this action.
    Unauthorized = 1,
    /// Caller does not have admin privileges.
    NotAdmin = 2,
    /// The specified credit line was not found.
    CreditLineNotFound = 3,
    /// Action cannot be performed because the credit line is closed.
    CreditLineClosed = 4,
    /// The requested amount is invalid (e.g., zero or negative where positive is expected).
    InvalidAmount = 5,
    /// The requested draw exceeds the available credit limit.
    OverLimit = 6,
    /// The credit limit cannot be negative.
    NegativeLimit = 7,
    /// The interest rate change exceeds the maximum allowed delta.
    RateTooHigh = 8,
    /// The risk score is above the acceptable maximum threshold.
    ScoreTooHigh = 9,
    /// Action cannot be performed because the credit line utilization is not zero.
    UtilizationNotZero = 10,
    /// Reentrancy detected during cross-contract calls.
    Reentrancy = 11,
    /// Math overflow occurred during calculation.
    Overflow = 12,
    /// Credit limit decrease requires immediate repayment of excess amount.
    LimitDecreaseRequiresRepayment = 13,
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
