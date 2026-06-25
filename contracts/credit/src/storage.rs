// SPDX-License-Identifier: MIT

//! Storage abstraction for the Credit contract.
//!
//! # What
//!
//! Defines the [`DataKey`] enum (30 variants) and provides typed getters /
//! setters for every persistent and instance storage slot in the credit
//! contract. Also owns the reentrancy guard, pause flag, monotonic-timestamp
//! assertion, and the per-borrower id ↔ address mapping used by enumeration.
//!
//! # How
//!
//! Variants are partitioned across Soroban's two long-lived storage tiers:
//!
//! - **Instance storage** — small, hot, always loaded with the contract.
//!   Holds admin/proposal/pause state, the rate formula and rate-change
//!   configs, global accumulators (`TotalUtilized`, `CreditLineCount`,
//!   `TreasuryBalance`), per-protocol caps, the oracle config and last
//!   price, and the auction-contract pointer.
//! - **Persistent storage** — keyed, per-borrower state with TTL. Holds the
//!   `CreditLineData` itself (under `CreditLineIdByBorrower(Address)`), the
//!   blocklist flag, utilization cap, rate floor, repayment schedule,
//!   collateral balance, draw audit / reversal log, and the
//!   `(borrower, settlement_id)` replay marker for default settlement.
//!
//! Every persistent read goes through helpers that call
//! [`bump_credit_line_ttl`] (which extends the entry to
//! `LEDGER_BUMP_AMOUNT ≈ 6 months` when the remaining TTL drops below
//! `LEDGER_BUMP_THRESHOLD ≈ 3 months`). This means an active borrower's
//! state is automatically refreshed on every interaction.
//!
//! # Why
//!
//! Centralizing every storage access here lets the contract enforce three
//! invariants in one place:
//!
//! 1. **TTL hygiene** — no caller forgets to bump.
//! 2. **TotalUtilized conservation** — [`persist_credit_line`] is the only
//!    write path for `CreditLineData` and atomically adjusts the global
//!    `TotalUtilized` accumulator using the caller-captured
//!    `previous_utilized`. `Overflow = 12` reverts if the delta over- or
//!    under-flows.
//! 3. **Monotonic timestamps** — [`assert_ts_monotonic`] is the single
//!    chokepoint that callers use to enforce `last_accrual_ts`,
//!    `last_rate_update_ts`, and `suspension_ts` are non-decreasing.
//!
//! # Reentrancy & pause primitives
//!
//! The instance `Symbol("reentrancy")` slot is set by
//! [`set_reentrancy_guard`] (which reverts `Reentrancy = 11` if already
//! set) and cleared by [`clear_reentrancy_guard`]. The instance
//! `Symbol("paused")` slot is consulted via [`assert_not_paused`] which
//! reverts `Paused = 18` when the protocol is paused.
//!
//! See [`docs/storage-layout.md`](../../../docs/storage-layout.md) for the
//! tier reference and
//! [`docs/PROTOCOL_SPEC.md`](../../../docs/PROTOCOL_SPEC.md) §3 for the
//! full per-variant tier table.

use crate::types::{ContractError, CreditLineData, RepaymentSchedule};
use soroban_sdk::{contracttype, Address, BytesN, Env, Symbol};

