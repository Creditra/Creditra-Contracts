use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Storage, Timestamp, Uint128};
use cw_storage_plus::{Item, Map};

use crate::error::ContractError;
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

pub use crate::key::{
    check_new_namespace_collision, validate_storage_key_catalog, StorageKeyFamily, StorageType,
    ALL_STORAGE_KEY_FAMILIES,
};

/// Singleton contract configuration containing administrative owner address.
///
/// - Namespace: `"config"`
/// - Key type: `()` (raw key: `b"config"`)
/// - Value type: [`Config`]
/// - Storage kind: [`cw_storage_plus::Item`]
pub const CONFIG: Item<Config> = Item::new("config");

/// Monotonically increasing counter of all credit lines ever created.
///
/// - Namespace: `"clc"`
/// - Key type: `()` (raw key: `b"clc"`)
/// - Value type: `u64`
/// - Storage kind: [`cw_storage_plus::Item`]
pub const CREDIT_LINE_COUNT: Item<u64> = Item::new("clc");

/// Credit line records keyed by stable numeric identifier.
///
/// - Namespace: `"cl"`
/// - Key type: `u64`
/// - Value type: [`CreditLine`]
/// - Storage kind: [`cw_storage_plus::Map`]
pub const CREDIT_LINES: Map<u64, CreditLine> = Map::new("cl");

/// Per-credit-line draw counter tracking total draws created on that line.
///
/// - Namespace: `"dcnt"`
/// - Key type: `u64`
/// - Value type: `u64`
/// - Storage kind: [`cw_storage_plus::Map`]
pub const DRAW_COUNT: Map<u64, u64> = Map::new("dcnt");

/// Draw records keyed by composite credit line and draw identifiers.
///
/// - Namespace: `"dr"`
/// - Key type: `(u64, u64)`
/// - Value type: [`Draw`]
/// - Storage kind: [`cw_storage_plus::Map`]
pub const DRAWS: Map<(u64, u64), Draw> = Map::new("dr");

/// Per-draw audit sequence counter.
///
/// - Namespace: `"dacnt"`
/// - Key type: `(u64, u64)`
/// - Value type: `u64`
/// - Storage kind: [`cw_storage_plus::Map`]
pub const DRAW_AUDIT_COUNT: Map<(u64, u64), u64> = Map::new("dacnt");

/// Immutable append-only audit trail entries for draw actions.
///
/// - Namespace: `"da"`
/// - Key type: `(u64, u64, u64)`
/// - Value type: [`DrawAuditEntry`]
/// - Storage kind: [`cw_storage_plus::Map`]
pub const DRAW_AUDIT: Map<(u64, u64, u64), DrawAuditEntry> = Map::new("da");

/// Sum of unrepaid draw amounts on a credit line.
///
/// Missing `DRAW_COUNT` is treated as zero draws. Missing individual draw
/// records are skipped. Overflow on the running sum returns
/// [`ContractError::Overflow`].
pub fn outstanding_utilization(
    storage: &dyn Storage,
    credit_line_id: u64,
) -> Result<Uint128, ContractError> {
    let draw_count = DRAW_COUNT.may_load(storage, credit_line_id)?.unwrap_or(0);
    let mut utilized = Uint128::zero();
    for did in 0..draw_count {
        if let Some(draw) = DRAWS.may_load(storage, (credit_line_id, did))? {
            if !draw.repaid {
                utilized = utilized
                    .checked_add(draw.amount)
                    .map_err(|_| ContractError::Overflow)?;
            }
        }
    }
    Ok(utilized)
}

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

