// SPDX-License-Identifier: MIT
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(coverage_nightly, coverage(off))]

//! Core data types for the Creditra contract.
//!
//! # What
//!
//! ABI-stable types that cross the contract boundary:
//!
//! - [`ContractError`] — 40-variant `#[repr(u32)]` error enum (discriminants
//!   pinned by `tests/error_discriminants.rs`). Each variant maps to a stable
//!   [`ContractErrorCategory`] via [`ContractError::category`]. See
//!   [`docs/contract-errors.md`](../../../docs/contract-errors.md) for the
//!   flat code table and
//!   [`docs/error-taxonomy.md`](../../../docs/error-taxonomy.md) for the
//!   categorized reference with recovery hints.
//! - [`CreditStatus`] — 5-variant state-machine label (Active=0,
//!   Suspended=1, Defaulted=2, Closed=3, Restricted=4). See
//!   [`docs/state-machine.md`](../../../docs/state-machine.md) for the
//!   transition graph.
//! - [`CreditLineData`] — the per-borrower record (limit, utilized, rate,
//!   score, status, accrual + suspension timestamps, accrued interest).
//! - [`RepaymentSchedule`] — installment metadata
//!   (`amount_per_period`, `period_seconds`, `next_due_ts`).
//! - [`RateChangeConfig`] — magnitude + cadence cap on
//!   `update_risk_parameters`.
//! - [`RateFormulaConfig`] — piecewise-linear rate formula parameters
//!   `(base_rate_bps, slope_bps_per_score, min_rate_bps, max_rate_bps)`.
//! - [`GracePeriodConfig`] / [`GraceWaiverMode`] — suspension grace policy
//!   (FullWaiver vs ReducedRate) consumed by [`crate::accrual`].
//! - [`OracleConfig`] — price-feed circuit-breaker parameters
//!   `(max_deviation_bps, max_age_seconds)`.
//! - [`ProtocolConfig`] / [`ProtocolSummary`] — host-side projections used by
//!   aggregate protocol queries (NOT `#[contracttype]`).
//!
//! # How
//!
//! All types are `#[contracttype]`-tagged unless explicitly marked
//! otherwise; this makes them cross the Soroban host ABI as structured
//! values. Discriminants on the two enums are ABI-stable; new variants must
//! be appended to preserve indexer and SDK compatibility.
//!
//! # Why
//!
//! These types are the protocol's externalized vocabulary. They are
//! consumed by off-chain indexers (`docs/indexer-integration.md`), by
//! SDK clients building transactions, and by integrators reading the
//! contract state for risk dashboards. Stability of the discriminants and
//! field layout is enforced by CI tests so a downstream consumer can pin
//! against a `major.minor.patch` of `CONTRACT_API_VERSION` (currently
//! `(1, 0, 0)`).

use soroban_sdk::{contracttype, Address};

/// Status of a borrower's credit line.
///
/// # Discriminant stability
/// The discriminants are part of the contract ABI. They must never be
/// reordered or renumbered; new variants must be appended.
///
/// # Transitions
/// See [`docs/state-machine.md`](../../../docs/state-machine.md) for the
/// authoritative state-transition diagram. In short:
///
/// - `Active` is the only state that permits new draws.
/// - `Restricted` allows draws but the numeric limit check will fail until
///   the borrower repays under the reduced ceiling.
/// - `Suspended` and `Defaulted` both block draws and allow repayments.
/// - `Closed` is terminal — no draws, no repayments.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreditStatus {
    /// Credit line is active; draws and repayments allowed.
    Active = 0,
    /// Credit line is temporarily frozen by admin. Draws blocked, repayments allowed.
    Suspended = 1,
    /// Credit line is in default; draws blocked, repayments allowed for cure.
    Defaulted = 2,
    /// Credit line is permanently closed. Draws blocked, repayments blocked.
    Closed = 3,
    /// Credit limit was decreased below utilized amount; excess must be repaid.
    /// Draws are not flat-blocked but will fail the numeric limit check until cured.
    Restricted = 4,
}