/// Storage keys used in instance and persistent storage.
///
/// # Storage tier convention
///
/// Variants in this enum are referenced from **two** Soroban storage tiers:
///
/// - **Instance storage** for global, single-row config and counters
///   (`LiquidityToken`, `LiquiditySource`, `DrawsFrozen`, `SchemaVersion`,
///   `CreditLineCount`, `TotalUtilized`, `MaxDrawAmount`, `MaxRepayAmount`,
///   `DrawMinIntervalSeconds`, `MinCreditLimit`, `MaxCreditLimit`,
///   `PenaltySurchargeBps`, `AuctionContract`, `MaxTotalExposure`,
///   `ProtocolFeeBps`, `TreasuryAddress`, `TreasuryBalance`,
///   `MinCollateralRatioBps`, `OracleConfig`, `OracleLastPrice`,
///   `OracleLastPriceTs`).
/// - **Persistent storage** for per-borrower / per-timestamp data
///   (`CreditLineIdByBorrower`, `CreditLineBorrowerById`, `LastDrawTs`,
///   `BlockedBorrower`, `UtilizationCapBps`, `RateFloorBps`,
///   `RepaymentSchedule`, `CollateralBalance`, `DrawAudit`,
///   `DrawReversedAmount`).
///
/// Helper functions in this module always pick the correct tier; callers
/// outside this module should never hit the storage API directly with these
/// keys.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    /// Address of the liquidity token (SAC or compatible token contract).
    LiquidityToken,
    /// Address of the liquidity source / reserve that funds draws.
    LiquiditySource,
    /// Global emergency switch: when `true`, all `draw_credit` calls revert.
    /// Does not affect repayments. Distinct from per-line `Suspended` status.
    DrawsFrozen,
    /// Storage schema version for migration and compatibility checks.
    SchemaVersion,
    /// Monotonic count of unique borrowers that have had a credit line recorded.
    CreditLineCount,
    /// Borrower → stable numeric id used for deterministic enumeration.
    CreditLineIdByBorrower(Address),
    /// Stable numeric id → borrower address.
    CreditLineBorrowerById(u32),
    /// Global sum of every credit line's utilized_amount.
    TotalUtilized,
    MaxDrawAmount,
    MaxRepayAmount,
    /// Minimum interval in seconds required between successive draws for any borrower.
    DrawMinIntervalSeconds,
    /// Per-borrower last successful draw timestamp.
    LastDrawTs(Address),
    /// Per-borrower block flag; when `true`, draw_credit is rejected.
    BlockedBorrower(Address),
    /// Per-borrower max utilization ratio cap in basis points (e.g. 8000 = 80%).
    /// When set, draw_credit enforces: utilized_amount <= credit_limit * cap_bps / 10_000.
    UtilizationCapBps(Address),
    /// Per-borrower interest rate floor in basis points.
    /// When set, the effective interest rate must be >= floor.
    RateFloorBps(Address),
    /// Per-borrower installment schedule for delinquency tracking.
    RepaymentSchedule(Address),
    /// Minimum allowed credit limit for new credit lines (admin-configurable).
    MinCreditLimit,
    /// Maximum allowed credit limit for new credit lines (admin-configurable).
    MaxCreditLimit,
    /// Penalty surcharge in basis points applied to delinquent credit lines.
    /// Admin-configurable via `set_penalty_surcharge_bps`. Default is 0.
    PenaltySurchargeBps,
    /// Flat fee charged once per missed installment period.
    LateFeeFlat,
    /// Address of the auction contract used for default-liquidation settlement hooks.
    /// Admin-configurable via `set_auction_contract`. Optional: when absent the hook
    /// is skipped and settlement proceeds as an accounting-only operation.
    AuctionContract,
    /// Maximum total exposure allowed across all credit lines (admin-configurable).
    MaxTotalExposure,
    /// Protocol fee in basis points applied to interest portion of repayments.
    ProtocolFeeBps,
    /// Treasury address where withdrawn fees will be sent.
    TreasuryAddress,
    /// Accumulated treasury balance held in contract (fees collected).
    TreasuryBalance,
    /// Per-borrower collateral balance.
    CollateralBalance(Address),
    /// Minimum collateral ratio in basis points.
    MinCollateralRatioBps,
    /// Per-borrower draw audit trail: (borrower, timestamp) → original draw amount.
    DrawAudit(Address, u64),
    /// Per-borrower draw reversal tracking: (borrower, timestamp) → total reversed amount.
    DrawReversedAmount(Address, u64),
    /// Oracle circuit-breaker configuration.
    OracleConfig,
    /// Last accepted oracle price.
    OracleLastPrice,
    /// Timestamp of the last accepted oracle price.
    OracleLastPriceTs,
    /// The currently active contract WASM hash.
    CurrentWasmHash,
}

