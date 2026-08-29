// SPDX-License-Identifier: MIT
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(coverage_nightly, coverage(off))]

//! Core data types for the Creditra contract.
//!
//! # What
//!
//! ABI-stable types that cross the contract boundary:
//!
//! - [`ContractError`] — 54-variant `#[repr(u32)]` error enum (discriminants
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
/// - `Suspended` (admin) and `SelfSuspended` (borrower) both block draws and
///   allow repayments; they are distinct for auditability and authorization —
///   see the module docs for `lifecycle::suspend_credit_line` vs
///   `lifecycle::self_suspend_credit_line`.
/// - `Defaulted` blocks draws and allows repayments for cure.
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
    /// Credit line is voluntarily frozen by the borrower. Draws blocked,
    /// repayments allowed. Distinct from `Suspended` (admin-initiated) for
    /// least-privilege: the borrower can self-unsuspend, while an admin
    /// suspension requires admin unsuspend.
    SelfSuspended = 5,
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
/// | 41   | `BountyNotSet`                 | Liquidity     | Bounty pool address is not configured |
/// | 42   | `NoPendingTreasuryWithdrawal`  | Misc          | No pending treasury withdrawal proposal exists |
/// | 43   | `TreasuryTimelockActive`       | Misc          | Treasury withdrawal timelock has not yet elapsed |
/// | 44   | `TreasuryProposalExists`       | Misc          | A treasury withdrawal proposal already exists |
/// | 45   | `CloseFactorAboveMax`          | Limit         | The supplied close_factor_bps exceeds the protocol maximum |
/// | 46   | `CreditLineFrozen`             | Block         | Credit line draws are frozen by admin (compliance hold) |
/// | 47   | `DrawReversalWindowExpired`    | Limit         | Draw reversal attempted after the allowed window expired |
/// | 48   | `OriginalDrawNotFound`         | Misc          | Original draw record not found for reversal |
/// | 49   | `AttestationBatchNotFound`     | Misc          | No attestation batch has been committed |
/// | 50   | `OracleQuorumNotMet`           | Oracle        | Oracle quorum condition not satisfied |
/// | 51   | `AlreadySettled`               | Lifecycle     | Liquidation settlement already processed for this (borrower, id) pair |
/// | 52   | `InvalidRiskWeight`            | Numeric       | Collateral risk weight exceeds the maximum allowed (10 000 bps) |
/// | 53   | `InvalidAttestation`           | Misc          | Attestation proof is invalid or no attestation batch has been committed |
/// | 54   | `RiskAdminCooldownActive`      | Risk          | Risk admin cooldown has not yet elapsed since the last mutation |
/// | 60   | `StaleStateTransition`         | Lifecycle     | Transition rejected: the credit line is already in the requested target state |
/// | 61   | `IncompatibleVersion`          | Handshake     | Auction contract protocol version is incompatible with credit contract |
/// | 62   | `AuctionCallFailed`            | Handshake     | Cross-contract auction CPI call failed or returned an unexpected value |
// `export = false`: this enum is intentionally huge and is used as the
// protocol's internal Rust error vocabulary for stable discriminants, but it
// is not meant to be exported through the generated Soroban contract spec.
// The SDK enforces a strict case cap on exported error UDTs, and this enum has
// already outgrown that limit. Keeping the ABI of the public contract stable
// for clients is still preserved by the fixed numeric discriminants.
#[soroban_sdk::contracterror(export = false)]
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
    // discriminant 13 reserved (LimitDecreaseRequiresRepayment, removed)
    AlreadyInitialized = 14,
    AdminAcceptTooEarly = 15,
    /// Borrower address does not match the credit line's registered borrower.
    BorrowerBlocked = 16,
    DrawExceedsMaxAmount = 17,
    Paused = 18,
    DrawsFrozen = 19,
    CreditLineSuspended = 20,
    CreditLineDefaulted = 21,
    MissingLiquidityToken = 22,
    MissingLiquiditySource = 23,
    InsufficientLiquidityReserve = 24,
    /// Liquidity token transfer call failed (observable error path).
    LiquidityTokenCallFailed = 25,
    /// Borrower repayment allowance is insufficient to cover the repayment amount.
    InsufficientRepaymentAllowance = 26,
    /// Borrower balance is insufficient to cover the repayment amount.
    InsufficientRepaymentBalance = 27,
    RepayExceedsMaxAmount = 28,
    DrawCooldownActive = 29,
    TreasuryNotSet = 30,
    ExposureCapExceeded = 31,
    AdminNotInitialized = 32,
    TimestampRegression = 33,
    LimitOutOfBounds = 34,
    CollateralRatioBelowMinimum = 35,
    OraclePriceInvalid = 36,
    OraclePriceStale = 37,
    OraclePriceDeviation = 38,
    InsufficientCollateralBalance = 39,
    BorrowerFrozen = 40,
    BountyNotSet = 41,
    NoPendingTreasuryWithdrawal = 42,
    /// Treasury withdrawal timelock has not yet elapsed since proposal.
    TreasuryTimelockActive = 43,
    /// A treasury withdrawal proposal already exists; cancel or execute it first.
    TreasuryProposalExists = 44,
    /// The supplied `close_factor_bps` exceeds the protocol-configured maximum.
    CloseFactorAboveMax = 45,
    CreditLineFrozen = 46,
    DrawReversalWindowExpired = 47,
    OriginalDrawNotFound = 48,
    AttestationBatchNotFound = 49,
    OracleQuorumNotMet = 50,
    AlreadySettled = 51,
    InvalidRiskWeight = 52,
    InvalidAttestation = 53,
    RiskAdminCooldownActive = 54,
    OracleNotFound = 55,
    // discriminant 56 reserved
    FreezeCooldownActive = 57,
    AdminCollateralCooldownActive = 58,
    LiquidationGraceActive = 59,
    /// Transition rejected because the credit line is already in the requested
    /// target state (stale/duplicate).
    StaleStateTransition = 60,
    /// Auction contract's protocol version does not match the credit contract.
    ///
    /// The version handshake check failed before any state mutation occurred.
    /// The reentrancy guard has been cleared; the settlement is safe to retry
    /// once the auction or credit contract is upgraded to a compatible version.
    IncompatibleVersion = 61,
    /// The cross-contract auction CPI call failed or returned an unexpected value.
    ///
    /// No credit-line state was mutated. The reentrancy guard has been cleared.
    /// The settlement is safe to retry with a corrected `recovered_amount` or
    /// after the auction contract issue is resolved.
    AuctionCallFailed = 62,
}

