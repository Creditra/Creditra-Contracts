use cosmwasm_std::{
    entry_point, to_json_binary, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdError,
    StdResult,
};

use crate::error::ContractError;
use crate::handshake::{self, ProtocolVersion};
use crate::msg::{ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg};
use crate::oracles;
use crate::state::{
    Config, CreditLine, Draw, DrawAction, DrawAuditEntry, OraclePriceRecord, BORROWER_TO_ID,
    CONFIG, CREDIT_LINES, CREDIT_LINE_COUNT, DRAWS, DRAW_AUDIT, DRAW_AUDIT_COUNT, DRAW_COUNT,
    ORACLE_PRICE_RECORD, ORACLE_QUORUM_CONFIG, DEFAULT_FEE_SHARE_BPS, MARKET_FEE_SHARE_BPS,
    TREASURY_ADDRESS, BOUNTY_ADDRESS, TREASURY_BALANCE, BOUNTY_BALANCE,
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
    handshake::initialize_version(deps.storage)?;
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
        ExecuteMsg::UpdateProtocolVersion { major, minor } => {
            execute_update_protocol_version(deps, info, major, minor)
        }
        ExecuteMsg::SetOracleQuorumConfig {
            min_quorum_k,
            max_deviation_bps,
            max_age_seconds,
        } => execute_set_oracle_quorum_config(deps, info, min_quorum_k, max_deviation_bps, max_age_seconds),
        ExecuteMsg::SubmitOraclePrices { prices } => {
            execute_submit_oracle_prices(deps, env, info, prices)
        }
        ExecuteMsg::SetTreasuryAddress { address } => {
            execute_set_treasury_address(deps, info, address)
        }
        ExecuteMsg::SetBountyAddress { address } => {
            execute_set_bounty_address(deps, info, address)
        }
        ExecuteMsg::SetDefaultFeeShareBps { bps } => {
            execute_set_default_fee_share_bps(deps, info, bps)
        }
        ExecuteMsg::SetMarketFeeShareBps {
            market_denom,
            bps,
        } => execute_set_market_fee_share_bps(deps, info, market_denom, bps),
        ExecuteMsg::RemoveMarketFeeShareBps { market_denom } => {
            execute_remove_market_fee_share_bps(deps, info, market_denom)
        }
        ExecuteMsg::AccrueProtocolFee {
            market_denom,
            amount,
        } => execute_accrue_protocol_fee(deps, info, market_denom, amount),
        ExecuteMsg::WithdrawTreasury {
            market_denom,
            amount,
        } => execute_withdraw_treasury(deps, info, market_denom, amount),
        ExecuteMsg::WithdrawBounty {
            market_denom,
            amount,
        } => execute_withdraw_bounty(deps, info, market_denom, amount),
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
        borrower: borrower_addr.clone(),
        collateral_denom,
        collateral_amount: collateral_amount.parse().map_err(|_| {
            ContractError::Std(cosmwasm_std::StdError::parse_err(
                "Uint128",
                collateral_amount,
            ))
        })?,
        credit_denom,
        credit_amount: credit_amount.parse().map_err(|_| {
            ContractError::Std(cosmwasm_std::StdError::parse_err("Uint128", credit_amount))
        })?,
        active: true,
    };

    CREDIT_LINES.save(deps.storage, count, &credit_line)?;
    CREDIT_LINE_COUNT.save(deps.storage, &(count + 1))?;

    // Store deterministic borrower → credit-line-id mapping for O(1) lookups.
    // cw_storage_plus::Map serialises Addr via its canonical bech32 bytes,
    // which guarantees deterministic + collision-free keys by construction.
    BORROWER_TO_ID.save(deps.storage, borrower_addr.clone(), &count)?;

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
    DRAW_AUDIT.save(
        deps.storage,
        (credit_line_id, draw_count, audit_seq),
        &audit_entry,
    )?;
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

    append_audit_entry(
        deps,
        env,
        info,
        credit_line_id,
        draw_id,
        DrawAction::Repaid,
        String::new(),
    )?;

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

    append_audit_entry(
        deps,
        env,
        info,
        credit_line_id,
        draw_id,
        DrawAction::MemoAdded,
        memo,
    )?;

    Ok(Response::default()
        .add_attribute("action", "add_audit_memo")
        .add_attribute("credit_line_id", credit_line_id.to_string())
        .add_attribute("draw_id", draw_id.to_string()))
}

