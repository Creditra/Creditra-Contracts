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
    /// Add an authorized oracle and its weight (admin only).
    AddOracle {
        oracle: String,
        weight: u32,
    },
    /// Remove an authorized oracle (admin only).
    RemoveOracle {
        oracle: String,
    },
    /// Submit an oracle report value.
    ReportValue {
        value: i128,
    },
    /// Submit N oracle prices and resolve a quorum canonical price (admin only).
    SubmitOraclePrices {
        prices: Vec<i128>,
    },
    /// Set or update the structured late-fee configuration (admin only).
    ///
    /// Pass `Some(LateFeeConfig::Flat(…))` for a fixed token amount per
    /// missed installment, or `Some(LateFeeConfig::AprBased(…))` for an
    /// additive basis-point surcharge.  Pass `None` to remove the config.
    SetLateFeeConfig {
        config: Option<LateFeeConfig>,
    },
    /// Deposit a collateral token on behalf of a borrower (admin only).
    DepositCollateral {
        borrower: String,
        denom: String,
        amount: String,
    },
    /// Withdraw a collateral token for a borrower (admin only).
    WithdrawCollateral {
        borrower: String,
        denom: String,
        amount: String,
    },
    /// Add a denomination to the collateral allowlist (admin only).
    AddCollateralToken {
        denom: String,
        risk_weight_bps: u32,
    },
    /// Remove a denomination from the collateral allowlist (admin only).
    RemoveCollateralToken {
        denom: String,
    },
    /// Update the risk weight for an allowed collateral token (admin only).
    SetCollateralRiskWeight {
        denom: String,
        risk_weight_bps: u32,
    },
    /// Set the protocol fee basis points (admin only).
    SetProtocolFeeBps {
        bps: u32,
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
    #[returns(LateFeeConfigResponse)]
    GetLateFeeConfig {},
    #[returns(CollateralBalanceResponse)]
    GetCollateralBalance {
        borrower: String,
        /// When `None`, returns all tokens; when `Some`, filters to that denom.
        denom: Option<String>,
    },
    #[returns(CollateralAllowlistResponse)]
    GetCollateralAllowlist {},
    #[returns(ProtocolFeeBpsResponse)]
    GetProtocolFeeBps {},
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

    /// Response for the late-fee configuration query.
    #[cw_serde]
    pub struct LateFeeConfigResponse {
        /// The currently configured late-fee config, or `None` if unset.
        pub config: Option<LateFeeConfig>,
    }

    /// A single entry in a borrower's multi-collateral portfolio.
    #[cw_serde]
    pub struct CollateralEntryResponse {
        /// Token denomination.
        pub denom: String,
        /// Raw deposited balance (before risk weighting).
        pub amount: Uint128,
        /// Risk weight in basis points applied to this token.
        pub risk_weight_bps: u32,
    }

    /// Response for the multi-collateral balance query.
    #[cw_serde]
    pub struct CollateralBalanceResponse {
        /// Borrower address.
        pub borrower: String,
        /// Per-token collateral breakdown.
        pub entries: Vec<CollateralEntryResponse>,
        /// Risk-weighted total across all tokens.
        pub weighted_total: Uint128,
    }

    /// Response for the collateral allowlist query.
    #[cw_serde]
    pub struct CollateralAllowlistResponse {
        /// Allowed token denominations.
        pub denoms: Vec<String>,
    }

    /// Response for the protocol fee basis points query.
    #[cw_serde]
    pub struct ProtocolFeeBpsResponse {
        /// Current protocol fee in basis points, or None if unset.
        pub bps: Option<u32>,
    }
