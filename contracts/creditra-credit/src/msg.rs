use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{Addr, Timestamp};

use crate::state::DrawAuditEvent;

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
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(DrawAuditTrailResponse)]
    DrawAuditTrail {
        credit_line_id: u64,
        draw_id: Option<u64>,
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
pub struct MigrateMsg {}