pub fn execute_update_protocol_version(
    deps: DepsMut,
    info: MessageInfo,
    major: u32,
    minor: u32,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }
    let version = ProtocolVersion { major, minor };
    handshake::set_protocol_version(deps, version)?;
    Ok(Response::default()
        .add_attribute("action", "update_protocol_version")
        .add_attribute("major", major.to_string())
        .add_attribute("minor", minor.to_string()))
}

/// Configure the multi-oracle quorum parameters (admin only).
pub fn execute_set_oracle_quorum_config(
    deps: DepsMut,
    info: MessageInfo,
    min_quorum_k: u32,
    max_deviation_bps: u32,
    max_age_seconds: u64,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }

    if min_quorum_k < 2 {
        return Err(ContractError::InvalidAmount);
    }
    if max_deviation_bps > 10_000 {
        return Err(ContractError::InvalidAmount);
    }
    if max_age_seconds == 0 {
        return Err(ContractError::InvalidAmount);
    }

    let qcfg = crate::state::OracleQuorumConfig {
        min_quorum_k,
        max_deviation_bps,
        max_age_seconds,
    };
    ORACLE_QUORUM_CONFIG.save(deps.storage, &qcfg)?;

    Ok(Response::default()
        .add_attribute("action", "set_oracle_quorum_config")
        .add_attribute("min_quorum_k", min_quorum_k.to_string())
        .add_attribute("max_deviation_bps", max_deviation_bps.to_string())
        .add_attribute("max_age_seconds", max_age_seconds.to_string()))
}

/// Submit N oracle prices and resolve a quorum canonical price (admin only).
pub fn execute_submit_oracle_prices(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    prices: Vec<i128>,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }

    let qcfg = ORACLE_QUORUM_CONFIG
        .may_load(deps.storage)?
        .ok_or(ContractError::OraclePriceInvalid)?;

    if prices.len() > crate::state::MAX_ORACLE_FEEDS {
        return Err(ContractError::OraclePriceInvalid);
    }

    let canonical_price = oracles::resolve_quorum_price(&prices, &qcfg)?;
    let now = env.block.time.seconds();

    let record = OraclePriceRecord {
        price: canonical_price,
        timestamp: now,
    };
    ORACLE_PRICE_RECORD.save(deps.storage, &record)?;

    Ok(Response::default()
        .add_attribute("action", "submit_oracle_prices")
        .add_attribute("canonical_price", canonical_price.to_string())
        .add_attribute("min_quorum_k", qcfg.min_quorum_k.to_string())
        .add_attribute("timestamp", now.to_string()))
}

/// Set the treasury address (admin only).
pub fn execute_set_treasury_address(
    deps: DepsMut,
    info: MessageInfo,
    address: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }

    let addr = deps.api.addr_validate(&address)?;
    TREASURY_ADDRESS.save(deps.storage, &addr)?;

    Ok(Response::default()
        .add_attribute("action", "set_treasury_address")
        .add_attribute("treasury", address))
}

/// Set the bounty address (admin only).
pub fn execute_set_bounty_address(
    deps: DepsMut,
    info: MessageInfo,
    address: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }

    let addr = deps.api.addr_validate(&address)?;
    BOUNTY_ADDRESS.save(deps.storage, &addr)?;

    Ok(Response::default()
        .add_attribute("action", "set_bounty_address")
        .add_attribute("bounty", address))
}

/// Set the default treasury fee share in basis points (admin only).
pub fn execute_set_default_fee_share_bps(
    deps: DepsMut,
    info: MessageInfo,
    bps: u32,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }

    if bps > crate::fees::MAX_FEE_SHARE_BPS {
        return Err(ContractError::InvalidFeeShareBps);
    }

    DEFAULT_FEE_SHARE_BPS.save(deps.storage, &bps)?;

    Ok(Response::default()
        .add_attribute("action", "set_default_fee_share_bps")
        .add_attribute("bps", bps.to_string()))
}

/// Set the per-market treasury fee share in basis points (admin only).
pub fn execute_set_market_fee_share_bps(
    deps: DepsMut,
    info: MessageInfo,
    market_denom: String,
    bps: u32,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }

    if bps > crate::fees::MAX_FEE_SHARE_BPS {
        return Err(ContractError::InvalidFeeShareBps);
    }

    MARKET_FEE_SHARE_BPS.save(deps.storage, &market_denom, &bps)?;

    Ok(Response::default()
        .add_attribute("action", "set_market_fee_share_bps")
        .add_attribute("market_denom", &market_denom)
        .add_attribute("bps", bps.to_string()))
}

