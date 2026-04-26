// SPDX-License-Identifier: MIT
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(coverage_nightly, coverage(off))]

//! Core data types for the Creditra contract.

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
///
/// # Stability guarantee
/// These discriminants are **permanent**. Never reorder or renumber existing
/// variants — doing so would break deployed SDK clients. New variants must be
/// appended at the end with the next available integer.
///
/// # Discriminant table (source of truth)
/// | Code | Variant                        | Description |
/// |------|--------------------------------|-------------|
/// | 1    | `Unauthorized`                 | Caller is not authorized |
/// | 2    | `NotAdmin`                     | Caller lacks admin privileges |
/// | 3    | `CreditLineNotFound`           | Credit line does not exist |
/// | 4    | `CreditLineClosed`             | Credit line is permanently closed |
/// | 5    | `InvalidAmount`                | Amount is zero, negative, or otherwise invalid |
/// | 6    | `OverLimit`                    | Draw would exceed the credit limit |
/// | 7    | `NegativeLimit`                | Credit limit cannot be negative |
/// | 8    | `RateTooHigh`                  | Interest rate exceeds the maximum allowed |
/// | 9    | `ScoreTooHigh`                 | Risk score exceeds the maximum allowed (100) |
/// | 10   | `UtilizationNotZero`           | Operation requires zero utilization |
/// | 11   | `Reentrancy`                   | Reentrancy detected during cross-contract call |
/// | 12   | `Overflow`                     | Arithmetic overflow during calculation |
/// | 13   | `LimitDecreaseRequiresRepayment` | Limit decrease below utilized amount |
/// | 14   | `AlreadyInitialized`           | Contract already initialized |
/// | 15   | `AdminAcceptTooEarly`          | Admin acceptance attempted before delay elapsed |
/// | 16   | `BorrowerBlocked`              | Borrower is on the blocked list |
/// | 17   | `DrawExceedsMaxAmount`         | Draw amount exceeds per-transaction cap |
/// | 18   | `Paused`                       | Protocol is paused; operation blocked by circuit breaker |
/// | 19   | `DrawsFrozen`                  | Draws are globally frozen |
/// | 20   | `CreditLineSuspended`          | Credit line is suspended |
/// | 21   | `CreditLineDefaulted`          | Credit line is defaulted |
/// | 22   | `MissingLiquidityToken`        | Liquidity token is not configured |
/// | 23   | `MissingLiquiditySource`       | Liquidity source is not configured |
/// | 24   | `InsufficientLiquidityReserve` | Reserve balance cannot cover the draw |
/// | 25   | `LiquidityTokenCallFailed`     | Liquidity token call failed where observable |
/// | 26   | `InsufficientRepaymentAllowance` | Borrower allowance cannot cover repayment |
/// | 27   | `InsufficientRepaymentBalance` | Borrower balance cannot cover repayment |
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
    /// Admin acceptance attempted before the delay window has elapsed.
    AdminAcceptTooEarly = 15,
    /// Borrower is blocked from drawing credit.
    BorrowerBlocked = 16,
    /// The requested draw exceeds the configured per-transaction maximum.
    DrawExceedsMaxAmount = 17,
    /// Protocol is paused by the emergency circuit breaker.
    Paused = 18,
    /// All draws are globally frozen by admin for liquidity reserve operations.
    DrawsFrozen = 19,
    /// Action cannot be performed because the credit line is suspended.
    CreditLineSuspended = 20,
    /// Action cannot be performed because the credit line is defaulted.
    CreditLineDefaulted = 21,
    /// Liquidity token has not been configured.
    MissingLiquidityToken = 22,
    /// Liquidity source has not been configured.
    MissingLiquiditySource = 23,
    /// Liquidity reserve balance is below the requested draw amount.
    InsufficientLiquidityReserve = 24,
    /// Liquidity token call failed where the contract can observe it.
    LiquidityTokenCallFailed = 25,
    /// Borrower's token allowance is below the effective repayment amount.
    InsufficientRepaymentAllowance = 26,
    /// Borrower's token balance is below the effective repayment amount.
    InsufficientRepaymentBalance = 27,
}

/// Stored credit line data for a borrower.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
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

/// Grace period configuration for Suspended credit lines.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GracePeriodConfig {
    /// Duration of the grace window in seconds.
    pub grace_period_seconds: u64,
    /// Type of waiver to apply during the grace period.
    pub waiver_mode: GraceWaiverMode,
    /// Reduced rate to apply when waiver_mode is ReducedRate.
    pub reduced_rate_bps: u32,
}

/// Grace period waiver modes.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraceWaiverMode {
    /// Full waiver - zero interest during grace period.
    FullWaiver = 0,
    /// Reduced rate - apply reduced_rate_bps during grace period.
    ReducedRate = 1,
}

/// Event emitted when the rate formula config is set or cleared.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RateFormulaConfigEvent {
    /// `true` when a config was set; `false` when cleared.
    pub enabled: bool,
}

/// Structured representation of the contract's API version (semver).
/// Grace period waiver mode for suspended credit lines.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraceWaiverMode {
    FullWaiver = 0,
    ReducedRate = 1,
}

/// Admin-configurable grace period policy for suspended credit lines.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GracePeriodConfig {
    pub grace_period_seconds: u64,
    pub waiver_mode: GraceWaiverMode,
    pub reduced_rate_bps: u32,
}

/// Mode of interest rate waiver during a grace period.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraceWaiverMode {
    /// Zero interest accrues during the grace window.
    FullWaiver = 0,
    /// Interest accrues at a reduced rate during the grace window.
    ReducedRate = 1,
}

/// Configuration for the grace period applied to Suspended credit lines.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GracePeriodConfig {
    /// Duration of the grace window in seconds.
    pub grace_period_seconds: u64,
    /// The waiver mode applied during the grace window.
    pub waiver_mode: GraceWaiverMode,
    /// The reduced rate applied when `waiver_mode` is `ReducedRate`.
    pub reduced_rate_bps: u32,
}

/// Global protocol configuration.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtocolConfig {
    /// Configured liquidity token.
    pub liquidity_token: Option<Address>,
    /// Configured liquidity source.
    pub liquidity_source: Option<Address>,
    /// Configured rate change limits.
    pub rate_change_config: Option<RateChangeConfig>,
}