/// Maximum number of credit lines returned per page.
/// Limits gas consumption and response size for enumeration queries.
pub const MAX_ENUMERATION_LIMIT: u32 = 100;

// ── Persistent storage TTL policy ────────────────────────────────────────────
//
// Soroban persistent entries can be archived if their TTL is not periodically
// extended. The credit contract stores live per-borrower state in persistent
// storage, so we proactively bump TTL on every read/write path.
//
// `extend_ttl(key, threshold, extend_to)` only writes when the remaining TTL is
// below `threshold`, so we can safely call these helpers frequently.
//
// Numbers below assume ~5 seconds/ledger close.
//
// Derivation:
//   30 days  = 2_592_000 s   = 518_400 ledgers
//   3 months = 7_776_000 s   = 1_555_200 ledgers
//   6 months = 15_552_000 s  = 3_110_400 ledgers
//
// We keep a 2:1 ratio between extend-to and refresh-threshold so the average
// number of TTL writes per active key is at most one per three months.
pub const LEDGER_BUMP_AMOUNT: u32 = 3_110_400; // ~6 months
pub const LEDGER_BUMP_THRESHOLD: u32 = 1_555_200; // ~3 months

/// Instance storage TTL policy (covers global config like admin/liquidity token).
pub const INSTANCE_BUMP_AMOUNT: u32 = LEDGER_BUMP_AMOUNT;
pub const INSTANCE_BUMP_THRESHOLD: u32 = LEDGER_BUMP_THRESHOLD;

pub fn bump_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);
}

fn bump_persistent_ttl<K>(env: &Env, key: &K)
where
    K: soroban_sdk::IntoVal<Env, soroban_sdk::Val>,
{
    bump_instance_ttl(env);
    env.storage()
        .persistent()
        .extend_ttl(key, LEDGER_BUMP_THRESHOLD, LEDGER_BUMP_AMOUNT);
}

/// Bump TTL for the borrower's `CreditLineData` entry (keyed by borrower address).
pub fn bump_credit_line_ttl(env: &Env, borrower: &Address) {
    bump_persistent_ttl(env, borrower);
}

/// Return the credit line for `borrower` and bump TTL if present.
pub fn get_credit_line(env: &Env, borrower: &Address) -> Option<CreditLineData> {
    if env.storage().persistent().has(borrower) {
        bump_credit_line_ttl(env, borrower);
        env.storage().persistent().get(borrower)
    } else {
        None
    }
}

/// Return the configured schema version, if any.
pub fn get_schema_version(env: &Env) -> Option<u32> {
    env.storage().instance().get(&DataKey::SchemaVersion)
}

/// Persist the schema version.
#[allow(dead_code)]
pub fn set_schema_version(env: &Env, version: u32) {
    env.storage()
        .instance()
        .set(&DataKey::SchemaVersion, &version);
}

/// Return the global total utilized accumulator.
pub fn get_total_utilized(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::TotalUtilized)
        .unwrap_or(0)
}

/// Return the number of indexed credit lines.
pub fn get_credit_line_count(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::CreditLineCount)
        .unwrap_or(0)
}

/// Return the configured global exposure cap, if set.
pub fn get_max_total_exposure(env: &Env) -> Option<i128> {
    env.storage().instance().get(&DataKey::MaxTotalExposure)
}

/// Set the global exposure cap. Passing `0` removes the cap.
pub fn set_max_total_exposure(env: &Env, cap: i128) {
    if cap == 0 {
        env.storage().instance().remove(&DataKey::MaxTotalExposure);
    } else {
        env.storage()
            .instance()
            .set(&DataKey::MaxTotalExposure, &cap);
    }
}

/// Return the stable id for a borrower, if present.
pub fn get_credit_line_id(env: &Env, borrower: &Address) -> Option<u32> {
    env.storage()
        .persistent()
        .get(&DataKey::CreditLineIdByBorrower(borrower.clone()))
}

