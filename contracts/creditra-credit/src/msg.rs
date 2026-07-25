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
    /// Step 1 of a two-step treasury withdrawal: propose sending `amount` of
    /// `denom` to `to`. Starts a 24h timelock; does not move funds. Owner only.
    ProposeWithdrawal {
        to: String,
        amount: String,
        denom: String,
    },
    /// Step 2: execute a previously proposed withdrawal once its timelock has
    /// elapsed. Owner only.
    ExecuteWithdrawal {},
    /// Cancel a pending withdrawal proposal without sending funds. Owner only.
    CancelWithdrawal {},
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
    /// The currently pending treasury withdrawal proposal, if any.
    #[returns(PendingWithdrawalResponse)]
    GetPendingWithdrawal {},
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

/// Response for the pending treasury withdrawal query.
#[cw_serde]
pub struct PendingWithdrawalResponse {
    pub to: Option<Addr>,
    pub amount: Option<Uint128>,
    pub denom: Option<String>,
    pub proposed_at: Option<Timestamp>,
    pub unlocks_at: Option<Timestamp>,
}