/// Stable category grouping for [`ContractError`] variants.
///
/// # Stability guarantee
/// Discriminants are permanent. New variants must be appended; existing values
/// must never be reordered or renumbered.
///
/// | Code | Category    | Description |
/// |------|-------------|-------------|
/// | 1    | `Auth`      | Authorization and authentication errors |
/// | 2    | `Lifecycle` | Credit-line state-machine transition errors |
/// | 3    | `Numeric`   | Arithmetic and bounds errors |
/// | 4    | `Limit`     | Per-transaction, exposure, or protocol-cap errors |
/// | 5    | `Liquidity` | Token transfer, reserve, and treasury errors |
/// | 6    | `Risk`      | Rate, score, and cooldown errors |
/// | 7    | `Oracle`    | Price-feed validity and quorum errors |
/// | 8    | `Collateral`| Collateral ratio and balance errors |
/// | 9    | `Block`     | Freeze, block, and borrower-control errors |
/// | 10   | `Reentrancy`| Reentrancy detection errors |
/// | 11   | `Misc`      | Miscellaneous errors not belonging elsewhere |
/// | 12   | `Handshake` | Cross-contract version and CPI call errors |
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ContractErrorCategory {
    Auth = 1,
    Lifecycle = 2,
    Numeric = 3,
    Limit = 4,
    Liquidity = 5,
    Risk = 6,
    Oracle = 7,
    Collateral = 8,
    Block = 9,
    Reentrancy = 10,
    Misc = 11,
    /// Cross-contract version negotiation and CPI call errors.
    Handshake = 12,
}

