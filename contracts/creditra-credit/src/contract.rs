use cosmwasm_std::{
    entry_point, to_json_binary, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdError,
    StdResult,
};

use crate::error::ContractError;
use crate::msg::{ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg};
use crate::state::{
    Config, CreditLine, Draw, DrawAction, DrawAuditEntry, CONFIG, CREDIT_LINE_COUNT, CREDIT_LINES,
    DRAW_AUDIT, DRAW_AUDIT_COUNT, DRAW_COUNT, DRAWS,
};
use crate::views;

#[entry_point]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> StdResult<Response> {
    let owner = deps.api.addr_validate(&msg.owner)?;
    let config = Config { owner };
    CONFIG.save(deps.storage, &config)?;
    CREDIT_LINE_COUNT.save(deps.storage, &0)?;
    Ok(Response::default())
}

#[entry_point]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::CreateCreditLine {
            borrower,
            collateral_denom,
            collateral_amount,
            credit_denom,
            credit_amount,
        } => execute_create_credit_line(
            deps,
            env,
            info,
            borrower,
            collateral_denom,
            collateral_amount,
            credit_denom,
            credit_amount,
        ),
        ExecuteMsg::CreateDraw {
            credit_line_id,
            amount,
            denom,
        } => execute_create_draw(deps, env, info, credit_line_id, amount, denom),
        ExecuteMsg::RepayDraw {
            credit_line_id,
            draw_id,
        } => execute_repay_draw(deps, env, info, credit_line_id, draw_id),
        ExecuteMsg::AddAuditMemo {
            credit_line_id,
            draw_id,
            memo,
        } => execute_add_audit_memo(deps, env, info, credit_line_id, draw_id, memo),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn execute_create_credit_line(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    borrower: String,
    collateral_denom: String,
    collateral_amount: String,
    credit_denom: String,
    credit_amount: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }

    let borrower_addr = deps.api.addr_validate(&borrower)?;
    let count = CREDIT_LINE_COUNT.load(deps.storage)?;

    let credit_line = CreditLine {
        id: count,
        borrower: borrower_addr,
        collateral_denom,
        collateral_amount: collateral_amount.parse().map_err(|_| ContractError::Std(
            cosmwasm_std::StdError::parse_err("Uint128", collateral_amount),
        ))?,
        credit_denom,
        credit_amount: credit_amount.parse().map_err(|_| ContractError::Std(
            cosmwasm_std::StdError::parse_err("Uint128", credit_amount),
        ))?,
        active: true,
    };

    CREDIT_LINES.save(deps.storage, count, &credit_line)?;
    CREDIT_LINE_COUNT.save(deps.storage, &(count + 1))?;

    Ok(Response::default()
        .add_attribute("action", "create_credit_line")
        .add_attribute("credit_line_id", count.to_string()))
}

pub fn execute_create_draw(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    credit_line_id: u64,
    amount: String,
    denom: String,
) -> Result<Response, ContractError> {
    let credit_line = CREDIT_LINES
        .may_load(deps.storage, credit_line_id)?
        .ok_or(ContractError::CreditLineNotFound(credit_line_id))?;

    if info.sender != credit_line.borrower {
        return Err(ContractError::Unauthorized);
    }

    let draw_count = DRAW_COUNT
        .may_load(deps.storage, credit_line_id)?
        .unwrap_or(0);

    let draw_amount: cosmwasm_std::Uint128 = amount
        .parse()
        .map_err(|_| ContractError::Std(cosmwasm_std::StdError::parse_err("Uint128", &amount)))?;

    let draw = Draw {
        id: draw_count,
        credit_line_id,
        amount: draw_amount,
        denom,
        drawn_at: env.block.time,
        drawn_by: info.sender.clone(),
        repaid: false,
    };

    DRAWS.save(deps.storage, (credit_line_id, draw_count), &draw)?;
    DRAW_COUNT.save(deps.storage, credit_line_id, &(draw_count + 1))?;

    let audit_seq = 0u64;
    let audit_entry = DrawAuditEntry {
        seq: audit_seq,
        draw_id: draw_count,
        credit_line_id,
        action: DrawAction::DrawCreated,
        timestamp: env.block.time,
        block_height: env.block.height,
        by: info.sender,
        memo: String::new(),
    };
    DRAW_AUDIT.save(deps.storage, (credit_line_id, draw_count, audit_seq), &audit_entry)?;
    DRAW_AUDIT_COUNT.save(deps.storage, (credit_line_id, draw_count), &1)?;

    Ok(Response::default()
        .add_attribute("action", "create_draw")
        .add_attribute("credit_line_id", credit_line_id.to_string())
        .add_attribute("draw_id", draw_count.to_string()))
}

pub fn execute_repay_draw(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    credit_line_id: u64,
    draw_id: u64,
) -> Result<Response, ContractError> {
    let mut draw = DRAWS
        .may_load(deps.storage, (credit_line_id, draw_id))?
        .ok_or(ContractError::DrawNotFound(draw_id, credit_line_id))?;

    if info.sender != draw.drawn_by {
        return Err(ContractError::Unauthorized);
    }

    draw.repaid = true;
    DRAWS.save(deps.storage, (credit_line_id, draw_id), &draw)?;

    append_audit_entry(deps, env, info, credit_line_id, draw_id, DrawAction::Repaid, String::new())?;

    Ok(Response::default()
        .add_attribute("action", "repay_draw")
        .add_attribute("credit_line_id", credit_line_id.to_string())
        .add_attribute("draw_id", draw_id.to_string()))
}

pub fn execute_add_audit_memo(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    credit_line_id: u64,
    draw_id: u64,
    memo: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }

    DRAWS
        .may_load(deps.storage, (credit_line_id, draw_id))?
        .ok_or(ContractError::DrawNotFound(draw_id, credit_line_id))?;

    append_audit_entry(deps, env, info, credit_line_id, draw_id, DrawAction::MemoAdded, memo)?;

    Ok(Response::default()
        .add_attribute("action", "add_audit_memo")
        .add_attribute("credit_line_id", credit_line_id.to_string())
        .add_attribute("draw_id", draw_id.to_string()))
}

fn append_audit_entry(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    credit_line_id: u64,
    draw_id: u64,
    action: DrawAction,
    memo: String,
) -> Result<(), ContractError> {
    let audit_count = DRAW_AUDIT_COUNT
        .may_load(deps.storage, (credit_line_id, draw_id))?
        .unwrap_or(0);

    let entry = DrawAuditEntry {
        seq: audit_count,
        draw_id,
        credit_line_id,
        action,
        timestamp: env.block.time,
        block_height: env.block.height,
        by: info.sender,
        memo,
    };

    DRAW_AUDIT.save(deps.storage, (credit_line_id, draw_id, audit_count), &entry)?;
    DRAW_AUDIT_COUNT.save(deps.storage, (credit_line_id, draw_id), &(audit_count + 1))?;

    Ok(())
}

#[entry_point]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::DrawAuditTrail {
            credit_line_id,
            draw_id,
        } => {
            let resp = views::query_draw_audit_trail(deps, credit_line_id, draw_id)
                .map_err(|e| StdError::generic_err(e.to_string()))?;
            to_json_binary(&resp)
        }
    }
}

#[entry_point]
pub fn migrate(_deps: DepsMut, _env: Env, _msg: MigrateMsg) -> StdResult<Response> {
    Ok(Response::default())
}
