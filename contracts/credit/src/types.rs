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
/// | 28   | `RepayExceedsMaxAmount`        | Repay amount exceeds per-transaction cap |
/// | 29   | `DrawCooldownActive`          | Borrower attempted to draw before cooldown elapsed |
/// | 30   | `TreasuryNotSet`              | Treasury address is not configured |
/// | 31   | `ExposureCapExceeded`         | Draw would exceed the global protocol exposure cap |
/// | 32   | `AdminNotInitialized`         | Admin address has not been initialized |
/// | 33   | `TimestampRegression`         | Timestamp regression detected |
/// | 34   | `LimitOutOfBounds`            | Credit limit is outside configured min/max bounds |
/// | 35   | `CollateralRatioBelowMinimum` | Collateral ratio is below the minimum required ratio |
/// | 36   | `OraclePriceInvalid`          | Oracle price is invalid (zero, negative, or malformed) |
/// | 37   | `OraclePriceStale`            | Oracle price is stale (exceeds max_age_seconds) |
/// | 38   | `OraclePriceDeviation`        | Oracle price deviation exceeds the configured maximum |
/// | 39   | `InsufficientCollateralBalance` | Borrower collateral balance cannot cover withdrawal |
/// | 40   | `BorrowerFrozen`               | Borrower's draws are temporarily frozen until expiry |
/// | 41   | `DrawReversalWindowExpired`     | Draw reversal window has expired |
/// | 42   | `OriginalDrawNotFound`           | Original draw record not found for borrower |
#[soroban_sdk::contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ContractError {
    /// Caller is not authorized to perform the requested action.
    Unauthorized = 1,
    /// Caller does not have admin privileges for this entrypoint.
    NotAdmin = 2,
    /// The requested borrower does not have an open credit line.
    CreditLineNotFound = 3,
    /// The credit line is permanently closed and cannot accept new activity.
    CreditLineClosed = 4,
    /// The requested amount is zero, negative, or otherwise invalid for this operation.
    InvalidAmount = 5,
    /// The requested draw would exceed the available credit line limit.
    OverLimit = 6,
    /// The proposed credit limit cannot be negative.
    NegativeLimit = 7,
    /// The requested rate exceeds the maximum allowed delta for the borrower.
    RateTooHigh = 8,
    /// The supplied risk score is above the acceptable maximum threshold.
    ScoreTooHigh = 9,
    /// The requested transition requires zero outstanding utilization first.
    UtilizationNotZero = 10,
    /// Reentrant execution was detected during a state-changing call.
    Reentrancy = 11,
    /// An arithmetic operation overflowed or underflowed during contract logic.
    Overflow = 12,
    /// Lowering the credit limit would leave outstanding debt above the new cap.
    LimitDecreaseRequiresRepayment = 13,
    /// The contract has already been initialized and cannot be initialized twice.
    AlreadyInitialized = 14,
    /// The oracle quorum requirement was not satisfied for the requested operation.
    QuorumNotMet = 15,
    /// The referenced oracle has not been registered in the configured registry.
    OracleNotFound = 16,
    /// The referenced oracle is already registered and cannot be re-added.
    OracleAlreadyExists = 17,
    /// Admin role acceptance was attempted before the required delay window elapsed.
    AdminAcceptTooEarly = 15,
    /// The borrower is blocked from drawing credit.
    BorrowerBlocked = 16,
    /// The requested draw exceeds the configured per-transaction maximum.
    DrawExceedsMaxAmount = 17,
    /// The protocol is currently paused and the requested action is blocked.
    Paused = 18,
    /// Global draws are temporarily frozen by admin for the protocol.
    DrawsFrozen = 19,
    /// The credit line is suspended and cannot accept the requested action.
    CreditLineSuspended = 20,
    /// The credit line is in default and requires cure or liquidation.
    CreditLineDefaulted = 21,
    /// The liquidity token address has not been configured for the contract.
    MissingLiquidityToken = 22,
    /// The liquidity source address has not been configured for the contract.
    MissingLiquiditySource = 23,
    /// The configured reserve balance cannot cover the requested draw.
    InsufficientLiquidityReserve = 24,
    /// The liquidity token call failed in a way that the contract could observe.
    LiquidityTokenCallFailed = 25,
    /// The borrower's token allowance is below the effective repayment amount.
    InsufficientRepaymentAllowance = 26,
    /// The borrower's token balance is below the effective repayment amount.
    InsufficientRepaymentBalance = 27,
    /// The repayment amount exceeds the configured per-transaction maximum.
    RepayExceedsMaxAmount = 28,
    /// The borrower attempted to draw again before the cooldown interval elapsed.
    DrawCooldownActive = 29,
    /// The treasury address is not configured for the requested withdrawal flow.
    TreasuryNotSet = 30,
    /// The requested draw would exceed the global protocol exposure cap.
    ExposureCapExceeded = 31,
    /// The admin address has not been initialized in contract storage.
    AdminNotInitialized = 32,
    /// A timestamp write regressed relative to the previously stored value.
    TimestampRegression = 33,
    /// The proposed credit limit falls outside the configured min/max bounds.
    LimitOutOfBounds = 34,
    /// The collateral ratio is below the minimum required threshold.
    CollateralRatioBelowMinimum = 35,
    /// The oracle price is zero, negative, or malformed and cannot be used.
    OraclePriceInvalid = 36,
    /// The oracle price is older than the configured freshness window.
    OraclePriceStale = 37,
    /// The oracle price deviates beyond the configured threshold.
    OraclePriceDeviation = 38,
    /// The borrower's collateral balance is insufficient for the requested operation.
    InsufficientCollateralBalance = 39,
    /// The borrower is temporarily frozen from drawing until the expiry timestamp.
    BorrowerFrozen = 40,
    /// Draw reversal window has expired (admin must reverse within DRAW_REVERSAL_WINDOW_SECS).
    DrawReversalWindowExpired = 41,
    /// Original draw audit record not found for the specified (borrower, timestamp) pair.
    OriginalDrawNotFound = 42,
    /// The borrower's credit line has an admin freeze with a structured reason; draws are blocked.
    CreditLineFrozen = 43,
    /// Bounty address has not been configured when attempting a bounty withdrawal.
    BountyNotSet = 44,
}

