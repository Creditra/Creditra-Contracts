use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Timestamp, Uint128};
use cw_storage_plus::{Item, Map};

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