/// Errors that can be returned by the Credit contract.
///
/// # Stability guarantee
/// These discriminants are **permanent**. Never reorder or renumber existing
/// variants — doing so would break deployed SDK clients. New variants must be
/// appended at the end with the next available integer.
///
/// # Category
/// Use [`ContractError::category`] to map any error to its
/// [`ContractErrorCategory`] for client-side grouping. See
/// [`docs/error-taxonomy.md`](../../../docs/error-taxonomy.md) for the
/// categorized reference with recovery actions.
///
/// # Discriminant table (source of truth)
/// | Code | Variant                        | Category      | Description |
/// |------|--------------------------------|---------------|-------------|
/// | 1    | `Unauthorized`                 | Auth          | Caller is not authorized |
/// | 2    | `NotAdmin`                     | Auth          | Caller lacks admin privileges |
/// | 3    | `CreditLineNotFound`           | Misc          | Credit line does not exist |
/// | 4    | `CreditLineClosed`             | Lifecycle     | Credit line is permanently closed |
/// | 5    | `InvalidAmount`                | Numeric       | Amount is zero, negative, or otherwise invalid |
/// | 6    | `OverLimit`                    | Limit         | Draw would exceed the credit limit |
/// | 7    | `NegativeLimit`                | Numeric       | Credit limit cannot be negative |
/// | 8    | `RateTooHigh`                  | Risk          | Interest rate exceeds the maximum allowed |
/// | 9    | `ScoreTooHigh`                 | Risk          | Risk score exceeds the maximum allowed (100) |
/// | 10   | `UtilizationNotZero`           | Limit         | Operation requires zero utilization |
/// | 11   | `Reentrancy`                   | Reentrancy    | Reentrancy detected during cross-contract call |
/// | 12   | `Overflow`                     | Numeric       | Arithmetic overflow during calculation |
/// | 13   | `LimitDecreaseRequiresRepayment` | Limit       | Limit decrease below utilized amount |
/// | 14   | `AlreadyInitialized`           | Lifecycle     | Contract already initialized |
/// | 15   | `AdminAcceptTooEarly`          | Misc          | Admin acceptance attempted before delay elapsed |
/// | 16   | `BorrowerBlocked`              | Block         | Borrower is on the blocked list |
/// | 17   | `DrawExceedsMaxAmount`         | Limit         | Draw amount exceeds per-transaction cap |
/// | 18   | `Paused`                       | Risk          | Protocol is paused; operation blocked by circuit breaker |
/// | 19   | `DrawsFrozen`                  | Block         | Draws are globally frozen |
/// | 20   | `CreditLineSuspended`          | Lifecycle     | Credit line is suspended |
/// | 21   | `CreditLineDefaulted`          | Lifecycle     | Credit line is defaulted |
/// | 22   | `MissingLiquidityToken`        | Liquidity     | Liquidity token is not configured |
/// | 23   | `MissingLiquiditySource`       | Liquidity     | Liquidity source is not configured |
/// | 24   | `InsufficientLiquidityReserve` | Liquidity     | Reserve balance cannot cover the draw |
/// | 25   | `LiquidityTokenCallFailed`     | Liquidity     | Liquidity token call failed where observable |
/// | 26   | `InsufficientRepaymentAllowance` | Liquidity   | Borrower allowance cannot cover repayment |
/// | 27   | `InsufficientRepaymentBalance` | Liquidity     | Borrower balance cannot cover repayment |
/// | 28   | `RepayExceedsMaxAmount`        | Limit         | Repay amount exceeds per-transaction cap |
/// | 29   | `DrawCooldownActive`          | Risk          | Borrower attempted to draw before cooldown elapsed |
/// | 30   | `TreasuryNotSet`              | Liquidity     | Treasury address is not configured |
/// | 31   | `ExposureCapExceeded`         | Liquidity     | Draw would exceed the global protocol exposure cap |
/// | 32   | `AdminNotInitialized`         | Auth          | Admin address has not been initialized |
/// | 33   | `TimestampRegression`         | Numeric       | Timestamp regression detected |
/// | 34   | `LimitOutOfBounds`            | Numeric       | Credit limit is outside configured min/max bounds |
/// | 35   | `CollateralRatioBelowMinimum` | Collateral    | Collateral ratio is below the minimum required ratio |
/// | 36   | `OraclePriceInvalid`          | Oracle        | Oracle price is invalid (zero, negative, or malformed) |
/// | 37   | `OraclePriceStale`            | Oracle        | Oracle price is stale (exceeds max_age_seconds) |
/// | 38   | `OraclePriceDeviation`        | Oracle        | Oracle price deviation exceeds the configured maximum |
/// | 39   | `InsufficientCollateralBalance` | Collateral  | Borrower collateral balance cannot cover withdrawal |
/// | 40   | `BorrowerFrozen`               | Block         | Borrower's draws are temporarily frozen until expiry |
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
    /// The requested repay exceeds the configured per-transaction maximum.
    RepayExceedsMaxAmount = 28,
    /// Borrower attempted to draw again before the cooldown interval elapsed.
    DrawCooldownActive = 29,
    /// Treasury address is not configured when attempting a treasury withdrawal.
    TreasuryNotSet = 30,
    /// Draw would exceed the global protocol exposure cap.
    ExposureCapExceeded = 31,
    /// Admin address has not been initialized in contract storage.
    AdminNotInitialized = 32,
    /// Timestamp regression detected (new timestamp is not greater than stored timestamp).
    TimestampRegression = 33,
    /// Credit limit is outside the configured minimum/maximum bounds.
    LimitOutOfBounds = 34,
    /// Collateral ratio is below the minimum required ratio.
    CollateralRatioBelowMinimum = 35,
    /// Oracle price is invalid (zero, negative, or malformed).
    OraclePriceInvalid = 36,
    /// Oracle price is stale (exceeds max_age_seconds).
    OraclePriceStale = 37,
    /// Oracle price deviation exceeds the configured maximum.
    OraclePriceDeviation = 38,
    /// Borrower's collateral balance is below the requested withdrawal amount.
    InsufficientCollateralBalance = 39,
    /// Borrower's draws are temporarily frozen until the specified expiry timestamp.
    BorrowerFrozen = 40,
    /// Credit line draws are frozen by admin with a structured [`FreezeReason`].
    CreditLineFrozen = 41,
}