/// Remove a per-market fee share override (admin only).
pub fn execute_remove_market_fee_share_bps(
    deps: DepsMut,
    info: MessageInfo,
    market_denom: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }

    MARKET_FEE_SHARE_BPS.remove(deps.storage, &market_denom);

    Ok(Response::default()
        .add_attribute("action", "remove_market_fee_share_bps")
        .add_attribute("market_denom", &market_denom))
}

/// Accrue a protocol fee for a given market (admin only).
pub fn execute_accrue_protocol_fee(
    mut deps: DepsMut,
    info: MessageInfo,
    market_denom: String,
    amount: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }

    let fee_amount: cosmwasm_std::Uint128 = amount.parse().map_err(|_| {
        ContractError::Std(cosmwasm_std::StdError::parse_err("Uint128", &amount))
    })?;

    let split = crate::fees::accrue_protocol_fee(&mut deps, &market_denom, fee_amount)?;

    Ok(Response::default()
        .add_attribute("action", "accrue_protocol_fee")
        .add_attribute("market_denom", &market_denom)
        .add_attribute("fee_amount", fee_amount.to_string())
        .add_attribute("treasury_amount", split.treasury_amount.to_string())
        .add_attribute("bounty_amount", split.bounty_amount.to_string()))
}

/// Withdraw treasury fees for a given market (admin only).
pub fn execute_withdraw_treasury(
    mut deps: DepsMut,
    info: MessageInfo,
    market_denom: String,
    amount: String,
) -> Result<Response, ContractError> {
    let withdraw_amount: cosmwasm_std::Uint128 = amount.parse().map_err(|_| {
        ContractError::Std(cosmwasm_std::StdError::parse_err("Uint128", &amount))
    })?;

    let treasury_addr = TREASURY_ADDRESS
        .may_load(deps.storage)?
        .ok_or(ContractError::TreasuryAddressNotSet)?;

    crate::fees::withdraw_treasury(&mut deps, &info.sender, &market_denom, withdraw_amount)?;

    let bank_msg = cosmwasm_std::CosmosMsg::Bank(cosmwasm_std::BankMsg::Send {
        to_address: treasury_addr.to_string(),
        amount: vec![cosmwasm_std::Coin {
            denom: market_denom.clone(),
            amount: withdraw_amount,
        }],
    });

    Ok(Response::default()
        .add_message(bank_msg)
        .add_attribute("action", "withdraw_treasury")
        .add_attribute("market_denom", &market_denom)
        .add_attribute("amount", withdraw_amount.to_string())
        .add_attribute("recipient", treasury_addr.to_string()))
}

