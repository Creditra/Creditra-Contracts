use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Timestamp, Uint128};
use cw_storage_plus::{Item, Map};

/// Top-level contract configuration.
#[cw_serde]
pub struct Config {
    /// Address of the contract owner / admin.
    pub owner: Addr,
}

/// A single credit line extended to a borrower.
#[cw_serde]
pub struct CreditLine {
    /// Sequential credit-line identifier assigned at creation.
    pub id: u64,
    /// Address of the borrower who owns this credit line.
    pub borrower: Addr,
    /// Denomination of the collateral deposited.
    pub collateral_denom: String,
    /// Amount of collateral deposited.
    pub collateral_amount: Uint128,
    /// Denomination in which credit is drawn.
    pub credit_denom: String,
    /// Maximum credit amount available.
    pub credit_amount: Uint128,
    /// Whether the credit line is currently active.
    pub active: bool,
}

/// A single draw against a credit line.
#[cw_serde]
pub struct Draw {
    /// Sequential draw identifier scoped to the credit line.
    pub id: u64,
    /// The credit line this draw belongs to.
    pub credit_line_id: u64,
    /// Amount drawn.
    pub amount: Uint128,
    /// Token denomination of the draw.
    pub denom: String,
    /// Block timestamp when the draw was created.
    pub drawn_at: Timestamp,
    /// Address that initiated the draw.
    pub drawn_by: Addr,
    /// Whether this draw has been repaid.
    pub repaid: bool,
}

/// Discriminated action recorded in the per-draw audit trail.
#[cw_serde]
#[derive(Copy, Eq, PartialEq)]
pub enum DrawAction {
    /// The draw was created.
    DrawCreated,
    /// The draw was fully repaid.
    Repaid,
    /// An audit memo was appended.
    MemoAdded,
}

/// A single entry in the per-draw audit trail.
#[cw_serde]
pub struct DrawAuditEntry {
    /// Monotonically-increasing sequence number within the draw.
    pub seq: u64,
    /// The draw this entry belongs to.
    pub draw_id: u64,
    /// The credit line this draw belongs to.
    pub credit_line_id: u64,
    /// The action that produced this entry.
    pub action: DrawAction,
    /// Block timestamp of the entry.
    pub timestamp: Timestamp,
    /// Block height at which the entry was recorded.
    pub block_height: u64,
    /// Address that performed the action.
    pub by: Addr,
    /// Optional human-readable note.
    pub memo: String,
}

/// Serializable audit event returned by query responses.
#[cw_serde]
pub struct DrawAuditEvent {
    pub seq: u64,
    pub draw_id: u64,
    pub credit_line_id: u64,
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
            draw_id: self.draw_id,
            credit_line_id: self.credit_line_id,
            action: self.action,
            timestamp: self.timestamp,
            block_height: self.block_height,
            by: self.by,
            memo: self.memo,
        }
    }
}

/// Health-factor view for a single credit line.
#[cw_serde]
pub struct CreditLineHealthResponse {
    pub credit_line_id: u64,
    pub collateral_denom: String,
    pub collateral_amount: Uint128,
    pub credit_denom: String,
    pub credit_amount: Uint128,
    pub utilized_amount: Uint128,
    pub health_factor_bps: u32,
}

/// Aggregated health-factor response for a borrower.
#[cw_serde]
pub struct BorrowerHealthFactorResponse {
    pub borrower: String,
    pub credit_lines: Vec<CreditLineHealthResponse>,
}

// ── Storage items ─────────────────────────────────────────────────────────────

/// Contract configuration (owner address).
pub const CONFIG: Item<Config> = Item::new("config");

/// Monotonically increasing counter of credit lines created.
pub const CREDIT_LINE_COUNT: Item<u64> = Item::new("credit_line_count");

/// Credit-line records keyed by sequential ID.
pub const CREDIT_LINES: Map<u64, CreditLine> = Map::new("credit_lines");

/// Number of draws per credit line.
pub const DRAW_COUNT: Map<u64, u64> = Map::new("draw_count");

/// Individual draws keyed by `(credit_line_id, draw_id)`.
pub const DRAWS: Map<(u64, u64), Draw> = Map::new("draws");

/// Number of audit entries per draw.
pub const DRAW_AUDIT_COUNT: Map<(u64, u64), u64> = Map::new("draw_audit_count");

/// Audit trail entries keyed by `(credit_line_id, draw_id, seq)`.
pub const DRAW_AUDIT: Map<(u64, u64, u64), DrawAuditEntry> = Map::new("draw_audit");
