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
//!   configs, global accumulators (`TotalUtilized`, `TotalCollateral`,
//!   `CreditLineCount`, `TreasuryBalance`), per-protocol caps, the oracle config and last
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
//! [`set_reentrancy_guard`] (which reverts `Reentrancy = 11` if already set)
//! and cleared by [`clear_reentrancy_guard`]. Guarded entrypoints should not
//! pair those two by hand: bind the [`ReentrancyGuard`] returned by
//! [`enter_reentrancy_guard`] for the duration of the guarded body and the
//! flag is released automatically when it goes out of scope. The instance
//! `Symbol("paused")` slot is consulted via [`assert_not_paused`] which
//! reverts `Paused = 18` when the protocol is paused.
//!
//! ## Revert-on-panic semantics
//!
//! Failure paths deliberately do **not** clear the guard. When a Soroban
//! frame traps — via [`Env::panic_with_error`], an arithmetic overflow, or a
//! panic propagated out of a nested contract call — the host discards
//! **every** storage write that frame performed, including the instance write
//! from [`set_reentrancy_guard`]. A panicking call therefore cannot leave the
//! flag set, and `Symbol("reentrancy")` needs no explicit cleanup before
//! `panic_with_error`. What *would* leak the flag is a success path that
//! forgets to release it, which the [`ReentrancyGuard`] drop glue makes
//! impossible.
//!
//! See [`docs/storage-layout.md`](../../../docs/storage-layout.md) for the
//! tier reference and
//! [`docs/PROTOCOL_SPEC.md`](../../../docs/PROTOCOL_SPEC.md) §3 for the
//! full per-variant tier table.

use crate::types::{
    ContractError, CreditLineData, CreditStatus, DrawsFreezeState, GracePeriodConfig,
    OracleQuorumConfig, RepaymentSchedule, TreasuryWithdrawalProposal,
};
use soroban_sdk::{contracttype, symbol_short, Address, Bytes, Env, Symbol};

/// Validates that a storage key encoding is canonical and does not contain
/// duplicated or ambiguous byte representations that could lead to collisions.
/// This prevents adverse conditions from causing silent data loss or inconsistent state.
pub fn validate_storage_key_encoding(env: &Env, key_bytes: &Bytes) {
    // In Soroban, XDR serialization is strictly canonical for built-in types.
    // However, if raw bytes are used as keys, this function ensures they don't
    // contain invalid padding or duplicate representations.
    let len = key_bytes.len();
    if len > 0 {
        // Example check: reject keys with trailing zero bytes which might be 
        // a duplicate encoding of a shorter key.
        if key_bytes.get(len - 1).unwrap() == 0 {
            env.panic_with_error(crate::types::ContractError::InvalidAmount); // Reusing an error code for simplicity
        }
    }
}
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
///   `PenaltySurchargeBps`, `LateFeeFlat`, `AuctionContract`, `MaxTotalExposure`,
///   `ProtocolFeeBps`, `TreasuryFeeShareBps`, `TreasuryAddress`, `TreasuryBalance`,
///   `BountyAddress`, `BountyBalance`,
///   `TotalCollateral`,
///   `MinCollateralRatioBps`, `OracleConfig`, `OracleLastPrice`,
///   `OracleLastPriceTs`, `OracleReferenceRefreshTs`).
/// - **Persistent storage** for per-borrower / per-timestamp data
///   (`CreditLineIdByBorrower`, `CreditLineBorrowerById`, `LastDrawTs`,
///   `BlockedBorrower`, `FrozenBorrower`, `UtilizationCapBps`, `RateFloorBps`,
///   `RepaymentSchedule`, `CollateralBalance`, `DrawAudit`,
///   `DrawReversedAmount`).
///
/// Helper functions in this module always pick the correct tier; callers
/// outside this module should never hit the storage API directly with these
/// keys.
// `export = false`: DataKey has grown past the 50-case limit the Soroban
// contract-spec XDR format (`SCSpecUdtUnionV0.cases<50>`) allows for an
// exported type spec. This is an internal storage-key type (never crosses
// the contract ABI), so skipping spec export has no client-visible effect.
#[contracttype(export = false)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollateralTokenKey {
    pub borrower: Address,
    pub token: Address,
}

/// Key payload for per-borrower per-timestamp draw audit and reversal storage entries.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DrawAuditKey {
    pub borrower: Address,
    pub timestamp: u64,
}

/// Composite key for per-borrower per-token collateral balance entries (v2).
/// Wraps `(borrower, token)` for use as a single storage key.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollateralBalanceV2Key {
    pub borrower: Address,
    pub token: Address,
}

/// Storage keys used in instance and persistent storage.
#[contracttype(export = false)]
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
    /// Count of currently Active credit lines.
    ActiveLineCount,
    /// Count of credit lines whose liquidation auction is currently active
    /// (i.e. lines in [`CreditStatus::Defaulted`] with an in-flight auction).
    ///
    /// Incremented when a line enters `Defaulted` and decremented when a line
    /// exits the `Defaulted` pipeline (full settlement, reinstate, admin
    /// force-close, or reopen). While non-zero, fee-configuration entrypoints
    /// revert with [`ContractError::AuctionActive`] so auction economics stay
    /// deterministic (Issue #1169).
    PendingAuctionCount,
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
    /// Per-borrower temporary freeze expiry timestamp; draws blocked while now < expiry_ts.
    /// When key is absent or expiry_ts <= now, the borrower is not frozen.
    FrozenBorrower(Address),
    /// Per-borrower credit-line draw freeze with structured reason taxonomy.
    /// When absent, the line is not admin-frozen (distinct from [`CreditStatus::Suspended`]).
    CreditLineFreeze(Address),

    /// Per-borrower max utilization ratio cap in basis points (e.g. 8000 = 80%).
    /// When set, draw_credit enforces: utilized_amount <= credit_limit * cap_bps / 10_000.
    UtilizationCapBps(Address),
    /// Minimum interval in seconds between critical borrow admin actions for one borrower.
    BorrowAdminCooldownSeconds,
    /// Last timestamp at which a critical borrow admin action mutated a borrower.
    LastBorrowAdminActionTs(Address),
    /// Minimum interval in seconds between critical accrual admin actions for one borrower.
    AccrualAdminCooldownSeconds,
    /// Last timestamp at which a critical accrual admin action mutated a borrower.
    LastAccrualAdminActionTs(Address),
    /// Per-borrower interest rate floor in basis points.
    /// When set, the effective interest rate must be >= floor.
    RateFloorBps(Address),
    /// Per-borrower interest rate ceiling in basis points.
    /// When set, the effective interest rate must be <= ceiling.
    RateCeilingBps(Address),
    /// Per-borrower installment schedule for delinquency tracking.
    RepaymentSchedule(Address),
    /// Per-borrower VRF commitment for credit score derivation.
    /// Stores the hash of the VRF output that the risk score must be derived from.
    VrfCommitment(Address),
    /// Minimum allowed credit limit for new credit lines (admin-configurable).
    MinCreditLimit,
    /// Maximum allowed credit limit for new credit lines (admin-configurable).
    MaxCreditLimit,
    /// Protocol-level max close factor in basis points for partial liquidation.
    CloseFactorBps,
    /// Penalty surcharge in basis points applied to delinquent credit lines.
    /// Admin-configurable via `set_penalty_surcharge_bps`. Default is 0.
    PenaltySurchargeBps,
    /// Flat fee charged per missed installment.
    /// Admin-configurable via `set_late_fee_flat`. Default is 0 (disabled).
    LateFeeFlat,
    LateFeeConfig,
    /// Address of the auction contract used for default-liquidation settlement hooks.
    /// Admin-configurable via `set_auction_contract`. Optional: when absent the hook
    /// is skipped and settlement proceeds as an accounting-only operation.
    AuctionContract,
    /// Maximum total exposure allowed across all credit lines (admin-configurable).
    MaxTotalExposure,
    /// Protocol fee in basis points applied to interest portion of repayments.
    ProtocolFeeBps,
    /// Treasury share of skimmed protocol fees in basis points (0..=10_000).
    /// When unset, defaults to 10_000 (100 % treasury).
    TreasuryFeeShareBps,
    /// Treasury address where withdrawn fees will be sent.
    TreasuryAddress,
    /// Accumulated treasury balance held in contract (fees collected).
    TreasuryBalance,
    /// Bounty pool address where withdrawn bounty fees will be sent.
    BountyAddress,
    /// Accumulated bounty pool balance held in contract (fee share).
    BountyBalance,
    /// Per-borrower collateral balance.
    CollateralBalance(Address),
    /// Per-borrower per-token collateral balance (multi-token support).
    CollateralBalanceV2(Address, Address),
    /// Minimum collateral ratio in basis points.
    MinCollateralRatioBps,
    /// Minimum ledger seconds between critical collateral admin actions (v7).
    AdminCollateralCooldownSeconds,
    /// Ledger timestamp of the last critical collateral admin action (v7).
    LastColAdminActionTs,
    /// Per-asset collateral risk weight in basis points (10_000 = 100%, full value).
    /// Absent for an asset means callers should treat it as 10_000 bps (unweighted).
    CollateralRiskWeightBps(Address),
    /// Per-borrower draw audit trail: (borrower, timestamp) → original draw amount.
    DrawAudit(DrawAuditKey),
    /// Per-borrower draw reversal tracking: (borrower, timestamp) → total reversed amount.
    DrawReversedAmount(DrawAuditKey),
    /// Oracle circuit-breaker configuration.
    OracleConfig,
    /// Last accepted oracle price.
    OracleLastPrice,
    /// Timestamp of the last accepted oracle price.
    OracleLastPriceTs,
    /// Timestamp of the last admin oracle-reference refresh.
    OracleReferenceRefreshTs,
    /// Multi-oracle quorum configuration.
    OracleQuorumConfig,
    /// Last resolved multi-oracle quorum price.
    OracleQuorumPrice,
    /// Timestamp of the last resolved multi-oracle quorum price.
    OracleQuorumPriceTs,
    /// Global sum of every borrower's collateral balance.
    TotalCollateral,
    /// Pending treasury withdrawal proposal (at most one at a time).
    /// Stored in instance storage; cleared after successful execution.
    PendingTreasuryWithdrawal,
    /// Structured reason for the most recent protocol pause (escape-hatch audit trail).
    /// Stored when admin invokes pause with a reason; cleared on unpause.
    PauseReason,
    /// Per-borrower maximum total exposure cap (absolute i128 amount).
    /// When set, draw_credit enforces: utilized_amount <= max_borrower_exposure.
    /// Pass 0 to remove the cap for this borrower.
    MaxBorrowerExposure(Address),
    /// Admin freeze cooldown duration in seconds.
    /// When set to a non-zero value, admin freeze/unfreeze actions are gated
    /// by a minimum interval between successive calls.
    FreezeCooldownSeconds,
    /// Ledger timestamp of the last admin freeze or unfreeze action.
    /// Used with [`FreezeCooldownSeconds`] to enforce the cooldown period.
    LastFreezeTimestamp,
    /// Per-borrower liquidation grace period in seconds.
    /// Specifies a grace window before a credit line can be defaulted or liquidated.
    LiquidationGracePeriod(Address),
    /// Per-borrower absolute exposure cap (i128 amount).
    BorrowerExposureCap(Address),
    /// Per-borrower allowlist of accepted multi-collateral token addresses.
    CollateralTokenAllowlist,
    /// Per-borrower committed attestation batch.
    AttestationBatch(Address),
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
pub const CREDIT_LINE_TTL_EXTEND_TO: u32 = LEDGER_BUMP_AMOUNT;
pub const CREDIT_LINE_TTL_THRESHOLD: u32 = LEDGER_BUMP_THRESHOLD;