/// Stable category grouping for [`ContractError`] variants.
///
/// Each category groups semantically related errors that share a common
/// SDK-side recovery action. See [`docs/error-taxonomy.md`](../../../docs/error-taxonomy.md)
/// for the full categorized reference.
///
/// # Stability guarantee
/// These discriminants are **permanent**. Never reorder or renumber existing
/// variants — doing so would break deployed SDK clients that match on
/// category codes. New variants must be appended at the end.
///
/// # Usage
/// Use [`ContractError::category`] to map any error to its category at
/// runtime. This allows SDK clients to group errors by category without
/// matching on individual error codes.

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
            | Self::ExposureCapExceeded
            | Self::BountyNotSet => ContractErrorCategory::Liquidity,
            Self::RateTooHigh | Self::ScoreTooHigh | Self::Paused | Self::DrawCooldownActive => {
                ContractErrorCategory::Risk
            }
            Self::OraclePriceInvalid | Self::OraclePriceStale | Self::OraclePriceDeviation => {
                ContractErrorCategory::Oracle
            }
            Self::CollateralRatioBelowMinimum | Self::InsufficientCollateralBalance => {
                ContractErrorCategory::Collateral
            }
            Self::BorrowerBlocked
            | Self::DrawsFrozen
            | Self::BorrowerFrozen
            | Self::CreditLineFrozen => ContractErrorCategory::Block,
            Self::Reentrancy => ContractErrorCategory::Reentrancy,
            Self::CreditLineNotFound
            | Self::AdminAcceptTooEarly
            | Self::DrawReversalWindowExpired
            | Self::OriginalDrawNotFound => ContractErrorCategory::Misc,
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
    /// Accumulated bounty pool fees awaiting bounty withdrawal.
    pub bounty_balance: i128,
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

/// Reason for protocol pause (escape-hatch audit trail).
///
/// Stored alongside the pause flag in instance storage when the admin invokes
/// `set_protocol_paused`. Intended for governance transparency and off-chain
/// monitoring — the reason is a human-readable symbol that indexers and
/// dashboards can display to explain why the protocol is paused.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PauseReason {
    /// Human-readable reason for pausing (e.g., "oracle-outage", "token-migration").
    pub reason: soroban_sdk::Symbol,
    /// Ledger timestamp when the pause was activated.
    pub timestamp: u64,
    /// Admin address that invoked the pause.
    pub actor: soroban_sdk::Address,
}