/// Return the borrower for a stable id, if present.
pub fn get_borrower_by_credit_line_id(env: &Env, id: u32) -> Option<Address> {
    env.storage()
        .persistent()
        .get(&DataKey::CreditLineBorrowerById(id))
}

/// Ensure a borrower has a stable enumeration id and return it.
pub fn ensure_credit_line_id(env: &Env, borrower: &Address) -> u32 {
    if let Some(existing_id) = get_credit_line_id(env, borrower) {
        return existing_id;
    }

    let next_id = get_credit_line_count(env);
    env.storage()
        .persistent()
        .set(&DataKey::CreditLineIdByBorrower(borrower.clone()), &next_id);
    env.storage()
        .persistent()
        .set(&DataKey::CreditLineBorrowerById(next_id), borrower);
    env.storage()
        .instance()
        .set(&DataKey::CreditLineCount, &next_id.saturating_add(1));
    next_id
}

/// Adjust the global utilized accumulator by the change in a single credit line.
pub fn adjust_total_utilized(env: &Env, previous_utilized: i128, new_utilized: i128) {
    let delta = new_utilized
        .checked_sub(previous_utilized)
        .unwrap_or_else(|| env.panic_with_error(ContractError::Overflow));
    if delta == 0 {
        return;
    }

    let updated_total = get_total_utilized(env)
        .checked_add(delta)
        .unwrap_or_else(|| env.panic_with_error(ContractError::Overflow));
    env.storage()
        .instance()
        .set(&DataKey::TotalUtilized, &updated_total);
}

/// Persist a credit line and atomically apply its contribution delta to the
/// global total utilized accumulator.
pub fn persist_credit_line(
    env: &Env,
    borrower: &Address,
    line: &CreditLineData,
    previous_utilized: i128,
) {
    ensure_credit_line_id(env, borrower);
    env.storage().persistent().set(borrower, line);
    bump_credit_line_ttl(env, borrower);
    adjust_total_utilized(env, previous_utilized, line.utilized_amount);
}

pub fn admin_key(env: &Env) -> Symbol {
    Symbol::new(env, "admin")
}

pub fn proposed_admin_key(env: &Env) -> Symbol {
    Symbol::new(env, "proposed_admin")
}

pub fn proposed_at_key(env: &Env) -> Symbol {
    Symbol::new(env, "proposed_at")
}

pub fn reentrancy_key(env: &Env) -> Symbol {
    Symbol::new(env, "reentrancy")
}

pub fn rate_cfg_key(env: &Env) -> Symbol {
    Symbol::new(env, "rate_cfg")
}

/// Instance storage key for the risk-score-based rate formula configuration.
pub fn rate_formula_key(env: &Env) -> Symbol {
    Symbol::new(env, "rate_form")
}

/// Instance storage key for the protocol pause flag.
pub fn paused_key(env: &Env) -> Symbol {
    Symbol::new(env, "paused")
}

/// Instance storage key for the grace period configuration.
pub fn grace_period_key(env: &Env) -> Symbol {
    Symbol::new(env, "grace_cfg")
}

/// Assert reentrancy guard is not set; set it for the duration of the call.
///
/// Panics with [`ContractError::Reentrancy`] if the guard is already active,
/// indicating a reentrant call. Caller **must** call [`clear_reentrancy_guard`]
/// on every success and failure path to release the guard.
///
/// # Storage
/// - **Type**: Instance storage (shared TTL with all instance keys)
/// - **Key**: `Symbol("reentrancy")`
/// - **TTL Note**: Guard is functionally temporary (set on entry, cleared on all exits)
///   but stored in instance storage for simplicity. Instance TTL must be maintained
///   separately via `extend_ttl()` calls in frequently-invoked functions.
pub fn set_reentrancy_guard(env: &Env) {
    let key = reentrancy_key(env);
    let current: bool = env.storage().instance().get(&key).unwrap_or(false);
    if current {
        env.panic_with_error(ContractError::Reentrancy);
    }
    env.storage().instance().set(&key, &true);
}