/// Instance storage TTL policy (covers global config like admin/liquidity token).
pub const INSTANCE_BUMP_AMOUNT: u32 = LEDGER_BUMP_AMOUNT;
pub const INSTANCE_BUMP_THRESHOLD: u32 = LEDGER_BUMP_THRESHOLD;

pub fn bump_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);
}

pub fn bump_persistent_ttl<K>(env: &Env, key: &K)
where
    K: soroban_sdk::IntoVal<Env, soroban_sdk::Val>,
{
    bump_instance_ttl(env);
    env.storage()
        .persistent()
        .extend_ttl(key, LEDGER_BUMP_THRESHOLD, LEDGER_BUMP_AMOUNT);
}

pub fn bump_credit_line_ttl(env: &Env, borrower: &Address) {
    bump_instance_ttl(env);
    env.storage()
        .persistent()
        .extend_ttl(borrower, CREDIT_LINE_TTL_THRESHOLD, CREDIT_LINE_TTL_EXTEND_TO);
}

/// Refresh the persistent TTL for a default-liquidation replay marker.
///
/// Mirrors the auction contract's `bump_settlement_marker_ttl`
/// (`gateway-contract/contracts/auction_contract/src/storage.rs`): the marker
/// lives under `(Symbol("liq_seen"), borrower, settlement_id)` and must
/// outlive every credit-line state it guards, so it is bumped to the same
/// ~6-month window as the credit-line entry on write and on the replay-check
/// read path. The bump is a no-op when the marker does not yet exist.
pub fn bump_settlement_marker_ttl(env: &Env, key: &(Symbol, Address, Symbol)) {
    if env.storage().persistent().has(key) {
        bump_persistent_ttl(env, key);
    }
}

/// Refresh the persistent TTL for a borrower's VRF commitment.
pub fn bump_vrf_commitment_ttl(env: &Env, borrower: &Address) {
    let key = DataKey::VrfCommitment(borrower.clone());
    bump_persistent_ttl(env, &key);
}

/// Refresh the persistent TTL for an active credit-line freeze record.
pub fn bump_credit_line_freeze_ttl(env: &Env, borrower: &Address) {
    let key = DataKey::CreditLineFreeze(borrower.clone());
    bump_persistent_ttl(env, &key);
}

/// Refresh the persistent TTL for a temporary borrower freeze record.
pub fn bump_borrower_frozen_ttl(env: &Env, borrower: &Address) {
    let key = DataKey::FrozenBorrower(borrower.clone());
    bump_persistent_ttl(env, &key);
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

/// Assert that the persisted `TotalUtilized` accumulator matches the live sum of
/// every credit line's `utilized_amount`.
///
/// This is a fail-closed invariant check: any drift in the aggregate is treated
/// as protocol corruption and aborts the transaction with a deterministic error
/// message so operators can diagnose storage drift without exposing sensitive
/// state.
pub fn assert_total_utilized_conserved(env: &Env) {
    let stored_total = get_total_utilized(env);
    let mut recomputed_total = 0_i128;
    let line_count = get_credit_line_count(env);

    for id in 0..line_count {
        let Some(borrower) = get_borrower_by_credit_line_id(env, id) else {
            continue;
        };
        // Load through the TTL-bumping getter so this consistency check never
        // bypasses the persistent TTL policy (Issue #1273).
        let Some(line) = get_credit_line(env, &borrower) else {
            continue;
        };
        recomputed_total = recomputed_total
            .checked_add(line.utilized_amount)
            .unwrap_or_else(|| panic!("utilization conservation: recomputed total overflow"));
    }

    if stored_total != recomputed_total {
        panic!(
            "utilization conservation: stored={stored_total}, recomputed={recomputed_total}"
        );
    }
}

/// Return the global collateral accumulator.
pub fn get_total_collateral(env: &Env) -> i128 {
    bump_instance_ttl(env);
    env.storage()
        .instance()
        .get(&DataKey::TotalCollateral)
        .unwrap_or(0)
}

/// Assert that the persisted `TotalCollateral` accumulator matches the live sum of
/// the tracked collateral balances for the known borrower set.
pub fn assert_total_collateral_conserved(env: &Env) {
    let stored_total = get_total_collateral(env);
    let mut recomputed_total = 0_i128;
    let line_count = get_credit_line_count(env);

    for id in 0..line_count {
        let Some(borrower) = get_borrower_by_credit_line_id(env, id) else {
            continue;
        };
        let balance = get_collateral_balance(env, &borrower);
        recomputed_total = recomputed_total
            .checked_add(balance)
            .unwrap_or_else(|| panic!("collateral conservation: recomputed total overflow"));
    }

    if stored_total != recomputed_total {
        panic!(
            "collateral conservation: stored={stored_total}, recomputed={recomputed_total}"
        );
    }
}

/// Return the number of indexed credit lines.
pub fn get_credit_line_count(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::CreditLineCount)
        .unwrap_or(0)
}

/// Return the count of currently Active credit lines.
pub fn get_active_line_count(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::ActiveLineCount)
        .unwrap_or(0)
}

