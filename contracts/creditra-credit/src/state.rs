use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Timestamp, Uint128};
use cw_storage_plus::{Item, Map};

use crate::penalties::LateFeeConfig;

#[cw_serde]
pub struct Config {
    pub owner: Addr,
}

/// A credit line represents a borrowing facility for a borrower.
#[cw_serde]
pub struct CreditLine {
    pub id: u64,
    pub borrower: Addr,
    pub collateral_denom: String,
    pub collateral_amount: Uint128,
    pub credit_denom: String,
    pub credit_amount: Uint128,
    pub active: bool,
}

/// A draw is a borrowing event drawn against a credit line.
#[cw_serde]
pub struct Draw {
    pub id: u64,
    pub credit_line_id: u64,
    pub amount: Uint128,
    pub denom: String,
    pub drawn_at: Timestamp,
    pub drawn_by: Addr,
    pub repaid: bool,
}

/// The type of action recorded in a draw audit entry.
#[cw_serde]
pub enum DrawAction {
    DrawCreated,
    Repaid,
    Liquidated,
    MemoAdded,
}

/// An audit entry recording an action performed on a draw.
#[cw_serde]
pub struct DrawAuditEntry {
    pub seq: u64,
    pub draw_id: u64,
    pub credit_line_id: u64,
    pub action: DrawAction,
    pub timestamp: Timestamp,
    pub block_height: u64,
    pub by: Addr,
    pub memo: String,
}

/// A human-readable audit event returned by queries.
#[cw_serde]
pub struct DrawAuditEvent {
    pub seq: u64,
    pub action: DrawAction,
    pub timestamp: Timestamp,
    pub block_height: u64,
    pub by: Addr,
    pub memo: String,
}

impl DrawAuditEntry {
    pub fn into_event(self) -> DrawAuditEvent {
        DrawAuditEvent {
            seq: self.seq,
            action: self.action,
            timestamp: self.timestamp,
            block_height: self.block_height,
            by: self.by,
            memo: self.memo,
        }
    }
}

pub const CONFIG: Item<Config> = Item::new("config");

pub const CREDIT_LINE_COUNT: Item<u64> = Item::new("clc");
pub const CREDIT_LINES: Map<u64, CreditLine> = Map::new("cl");

pub const DRAW_COUNT: Map<u64, u64> = Map::new("dcnt");
pub const DRAWS: Map<(u64, u64), Draw> = Map::new("dr");

pub const DRAW_AUDIT_COUNT: Map<(u64, u64), u64> = Map::new("dacnt");
pub const DRAW_AUDIT: Map<(u64, u64, u64), DrawAuditEntry> = Map::new("da");

/// Deterministic, collision-free mapping from borrower address to their
/// stable credit-line id.  Every `open_credit_line` call for a new borrower
/// creates a unique id; subsequent look-ups are O(1) with no collision risk
/// because each `Addr` serialises to a distinct canonical bech32 byte string.
pub const BORROWER_TO_ID: Map<Addr, u64> = Map::new("bid");

/// Multi-oracle quorum configuration for redundancy median resolution.
#[cw_serde]
pub struct OracleQuorumConfig {
    /// Minimum number of submitted prices that must agree within
    /// `max_deviation_bps` to form a valid quorum.
    pub min_quorum_k: u32,
    /// Maximum allowed price deviation between the highest and lowest prices
    /// in the qualifying quorum window, in basis points (e.g. 500 = 5%).
    pub max_deviation_bps: u32,
    /// Maximum age of the stored quorum price in seconds before it is
    /// considered stale for settlement purposes.
    pub max_age_seconds: u64,
}

/// Stored quorum-resolved canonical price and its ledger timestamp.
#[cw_serde]
pub struct OraclePriceRecord {
    /// The resolved canonical price from the last quorum computation.
    pub price: i128,
    /// Ledger timestamp (seconds) when the price was resolved.
    pub timestamp: u64,
}

/// Maximum number of oracle price feeds accepted per `resolve_quorum_price` call.
///
/// Limits gas consumption and keeps the stack buffer within WASM limits.
/// Adjust after gas profiling if the protocol sources more feeds.
pub const MAX_ORACLE_FEEDS: usize = 20;

/// Storage key for the oracle quorum configuration.
pub const ORACLE_QUORUM_CONFIG: Item<OracleQuorumConfig> = Item::new("orc_qcfg");

/// Storage key for the last resolved oracle price record.
pub const ORACLE_PRICE_RECORD: Item<OraclePriceRecord> = Item::new("orc_prc");

/// Default treasury fee share when no per-market override is configured.
///
/// Stored as basis points (10_000 = 100 % to treasury). When absent the
/// runtime default [`DEFAULT_TREASURY_FEE_SHARE_BPS`] (10_000) applies.
pub const DEFAULT_FEE_SHARE_BPS: Item<u32> = Item::new("dfsb");

/// Per-market treasury fee-share override.
///
/// Maps market denomination → treasury share in basis points.
/// Overrides [`DEFAULT_FEE_SHARE_BPS`] for that market when present.
pub const MARKET_FEE_SHARE_BPS: Map<&str, u32> = Map::new("mfsb");

/// Accumulated treasury balance per market denomination.
///
/// Credited during `accrue_protocol_fee` and debited by `withdraw_treasury`.
pub const TREASURY_BALANCE: Map<&str, Uint128> = Map::new("trb");

/// Accumulated bounty balance per market denomination.
///
/// Credited during `accrue_protocol_fee` and debited by `withdraw_bounty`.
pub const BOUNTY_BALANCE: Map<&str, Uint128> = Map::new("bnb");

/// Storage key for the structured late-fee configuration.
///
/// When absent the contract has no late-fee penalty configured.
pub const LATE_FEE_CONFIG: Item<LateFeeConfig> = Item::new("lfc");

/// Per-borrower per-token collateral balance.
///
/// Maps `(borrower, denom)` to the deposited amount. A missing entry is
/// equivalent to zero. This gives N borrowers × M tokens of granularity
/// without requiring changes to the existing `CreditLine` schema.
pub const COLLATERAL_BALANCES: Map<(&Addr, &str), Uint128> = Map::new("cb");

/// Tracks which denominations a borrower has deposited (for enumeration).
///
/// Updated atomically with [`COLLATERAL_BALANCES`] so queries can iterate
/// a borrower's full set of collateral tokens without scanning all keys.
pub const BORROWER_COLLATERAL_TOKENS: Map<&Addr, Vec<String>> = Map::new("bct");

/// Admin-managed list of accepted collateral token denominations.
///
/// Only denominations in this list may be deposited via
/// [`execute_deposit_collateral`]. An empty or absent list means *no* tokens
/// are allowed (the contract must be configured first).
pub const COLLATERAL_TOKEN_ALLOWLIST: Item<Vec<String>> = Item::new("ctal");

/// Per-token risk weight in basis points (100 % = 10_000 bps).
///
/// When computing aggregate collateral value, each token's balance is
/// multiplied by its risk weight and divided by 10_000. An unconfigured
/// token defaults to [`DEFAULT_COLLATERAL_RISK_WEIGHT_BPS`].
pub const COLLATERAL_RISK_WEIGHTS: Map<&str, u32> = Map::new("crw");

/// Default risk weight for tokens without an explicit override.
///
/// 10_000 bps = 100 % (full notional value).
pub const DEFAULT_COLLATERAL_RISK_WEIGHT_BPS: u32 = 10_000;