/// Clear the reentrancy guard set by [`set_reentrancy_guard`].
///
/// Must be called on every exit path (success and failure) of any function
/// that called [`set_reentrancy_guard`].
///
/// # Storage
/// - **Type**: Instance storage
/// - **Key**: `Symbol("reentrancy")`
/// - **TTL Note**: Guard is cleared immediately after call; instance TTL is maintained
///   separately via `extend_ttl()` calls in frequently-invoked functions.
pub fn clear_reentrancy_guard(env: &Env) {
    let key = reentrancy_key(env);
    env.storage().instance().set(&key, &false);
}

/// Set a per-borrower interest rate floor (admin only, enforced by caller).
pub fn set_borrower_rate_floor(env: &Env, borrower: &Address, floor_bps: Option<u32>) {
    if let Some(floor) = floor_bps {
        assert!(floor <= crate::risk::MAX_INTEREST_RATE_BPS, "floor exceeds max rate");
    }
    if let Some(floor) = floor_bps {
        env.storage()
            .persistent()
            .set(&DataKey::RateFloorBps(borrower.clone()), &floor);
    } else {
        env.storage()
            .persistent()
            .remove(&DataKey::RateFloorBps(borrower.clone()));
    }
}

/// Get the per-borrower interest rate floor, if set.
pub fn get_borrower_rate_floor(env: &Env, borrower: &Address) -> Option<u32> {
    env.storage()
        .persistent()
        .get(&DataKey::RateFloorBps(borrower.clone()))
}

/// Set or clear the per-borrower max utilization ratio cap in basis points.
pub fn set_utilization_cap_bps(env: &Env, borrower: &Address, cap_bps: Option<u32>) {
    let key = DataKey::UtilizationCapBps(borrower.clone());
    match cap_bps {
        Some(v) => {
            env.storage().persistent().set(&key, &v);
        }
        None => {
            if env.storage().persistent().has(&key) {
                env.storage().persistent().remove(&key);
            }
        }
    }
}

/// Get the per-borrower max utilization ratio cap, if set.
pub fn get_utilization_cap_bps(env: &Env, borrower: &Address) -> Option<u32> {
    env.storage()
        .persistent()
        .get(&DataKey::UtilizationCapBps(borrower.clone()))
}

/// Clear the installment schedule for a borrower.
pub fn clear_repayment_schedule(env: &Env, borrower: &Address) {
    env.storage()
        .persistent()
        .remove(&DataKey::RepaymentSchedule(borrower.clone()));
}

/// Store `borrower` as blocked.
pub fn set_borrower_blocked(env: &Env, borrower: &Address) {
    env.storage()
        .persistent()
        .set(&DataKey::BlockedBorrower(borrower.clone()), &true);
}

/// Remove the blocked entry for `borrower`.
pub fn set_borrower_unblocked(env: &Env, borrower: &Address) {
    let key = DataKey::BlockedBorrower(borrower.clone());
    if env.storage().persistent().has(&key) {
        env.storage().persistent().remove(&key);
    }
}

/// Check if a borrower is blocked from drawing.
pub fn is_borrower_blocked(env: &Env, borrower: &Address) -> bool {
    env.storage()
        .persistent()
        .get(&DataKey::BlockedBorrower(borrower.clone()))
        .unwrap_or(false)
}

/// Get the configured minimum credit limit, if set.
pub fn get_min_credit_limit(env: &Env) -> Option<i128> {
    env.storage().instance().get(&DataKey::MinCreditLimit)
}

/// Set the minimum credit limit (admin only, enforced by caller).
pub fn set_min_credit_limit(env: &Env, min: i128) {
    env.storage().instance().set(&DataKey::MinCreditLimit, &min);
}