/// Increment the count of currently Active credit lines.
pub fn increment_active_line_count(env: &Env) {
    let count = get_active_line_count(env);
    env.storage()
        .instance()
        .set(&DataKey::ActiveLineCount, &count.saturating_add(1));
}

/// Decrement the count of currently Active credit lines.
pub fn decrement_active_line_count(env: &Env) {
    let count = get_active_line_count(env);
    env.storage()
        .instance()
        .set(&DataKey::ActiveLineCount, &count.saturating_sub(1));
}

// ── Active liquidation auction tracking (Issue #1169) ────────────────────────
//
// A liquidation auction is considered **active** from the moment a credit line
// enters `CreditStatus::Defaulted` until that line exits the `Defaulted`
// pipeline through one of the four terminal paths:
//
//   1. `settle_default_liquidation` with full recovery   (Defaulted → Closed)
//   2. `reinstate_credit_line`                           (Defaulted → Active/Restricted)
//   3. `close_credit_line` admin force-close              (Defaulted → Closed)
//   4. `open_credit_line` admin reopen                    (Defaulted → Active)
//
// A **partial** settlement leaves the line in `Defaulted`, so the auction stays
// active and the counter is not touched.
//
// The counter is maintained atomically with the credit-line status transition
// inside the same host transaction, so the guard below sees a consistent,
// deterministic value under any interleaving of concurrent calls.

/// Return the number of credit lines with an active liquidation auction.
pub fn get_pending_auction_count(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::PendingAuctionCount)
        .unwrap_or(0)
}

/// Mark one more liquidation auction as active (line entered `Defaulted`).
pub fn increment_pending_auction_count(env: &Env) {
    let count = get_pending_auction_count(env);
    env.storage()
        .instance()
        .set(&DataKey::PendingAuctionCount, &count.saturating_add(1));
}

/// Mark one liquidation auction as no longer active (line exited `Defaulted`).
pub fn decrement_pending_auction_count(env: &Env) {
    let count = get_pending_auction_count(env);
    env.storage()
        .instance()
        .set(&DataKey::PendingAuctionCount, &count.saturating_sub(1));
}

/// Revert with [`crate::types::ContractError::AuctionActive`] if any
/// liquidation auction is currently active.
///
/// This is the deterministic state check injected into fee-configuration
/// entrypoints (Issue #1169): while an auction is in flight, changing the
/// protocol fee, fee-share split, penalty surcharge, or late-fee schedule
/// could silently change the economics of the in-flight auction or its
/// eventual settlement, so such changes are rejected atomically.
pub fn assert_no_active_auctions(env: &Env) {
    if get_pending_auction_count(env) > 0 {
        env.panic_with_error(crate::types::ContractError::AuctionActive);
    }
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
///
/// Bumps the persistent TTL of the entry so it is not archived independently
/// of the credit line.
pub fn get_credit_line_id(env: &Env, borrower: &Address) -> Option<u32> {
    let key = DataKey::CreditLineIdByBorrower(borrower.clone());
    if env.storage().persistent().has(&key) {
        bump_persistent_ttl(env, &key);
    }
    env.storage().persistent().get(&key)
}

/// Return the borrower for a stable id, if present.
///
/// Bumps the persistent TTL of the entry so it is not archived independently
/// of the credit line.
pub fn get_borrower_by_credit_line_id(env: &Env, id: u32) -> Option<Address> {
    let key = DataKey::CreditLineBorrowerById(id);
    if env.storage().persistent().has(&key) {
        bump_persistent_ttl(env, &key);
    }
    env.storage().persistent().get(&key)
}

/// Ensure a borrower has a stable enumeration id and return it.
///
/// Both id↔borrower index entries are persistent per-borrower state, so each
/// write also refreshes its TTL. Without that, an index entry created once at
/// the network minimum TTL would be archived long before the credit line it
/// points at, breaking enumeration (`enumerate_credit_lines`) and any later
/// lookup keyed by id even though the line itself stays live (Issue #1273).
pub fn ensure_credit_line_id(env: &Env, borrower: &Address) -> u32 {
    if let Some(existing_id) = get_credit_line_id(env, borrower) {
        return existing_id;
    }

    let next_id = get_credit_line_count(env);

    let id_key = DataKey::CreditLineIdByBorrower(borrower.clone());
    env.storage().persistent().set(&id_key, &next_id);
    bump_persistent_ttl(env, &id_key);

    let reverse_key = DataKey::CreditLineBorrowerById(next_id);
    env.storage().persistent().set(&reverse_key, borrower);
    bump_persistent_ttl(env, &reverse_key);

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
    assert_total_utilized_conserved(env);
}

/// Adjust the global collateral accumulator by the change in one borrower balance.
pub fn adjust_total_collateral(env: &Env, previous_balance: i128, new_balance: i128) {
    let delta = new_balance
        .checked_sub(previous_balance)
        .unwrap_or_else(|| env.panic_with_error(ContractError::Overflow));
    if delta == 0 {
        return;
    }

    let updated_total = get_total_collateral(env)
        .checked_add(delta)
        .unwrap_or_else(|| env.panic_with_error(ContractError::Overflow));
    env.storage()
        .instance()
        .set(&DataKey::TotalCollateral, &updated_total);
    assert_total_collateral_conserved(env);
}

/// Persist a credit line and atomically apply its contribution delta to the
/// global total utilized accumulator.
pub fn persist_credit_line(
    env: &Env,
    borrower: &Address,
    line: &CreditLineData,
    previous_utilized: i128,
    previous_status: Option<CreditStatus>,
) {
    ensure_credit_line_id(env, borrower);
    env.storage().persistent().set(borrower, line);
    bump_credit_line_ttl(env, borrower);
    adjust_total_utilized(env, previous_utilized, line.utilized_amount);

    let is_now_active = line.status == CreditStatus::Active;
    let was_active = previous_status == Some(CreditStatus::Active);

    if is_now_active && !was_active {
        increment_active_line_count(env);
    } else if !is_now_active && was_active {
        decrement_active_line_count(env);
    }
}

/// Return a borrower's collateral balance and bump its persistent TTL.
///
/// The collateral balance is stored in a separate persistent entry from the
/// credit line, so its TTL must be independently refreshed on every read path
/// (deposit, withdraw, partial release, draw-credit ratio check, and the
/// `get_collateral` query). Without a bump the entry would be archived
/// independently of the credit-line entry, causing the borrower's collateral
/// to appear as zero.
pub fn get_collateral_balance(env: &Env, borrower: &Address) -> i128 {
    let key = DataKey::CollateralBalance(borrower.clone());
    if env.storage().persistent().has(&key) {
        bump_persistent_ttl(env, &key);
    }
    env.storage().persistent().get(&key).unwrap_or(0)
}

/// Persist a borrower collateral balance and update the global accumulator.
///
/// Bumps the TTL of the persistent `CollateralBalance(borrower)` entry so an
/// active borrower's collateral is never archived independently of the credit
/// line.
///
/// Note: the previous balance is read directly from storage (not via
/// [`get_collateral_balance`]) to avoid a redundant TTL bump on the read
/// path; the write path bumps TTL immediately after the set.
pub fn set_collateral_balance(env: &Env, borrower: &Address, balance: i128) {
    let key = DataKey::CollateralBalance(borrower.clone());
    let previous_balance = env.storage().persistent().get(&key).unwrap_or(0);
    env.storage().persistent().set(&key, &balance);
    bump_persistent_ttl(env, &key);
    adjust_total_collateral(env, previous_balance, balance);
}

/// Return the token used for collateral accounting.
///
/// The current contract uses the configured liquidity token for collateral
/// transfers as well.
pub fn get_collateral_token(env: &Env) -> Option<Address> {
    bump_instance_ttl(env);
    env.storage().instance().get(&DataKey::LiquidityToken)
}

/// Return the minimum collateral ratio in basis points, if configured.
pub fn get_min_collateral_ratio_bps(env: &Env) -> Option<u32> {
    bump_instance_ttl(env);
    env.storage()
        .instance()
        .get(&DataKey::MinCollateralRatioBps)
}

/// Set the minimum collateral ratio in basis points.
pub fn set_min_collateral_ratio_bps(env: &Env, ratio_bps: u32) {
    env.storage()
        .instance()
        .set(&DataKey::MinCollateralRatioBps, &ratio_bps);
}

/// Return the configured admin collateral cool-off interval, if set.
pub fn get_admin_collateral_cooldown_seconds(env: &Env) -> Option<u64> {
    env.storage()
        .instance()
        .get(&DataKey::AdminCollateralCooldownSeconds)
}

/// Set the admin collateral cool-off interval (admin only, enforced by caller).
pub fn set_admin_collateral_cooldown_seconds(env: &Env, seconds: u64) {
    env.storage()
        .instance()
        .set(&DataKey::AdminCollateralCooldownSeconds, &seconds);
}

/// Get the ledger timestamp of the last critical collateral admin action, if any.
pub fn get_last_admin_collateral_critical_action_ts(env: &Env) -> Option<u64> {
    env.storage()
        .instance()
        .get(&DataKey::LastColAdminActionTs)
}

/// Record the ledger timestamp of the last critical collateral admin action.
pub fn set_last_admin_collateral_critical_action_ts(env: &Env, ts: u64) {
    env.storage()
        .instance()
        .set(&DataKey::LastColAdminActionTs, &ts);
}
/// Return the risk weight for a specific collateral asset, in basis points,
/// if explicitly configured. `None` means no weight was ever set for this
/// asset; callers should treat that as 10_000 bps (100%, full value).
pub fn get_collateral_risk_weight_bps(env: &Env, asset: &Address) -> Option<u32> {
    env.storage()
        .instance()
        .get(&DataKey::CollateralRiskWeightBps(asset.clone()))
}

/// Set the risk weight for a specific collateral asset, in basis points.
/// Caller is responsible for admin auth and for validating `weight_bps <= 10_000`.
pub fn set_collateral_risk_weight_bps(env: &Env, asset: &Address, weight_bps: u32) {
    env.storage()
        .instance()
        .set(&DataKey::CollateralRiskWeightBps(asset.clone()), &weight_bps);
}

/// Return configured protocol fee basis points, if set.
pub fn get_protocol_fee_bps(env: &Env) -> Option<u32> {
    env.storage().instance().get(&DataKey::ProtocolFeeBps)
}

/// Persist protocol fee basis points.
pub fn set_protocol_fee_bps(env: &Env, bps: u32) {
    env.storage().instance().set(&DataKey::ProtocolFeeBps, &bps);
}

/// Return configured treasury address, if set.
pub fn get_treasury_address(env: &Env) -> Option<Address> {
    env.storage().instance().get(&DataKey::TreasuryAddress)
}

/// Persist configured treasury address.
pub fn set_treasury_address(env: &Env, treasury: &Address) {
    env.storage()
        .instance()
        .set(&DataKey::TreasuryAddress, treasury);
}

/// Return accumulated treasury balance.
pub fn get_treasury_balance(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::TreasuryBalance)
        .unwrap_or(0)
}