/// Structured taxonomy for credit-line and global draw freezes.
///
/// # Discriminant stability
/// Discriminants are part of the contract ABI. New variants must be appended;
/// existing values must never be reordered or renumbered.
///
/// # Usage
/// - [`crate::freeze::freeze_draws`] records a global reason alongside the
///   contract-wide draw kill-switch.
/// - [`crate::freeze::freeze_credit_line`] records a per-borrower reason without
///   mutating [`CreditStatus`], preserving lifecycle history for indexers.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FreezeReason {
    /// Scheduled reserve or treasury operations affecting draw liquidity.
    LiquidityReserve = 0,
    /// Regulatory or compliance-mandated draw pause.
    Compliance = 1,
    /// Active risk investigation or off-chain risk signal.
    RiskInvestigation = 2,
    /// Planned operational maintenance window.
    OperationalMaintenance = 3,
    /// Borrower-initiated voluntary draw pause.
    BorrowerRequest = 4,
}

/// Global draw-freeze state stored under [`crate::storage::DataKey::DrawsFrozen`].
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DrawsFreezeState {
    /// Whether draws are currently frozen contract-wide.
    pub frozen: bool,
    /// Structured reason recorded when the freeze was last activated.
    pub reason: FreezeReason,
}

/// Stable category codes for [`ContractError`] variants.
///
/// Each category groups semantically related errors that share a common
/// SDK-side recovery action. See [`docs/error-taxonomy.md`](../../../docs/error-taxonomy.md)
/// for the full categorized reference.
///
/// # Stability guarantee
/// Discriminants are **permanent**. Never reorder or renumber existing
/// variants — doing so would break deployed SDK clients. New categories
/// must be appended at the end with the next available integer.
///
/// # Discriminant table
/// | Code | Category      | Variants |
/// |------|---------------|----------|
/// | 1    | `Auth`        | `Unauthorized`, `NotAdmin`, `AdminNotInitialized` |
/// | 2    | `Lifecycle`   | `CreditLineClosed`, `AlreadyInitialized`, `CreditLineSuspended`, `CreditLineDefaulted` |
/// | 3    | `Numeric`     | `InvalidAmount`, `NegativeLimit`, `Overflow`, `TimestampRegression`, `LimitOutOfBounds` |
/// | 4    | `Limit`       | `OverLimit`, `UtilizationNotZero`, `LimitDecreaseRequiresRepayment`, `DrawExceedsMaxAmount`, `RepayExceedsMaxAmount` |
/// | 5    | `Liquidity`   | `MissingLiquidityToken`, `MissingLiquiditySource`, `InsufficientLiquidityReserve`, `LiquidityTokenCallFailed`, `InsufficientRepaymentAllowance`, `InsufficientRepaymentBalance`, `TreasuryNotSet`, `ExposureCapExceeded` |
/// | 6    | `Risk`        | `RateTooHigh`, `ScoreTooHigh`, `Paused`, `DrawCooldownActive` |
/// | 7    | `Oracle`      | `OraclePriceInvalid`, `OraclePriceStale`, `OraclePriceDeviation` |
/// | 8    | `Collateral`  | `CollateralRatioBelowMinimum`, `InsufficientCollateralBalance` |
/// | 9    | `Block`       | `BorrowerBlocked`, `DrawsFrozen`, `BorrowerFrozen` |
/// | 10   | `Reentrancy`  | `Reentrancy` |
/// | 11   | `Misc`        | `CreditLineNotFound`, `AdminAcceptTooEarly` |
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ContractErrorCategory {
    /// Authorization failures (wrong caller, missing admin).
    Auth = 1,
    /// Credit-line lifecycle state violations.
    Lifecycle = 2,
    /// Numeric validation or arithmetic errors.
    Numeric = 3,
    /// Credit limit and per-transaction cap violations.
    Limit = 4,
    /// Liquidity, reserve, and treasury availability failures.
    Liquidity = 5,
    /// Risk-score, rate, pause, and cooldown violations.
    Risk = 6,
    /// Oracle price-feed circuit-breaker failures.
    Oracle = 7,
    /// Collateral ratio or balance violations.
    Collateral = 8,
    /// Borrower-blocked or draw-freeze state.
    Block = 9,
    /// Reentrancy guard triggered.
    Reentrancy = 10,
    /// Miscellaneous errors that do not fit a specific category.
    Misc = 11,
}