impl ContractError {
    /// Return the stable category for this error variant.
    pub fn category(&self) -> ContractErrorCategory {
        use ContractErrorCategory::*;
        match self {
            Self::Unauthorized
            | Self::NotAdmin
            | Self::AdminNotInitialized => Auth,

            Self::CreditLineClosed
            | Self::AlreadyInitialized
            | Self::CreditLineSuspended
            | Self::CreditLineDefaulted
            | Self::AlreadySettled
            | Self::LiquidationGraceActive
            | Self::StaleStateTransition => Lifecycle,

            Self::InvalidAmount
            | Self::NegativeLimit
            | Self::Overflow
            | Self::TimestampRegression
            | Self::LimitOutOfBounds
            | Self::InvalidRiskWeight => Numeric,

            Self::OverLimit
            | Self::UtilizationNotZero
            | Self::DrawExceedsMaxAmount
            | Self::RepayExceedsMaxAmount
            | Self::DrawReversalWindowExpired
            | Self::CloseFactorAboveMax => Limit,

            Self::MissingLiquidityToken
            | Self::MissingLiquiditySource
            | Self::InsufficientLiquidityReserve
            | Self::LiquidityTokenCallFailed
            | Self::InsufficientRepaymentAllowance
            | Self::InsufficientRepaymentBalance
            | Self::TreasuryNotSet
            | Self::ExposureCapExceeded
            | Self::BountyNotSet => Liquidity,

            Self::RateTooHigh
            | Self::ScoreTooHigh
            | Self::Paused
            | Self::DrawCooldownActive
            | Self::RiskAdminCooldownActive => Risk,

            Self::OraclePriceInvalid
            | Self::OraclePriceStale
            | Self::OraclePriceDeviation
            | Self::OracleQuorumNotMet
            | Self::OracleNotFound => Oracle,

            Self::CollateralRatioBelowMinimum
            | Self::InsufficientCollateralBalance
            | Self::AdminCollateralCooldownActive => Collateral,

            Self::DrawsFrozen
            | Self::BorrowerFrozen
            | Self::BorrowerBlocked
            | Self::CreditLineFrozen
            | Self::FreezeCooldownActive => Block,

            Self::Reentrancy => Reentrancy,

            Self::CreditLineNotFound
            | Self::AdminAcceptTooEarly
            | Self::NoPendingTreasuryWithdrawal
            | Self::TreasuryTimelockActive
            | Self::TreasuryProposalExists
            | Self::OriginalDrawNotFound
            | Self::AttestationBatchNotFound
            | Self::InvalidAttestation => Misc,

            // Cross-contract handshake errors — guard is always cleared before
            // these are emitted so the settlement path is safe to retry.
            Self::IncompatibleVersion | Self::AuctionCallFailed => Handshake,
        }
    }
}

/// Configuration emitted when the risk admin cooldown is set or changed.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RiskAdminCooldownConfig {
    /// New cooldown duration in seconds. `0` means disabled.
    pub cooldown_seconds: u64,
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

/// Structured kind discriminant for collateral lifecycle events.
///
/// # Discriminant stability
/// Same rule as [`FreezeReason`] / [`CreditStatus`]: discriminants are part
/// of the contract ABI and must never be reordered or renumbered; new
/// variants must be appended.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CollateralEventKind {
    /// Collateral tokens deposited into the contract.
    Deposited = 0,
    /// Collateral tokens withdrawn by the borrower (generic withdrawal path).
    Withdrawn = 1,
    /// Collateral tokens released via the health-factor-gated partial release path.
    PartiallyReleased = 2,
    /// Collateral tokens released internally as part of an atomic repay+release flow.
    Released = 3,
}

/// Persisted state for the global draw-freeze switch ([`crate::storage::DataKey::DrawsFrozen`]).
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DrawsFreezeState {
    /// `true` when all draws are currently frozen.
    pub frozen: bool,
    /// Structured reason recorded on the most recent freeze/unfreeze action.
    pub reason: FreezeReason,
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

/// Aggregated, single-call read-only view of a borrower's full credit-line state.
///
/// Assembles [`CreditLineData`], collateral balance, health factor, repayment
/// schedule, and delinquency status in one call, avoiding the multiple
/// round-trips a caller would otherwise need for `get_credit_line` +
/// `get_collateral` + `get_health_factor` + `get_repayment_schedule` +
/// `is_delinquent`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreditLineSnapshot {
    /// The core credit line record.
    pub line: CreditLineData,
    /// Collateral balance for the borrower (single-token collateral path).
    pub collateral_balance: i128,
    /// Collateral-aware health factor in basis points. `u32::MAX` when
    /// `utilized_amount == 0`.
    pub health_factor_bps: u32,
    /// The borrower's installment repayment schedule, if configured.
    /// Empty when no schedule is set; exactly one element when a schedule
    /// is configured. Represented as a `Vec` rather than `Option<T>`
    /// because the Soroban SDK's struct-field XDR codegen does not support
    /// `Option<CustomStruct>` fields (`Option<T>` requires `T: Into<ScVal>`,
    /// which `#[contracttype]`-derived UDTs only implement fallibly).
    pub repayment_schedule: soroban_sdk::Vec<RepaymentSchedule>,
    /// `true` when the borrower is past the delinquency grace window.
    pub is_delinquent: bool,
}