/// Add to accumulated treasury balance.
pub fn add_treasury_balance(env: &Env, amount: i128) {
    if amount == 0 {
        return;
    }
    let updated_balance = get_treasury_balance(env)
        .checked_add(amount)
        .unwrap_or_else(|| env.panic_with_error(ContractError::Overflow));
    env.storage()
        .instance()
        .set(&DataKey::TreasuryBalance, &updated_balance);
}

/// Deduct a timelocked withdrawal snapshot without discarding newer fees.
/// Returns the balance remaining after the withdrawal.
pub fn subtract_treasury_balance(env: &Env, amount: i128) -> i128 {
    let remaining = get_treasury_balance(env)
        .checked_sub(amount)
        .filter(|balance| amount >= 0 && *balance >= 0)
        .unwrap_or_else(|| env.panic_with_error(ContractError::InsufficientTreasuryBalance));
    env.storage()
        .instance()
        .set(&DataKey::TreasuryBalance, &remaining);
    remaining
}

/// Clear accumulated treasury balance after withdrawal.
pub fn clear_treasury_balance(env: &Env) {
    env.storage()
        .instance()
        .set(&DataKey::TreasuryBalance, &0_i128);
}

/// Return configured treasury fee share in basis points, if set.
pub fn get_treasury_fee_share_bps(env: &Env) -> Option<u32> {
    env.storage().instance().get(&DataKey::TreasuryFeeShareBps)
}

/// Persist treasury fee share in basis points.
pub fn set_treasury_fee_share_bps(env: &Env, bps: u32) {
    env.storage()
        .instance()
        .set(&DataKey::TreasuryFeeShareBps, &bps);
}

/// Return configured bounty pool address, if set.
pub fn get_bounty_address(env: &Env) -> Option<Address> {
    env.storage().instance().get(&DataKey::BountyAddress)
}

/// Persist configured bounty pool address.
pub fn set_bounty_address(env: &Env, bounty: &Address) {
    env.storage()
        .instance()
        .set(&DataKey::BountyAddress, bounty);
}

/// Return accumulated bounty pool balance.
pub fn get_bounty_balance(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::BountyBalance)
        .unwrap_or(0)
}

/// Add to accumulated bounty pool balance.
pub fn add_bounty_balance(env: &Env, amount: i128) {
    if amount == 0 {
        return;
    }
    let updated_balance = get_bounty_balance(env)
        .checked_add(amount)
        .unwrap_or_else(|| env.panic_with_error(ContractError::Overflow));
    env.storage()
        .instance()
        .set(&DataKey::BountyBalance, &updated_balance);
}

/// Clear accumulated bounty pool balance after withdrawal.
pub fn clear_bounty_balance(env: &Env) {
    env.storage()
        .instance()
        .set(&DataKey::BountyBalance, &0_i128);
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
/// indicating a reentrant call. Prefer [`enter_reentrancy_guard`], which sets
/// and releases the flag for you; call this directly only when the guard
/// lifetime cannot be expressed as a scope. The flag does not need to be
/// cleared on failure paths — see the module-level revert-on-panic notes.
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
/// Runs automatically when a [`ReentrancyGuard`] drops, i.e. on the success
/// path of a guarded entrypoint. Failure paths leave the flag alone on
/// purpose: a panic rolls the write back, so clearing first would be
/// redundant (see the module-level revert-on-panic notes).
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

/// RAII handle for the contract-wide reentrancy guard.
///
/// Returned by [`enter_reentrancy_guard`]; releases the guard when dropped, so
/// a guarded entrypoint only has to bind it once at the top:
///
/// ```ignore
/// pub fn draw_credit(env: Env, borrower: Address, amount: i128) {
///     let _guard = enter_reentrancy_guard(&env);
///     // ... validation and mutation ...
///     // error branches just `panic_with_error`; the guard is reverted with
///     // the rest of the frame, and released here on the success path.
/// }
/// ```
///
/// A guard that is never bound is released immediately, so the value is
/// `#[must_use]` and discarding it as a bare expression statement (for example
/// `enter_reentrancy_guard(&env);`) is a lint error. Bind it to a named
/// variable that lives as long as the guarded body:
///
/// ```ignore
/// let _guard = enter_reentrancy_guard(&env); // held for the whole body
/// ```
///
/// Note that `let _ = enter_reentrancy_guard(&env);` also releases the guard at
/// once, because `_` drops the value on the spot; use `_guard`, not `_`.
#[must_use = "bind the guard to a named variable for the duration of the guarded scope"]
pub struct ReentrancyGuard<'a> {
    env: &'a Env,
}

impl Drop for ReentrancyGuard<'_> {
    fn drop(&mut self) {
        clear_reentrancy_guard(self.env);
    }
}

/// Acquire the contract-wide reentrancy guard for the current scope.
///
/// Reverts with [`ContractError::Reentrancy`] (`Reentrancy = 11`) if the guard
/// is already held, i.e. on a re-entrant call. Otherwise returns a
/// [`ReentrancyGuard`] that clears the flag when it drops, which happens on
/// the success path of the calling entrypoint. Failure paths need no cleanup:
/// a panic reverts the flag write along with the rest of the frame (see the
/// module-level revert-on-panic notes).
pub fn enter_reentrancy_guard(env: &Env) -> ReentrancyGuard<'_> {
    set_reentrancy_guard(env);
    ReentrancyGuard { env }
}

