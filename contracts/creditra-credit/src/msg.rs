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
    /// Full read-only snapshot of a single credit line by its stable numeric id.
    ///
    /// Aggregates the core credit-line record, all active draw balances,
    /// collateral holdings (single-token + multi-token), health factor,
    /// and active status in a single round-trip, avoiding the multiple
    /// separate queries a caller would otherwise need.
    ///
    /// Returns `None` when no credit line exists for `credit_line_id`.
    #[returns(Option<CreditLineSnapshotResponse>)]
    CreditLineSnapshot { credit_line_id: u64 },
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

/// A single draw entry included in the credit-line snapshot.
#[cw_serde]
pub struct DrawSnapshotEntry {
    /// Per-line numeric draw id.
    pub draw_id: u64,
    /// Principal drawn.
    pub amount: Uint128,
    /// Token denomination of this draw.
    pub denom: String,
    /// Block time when the draw was created.
    pub drawn_at: Timestamp,
    /// Address that initiated the draw.
    pub drawn_by: Addr,
    /// `true` when the draw has been fully repaid.
    pub repaid: bool,
}

/// Full read-only snapshot of a credit line and its associated state.
///
/// Returned by [`QueryMsg::CreditLineSnapshot`]. Aggregates the core credit-line
/// record, all draws, collateral balances, and the derived health factor in a
/// single response so callers avoid multiple round-trip queries.
///
/// # Health factor semantics
///
/// - `health_factor_bps == u32::MAX` when `total_utilized == 0` (no outstanding debt).
/// - A value below `10_000` indicates the position is under-collateralized.
/// - A value of `10_000` means collateral exactly covers the utilized amount.
/// - A value above `10_000` means the position is over-collateralized.
#[cw_serde]
pub struct CreditLineSnapshotResponse {
    /// Stable numeric id of the credit line.
    pub credit_line_id: u64,
    /// Borrower address.
    pub borrower: Addr,
    /// Primary collateral token denomination.
    pub collateral_denom: String,
    /// Primary collateral balance held against this credit line.
    pub collateral_amount: Uint128,
    /// Credit (borrowable) token denomination.
    pub credit_denom: String,
    /// Maximum principal that may be outstanding across all draws.
    pub credit_amount: Uint128,
    /// Whether the credit line is currently active.
    pub active: bool,
    /// Sum of all un-repaid draw amounts (outstanding principal).
    pub total_utilized: Uint128,
    /// Multi-token collateral breakdown (may be empty when no multi-token
    /// collateral has been deposited).
    pub multi_collateral: Vec<CollateralEntryResponse>,
    /// Risk-weighted total across all collateral tokens.
    pub weighted_collateral_total: Uint128,
    /// Collateral-aware health factor in basis points.
    /// `u32::MAX` when `total_utilized == 0`.
    pub health_factor_bps: u32,
    /// All draws associated with this credit line (active and repaid).
    pub draws: Vec<DrawSnapshotEntry>,
}