impl ContractError {
    /// Return the stable category for this error variant.
    ///
    /// Clients can use this to group errors for UI display, analytics,
    /// or recovery heuristics without hard-coding variant-to-category
    /// mappings.
    pub fn category(&self) -> ContractErrorCategory {
        match self {
            Self::Unauthorized | Self::NotAdmin | Self::AdminNotInitialized => {
                ContractErrorCategory::Auth
            }
            Self::CreditLineClosed
            | Self::AlreadyInitialized
            | Self::CreditLineSuspended
            | Self::CreditLineDefaulted => ContractErrorCategory::Lifecycle,
            Self::InvalidAmount
            | Self::NegativeLimit
            | Self::Overflow
            | Self::TimestampRegression
            | Self::LimitOutOfBounds => ContractErrorCategory::Numeric,
            Self::OverLimit
            | Self::UtilizationNotZero
            | Self::LimitDecreaseRequiresRepayment
            | Self::DrawExceedsMaxAmount
            | Self::RepayExceedsMaxAmount => ContractErrorCategory::Limit,
            Self::MissingLiquidityToken
            | Self::MissingLiquiditySource
            | Self::InsufficientLiquidityReserve
            | Self::LiquidityTokenCallFailed
            | Self::InsufficientRepaymentAllowance
            | Self::InsufficientRepaymentBalance
            | Self::TreasuryNotSet
            | Self::ExposureCapExceeded => ContractErrorCategory::Liquidity,
            Self::RateTooHigh | Self::ScoreTooHigh | Self::Paused | Self::DrawCooldownActive => {
                ContractErrorCategory::Risk
            }
            Self::OraclePriceInvalid | Self::OraclePriceStale | Self::OraclePriceDeviation => {
                ContractErrorCategory::Oracle
            }
            Self::CollateralRatioBelowMinimum | Self::InsufficientCollateralBalance => {
                ContractErrorCategory::Collateral
            }
            Self::BorrowerBlocked | Self::DrawsFrozen | Self::BorrowerFrozen => {
                ContractErrorCategory::Block
            }
            Self::Reentrancy => ContractErrorCategory::Reentrancy,
            Self::CreditLineNotFound | Self::AdminAcceptTooEarly => ContractErrorCategory::Misc,
        }
    }
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

/// Optional installment repayment schedule attached to a credit line.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RepaymentSchedule {
    /// Required repayment amount for each installment period.
    pub amount_per_period: i128,
    /// Duration of a single installment period in seconds.
    pub period_seconds: u64,
    /// Timestamp at which the next installment is due.
    pub next_due_ts: u64,
}