/// Set a per-borrower interest rate floor (admin only, enforced by caller).
///
/// Bumps the persistent TTL of the entry on write so it stays live for the
/// lifetime of an active credit line.
pub fn set_borrower_rate_floor(env: &Env, borrower: &Address, floor_bps: Option<u32>) {
    if let Some(floor) = floor_bps {
        assert!(
            floor <= crate::risk::MAX_INTEREST_RATE_BPS,
            "floor exceeds max rate"
        );
    }
    let key = DataKey::RateFloorBps(borrower.clone());
    if let Some(floor) = floor_bps {
        env.storage()
            .persistent()
            .set(&key, &floor);
    } else {
        env.storage()
            .persistent()
            .remove(&key);
    }
    bump_persistent_ttl(env, &key);
}

/// Get the per-borrower interest rate floor, if set.
///
/// Bumps the persistent TTL of the entry so it is not archived independently
/// of the credit line.
pub fn get_borrower_rate_floor(env: &Env, borrower: &Address) -> Option<u32> {
    let key = DataKey::RateFloorBps(borrower.clone());
    if env.storage().persistent().has(&key) {
        bump_persistent_ttl(env, &key);
    }
    env.storage().persistent().get(&key)
}

/// Set a per-borrower interest rate ceiling (admin only, enforced by caller).
///
/// Bumps the persistent TTL of the entry on write so it stays live for the
/// lifetime of an active credit line.
pub fn set_borrower_rate_ceiling(env: &Env, borrower: &Address, ceiling_bps: Option<u32>) {
    if let Some(ceiling) = ceiling_bps {
        assert!(
            ceiling <= crate::risk::MAX_INTEREST_RATE_BPS,
            "ceiling exceeds max rate"
        );
    }
    let key = DataKey::RateCeilingBps(borrower.clone());
    if let Some(ceiling) = ceiling_bps {
        env.storage()
            .persistent()
            .set(&key, &ceiling);
    } else {
        env.storage()
            .persistent()
            .remove(&key);
    }
    bump_persistent_ttl(env, &key);
}

/// Get the per-borrower interest rate ceiling, if set.
///
/// Bumps the persistent TTL of the entry so it is not archived independently
/// of the credit line.
pub fn get_borrower_rate_ceiling(env: &Env, borrower: &Address) -> Option<u32> {
    let key = DataKey::RateCeilingBps(borrower.clone());
    if env.storage().persistent().has(&key) {
        bump_persistent_ttl(env, &key);
    }
    env.storage().persistent().get(&key)
}

/// Set a per-borrower max utilization ratio cap in basis points (admin only).
/// Pass `None` to remove the cap.
///
/// Bumps the persistent TTL of the entry on write so it stays live for the
/// lifetime of an active credit line.
pub fn set_utilization_cap_bps(env: &Env, borrower: &Address, cap_bps: Option<u32>) {
    let key = DataKey::UtilizationCapBps(borrower.clone());
    if let Some(cap) = cap_bps {
        env.storage()
            .persistent()
            .set(&key, &cap);
    } else {
        env.storage()
            .persistent()
            .remove(&key);
    }
    bump_persistent_ttl(env, &key);
}

/// Get the per-borrower max utilization ratio cap, if set.
///
/// Bumps the persistent TTL of the entry so an active borrower's utilization
/// cap is not silently archived by the network.
pub fn get_utilization_cap_bps(env: &Env, borrower: &Address) -> Option<u32> {
    let key = DataKey::UtilizationCapBps(borrower.clone());
    if env.storage().persistent().has(&key) {
        bump_persistent_ttl(env, &key);
    }
    env.storage().persistent().get(&key)
}

/// Clear the installment schedule for a borrower.
pub fn clear_repayment_schedule(env: &Env, borrower: &Address) {
    env.storage()
        .persistent()
        .remove(&DataKey::RepaymentSchedule(borrower.clone()));
}

/// Block a borrower from drawing (admin only, enforced by caller).
///
/// Bumps the persistent TTL of the entry on write so it stays live for the
/// lifetime of an active credit line.
pub fn set_borrower_blocked(env: &Env, borrower: &Address, blocked: bool) {
    let key = DataKey::BlockedBorrower(borrower.clone());
    if blocked {
        env.storage()
            .persistent()
            .set(&key, &true);
        bump_persistent_ttl(env, &key);
    } else {
        env.storage()
            .persistent()
            .remove(&key);
        // Only the instance TTL can be bumped here: the per-borrower record has
        // just been removed, and `extend_ttl` on a removed key reverts with
        // `Storage(MissingValue)`. Bumping unconditionally made
        // `unblock_borrower` revert for every borrower and left the block in
        // place. Unblocking is now idempotent, as the entrypoint documents.
        bump_instance_ttl(env);
    }
}

/// Check if a borrower is blocked from drawing.
///
/// Bumps the persistent TTL of the entry so a blocked borrower's flag is
/// not silently archived by the network.
pub fn is_borrower_blocked(env: &Env, borrower: &Address) -> bool {
    let key = DataKey::BlockedBorrower(borrower.clone());
    if env.storage().persistent().has(&key) {
        bump_persistent_ttl(env, &key);
    }
    env.storage().persistent().get(&key).unwrap_or(false)
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
    env.storage()
        .instance()
        .set(&DataKey::AuctionContract, addr);
}

// ── Close factor (partial liquidation cap) ─────────────────────────────────────

/// Return the protocol-level max close factor in basis points.
/// Defaults to 10_000 (full liquidation allowed) when not set.
pub fn get_close_factor_bps(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::CloseFactorBps)
        .unwrap_or(10_000)
}

/// Set the protocol-level max close factor in basis points (admin only).
/// Supply `10_000` for full-liquidation-only (no partial), or any value
/// `1..=10_000` to cap partial settlements.
pub fn set_close_factor_bps(env: &Env, bps: u32) {
    env.storage().instance().set(&DataKey::CloseFactorBps, &bps);
}

/// Return the installment schedule for a borrower, if configured.
///
/// Bumps the persistent TTL of the entry on read so an active borrower's
/// schedule is not silently archived by the network (Issue #1273).
pub fn get_repayment_schedule(env: &Env, borrower: &Address) -> Option<RepaymentSchedule> {
    let key = DataKey::RepaymentSchedule(borrower.clone());
    if env.storage().persistent().has(&key) {
        bump_persistent_ttl(env, &key);
    }
    env.storage().persistent().get(&key)
}

/// Persist the installment schedule for a borrower and bump its persistent TTL.
///
/// The schedule lives in its own persistent entry, so writing it must extend
/// that entry's TTL as well; otherwise a schedule installed once could expire
/// even while the credit line itself keeps being refreshed on read (Issue
/// #1273).
pub fn set_repayment_schedule(env: &Env, borrower: &Address, schedule: &RepaymentSchedule) {
    let key = DataKey::RepaymentSchedule(borrower.clone());
    env.storage().persistent().set(&key, schedule);
    bump_persistent_ttl(env, &key);
}

/// Get the last draw timestamp for a borrower, if any.
///
/// Bumps the persistent TTL of the entry so an active borrower's last-draw
/// timestamp is not silently archived by the network.
pub fn get_last_draw_ts(env: &Env, borrower: &Address) -> Option<u64> {
    let key = DataKey::LastDrawTs(borrower.clone());
    if env.storage().persistent().has(&key) {
        bump_persistent_ttl(env, &key);
    }
    env.storage().persistent().get(&key)
}

/// Set the last draw timestamp for a borrower and bump its persistent TTL.
///
/// The timestamp lives in its own persistent entry, so writing it must also
/// extend that entry's TTL to keep it live for the lifetime of an active
/// credit line.
pub fn set_last_draw_ts(env: &Env, borrower: &Address, ts: u64) {
    let key = DataKey::LastDrawTs(borrower.clone());
    env.storage().persistent().set(&key, &ts);
    bump_persistent_ttl(env, &key);
}

/// Return the recorded original draw amount for `(borrower, timestamp)`, if any.
///
/// The draw audit trail is per-borrower persistent state, so the read path
/// refreshes its TTL just like the credit-line getter does; otherwise an entry
/// could be archived while the draw is still reversible (Issue #1273).
pub fn get_draw_audit(env: &Env, borrower: &Address, timestamp: u64) -> Option<i128> {
    let key = DataKey::DrawAudit(DrawAuditKey {
        borrower: borrower.clone(),
        timestamp,
    });
    if env.storage().persistent().has(&key) {
        bump_persistent_ttl(env, &key);
    }
    env.storage().persistent().get(&key)
}