/// Withdraw bounty fees for a given market (admin only).
pub fn execute_withdraw_bounty(
    mut deps: DepsMut,
    info: MessageInfo,
    market_denom: String,
    amount: String,
) -> Result<Response, ContractError> {
    let withdraw_amount: cosmwasm_std::Uint128 = amount.parse().map_err(|_| {
        ContractError::Std(cosmwasm_std::StdError::parse_err("Uint128", &amount))
    })?;

    let bounty_addr = BOUNTY_ADDRESS
        .may_load(deps.storage)?
        .ok_or(ContractError::BountyAddressNotSet)?;

    crate::fees::withdraw_bounty(&mut deps, &info.sender, &market_denom, withdraw_amount)?;

    let bank_msg = cosmwasm_std::CosmosMsg::Bank(cosmwasm_std::BankMsg::Send {
        to_address: bounty_addr.to_string(),
        amount: vec![cosmwasm_std::Coin {
            denom: market_denom.clone(),
            amount: withdraw_amount,
        }],
    });

    Ok(Response::default()
        .add_message(bank_msg)
        .add_attribute("action", "withdraw_bounty")
        .add_attribute("market_denom", &market_denom)
        .add_attribute("amount", withdraw_amount.to_string())
        .add_attribute("recipient", bounty_addr.to_string()))
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
        QueryMsg::ProofOfReserve { denom } => {
            let resp = views::query_proof_of_reserve(deps, denom)
                .map_err(|e| StdError::generic_err(e.to_string()))?;
            to_json_binary(&resp)
        }
        QueryMsg::BorrowerHealthFactor { borrower } => {
            let resp = views::query_borrower_health_factor(deps, borrower)
                .map_err(|e| StdError::generic_err(e.to_string()))?;
            to_json_binary(&resp)
        }
        QueryMsg::GetOracleQuorumConfig {} => {
            let config = ORACLE_QUORUM_CONFIG
                .may_load(deps.storage)
                .map_err(|e| StdError::generic_err(e.to_string()))?;
            let resp = crate::msg::OracleQuorumConfigResponse { config };
            to_json_binary(&resp)
        }
        QueryMsg::GetOraclePrice {} => {
            let record = ORACLE_PRICE_RECORD
                .may_load(deps.storage)
                .map_err(|e| StdError::generic_err(e.to_string()))?;
            let resp = crate::msg::OraclePriceResponse {
                price: record.as_ref().map(|r| r.price),
                timestamp: record.as_ref().map(|r| r.timestamp),
            };
            to_json_binary(&resp)
        }
        QueryMsg::GetTreasuryAddress {} => {
            let addr = TREASURY_ADDRESS
                .may_load(deps.storage)
                .map_err(|e| StdError::generic_err(e.to_string()))?;
            let resp = crate::msg::TreasuryAddressResponse {
                address: addr.map(|a| a.to_string()),
            };
            to_json_binary(&resp)
        }
        QueryMsg::GetBountyAddress {} => {
            let addr = BOUNTY_ADDRESS
                .may_load(deps.storage)
                .map_err(|e| StdError::generic_err(e.to_string()))?;
            let resp = crate::msg::BountyAddressResponse {
                address: addr.map(|a| a.to_string()),
            };
            to_json_binary(&resp)
        }
        QueryMsg::GetDefaultFeeShareBps {} => {
            let bps = DEFAULT_FEE_SHARE_BPS
                .may_load(deps.storage)
                .map_err(|e| StdError::generic_err(e.to_string()))?
                .unwrap_or(crate::fees::DEFAULT_TREASURY_FEE_SHARE_BPS);
            let resp = crate::msg::DefaultFeeShareBpsResponse { bps };
            to_json_binary(&resp)
        }
        QueryMsg::GetMarketFeeShareBps { market_denom } => {
            let bps = MARKET_FEE_SHARE_BPS
                .may_load(deps.storage, &market_denom)
                .map_err(|e| StdError::generic_err(e.to_string()))?;
            let resp = crate::msg::MarketFeeShareBpsResponse { market_denom, bps };
            to_json_binary(&resp)
        }
        QueryMsg::GetTreasuryBalance { market_denom } => {
            let balance = TREASURY_BALANCE
                .may_load(deps.storage, &market_denom)
                .map_err(|e| StdError::generic_err(e.to_string()))?
                .unwrap_or(cosmwasm_std::Uint128::zero());
            let resp = crate::msg::TreasuryBalanceResponse {
                market_denom,
                balance,
            };
            to_json_binary(&resp)
        }
        QueryMsg::GetBountyBalance { market_denom } => {
            let balance = BOUNTY_BALANCE
                .may_load(deps.storage, &market_denom)
                .map_err(|e| StdError::generic_err(e.to_string()))?
                .unwrap_or(cosmwasm_std::Uint128::zero());
            let resp = crate::msg::BountyBalanceResponse {
                market_denom,
                balance,
            };
            to_json_binary(&resp)
        }
        QueryMsg::GetFeeSplitPreview {
            market_denom,
            amount,
        } => {
            let fee_amount: cosmwasm_std::Uint128 = amount.parse().map_err(|_| {
                StdError::generic_err(format!("Invalid amount: {}", amount))
            })?;
            let treasury_share_bps =
                crate::fees::get_treasury_fee_share_bps(deps, &market_denom);
            let split = crate::fees::split_protocol_fee(fee_amount, treasury_share_bps)
                .map_err(|e| StdError::generic_err(e.to_string()))?;
            let resp = crate::msg::FeeSplitPreviewResponse {
                market_denom,
                total_fee: fee_amount,
                treasury_share_bps,
                treasury_amount: split.treasury_amount,
                bounty_amount: split.bounty_amount,
            };
            to_json_binary(&resp)
        }
    }
}

#[entry_point]
pub fn migrate(_deps: DepsMut, _env: Env, _msg: MigrateMsg) -> StdResult<Response> {
    Ok(Response::default())
}
