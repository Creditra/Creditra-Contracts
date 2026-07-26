// SPDX-License-Identifier: MIT
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(coverage_nightly, coverage(off))]

//! Core data types for the Creditra contract.
//!
//! # What
//!
//! ABI-stable types that cross the contract boundary:
//!
//! - [`ContractError`] — 52-variant `#[repr(u32)]` error enum (discriminants
//!   pinned by `tests/error_discriminants.rs`). Each variant maps to a stable
//!   [`ContractErrorCategory`] via [`ContractError::category`]. See
//!   [`docs/ERROR_CODES.md`](../../../docs/ERROR_CODES.md) for the
//!   categorized reference with codes and recovery hints.
//! - [`ContractErrorCategory`] — 11-category enum (discriminants 1–11)
//!   returned by [`ContractError::category`] for client-side grouping.
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

use soroban_sdk::{contracterror, contracttype, Address, Symbol};

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
/// Structured reason for freezing draws on a credit line.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FreezeReason {
    LiquidityReserve = 0,
    Compliance = 1,
    RiskInvestigation = 2,
    OperationalMaintenance = 3,
    BorrowerRequest = 4,
}

/// Structured state for global draw freezes.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DrawsFreezeState {
    pub frozen: bool,
    pub reason: FreezeReason,
}


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

/// Errors returned by the Credit contract.
#[soroban_sdk::contracterror(export = false)]
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
    /// Bounty pool address is not configured when attempting a bounty withdrawal.
    BountyNotSet = 41,
    /// No pending treasury withdrawal proposal exists when attempting execution.
    NoPendingTreasuryWithdrawal = 42,
    /// The 24-hour treasury withdrawal timelock has not yet elapsed.
    TreasuryTimelockActive = 43,
    /// A treasury withdrawal proposal already exists; cancel or execute it first.
    TreasuryProposalExists = 44,
    /// The supplied close_factor_bps exceeds the protocol-configured maximum.
    CloseFactorAboveMax = 45,
    /// Credit line draws are frozen by admin (compliance or investigation hold).
    CreditLineFrozen = 46,
    /// Draw reversal attempted after the allowed reversal window has expired.
    DrawReversalWindowExpired = 47,
    /// Original draw audit record not found when attempting a reversal.
    OriginalDrawNotFound = 48,
    /// No attestation batch has been committed for the specified borrower.
    AttestationBatchNotFound = 49,
    /// Oracle quorum condition was not satisfied (too few agreeing feeds).
    OracleQuorumNotMet = 50,
    /// Liquidation settlement already processed for this (borrower, settlement_id) pair.
    AlreadySettled = 51,
    /// Collateral risk weight exceeds the maximum allowed (10 000 bps).
    InvalidRiskWeight = 52,
    /// Attestation proof is invalid or no attestation batch has been committed.
    InvalidAttestation = 53,
    /// Critical borrower admin action attempted before the cooldown elapsed.
    AdminCooldownActive = 54,
    /// Per-borrower liquidation grace window has not yet elapsed.
    LiquidationGraceActive = 55,
    /// Admin freeze cooldown is active.
    FreezeCooldownActive = 56,
}

/// ABI-stable category label for [`ContractError`] variants.
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
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, Hash)]
#[repr(u32)]
pub enum ContractErrorCategory {
    /// Authentication / authorization failures.
    Auth = 1,
    /// Credit-line lifecycle state violations.
    Lifecycle = 2,
    /// Numeric computation failures (overflow, invalid input, bounds).
    Numeric = 3,
    /// Credit limit / draw / repay cap violations.
    Limit = 4,
    /// Liquidity configuration or reserve failures.
    Liquidity = 5,
    /// Risk-parameter violations (rate, score, cooldown, pause).
    Risk = 6,
    /// Oracle price-feed failures.
    Oracle = 7,
    /// Collateral ratio or balance violations.
    Collateral = 8,
    /// Draw-block conditions (blocked, frozen).
    Block = 9,
    /// Reentrancy guard violations.
    Reentrancy = 10,
    /// Miscellaneous errors (not found, admin timelock, treasury proposals).
    Misc = 11,
}