/// Admin-configurable limits on interest-rate changes.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
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

/// Oracle circuit-breaker configuration.
///
/// When set, `settle_default_liquidation` validates the supplied `oracle_price`
/// against the last accepted price and the current ledger timestamp before
/// applying the settlement.
///
/// # Invariants
/// - `max_deviation_bps` must be in `1..=10_000` (0.01 % – 100 %).
/// - `max_age_seconds` must be > 0.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OracleConfig {
    /// Maximum allowed price deviation from the last accepted price, in basis points.
    /// E.g. 500 = 5 %.
    pub max_deviation_bps: u32,
    /// Maximum age of an oracle price in seconds before it is considered stale.
    pub max_age_seconds: u64,
}

/// Event emitted when the rate formula config is set or cleared.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RateFormulaConfigEvent {
    /// `true` when a config was set; `false` when cleared.
    pub enabled: bool,
}

/// Global protocol configuration.
///
/// A projection of the instance-storage keys
/// [`crate::storage::DataKey::LiquidityToken`] and
/// [`crate::storage::DataKey::LiquiditySource`], returned by
/// `get_protocol_config` for integrators who need to inspect both
/// values in a single call.
///
/// Either field may be `None` if the corresponding key has not been set; in
/// that case the relevant entrypoints panic with
/// [`ContractError::MissingLiquidityToken`] or
/// [`ContractError::MissingLiquiditySource`].
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtocolConfig {
    /// Configured liquidity token.
    pub liquidity_token: Option<Address>,
    /// Configured liquidity source.
    pub liquidity_source: Option<Address>,
}

/// Global protocol aggregate balances.
///
/// Returned by `get_protocol_summary` as a Soroban ABI value.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtocolSummary {
    /// Number of indexed credit lines.
    pub count: u32,
    /// Global utilized principal accumulator.
    pub total_utilized: i128,
    /// Global collateral balance accumulator.
    pub total_collateral: i128,
    /// Accumulated protocol fees awaiting treasury withdrawal.
    pub treasury_balance: i128,
}

/// Protocol summary returned by the specific query view for GrantFox campaign.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtocolSummaryView {
    /// Global utilized principal accumulator.
    pub total_utilized: i128,
    /// Global collateral balance accumulator.
    pub total_collateral: i128,
    /// Count of currently Active credit lines.
    pub active_line_count: u32,
}
