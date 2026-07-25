use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{Addr, Timestamp, Uint128};

use crate::state::DrawAuditEvent;
use crate::state::OracleQuorumConfig;

#[cw_serde]
pub struct InstantiateMsg {
    pub owner: String,
}

#[cw_serde]
pub enum ExecuteMsg {
    CreateCreditLine {
        borrower: String,
        collateral_denom: String,
        collateral_amount: String,
        credit_denom: String,
        credit_amount: String,
    },
    CreateDraw {
        credit_line_id: u64,
        amount: String,
        denom: String,
    },
    RepayDraw {
        credit_line_id: u64,
        draw_id: u64,
    },
    AddAuditMemo {
        credit_line_id: u64,
        draw_id: u64,
        memo: String,
    },
    UpdateProtocolVersion {
        major: u32,
        minor: u32,
    },
    /// Configure the multi-oracle quorum parameters (admin only).
    SetOracleQuorumConfig {
        min_quorum_k: u32,
        max_deviation_bps: u32,
        max_age_seconds: u64,
    },
    /// Submit N oracle prices and resolve a quorum canonical price (admin only).
    SubmitOraclePrices {
        prices: Vec<i128>,
    },
    /// Set the protocol-wide default per-borrower rate ceiling in basis points
    /// (admin only). Must not exceed `limits::MAX_RATE_BPS`.
    SetDefaultRateCeiling {
        max_rate_bps: u32,
    },
    /// Set a per-borrower rate-ceiling override in basis points (admin only).
    /// Must not exceed `limits::MAX_RATE_BPS`.
    SetBorrowerRateCeiling {
        borrower: String,
        max_rate_bps: u32,
    },
    /// Remove a per-borrower rate-ceiling override, reverting that borrower to
    /// the protocol-wide default (admin only).
    ClearBorrowerRateCeiling {
        borrower: String,
    },
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(DrawAuditTrailResponse)]
    DrawAuditTrail {
        credit_line_id: u64,
        draw_id: Option<u64>,
    },
    #[returns(ProofOfReserveResponse)]
    ProofOfReserve { denom: Option<String> },
    #[returns(BorrowerHealthFactorResponse)]
    BorrowerHealthFactor { borrower: String },
    #[returns(OracleQuorumConfigResponse)]
    GetOracleQuorumConfig {},
    #[returns(OraclePriceResponse)]
    GetOraclePrice {},
    /// Resolve the effective rate ceiling for a borrower, along with the
    /// override and default it was derived from.
    #[returns(BorrowerRateCeilingResponse)]
    GetBorrowerRateCeiling { borrower: String },
}

#[cw_serde]
pub struct DrawAuditTrailResponse {
    pub credit_line_id: u64,
    pub draw_id: u64,
    pub draw_amount: String,
    pub draw_denom: String,
    pub drawn_at: Timestamp,
    pub drawn_by: Addr,
    pub repaid: bool,
    pub events: Vec<DrawAuditEvent>,
}

#[cw_serde]
pub struct ProofOfReserveResponse {
    pub total_credit_lines: u64,
    pub active_credit_lines: u64,
    pub total_collateral: Uint128,
    pub total_credit_limit: Uint128,
    pub total_drawn: Uint128,
    pub total_repaid: Uint128,
    pub net_outstanding: Uint128,
    pub reserves_by_denom: Vec<DenomReserve>,
}

#[cw_serde]
pub struct DenomReserve {
    pub denom: String,
    pub collateral_amount: Uint128,
    pub credit_limit: Uint128,
    pub drawn_amount: Uint128,
    pub repaid_amount: Uint128,
    pub net_outstanding: Uint128,
}

#[cw_serde]
pub struct BorrowerHealthFactorResponse {
    pub borrower: String,
    pub credit_lines: Vec<CreditLineHealthResponse>,
}

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

#[cw_serde]
pub struct MigrateMsg {}

/// Response for oracle quorum configuration query.
#[cw_serde]
pub struct OracleQuorumConfigResponse {
    pub config: Option<OracleQuorumConfig>,
}

/// Response for oracle price query.
#[cw_serde]
pub struct OraclePriceResponse {
    pub price: Option<i128>,
    pub timestamp: Option<u64>,
}

/// Response for the per-borrower rate-ceiling query.
#[cw_serde]
pub struct BorrowerRateCeilingResponse {
    /// The borrower address the ceiling was resolved for.
    pub borrower: String,
    /// The effective ceiling in basis points: the override when set, otherwise
    /// the default. `None` when neither has been configured.
    pub effective_ceiling_bps: Option<u32>,
    /// The per-borrower override in basis points, if one is set.
    pub override_bps: Option<u32>,
    /// The protocol-wide default in basis points, if one is set.
    pub default_bps: Option<u32>,
}