/// Get the configured maximum credit limit, if set.
pub fn get_max_credit_limit(env: &Env) -> Option<i128> {
    env.storage().instance().get(&DataKey::MaxCreditLimit)
}

/// Set the maximum credit limit (admin only, enforced by caller).
pub fn set_max_credit_limit(env: &Env, max: i128) {
    env.storage().instance().set(&DataKey::MaxCreditLimit, &max);
}

// ── Auction contract hook ─────────────────────────────────────────────────────

/// Return the configured auction contract address, if set.
///
/// Used by `settle_default_liquidation` to validate cross-contract settlement
/// hooks. When absent, the hook is skipped and settlement proceeds as an
/// accounting-only operation.
pub fn get_auction_contract(env: &Env) -> Option<Address> {
    env.storage().instance().get(&DataKey::AuctionContract)
}

/// Persist the auction contract address (admin only, enforced by caller).
pub fn set_auction_contract(env: &Env, addr: &Address) {
    env.storage().instance().set(&DataKey::AuctionContract, addr);
}

/// Return the installment schedule for a borrower, if configured.
pub fn get_repayment_schedule(env: &Env, borrower: &Address) -> Option<RepaymentSchedule> {
    env.storage()
        .persistent()
        .get(&DataKey::RepaymentSchedule(borrower.clone()))
}

/// Persist the installment schedule for a borrower.
pub fn set_repayment_schedule(env: &Env, borrower: &Address, schedule: &RepaymentSchedule) {
    env.storage()
        .persistent()
        .set(&DataKey::RepaymentSchedule(borrower.clone()), schedule);
}

/// Get the last draw timestamp for a borrower, if any.
pub fn get_last_draw_ts(env: &Env, borrower: &Address) -> Option<u64> {
    env.storage()
        .persistent()
        .get(&DataKey::LastDrawTs(borrower.clone()))
}

/// Set the last draw timestamp for a borrower.
pub fn set_last_draw_ts(env: &Env, borrower: &Address, ts: u64) {
    env.storage()
        .persistent()
        .set(&DataKey::LastDrawTs(borrower.clone()), &ts);
}

/// Get the configured max draw amount, if set.
pub fn get_max_draw_amount(env: &Env) -> Option<i128> {
    env.storage().instance().get(&DataKey::MaxDrawAmount)
}

/// Set the max draw amount (admin only, enforced by caller).
pub fn set_max_draw_amount(env: &Env, amount: i128) {
    env.storage().instance().set(&DataKey::MaxDrawAmount, &amount);
}

/// Get the configured max repay amount, if set.
pub fn get_max_repay_amount(env: &Env) -> Option<i128> {
    env.storage().instance().get(&DataKey::MaxRepayAmount)
}

/// Set the max repay amount (admin only, enforced by caller).
pub fn set_max_repay_amount(env: &Env, amount: i128) {
    env.storage().instance().set(&DataKey::MaxRepayAmount, &amount);
}

/// Get the configured draw min interval, if set.
pub fn get_draw_min_interval(env: &Env) -> Option<u64> {
    env.storage().instance().get(&DataKey::DrawMinIntervalSeconds)
}

/// Set the draw min interval (admin only, enforced by caller).
pub fn set_draw_min_interval(env: &Env, seconds: u64) {
    env.storage()
        .instance()
        .set(&DataKey::DrawMinIntervalSeconds, &seconds);
}

/// Set/unset the global draws frozen flag (admin only, enforced by caller).
pub fn set_draws_frozen(env: &Env, frozen: bool) {
    env.storage().instance().set(&DataKey::DrawsFrozen, &frozen);
}

/// Check if draws are globally frozen.
pub fn is_draws_frozen(env: &Env) -> bool {
    env.storage().instance().get(&DataKey::DrawsFrozen).unwrap_or(false)
}

/// Check if the protocol is paused.
pub fn is_paused(env: &Env) -> bool {
    env.storage().instance().get(&paused_key(env)).unwrap_or(false)
}