/// Read-only capabilities bitmap for a borrower's credit line.
///
/// Returned by `borrow_capabilities` to let off-chain clients and
/// on-chain integrators inspect which operations are currently
/// permitted for a borrower, without needing to simulate the full
/// entrypoint logic.
///
/// Each `bool` field represents a single operation; `true` means the
/// operation should succeed assuming valid parameters (amount, etc.).
/// Amount-dependent checks (credit limit, collateral ratio, draw
/// cooldown, exposure caps) are NOT evaluated because this view does
/// not know the intended draw amount.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BorrowCapabilities {
    /// Whether the borrower can draw credit. False when the credit line
    /// does not exist, the protocol is paused, draws are frozen, the
    /// borrower is blocked/frozen, or the credit-line status is not
    /// draw-allowed (Active/Restricted).
    pub can_draw: bool,
    /// Whether the borrower can repay credit. False when the credit line
    /// does not exist or is permanently Closed.
    pub can_repay: bool,
    /// Whether the borrower can self-suspend their credit line. True
    /// only when the credit line exists and is currently Active.
    pub can_self_suspend: bool,
}

/// Read-only capabilities bitmap for a credit line's lifecycle transitions (v7).
///
/// Returned by the `lifecycle_capabilities` view to let off-chain clients and
/// on-chain integrators inspect which lifecycle transitions are currently
/// permitted for a borrower's credit line, without simulating the full
/// entrypoint logic (state lookup + status checks + pause guard).
///
/// Every field is derived purely from the credit line's current
/// [`CreditStatus`] and the protocol pause flag — no token CPIs, no auth
/// checks, no mutation. See [`crate::lifecycle`] for the authoritative
/// transition rules each field mirrors.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LifecycleCapabilities {
    /// Whether an admin can suspend this line via `suspend_credit_line`.
    /// True only when the line exists, the protocol is not paused, and the
    /// status is [`CreditStatus::Active`].
    pub can_suspend: bool,
    /// Whether the borrower can self-suspend via `self_suspend_credit_line`.
    /// Same precondition as `can_suspend` (only `Active` self-suspends).
    pub can_self_suspend: bool,
    /// Whether an admin can force-close this line via `close_credit_line`
    /// unconditionally (any non-`Closed` status). False when the line does
    /// not exist, the protocol is paused, or the status is already `Closed`.
    pub can_close_admin: bool,
    /// Whether the borrower can self-close via `close_credit_line`. Requires
    /// the same preconditions as `can_close_admin` **plus**
    /// `utilized_amount == 0`.
    pub can_close_borrower: bool,
    /// Whether an admin can move this line to `Defaulted` via
    /// `default_credit_line`. True when the line exists, the protocol is not
    /// paused, and the status is `Active`, `Restricted`, or `Suspended`.
    pub can_default: bool,
    /// Whether an admin can cure this line via `reinstate_credit_line`. True
    /// only when the line exists, the protocol is not paused, and the status
    /// is `Defaulted`.
    pub can_reinstate: bool,
}