/// Return the amount already reversed against an original draw (0 if none).
///
/// Bumps the persistent TTL on read so the reversal tracker stays live for as
/// long as the original draw can be reversed (Issue #1273).
pub fn get_draw_reversed_amount(env: &Env, borrower: &Address, timestamp: u64) -> i128 {
    let key = DataKey::DrawReversedAmount(DrawAuditKey {
        borrower: borrower.clone(),
        timestamp,
    });
    if env.storage().persistent().has(&key) {
        bump_persistent_ttl(env, &key);
    }
    env.storage().persistent().get(&key).unwrap_or(0)
}

/// Persist the amount reversed against an original draw and bump its TTL.
///
/// The reversal tracker lives in its own persistent entry, so writing it must
/// extend that entry's TTL as well (Issue #1273).
pub fn set_draw_reversed_amount(env: &Env, borrower: &Address, timestamp: u64, amount: i128) {
    let key = DataKey::DrawReversedAmount(DrawAuditKey {
        borrower: borrower.clone(),
        timestamp,
    });
    env.storage().persistent().set(&key, &amount);
    bump_persistent_ttl(env, &key);
}

/// Get the configured draw min interval, if set.
pub fn get_draw_min_interval(env: &Env) -> Option<u64> {
    env.storage()
        .instance()
        .get(&DataKey::DrawMinIntervalSeconds)
}

/// Set the draw min interval (admin only, enforced by caller).
pub fn set_draw_min_interval(env: &Env, seconds: u64) {
    env.storage()
        .instance()
        .set(&DataKey::DrawMinIntervalSeconds, &seconds);
}

/// Get the configured per-borrower admin action cooldown, if set.
/// Return the configured borrow admin cooldown, if set.
pub fn get_borrow_admin_cooldown(env: &Env) -> Option<u64> {
    env.storage()
        .instance()
        .get(&DataKey::BorrowAdminCooldownSeconds)
}

/// Persist the borrow admin cooldown in seconds.
pub fn set_borrow_admin_cooldown(env: &Env, seconds: u64) {
    env.storage()
        .instance()
        .set(&DataKey::BorrowAdminCooldownSeconds, &seconds);
}

/// Return the last successful critical borrow admin-action timestamp for `borrower`, if any.
pub fn get_last_borrow_admin_action_ts(env: &Env, borrower: &Address) -> Option<u64> {
    env.storage()
        .persistent()
        .get(&DataKey::LastBorrowAdminActionTs(borrower.clone()))
}

/// Persist the last successful critical borrow admin-action timestamp for `borrower`.
pub fn set_last_borrow_admin_action_ts(env: &Env, borrower: &Address, ts: u64) {
    let key = DataKey::LastBorrowAdminActionTs(borrower.clone());
    env.storage().persistent().set(&key, &ts);
    bump_persistent_ttl(env, &key);
}

/// Return the configured accrual admin cooldown, if set.
pub fn get_accrual_admin_cooldown(env: &Env) -> Option<u64> {
    env.storage()
        .instance()
        .get(&DataKey::AccrualAdminCooldownSeconds)
}

/// Persist the accrual admin cooldown in seconds.
pub fn set_accrual_admin_cooldown(env: &Env, seconds: u64) {
    env.storage()
        .instance()
        .set(&DataKey::AccrualAdminCooldownSeconds, &seconds);
}

/// Return the last successful critical accrual admin-action timestamp for `borrower`, if any.
pub fn get_last_accrual_admin_action_ts(env: &Env, borrower: &Address) -> Option<u64> {
    env.storage()
        .persistent()
        .get(&DataKey::LastAccrualAdminActionTs(borrower.clone()))
}

/// Persist the last successful critical accrual admin-action timestamp for `borrower`.
pub fn set_last_accrual_admin_action_ts(env: &Env, borrower: &Address, ts: u64) {
    let key = DataKey::LastAccrualAdminActionTs(borrower.clone());
    env.storage().persistent().set(&key, &ts);
    bump_persistent_ttl(env, &key);
}

/// Check if the protocol is paused.
pub fn is_paused(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&paused_key(env))
        .unwrap_or(false)
}

/// Set the protocol pause state (admin only, enforced by caller).
///
/// This is a **pure flag write** — it never touches the pause reason. Reason
/// maintenance (clear on unpause / on reason-less pause, write on pause-with-
/// reason) is the responsibility of the entrypoints so that idempotent
/// no-ops cannot clobber the audit trail. See [`set_pause_reason`] and
/// [`clear_pause_reason`].
///
/// Callers are expected to detect no-op transitions (requesting the state the
/// contract is already in) *before* calling this, so that duplicate pause or
/// unpause requests do not emit misleading transition events.
///
/// # Storage
/// - **Type**: Instance storage (shared TTL with all instance keys)
/// - **Key**: `Symbol("paused")`
/// - **TTL Note**: Shares instance TTL — extend alongside other instance keys.
pub fn set_paused(env: &Env, paused: bool) {
    env.storage().instance().set(&paused_key(env), &paused);
}

/// Clear any stored pause reason.
///
/// Called on unpause and on a reason-less pause so the stored reason always
/// reflects the most recent pause invocation. This prevents a stale reason
/// (recorded by an earlier pause-with-reason) from surviving into a later
/// reason-less pause or unpause.
pub fn clear_pause_reason(env: &Env) {
    env.storage().instance().remove(&DataKey::PauseReason);
}

/// Get the structured pause reason, if one was recorded during the last pause.
///
/// Returns `None` when no pause reason was set (e.g., before any pause or
/// if the admin used the reason-less `set_protocol_paused(bool)`).
pub fn get_pause_reason(env: &Env) -> Option<crate::types::PauseReason> {
    env.storage().instance().get(&DataKey::PauseReason)
}

/// Store a structured pause reason alongside the pause flag.
///
/// Should be called by the entrypoint that sets the pause flag, so the reason
/// and the flag are written atomically within the same host transaction.
pub fn set_pause_reason(env: &Env, reason: &crate::types::PauseReason) {
    env.storage().instance().set(&DataKey::PauseReason, reason);
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
//   OracleReferenceRefreshTs — timestamp of last admin recovery refresh
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
    env.storage()
        .instance()
        .set(&DataKey::OracleLastPrice, &price);
    env.storage()
        .instance()
        .set(&DataKey::OracleLastPriceTs, &ts);
}

/// Get the timestamp of the last admin oracle-reference refresh.
pub fn get_oracle_reference_refresh_ts(env: &Env) -> Option<u64> {
    env.storage()
        .instance()
        .get(&DataKey::OracleReferenceRefreshTs)
}

/// Persist the timestamp of the last admin oracle-reference refresh.
pub fn set_oracle_reference_refresh_ts(env: &Env, ts: u64) {
    env.storage()
        .instance()
        .set(&DataKey::OracleReferenceRefreshTs, &ts);
}

// ── Multi-oracle quorum price (resolved from multiple feeds) ────────────────

/// Get the last accepted multi-oracle quorum price, if any.
///
/// Returns `None` before the first successful `submit_oracle_prices` call.
/// Always read together with [`get_oracle_quorum_price_ts`] to interpret staleness.
pub fn get_oracle_quorum_price(env: &Env) -> Option<i128> {
    env.storage()
        .instance()
        .get(&DataKey::OracleQuorumPrice)
}

/// Get the timestamp of the last accepted multi-oracle quorum price, if any.
pub fn get_oracle_quorum_price_ts(env: &Env) -> Option<u64> {
    env.storage()
        .instance()
        .get(&DataKey::OracleQuorumPriceTs)
}

/// Persist a newly resolved quorum price and its timestamp atomically.
///
/// Called after `submit_oracle_prices` resolves the quorum median.
/// The two `instance().set(..)` calls are part of the same host transaction.
pub fn set_oracle_quorum_price(env: &Env, price: i128, ts: u64) {
    env.storage()
        .instance()
        .set(&DataKey::OracleQuorumPrice, &price);
    env.storage()
        .instance()
        .set(&DataKey::OracleQuorumPriceTs, &ts);
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
    env.storage()
        .instance()
        .get(&DataKey::PenaltySurchargeBps)
        .unwrap_or(0)
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
    env.storage()
        .instance()
        .set(&DataKey::PenaltySurchargeBps, &bps);
}

// ── Flat late fee per missed installment ─────────────────────────────────────