#[cw_serde]
pub struct OracleReportData {
    pub value: i128,
    pub timestamp: u64,
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

/// Maximum number of collateral denominations allowed in the allowlist.
///
/// Bounds storage growth and keeps the per-deposit allowlist membership scan
/// (`is_collateral_token_allowed`, O(n)) inside predictable transaction
/// resource limits. The allowlist is also returned wholesale by
/// [`crate::collateral::query_collateral_allowlist`], so an unbounded list
/// would make that query unbounded too. Adjust after gas profiling if the
/// protocol lists more assets.
pub const MAX_COLLATERAL_TOKENS: usize = 50;

/// Multi-oracle quorum configuration for redundancy median resolution.
///
/// - Namespace: `"orc_qcfg"`
/// - Key type: `()` (raw key: `b"orc_qcfg"`)
/// - Value type: [`OracleQuorumConfig`]
/// - Storage kind: [`cw_storage_plus::Item`]
pub const ORACLE_QUORUM_CONFIG: Item<OracleQuorumConfig> = Item::new("orc_qcfg");

/// Stored quorum-resolved canonical price record.
///
/// - Namespace: `"orc_prc"`
/// - Key type: `()` (raw key: `b"orc_prc"`)
/// - Value type: [`OraclePriceRecord`]
/// - Storage kind: [`cw_storage_plus::Item`]
pub const ORACLE_PRICE_RECORD: Item<OraclePriceRecord> = Item::new("orc_prc");

/// Authorized oracle feed provider addresses.
///
/// - Namespace: `"orc_lst"`
/// - Key type: `()` (raw key: `b"orc_lst"`)
/// - Value type: `Vec<Addr>`
/// - Storage kind: [`cw_storage_plus::Item`]
pub const ORACLE_LIST: Item<Vec<Addr>> = Item::new("orc_lst");

/// Individual oracle voting weight in basis points.
///
/// - Namespace: `"orc_w"`
/// - Key type: `Addr`
/// - Value type: `u32`
/// - Storage kind: [`cw_storage_plus::Map`]
pub const ORACLE_WEIGHT: Map<Addr, u32> = Map::new("orc_w");

/// Latest submitted price report data per oracle address.
///
/// - Namespace: `"orc_rpt"`
/// - Key type: `Addr`
/// - Value type: [`OracleReportData`]
/// - Storage kind: [`cw_storage_plus::Map`]
pub const ORACLE_REPORT: Map<Addr, OracleReportData> = Map::new("orc_rpt");

/// Structured late-fee configuration.
///
/// - Namespace: `"lfc"`
/// - Key type: `()` (raw key: `b"lfc"`)
/// - Value type: [`LateFeeConfig`]
/// - Storage kind: [`cw_storage_plus::Item`]
pub const LATE_FEE_CONFIG: Item<LateFeeConfig> = Item::new("lfc");

/// Tokens currently posted by each borrower.
///
/// - Namespace: `"bct"`
/// - Key type: `&Addr`
/// - Value type: `Vec<String>`
/// - Storage kind: [`cw_storage_plus::Map`]
pub const BORROWER_COLLATERAL_TOKENS: Map<&Addr, Vec<String>> = Map::new("bct");

/// Raw collateral balance keyed by borrower and token denomination.
///
/// - Namespace: `"cb"`
/// - Key type: `(&Addr, &str)`
/// - Value type: [`Uint128`]
/// - Storage kind: [`cw_storage_plus::Map`]
pub const COLLATERAL_BALANCES: Map<(&Addr, &str), Uint128> = Map::new("cb");

/// Optional risk-weight overrides in basis points keyed by token denomination.
///
/// - Namespace: `"crw"`
/// - Key type: `&str`
/// - Value type: `u32`
/// - Storage kind: [`cw_storage_plus::Map`]
pub const COLLATERAL_RISK_WEIGHTS: Map<&str, u32> = Map::new("crw");

/// Admin-managed allowlist of accepted collateral token denominations.
///
/// - Namespace: `"cta"`
/// - Key type: `()` (raw key: `b"cta"`)
/// - Value type: `Vec<String>`
/// - Storage kind: [`cw_storage_plus::Item`]
pub const COLLATERAL_TOKEN_ALLOWLIST: Item<Vec<String>> = Item::new("cta");

/// Default collateral risk weight: 100%.
pub const DEFAULT_COLLATERAL_RISK_WEIGHT_BPS: u32 = 10_000;

/// Default treasury fee share in basis points (0..=10_000).
/// When unset, defaults to 10_000 (100% treasury, backward compatible).
///
/// - Namespace: `"default_fee_share"`
/// - Key type: `()` (raw key: `b"default_fee_share"`)
/// - Value type: `u32`
/// - Storage kind: [`cw_storage_plus::Item`]
pub const DEFAULT_FEE_SHARE_BPS: Item<u32> = Item::new("default_fee_share");

/// Per-market treasury fee share override in basis points (0..=10_000).
/// Keyed by market denomination (the `credit_denom` of a credit line).
/// When absent for a market, the [`DEFAULT_FEE_SHARE_BPS`] applies.
///
/// - Namespace: `"mkt_fee_share"`
/// - Key type: `&str`
/// - Value type: `u32`
/// - Storage kind: [`cw_storage_plus::Map`]
pub const MARKET_FEE_SHARE_BPS: Map<&str, u32> = Map::new("mkt_fee_share");

/// Per-market accumulated treasury balance held in contract (fees collected).
/// Keyed by market denomination.
///
/// - Namespace: `"treasury_bal"`
/// - Key type: `&str`
/// - Value type: [`Uint128`]
/// - Storage kind: [`cw_storage_plus::Map`]
pub const TREASURY_BALANCE: Map<&str, Uint128> = Map::new("treasury_bal");

/// Per-market accumulated bounty pool balance held in contract (fee share).
/// Keyed by market denomination.
///
/// - Namespace: `"bounty_bal"`
/// - Key type: `&str`
/// - Value type: [`Uint128`]
/// - Storage kind: [`cw_storage_plus::Map`]
pub const BOUNTY_BALANCE: Map<&str, Uint128> = Map::new("bounty_bal");