impl ContractError {
    pub const AdminCollateralCooldownActive: Self = Self::AdminCooldownActive;

    /// Map this error to its [`ContractErrorCategory`] for client-side grouping.
    ///
    /// # Example
    /// ```no_run
    /// use creditra_credit::types::{ContractError, ContractErrorCategory};
    ///
    /// assert_eq!(
    ///     ContractError::Unauthorized.category(),
    ///     ContractErrorCategory::Auth
    /// );
    /// assert_eq!(
    ///     ContractError::Overflow.category(),
    ///     ContractErrorCategory::Numeric
    /// );
    /// ```
    pub fn category(&self) -> ContractErrorCategory {
        use ContractErrorCategory::*;
        match self {
            // Auth (1)
            Self::Unauthorized => Auth,
            Self::NotAdmin => Auth,
            Self::AdminNotInitialized => Auth,
            Self::BorrowerMismatch => Auth,
            // Lifecycle (2)
            Self::CreditLineClosed => Lifecycle,
            Self::AlreadyInitialized => Lifecycle,
            Self::CreditLineSuspended => Lifecycle,
            Self::CreditLineDefaulted => Lifecycle,
            Self::AlreadySettled => Lifecycle,
            Self::LiquidationGraceActive => Lifecycle,
            // Numeric (3)
            Self::InvalidAmount => Numeric,
            Self::NegativeLimit => Numeric,
            Self::Overflow => Numeric,
            Self::TimestampRegression => Numeric,
            Self::LimitOutOfBounds => Numeric,
            Self::InvalidRiskWeight => Numeric,
            // Misc (11)
            Self::InvalidAttestation => Misc,
            // Limit (4)
            Self::OverLimit => Limit,
            Self::UtilizationNotZero => Limit,
            Self::LimitDecreaseRequiresRepayment => Limit,
            Self::DrawExceedsMaxAmount => Limit,
            Self::RepayExceedsMaxAmount => Limit,
            Self::CloseFactorAboveMax => Limit,
            Self::DrawReversalWindowExpired => Limit,
            Self::UtilizedNotZero => Limit,
            // Liquidity (5)
            Self::MissingLiquidityToken => Liquidity,
            Self::MissingLiquiditySource => Liquidity,
            Self::InsufficientLiquidityReserve => Liquidity,
            Self::LiquidityTokenCallFailed => Liquidity,
            Self::InsufficientRepaymentAllowance => Liquidity,
            Self::InsufficientRepaymentBalance => Liquidity,
            Self::TreasuryNotSet => Liquidity,
            Self::ExposureCapExceeded => Liquidity,
            Self::BountyNotSet => Liquidity,
            Self::InsufficientReserve => Liquidity,
            Self::InsufficientAllowance => Liquidity,
            Self::InsufficientBalance => Liquidity,
            // Risk (6)
            Self::RateTooHigh => Risk,
            Self::ScoreTooHigh => Risk,
            Self::Paused => Risk,
            Self::DrawCooldownActive => Risk,
            Self::AdminQueryCooldownActive => Risk,
            // Oracle (7)
            Self::OraclePriceInvalid => Oracle,
            Self::OraclePriceStale => Oracle,
            Self::OraclePriceDeviation => Oracle,
            Self::OracleQuorumNotMet => Oracle,
            Self::OracleNotFound => Oracle,
            // Collateral (8)
            Self::CollateralRatioBelowMinimum => Collateral,
            Self::InsufficientCollateralBalance => Collateral,
            // Block (9)
            Self::BorrowerBlocked => Block,
            Self::DrawsFrozen => Block,
            Self::BorrowerFrozen => Block,
            Self::CreditLineFrozen => Block,
            Self::FreezeCooldownActive => Block,
            // Reentrancy (10)
            Self::Reentrancy => Reentrancy,
            // Misc (11)
            Self::CreditLineNotFound => Misc,
            Self::AdminAcceptTooEarly => Misc,
            Self::NoPendingTreasuryWithdrawal => Misc,
            Self::TreasuryTimelockActive => Misc,
            Self::TreasuryProposalExists => Misc,
            Self::OriginalDrawNotFound => Misc,
            Self::AttestationBatchNotFound => Misc,
            // Limit (4) — continued
            Self::BorrowerExposureCapExceeded => Limit,
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

/// Multi-oracle quorum configuration.
///
/// When set, `submit_oracle_prices` runs the quorum-of-K algorithm over the
/// supplied prices before storing the resolved canonical price. Settlement via
/// `settle_default_liquidation` then only validates that this stored price is
/// still within `max_age_seconds`; the per-call deviation check is replaced by
/// the quorum consensus established at submission time.
///
/// # Invariants
/// - `min_quorum_k` must be ≥ 2 (a single feed is not a quorum).
/// - `max_deviation_bps` must be in `0..=10_000` (0 = exact match required).
/// - `max_age_seconds` must be > 0.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OracleQuorumConfig {
    /// Minimum number of submitted prices that must agree within
    /// `max_deviation_bps` to form a valid quorum.
    pub min_quorum_k: u32,
    /// Maximum allowed price deviation between the highest and lowest prices
    /// in the qualifying quorum window, in basis points.
    /// E.g. 500 = 5%.
    pub max_deviation_bps: u32,
    /// Maximum age of the stored quorum price in seconds before it is
    /// considered stale for settlement purposes.
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

/// Paginated list of credit lines returned by `get_credit_lines_paginated`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreditLinesPage {
    /// Credit lines included in this page.
    pub lines: soroban_sdk::Vec<CreditLineData>,
    /// Cursor for fetching the next page, if more items exist.
    pub next_cursor: Option<u32>,
    /// `true` if there are additional lines beyond this page.
    pub has_more: bool,
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

/// Read-only capabilities bitmap for the accrual (v7) subsystem.
///
/// Returned by [`crate::views::accrual_capabilities`] / the `capabilities()`
/// entrypoint on the accrual contract. All fields are pure boolean flags
/// derived from protocol and per-borrower state; no state is mutated.
///
/// # Bit semantics
///
/// | Field                    | `true` when …                                                          |
/// |--------------------------|------------------------------------------------------------------------|
/// | `can_accrue`             | `accrue_batch` will process the borrower (line exists, Active, utilization > 0, protocol not paused) |
/// | `batch_open`             | protocol accepts new `accrue_batch` submissions (not paused, batch size < `ACCRUE_BATCH_MAX`) |
/// | `penalty_rate_active`    | a penalty surcharge is configured and the borrower is currently delinquent |
/// | `grace_waiver_active`    | a grace-period config exists and the borrower is within the grace window |
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccrualCapabilities {
    /// `true` when `accrue_batch` will capitalize interest for this borrower:
    /// the credit line exists, is `Active`, has `utilized_amount > 0`, and the
    /// protocol is not paused.
    pub can_accrue: bool,
    /// `true` when the protocol is accepting new `accrue_batch` submissions
    /// (i.e. the circuit breaker is not engaged).
    pub batch_open: bool,
    /// `true` when a positive `penalty_surcharge_bps` is configured and the
    /// borrower is currently past their due date (delinquent). This means
    /// the next accrual will apply the elevated penalty rate.
    pub penalty_rate_active: bool,
    /// `true` when a `GracePeriodConfig` is set and the current ledger
    /// timestamp is within the borrower's grace window (i.e.
    /// `now <= suspension_ts + grace_period_seconds`). Only meaningful when
    /// the line is `Suspended`.
    pub grace_waiver_active: bool,
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
/// `set_protocol_paused_with_reason`. Intended for governance transparency and
/// off-chain monitoring.
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