/// Get the configured flat late fee per missed installment, if set.
///
/// Returns `0` when the key is absent. A return of `0` means no flat late fee
/// is charged, preserving existing behavior for contracts that do not use this
/// feature.
///
/// # Storage
/// - **Type**: Instance storage
/// - **Key**: [`DataKey::LateFeeFlat`]
pub fn get_late_fee_flat(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::LateFeeFlat)
        .unwrap_or(0)
}

/// Set the flat late fee per missed installment.
///
/// When non-zero, this fee is credited to `TreasuryBalance` for each
/// installment that is detected as overdue during
/// [`advance_repayment_schedule_after_repay`]. Admin auth must be enforced
/// by the caller.
///
/// # Storage
/// - **Type**: Instance storage
/// - **Key**: [`DataKey::LateFeeFlat`]
pub fn set_late_fee_flat(env: &Env, fee: i128) {
    env.storage().instance().set(&DataKey::LateFeeFlat, &fee);
}

// ── LateFeeConfig helpers ─────────────────────────────────────────────────────

/// Get the structured late-fee configuration, if set.
///
/// Returns `None` when no structured config has been stored, meaning the
/// contract falls back to the legacy [`DataKey::LateFeeFlat`] and
/// [`DataKey::PenaltySurchargeBps`] keys.
///
/// # Storage
/// - **Type**: Instance storage
/// - **Key**: [`DataKey::LateFeeConfig`]
pub fn get_late_fee_config(env: &Env) -> Option<crate::penalties::LateFeeConfig> {
    env.storage().instance().get(&DataKey::LateFeeConfig)
}

/// Persist a structured late-fee configuration.
///
/// Passing `None` removes the entry, reverting to the legacy flat/surcharge
/// keys.  Admin auth must be enforced by the caller.
///
/// # Storage
/// - **Type**: Instance storage
/// - **Key**: [`DataKey::LateFeeConfig`]
pub fn set_late_fee_config(env: &Env, config: Option<crate::penalties::LateFeeConfig>) {
    match config {
        Some(c) => env.storage().instance().set(&DataKey::LateFeeConfig, &c),
        None => env.storage().instance().remove(&DataKey::LateFeeConfig),
    }
}

// ── Borrower blocklist helpers ───────────────────────────────────────────────

/// Unblock a borrower (convenience wrapper).
pub fn set_borrower_unblocked(env: &Env, borrower: &Address) {
    set_borrower_blocked(env, borrower, false);
}

// ── Borrower temporary freeze helpers ────────────────────────────────────────

/// Freeze a borrower's draws until the specified expiry timestamp (admin only,
/// enforced by caller).
///
/// Stores the expiry `u64` timestamp under [`DataKey::FrozenBorrower(Address)`]
/// in persistent storage. Draws are blocked when `env.ledger().timestamp() < expiry_ts`.
/// Once the expiry is reached or passed, draws automatically resume — no admin
/// unfreeze call is required.
///
/// # Auto-expiry
/// The freeze is time-bounded: [`is_borrower_frozen`] compares the current
/// ledger timestamp against the stored expiry. When `now >= expiry_ts`, it
/// returns `false` without any admin intervention.
///
/// # Storage
/// - **Type**: Persistent storage (per-borrower, shares TTL with other persistent keys)
/// - **Key**: [`DataKey::FrozenBorrower`]
pub fn set_borrower_frozen_until(env: &Env, borrower: &Address, expiry_ts: u64) {
    let key = DataKey::FrozenBorrower(borrower.clone());
    env.storage().persistent().set(&key, &expiry_ts);
    bump_borrower_frozen_ttl(env, borrower);
}

/// Check if a borrower is temporarily frozen from drawing.
///
/// Returns `true` when the current ledger timestamp is strictly less than the
/// stored expiry timestamp. Returns `false` when:
/// - No freeze has been set (key is absent),
/// - The freeze has expired (`now >= expiry_ts`).
///
/// Bumps the persistent TTL of the entry so an active borrower's freeze
/// state is not silently archived by the network.
///
/// # Time semantics
/// Uses `env.ledger().timestamp()` so the check is deterministic per ledger.
///
/// # Storage
/// - **Type**: Persistent storage read
/// - **Key**: [`DataKey::FrozenBorrower`]
pub fn is_borrower_frozen(env: &Env, borrower: &Address) -> bool {
    let now = env.ledger().timestamp();
    let key = DataKey::FrozenBorrower(borrower.clone());
    if env.storage().persistent().has(&key) {
        bump_borrower_frozen_ttl(env, borrower);
        env.storage()
            .persistent()
            .get(&key)
            .is_some_and(|expiry: u64| now < expiry)
    } else {
        false
    }
}

/// Get the freeze expiry timestamp for a borrower, if one is set.
///
/// Returns `Some(expiry_ts)` when a temporary freeze is in effect (even if
/// expired — callers should compare against `now` themselves). Returns `None`
/// when no freeze has ever been set.
pub fn get_borrower_frozen_until(env: &Env, borrower: &Address) -> Option<u64> {
    let key = DataKey::FrozenBorrower(borrower.clone());
    if env.storage().persistent().has(&key) {
        bump_borrower_frozen_ttl(env, borrower);
        env.storage().persistent().get(&key)
    } else {
        None
    }
}

/// Remove the temporary freeze for a borrower (admin only, enforced by caller).
///
/// This is a convenience for an admin who wants to lift a freeze before its
/// natural expiry. If no freeze was set, this is a no-op.
pub fn clear_borrower_frozen(env: &Env, borrower: &Address) {
    env.storage()
        .persistent()
        .remove(&DataKey::FrozenBorrower(borrower.clone()));
}

// ── Multi-collateral (per-borrower, per-token) ────────────────────────────────

/// Return a borrower's balance for a specific collateral token and bump
/// the persistent entry's TTL so it remains live alongside the credit line.
pub fn get_collateral_balance_for_token(env: &Env, borrower: &Address, token: &Address) -> i128 {
    let key = DataKey::CollateralBalanceV2(borrower.clone(), token.clone());
    if env.storage().persistent().has(&key) {
        bump_persistent_ttl(env, &key);
    }
    env.storage().persistent().get(&key).unwrap_or(0)
}

/// Persist a borrower's balance for a specific collateral token and update the global accumulator.
pub fn set_collateral_balance_for_token(env: &Env, borrower: &Address, token: &Address, balance: i128) {
    let key = DataKey::CollateralBalanceV2(borrower.clone(), token.clone());
    let previous = get_collateral_balance_for_token(env, borrower, token);
    env.storage().persistent().set(&key, &balance);
    bump_persistent_ttl(env, &key);
    adjust_total_collateral(env, previous, balance);
}

/// Return the allowlisted collateral token addresses, or an empty vec.
pub fn get_collateral_token_allowlist(env: &Env) -> soroban_sdk::Vec<Address> {
    env.storage()
        .instance()
        .get(&DataKey::CollateralTokenAllowlist)
        .unwrap_or_else(|| soroban_sdk::Vec::new(env))
}

/// Overwrite the collateral token allowlist.
pub fn set_collateral_token_allowlist(env: &Env, tokens: &soroban_sdk::Vec<Address>) {
    env.storage()
        .instance()
        .set(&DataKey::CollateralTokenAllowlist, tokens);
}

/// Return `true` when `token` is in the collateral allowlist.
pub fn is_collateral_token_allowed(env: &Env, token: &Address) -> bool {
    get_collateral_token_allowlist(env).contains(token.clone())
}

// ── Risk admin cooldown helpers ─────────────────────────────────────────────

/// Get the configured risk admin cooldown duration in seconds.
/// Returns `0` when the cooldown is disabled (default).
pub fn get_risk_admin_cooldown_seconds(env: &Env) -> u64 {
    let key = symbol_short!("rad_cool");
    env.storage()
        .instance()
        .get(&key)
        .unwrap_or(0)
}

/// Set the risk admin cooldown duration in seconds.
pub fn set_risk_admin_cooldown_seconds(env: &Env, seconds: u64) {
    let key = symbol_short!("rad_cool");
    env.storage()
        .instance()
        .set(&key, &seconds);
}

/// Get the timestamp of the last risk admin action.
/// Returns `0` when no action has been recorded yet.
///
/// # Deprecation note
/// This global key is retained for backward compatibility. New code should
/// use [`get_last_risk_admin_action_ts_for`] which is scoped per borrower.
pub fn get_last_risk_admin_action_ts(env: &Env) -> u64 {
    let key = symbol_short!("rad_last");
    env.storage()
        .instance()
        .get(&key)
        .unwrap_or(0)
}