/// Read-only capabilities bitmap for the query (v7) subsystem.
///
/// Returned by `query_capabilities` / the `capabilities` anchor in
/// `contracts/query/src/views.rs` so off-chain clients and keepers can
/// inspect which borrower-scoped query results are currently meaningful
/// without issuing multiple separate reads.
///
/// Every field is derived purely from storage — no token CPIs, no auth
/// checks, no mutation.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueryCapabilities {
    /// `get_credit_line` returns `Some` for this borrower.
    pub has_credit_line: bool,
    /// `get_repayment_schedule` returns `Some` for this borrower.
    pub has_repayment_schedule: bool,
    /// Health factor is debt-sensitive (`utilized_amount > 0`). When
    /// `false`, `get_health_factor` returns `u32::MAX` (no outstanding debt).
    pub health_factor_applicable: bool,
    /// Delinquency checks can return `true` (open line with utilization and
    /// a configured repayment schedule). Mirrors the short-circuit gates in
    /// [`crate::query::is_delinquent`].
    pub delinquency_applicable: bool,
    /// Current delinquency status from [`crate::query::is_delinquent`].
    /// Always `false` when `delinquency_applicable` is `false`.
    pub is_delinquent: bool,
}

/// Proof-of-reserve view for the protocol treasury.
///
/// Exposes the accumulated reserves held by the protocol in a single
/// read-only call. Indexers and dashboards can use this to verify that
/// the protocol's accounting balances are consistent.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProofOfReserve {
    /// Accumulated protocol fees held in the contract (treasury share).
    pub treasury_balance: i128,
    /// Accumulated bounty pool fees held in the contract.
    pub bounty_balance: i128,
}

/// Paginated view of credit lines for off-chain reporting.
///
/// Returned by `get_credit_lines_paginated` to enable efficient navigation
/// through large sets of credit lines using cursor-based pagination.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreditLinesPage {
    /// Vector of credit line data for this page.
    pub lines: soroban_sdk::Vec<CreditLineData>,
    /// Cursor for the next page, or `None` if this is the last page.
    pub next_cursor: Option<u32>,
    /// Whether more results are available beyond this page.
    pub has_more: bool,
}

/// Oracle quorum configuration for multi-oracle price feeds.
///
/// Used by `set_oracle_quorum_config` to configure the quorum threshold,
/// deviation bound, and staleness window for the quorum-of-K price
/// resolution algorithm.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleQuorumConfig {
    /// Minimum number of oracle prices that must agree within `max_deviation_bps`.
    pub min_quorum_k: u32,
    /// Maximum allowed deviation between the lowest and highest price in a
    /// qualifying window, in basis points.
    pub max_deviation_bps: u32,
    /// Maximum age of a quorum price in seconds before it is considered stale.
    pub max_age_seconds: u64,
}

/// Full state snapshot for a borrower's credit line.
///
/// Returned by `get_borrow_state` to provide a comprehensive view of the
/// borrower's current state in a single read-only call. This includes
/// credit line data, collateral balance, and borrow capabilities.
///
/// # Field encoding
/// `credit_line` is a `Vec` with 0 or 1 element rather than `Option<T>`
/// because the Soroban SDK's `#[contracttype]` XDR codegen does not support
/// `Option<CustomStruct>` for `#[contracttype]`-derived UDTs.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BorrowStateSnapshot {
    /// The full credit line data if it exists, or an empty vec.
    pub credit_line: soroban_sdk::Vec<CreditLineData>,
    /// The borrower's collateral balance.
    pub collateral_balance: i128,
    /// The borrower's current borrow capabilities.
    pub capabilities: BorrowCapabilities,
}


/// A pending treasury withdrawal proposal created by `propose_treasury_withdrawal`.
///
/// Exactly one proposal can exist at a time. It must be executed (or superseded
/// only after a successful `execute_treasury_withdrawal` clears it) no sooner
/// than 24 hours after it was proposed.
///
/// # Timelock
/// `execute_after` is set to `proposal_ts + 86_400` (24 hours in seconds) at
/// proposal time. The execution entrypoint rejects calls when
/// `env.ledger().timestamp() < execute_after`.
///
/// # Storage
/// Stored in instance storage under [`crate::storage::DataKey::PendingTreasuryWithdrawal`].
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreasuryWithdrawalProposal {
    /// The treasury address that will receive the funds.
    pub recipient: Address,
    /// Amount to transfer (snapshot of `TreasuryBalance` at proposal time).
    pub amount: i128,
    /// Address of the admin who submitted the proposal.
    pub proposer: Address,
    /// Ledger timestamp at which the proposal was created.
    pub proposed_at: u64,
    /// Earliest ledger timestamp at which execution is permitted (`proposed_at + 86_400`).
    pub execute_after: u64,
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