/// Set the protocol pause state (admin only, enforced by caller).
///
/// # Storage
/// - **Type**: Instance storage (shared TTL with all instance keys)
/// - **Key**: `Symbol("paused")`
/// - **TTL Note**: Shares instance TTL — extend alongside other instance keys.
pub fn set_paused(env: &Env, paused: bool) {
    env.storage().instance().set(&paused_key(env), &paused);
}

/// Assert the protocol is not paused. Reverts with ContractError::Paused if paused.
/// This is the circuit breaker guard injected into all mutating entrypoints except repay_credit.
pub fn assert_not_paused(env: &Env) {
    if is_paused(env) {
        env.panic_with_error(crate::types::ContractError::Paused);
    }
}

/// Assert that a timestamp update is monotonic.
///
/// Reverts if `new_ts <= stored_ts` and `stored_ts != 0`.
/// A `stored_ts` of 0 is treated as "never written" and always passes.
pub fn assert_ts_monotonic(env: &Env, stored_ts: u64, new_ts: u64) {
    if stored_ts != 0 && new_ts <= stored_ts {
        env.panic_with_error(crate::types::ContractError::TimestampRegression);
    }
}

// ── Oracle circuit-breaker storage ───────────────────────────────────────────
//
// The oracle circuit breaker has three independent storage entries:
//
//   OracleConfig         — admin-supplied policy (deviation & staleness limits)
//   OracleLastPrice      — last price that passed the breaker
//   OracleLastPriceTs    — ledger timestamp of that price
//
// `set_oracle_last_price` updates the two `Last*` entries atomically; readers
// should always treat them as a pair to avoid races against an in-flight
// settlement.

/// Get the oracle circuit-breaker config, if set.
///
/// When `None`, the breaker is disabled and oracle prices are accepted with
/// no deviation or staleness check. See [`crate::types::OracleConfig`] for
/// invariants on the stored values.
pub fn get_oracle_config(env: &Env) -> Option<crate::types::OracleConfig> {
    env.storage().instance().get(&DataKey::OracleConfig)
}

/// Set the oracle circuit-breaker config.
///
/// Caller is responsible for admin auth and for validating that the supplied
/// config satisfies the invariants documented on [`crate::types::OracleConfig`].
pub fn set_oracle_config(env: &Env, cfg: &crate::types::OracleConfig) {
    env.storage().instance().set(&DataKey::OracleConfig, cfg);
}

/// Get the last accepted oracle price, if any.
///
/// Returns `None` before the first successful settlement. Always read
/// together with [`get_oracle_last_price_ts`] to interpret staleness.
pub fn get_oracle_last_price(env: &Env) -> Option<i128> {
    env.storage().instance().get(&DataKey::OracleLastPrice)
}

/// Get the timestamp of the last accepted oracle price, if any.
pub fn get_oracle_last_price_ts(env: &Env) -> Option<u64> {
    env.storage().instance().get(&DataKey::OracleLastPriceTs)
}

/// Persist a newly accepted oracle price and its timestamp atomically.
///
/// The two `instance().set(..)` calls are part of the same host transaction,
/// so observers cannot see a half-updated price/timestamp pair.
pub fn set_oracle_last_price(env: &Env, price: i128, ts: u64) {
    env.storage().instance().set(&DataKey::OracleLastPrice, &price);
    env.storage().instance().set(&DataKey::OracleLastPriceTs, &ts);
}

// ── Penalty surcharge for delinquent lines ───────────────────────────────────

/// Get the configured penalty surcharge in basis points, if set.
///
/// Returns `0` when the key is absent. A return of `0` is indistinguishable
/// from an explicit `0`-bps setting, which is fine because both mean the same
/// thing: no extra surcharge above the base rate.
///
/// # Storage
/// - **Type**: Instance storage
/// - **Key**: [`DataKey::PenaltySurchargeBps`]
pub fn get_penalty_surcharge_bps(env: &Env) -> u32 {
    env.storage().instance().get(&DataKey::PenaltySurchargeBps).unwrap_or(0)
}