/// Set the timestamp of the last risk admin action.
///
/// # Deprecation note
/// This global key is retained for backward compatibility. New code should
/// use [`set_last_risk_admin_action_ts_for`] which is scoped per borrower.
pub fn set_last_risk_admin_action_ts(env: &Env, ts: u64) {
    let key = symbol_short!("rad_last");
    env.storage()
        .instance()
        .set(&key, &ts);
}

/// Get the timestamp of the last risk admin action for a specific borrower.
///
/// Keyed by `(symbol_short!("rad_last"), borrower)` in persistent storage,
/// mirroring `LastAccrualAdminActionTs` which is also per-borrower.
/// Returns `0` when no action has been recorded for this borrower yet.
pub fn get_last_risk_admin_action_ts_for(env: &Env, borrower: &Address) -> u64 {
    let key = (symbol_short!("rad_last"), borrower.clone());
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(0)
}

/// Set the timestamp of the last risk admin action for a specific borrower.
///
/// Keyed by `(symbol_short!("rad_last"), borrower)` in persistent storage.
pub fn set_last_risk_admin_action_ts_for(env: &Env, borrower: &Address, ts: u64) {
    let key = (symbol_short!("rad_last"), borrower.clone());
    env.storage()
        .persistent()
        .set(&key, &ts);
}

/// Assert that the risk admin cooldown has elapsed since the last action
/// for the given borrower.
///
/// # Scope (Issue #1280)
/// The cooldown is now **per-borrower**: each borrower's last-action timestamp
/// is tracked independently. Updating borrower A's risk parameters does not
/// start or reset the cooldown for borrower B.
///
/// # Behaviour
/// - When `cooldown_seconds == 0` (default): always passes (disabled).
/// - When no prior action exists for this borrower (`last_ts == 0`): always
///   passes (first update is never blocked).
/// - Otherwise: reverts with [`ContractError::RiskAdminCooldownActive`] if
///   `now < last_ts + cooldown_seconds`.
pub fn assert_risk_admin_cooldown_elapsed_for(env: &Env, borrower: &Address) {
    let cooldown = get_risk_admin_cooldown_seconds(env);
    if cooldown == 0 {
        return;
    }
    let last_ts = get_last_risk_admin_action_ts_for(env, borrower);
    if last_ts == 0 {
        return;
    }
    let now = env.ledger().timestamp();
    if now < last_ts.saturating_add(cooldown) {
        env.panic_with_error(ContractError::RiskAdminCooldownActive);
    }
}

/// Assert that the risk admin cooldown has elapsed since the last action.
/// Panics with `RiskAdminCooldownActive` if the cooldown has not yet elapsed.
/// When `last_ts` is `0` (no prior action recorded), the cooldown is not
/// enforced so the first call always succeeds.
///
/// # Deprecation note
/// This checks the legacy global timestamp. Prefer
/// [`assert_risk_admin_cooldown_elapsed_for`] for per-borrower enforcement.
pub fn assert_risk_admin_cooldown_elapsed(env: &Env) {
    let cooldown = get_risk_admin_cooldown_seconds(env);
    if cooldown == 0 {
        return;
    }
    let last_ts = get_last_risk_admin_action_ts(env);
    if last_ts == 0 {
        return;
    }
    let now = env.ledger().timestamp();
    if now < last_ts.saturating_add(cooldown) {
        env.panic_with_error(ContractError::RiskAdminCooldownActive);
    }
}

// ── Freeze cooldown helpers ─────────────────────────────────────────────────

pub fn get_freeze_cooldown_seconds(env: &Env) -> Option<u64> {
    env.storage()
        .instance()
        .get(&DataKey::FreezeCooldownSeconds)
        .filter(|&secs: &u64| secs > 0)
}

pub fn set_freeze_cooldown_seconds(env: &Env, seconds: u64) {
    if seconds == 0 {
        env.storage()
            .instance()
            .remove(&DataKey::FreezeCooldownSeconds);
    } else {
        env.storage()
            .instance()
            .set(&DataKey::FreezeCooldownSeconds, &seconds);
    }
}

pub fn get_last_freeze_timestamp(env: &Env) -> Option<u64> {
    env.storage()
        .instance()
        .get(&DataKey::LastFreezeTimestamp)
}

pub fn set_last_freeze_timestamp(env: &Env, ts: u64) {
    env.storage()
        .instance()
        .set(&DataKey::LastFreezeTimestamp, &ts);
}

pub fn record_freeze_timestamp_if_cooldown(env: &Env) {
    if get_freeze_cooldown_seconds(env).is_some() {
        set_last_freeze_timestamp(env, env.ledger().timestamp());
    }
}

pub fn enforce_freeze_cooldown(env: &Env) {
    let Some(cooldown_secs) = get_freeze_cooldown_seconds(env) else {
        return;
    };
    if let Some(last_ts) = get_last_freeze_timestamp(env) {
        let now = env.ledger().timestamp();
        if now < last_ts.saturating_add(cooldown_secs) {
            env.panic_with_error(ContractError::FreezeCooldownActive);
        }
    }
}

// ── Grace period config (instance) ───────────────────────────────────────────

pub fn get_grace_period_config(env: &Env) -> Option<GracePeriodConfig> {
    env.storage().instance().get(&grace_period_key(env))
}

// ── Per-borrower liquidation grace (persistent) ───────────────────────────────

pub fn get_per_borrower_liquidation_grace(env: &Env, borrower: &Address) -> u64 {
    env.storage()
        .persistent()
        .get(&DataKey::LiquidationGracePeriod(borrower.clone()))
        .unwrap_or(0)
}

pub fn set_per_borrower_liquidation_grace(env: &Env, borrower: &Address, secs: u64) {
    let key = DataKey::LiquidationGracePeriod(borrower.clone());
    if secs == 0 {
        env.storage().persistent().remove(&key);
    } else {
        env.storage().persistent().set(&key, &secs);
        bump_persistent_ttl(env, &key);
    }
}

// ── Oracle quorum config (instance) ─────────────────────────────────────────

pub fn get_oracle_quorum_config(env: &Env) -> Option<OracleQuorumConfig> {
    env.storage().instance().get(&DataKey::OracleQuorumConfig)
}

pub fn set_oracle_quorum_config(env: &Env, cfg: &OracleQuorumConfig) {
    env.storage().instance().set(&DataKey::OracleQuorumConfig, cfg);
}

// ── Treasury withdrawal proposal (instance) ───────────────────────────────────

pub fn get_pending_treasury_withdrawal(env: &Env) -> Option<TreasuryWithdrawalProposal> {
    env.storage()
        .instance()
        .get(&DataKey::PendingTreasuryWithdrawal)
}

pub fn set_pending_treasury_withdrawal(env: &Env, proposal: &TreasuryWithdrawalProposal) {
    env.storage()
        .instance()
        .set(&DataKey::PendingTreasuryWithdrawal, proposal);
}

pub fn clear_pending_treasury_withdrawal(env: &Env) {
    env.storage()
        .instance()
        .remove(&DataKey::PendingTreasuryWithdrawal);
}

// ── Max borrower exposure (persistent) ────────────────────────────────────────

/// Get the per-borrower absolute exposure cap, if set.
///
/// Bumps the persistent TTL on every read so an active borrower's cap is not
/// silently archived by the Soroban network between interactions.
pub fn get_max_borrower_exposure(env: &Env, borrower: &Address) -> Option<i128> {
    let key = DataKey::MaxBorrowerExposure(borrower.clone());
    if env.storage().persistent().has(&key) {
        bump_persistent_ttl(env, &key);
    }
    env.storage().persistent().get(&key)
}

/// Set (or clear) the per-borrower absolute exposure cap.
///
/// - `cap > 0` — writes the cap to persistent storage and bumps TTL.
/// - `cap == 0` — removes the entry, making the borrower uncapped again.
///
/// Callers are responsible for validating `cap >= 0` before calling this
/// function; passing a negative value will store it as-is and every draw
/// will be blocked for that borrower.
pub fn set_max_borrower_exposure(env: &Env, borrower: &Address, cap: i128) {
    let key = DataKey::MaxBorrowerExposure(borrower.clone());
    if cap == 0 {
        env.storage().persistent().remove(&key);
    } else {
        env.storage().persistent().set(&key, &cap);
        bump_persistent_ttl(env, &key);
    }
}
