use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{Addr, Timestamp, Uint128};

use crate::penalties::LateFeeConfig;
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
    /// Set the treasury address where withdrawn treasury fees are sent (admin only).
    SetTreasuryAddress {
        address: String,
    },
    /// Set the bounty address where withdrawn bounty fees are sent (admin only).
    SetBountyAddress {
        address: String,
    },
    /// Set the default treasury fee share in basis points (admin only).
    /// 0 = all to bounty, 10_000 = all to treasury.
    SetDefaultFeeShareBps {
        bps: u32,
    },
    /// Set the per-market treasury fee share in basis points (admin only).
    /// Overrides the default for the specified market denomination.
    SetMarketFeeShareBps {
        market_denom: String,
        bps: u32,
    },
    /// Remove a per-market fee share override, reverting to the default (admin only).
    RemoveMarketFeeShareBps {
        market_denom: String,
    },
    /// Accrue a protocol fee for a given market denomination (admin only).
    /// Splits the fee between treasury and bounty per the configured ratio.
    AccrueProtocolFee {
        market_denom: String,
        amount: String,
    },
    /// Withdraw accumulated treasury fees for a market (admin only).
    WithdrawTreasury {
        market_denom: String,
        amount: String,
    },
    /// Withdraw accumulated bounty fees for a market (admin only).
    WithdrawBounty {
        market_denom: String,
        amount: String,
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
    /// Query the treasury address.
    #[returns(TreasuryAddressResponse)]
    GetTreasuryAddress {},
    /// Query the bounty address.
    #[returns(BountyAddressResponse)]
    GetBountyAddress {},
    /// Query the default treasury fee share in basis points.
    #[returns(DefaultFeeShareBpsResponse)]
    GetDefaultFeeShareBps {},
    /// Query the per-market treasury fee share in basis points.
    #[returns(MarketFeeShareBpsResponse)]
    GetMarketFeeShareBps { market_denom: String },
    /// Query the treasury balance for a given market denomination.
    #[returns(TreasuryBalanceResponse)]
    GetTreasuryBalance { market_denom: String },
    /// Query the bounty balance for a given market denomination.
    #[returns(BountyBalanceResponse)]
    GetBountyBalance { market_denom: String },
    /// Preview a fee split for a given market and amount (read-only).
    #[returns(FeeSplitPreviewResponse)]
    GetFeeSplitPreview {
        market_denom: String,
        amount: String,
    },
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

/// Response for treasury address query.
#[cw_serde]
pub struct TreasuryAddressResponse {
    pub address: Option<String>,
}

/// Response for bounty address query.
#[cw_serde]
pub struct BountyAddressResponse {
    pub address: Option<String>,
}

/// Response for default fee share query.
#[cw_serde]
pub struct DefaultFeeShareBpsResponse {
    pub bps: u32,
}

/// Response for per-market fee share query.
#[cw_serde]
pub struct MarketFeeShareBpsResponse {
    pub market_denom: String,
    pub bps: Option<u32>,
}

/// Response for treasury balance query.
#[cw_serde]
pub struct TreasuryBalanceResponse {
    pub market_denom: String,
    pub balance: Uint128,
}

/// Response for bounty balance query.
#[cw_serde]
pub struct BountyBalanceResponse {
    pub market_denom: String,
    pub balance: Uint128,
}

/// Response for fee split preview query.
#[cw_serde]
pub struct FeeSplitPreviewResponse {
    pub market_denom: String,
    pub total_fee: Uint128,
    pub treasury_share_bps: u32,
    pub treasury_amount: Uint128,
    pub bounty_amount: Uint128,
}
