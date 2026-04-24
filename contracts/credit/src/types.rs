// SPDX-License-Identifier: MIT

//! Core data types for the Credit contract.

use soroban_sdk::{contracttype, Address};

/// Status of a borrower's credit line.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreditStatus {
    /// Credit line is active and draws are allowed.
    Active = 0,
    /// Credit line is temporarily frozen by admin.
    Suspended = 1,
    /// Credit line is in default; draws are disabled.
    Defaulted = 2,
    /// Credit line is permanently closed.
    Closed = 3,
    /// Credit limit was decreased below utilized amount; excess must be repaid.
    Restricted = 4,
}

/// Errors that can be returned by the Credit contract.
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
    /// Contract has already been initialized; `init` may only be called once.
    AlreadyInitialized = 14,
    /// Per-transaction draw cap exceeded.
    DrawExceedsMaxAmount = 15,
    /// Admin acceptance attempted before the configured delay has elapsed.
    AdminAcceptTooEarly = 16,
    /// Borrower is blocked from drawing credit.
    BorrowerBlocked = 17,
}

/// Stored credit line data for a borrower.
#[contracttype]
pub struct CreditLineData {
    /// Address of the borrower.
    pub borrower: Address,
    /// Maximum borrowable amount for this line.
    pub credit_limit: i128,
    /// Current outstanding principal.
    pub utilized_amount: i128,
    /// Annual interest rate in basis points (1 bp = 0.01%).
    pub interest_rate_bps: u32,
    /// Borrower's risk score (0-100).
    pub risk_score: u32,
    /// Current status of the credit line.
    pub status: CreditStatus,
    /// Ledger timestamp of the last interest-rate update.
    /// Zero means no rate update has occurred yet.
    pub last_rate_update_ts: u64,
    /// Total accrued interest that has been added to the utilized amount.
    /// This tracks the cumulative interest that has been capitalized.
    pub accrued_interest: i128,
    /// Ledger timestamp of the last interest accrual calculation.
    /// Zero means no accrual has been calculated yet.
    pub last_accrual_ts: u64,
    /// Ledger timestamp when the credit line was most recently suspended.
    /// Zero when the line has never been suspended or has been reinstated.
    /// Used by the grace period logic to determine whether the waiver window
    /// is still active.
    pub suspension_ts: u64,
}

/// Admin-configurable limits on interest-rate changes.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RateChangeConfig {
    /// Maximum absolute change in `interest_rate_bps` allowed per single update.
    pub max_rate_change_bps: u32,
    /// Minimum elapsed seconds between two consecutive rate changes.
    pub rate_change_min_interval: u64,
}

/// Admin-configurable piecewise-linear rate formula.
///
/// When stored in instance storage, `update_risk_parameters` computes
/// `interest_rate_bps` from the borrower's `risk_score` instead of using
/// the manually supplied rate.
///
/// # Formula
/// ```text
/// raw_rate = base_rate_bps + (risk_score * slope_bps_per_score)
/// effective_rate = clamp(raw_rate, min_rate_bps, min(max_rate_bps, 10_000))
/// ```
///
/// # Invariants
/// - `min_rate_bps <= max_rate_bps <= 10_000`
/// - `base_rate_bps <= 10_000`
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RateFormulaConfig {
    /// Base interest rate in bps applied at risk_score = 0.
    pub base_rate_bps: u32,
    /// Additional bps per unit of risk_score (0–100).
    pub slope_bps_per_score: u32,
    /// Minimum allowed computed rate (floor).
    pub min_rate_bps: u32,
    /// Maximum allowed computed rate (ceiling), must be <= 10_000.
    pub max_rate_bps: u32,
}

// ─── Grace period policy ──────────────────────────────────────────────────────

/// How interest is treated for a Suspended line that is within its grace window.
///
/// # Economics
/// - [`GraceWaiverMode::FullWaiver`]: no interest accrues during the grace window.
///   Borrowers get a complete interest holiday to support recovery. The protocol
///   absorbs the cost of foregone interest.
/// - [`GraceWaiverMode::ReducedRate`]: interest accrues at a lower rate during the
///   grace window. Provides partial relief while keeping the borrower accountable.
///   `reduced_rate_bps` must be ≤ the line's `interest_rate_bps`.
///
/// # Risks
/// - Full waiver creates a moral-hazard incentive to trigger suspension.
///   Mitigate by requiring admin approval for suspension and limiting grace duration.
/// - Reduced rate still accrues debt; borrowers must be informed of the residual cost.
///
/// # Interaction with `default_credit_line`
/// If admin calls `default_credit_line` while a grace period is active, the grace
/// period ends immediately. Interest resumes at the full rate from the moment of
/// default (the accrual checkpoint is updated at the time of the status transition).
///
/// # Interaction with `reinstate_credit_line`
/// Reinstatement transitions Defaulted → Active. The grace period only applies to
/// Suspended lines; a reinstated line accrues at its full rate immediately.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraceWaiverMode {
    /// Interest is fully waived (zero accrual) during the grace window.
    FullWaiver = 0,
    /// Interest accrues at a reduced rate (in bps) during the grace window.
    ReducedRate = 1,
}

/// Admin-configurable grace period policy for Suspended credit lines.
///
/// When set, a Suspended line that is within `grace_period_seconds` of its
/// suspension timestamp accrues interest according to `waiver_mode` instead of
/// the full rate. After the window expires, normal accrual resumes.
///
/// # Defaults
/// The policy is **disabled by default** (not stored). No grace period applies
/// unless an admin explicitly calls `set_grace_period_config`.
///
/// # Configuration
/// - `grace_period_seconds`: Duration of the grace window in ledger seconds.
///   Set to `0` to disable the grace period without removing the config.
/// - `waiver_mode`: [`GraceWaiverMode::FullWaiver`] or [`GraceWaiverMode::ReducedRate`].
/// - `reduced_rate_bps`: Effective rate during the grace window when `waiver_mode`
///   is [`GraceWaiverMode::ReducedRate`]. Ignored for `FullWaiver`. Must be ≤ 10 000.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GracePeriodConfig {
    /// Seconds after suspension during which the waiver applies.
    /// Zero disables the grace period.
    pub grace_period_seconds: u64,
    /// How interest is treated within the grace window.
    pub waiver_mode: GraceWaiverMode,
    /// Interest rate in bps applied during the grace window when
    /// `waiver_mode == ReducedRate`. Ignored for `FullWaiver`.
    pub reduced_rate_bps: u32,
}