/// Set the penalty surcharge in basis points.
///
/// The surcharge is added to the base interest rate when a line is delinquent
/// (i.e. [`crate::query::is_delinquent`] returns `true`). Admin auth must be
/// enforced by the caller — this helper does not check authorization itself.
///
/// # Storage
/// - **Type**: Instance storage
/// - **Key**: [`DataKey::PenaltySurchargeBps`]
pub fn set_penalty_surcharge_bps(env: &Env, bps: u32) {
    env.storage().instance().set(&DataKey::PenaltySurchargeBps, &bps);
}
/// Returns configured flat late fee.
/// Defaults to 0 to preserve existing behaviour.
pub fn get_late_fee_flat(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::LateFeeFlat)
        .unwrap_or(0)
}

/// Stores flat late fee amount.
pub fn set_late_fee_flat(env: &Env, fee: i128) {
    env.storage()
        .instance()
        .set(&DataKey::LateFeeFlat, &fee);
}

pub fn get_treasury_balance(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::TreasuryBalance)
        .unwrap_or(0)
}

/// Stores treasury balance.
pub fn set_treasury_balance(env: &Env, amount: i128) {
    env.storage()
        .instance()
        .set(&DataKey::TreasuryBalance, &amount);
}

pub fn get_current_wasm_hash(env: &Env) -> BytesN<32> {
    env.storage()
        .instance()
        .get(&DataKey::CurrentWasmHash)
        .unwrap_or_else(|| BytesN::from_array(env, &[0; 32]))
}

pub fn set_current_wasm_hash(env: &Env, hash: &BytesN<32>) {
    env.storage().instance().set(&DataKey::CurrentWasmHash, hash);
}

pub fn get_collateral_token(env: &Env) -> Option<Address> {
    env.storage().instance().get(&DataKey::LiquidityToken)
}

pub fn get_collateral_balance(env: &Env, borrower: &Address) -> i128 {
    let key = DataKey::CollateralBalance(borrower.clone());
    if env.storage().persistent().has(&key) {
        bump_persistent_ttl(env, &key);
        env.storage().persistent().get(&key).unwrap_or(0)
    } else {
        0
    }
}

pub fn set_collateral_balance(env: &Env, borrower: &Address, amount: i128) {
    let key = DataKey::CollateralBalance(borrower.clone());
    env.storage().persistent().set(&key, &amount);
    bump_persistent_ttl(env, &key);
}

pub fn get_min_collateral_ratio_bps(env: &Env) -> Option<u32> {
    env.storage().instance().get(&DataKey::MinCollateralRatioBps)
}

pub fn set_min_collateral_ratio_bps(env: &Env, ratio_bps: u32) {
    assert!(ratio_bps <= 10_000, "ratio_bps must be <= 10000");
    env.storage().instance().set(&DataKey::MinCollateralRatioBps, &ratio_bps);
}

pub fn get_protocol_fee_bps(env: &Env) -> Option<u32> {
    env.storage().instance().get(&DataKey::ProtocolFeeBps)
}

pub fn set_protocol_fee_bps(env: &Env, bps: u32) {
    env.storage().instance().set(&DataKey::ProtocolFeeBps, &bps);
}

pub fn get_treasury_address(env: &Env) -> Option<Address> {
    env.storage().instance().get(&DataKey::TreasuryAddress)
}

pub fn set_treasury_address(env: &Env, addr: &Address) {
    env.storage().instance().set(&DataKey::TreasuryAddress, addr);
}

pub fn add_treasury_balance(env: &Env, amount: i128) {
    let cur = get_treasury_balance(env);
    let updated = cur
        .checked_add(amount)
        .unwrap_or_else(|| env.panic_with_error(ContractError::Overflow));
    env.storage().instance().set(&DataKey::TreasuryBalance, &updated);
}

pub fn clear_treasury_balance(env: &Env) {
    env.storage().instance().set(&DataKey::TreasuryBalance, &0_i128);
}