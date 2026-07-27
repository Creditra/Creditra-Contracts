use cosmwasm_std::{
    entry_point, to_json_binary, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdError,
    StdResult, Uint128,
};

use crate::collateral;
use crate::error::ContractError;
use crate::handshake::{self, ProtocolVersion};
use crate::msg::{
    CollateralAllowlistResponse, CollateralBalanceResponse, CollateralEntryResponse,
    ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg,
};
use crate::oracles;
use crate::penalties::LateFeeConfig;
use crate::state::{
    Config, CreditLine, Draw, DrawAction, DrawAuditEntry, OraclePriceRecord, BORROWER_TO_ID,
    CONFIG, CREDIT_LINES, CREDIT_LINE_COUNT, DRAWS, DRAW_AUDIT, DRAW_AUDIT_COUNT, DRAW_COUNT,
    LATE_FEE_CONFIG, ORACLE_PRICE_RECORD, ORACLE_QUORUM_CONFIG,
};
use crate::views;
use crate::fees;

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
        } => execute_set_oracle_quorum_config(
            deps,
            info,
            min_quorum_k,
            max_deviation_bps,
            max_age_seconds,
        ),
        ExecuteMsg::SubmitOraclePrices { prices } => {
            execute_submit_oracle_prices(deps, env, info, prices)
        }
        ExecuteMsg::SetLateFeeConfig { config } => {
            execute_set_late_fee_config(deps, info, config)
        }
        ExecuteMsg::DepositCollateral {
            borrower,
            denom,
            amount,
        } => execute_deposit_collateral(deps, info, borrower, denom, amount),
        ExecuteMsg::WithdrawCollateral {
            borrower,
            denom,
            amount,
        } => execute_withdraw_collateral(deps, info, borrower, denom, amount),
        ExecuteMsg::AddCollateralToken {
            denom,
            risk_weight_bps,
        } => execute_add_collateral_token(deps, info, denom, risk_weight_bps),
        ExecuteMsg::RemoveCollateralToken { denom } => {
            execute_remove_collateral_token(deps, info, denom)
        }
        ExecuteMsg::SetCollateralRiskWeight {
            denom,
            risk_weight_bps,
        } => execute_set_collateral_risk_weight(deps, info, denom, risk_weight_bps),
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
    mut deps: DepsMut,
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

    let fee_bps = fees::PROTOCOL_FEE_BPS.may_load(deps.storage)?.unwrap_or(0);
    let mut fee_amount = Uint128::zero();
    if fee_bps > 0 && !draw.amount.is_zero() {
        fee_amount = draw.amount.multiply_ratio(fee_bps, 10_000u32);
    }

    if !fee_amount.is_zero() {
        fees::accrue_protocol_fee(deps.branch(), &draw.denom, fee_amount)?;
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

    let mut response = Response::default()
        .add_attribute("action", "repay_draw")
        .add_attribute("credit_line_id", credit_line_id.to_string())
        .add_attribute("draw_id", draw_id.to_string());

    if !fee_amount.is_zero() {
        response = response.add_attribute("protocol_fee_skimmed", fee_amount.to_string());
    }

    Ok(response)
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

/// Configure the late-fee penalty model (admin only).
///
/// Sets the active [`LateFeeConfig`] — either a flat amount per missed
/// installment or an APR-based surcharge applied during delinquency.
/// Pass `None` to clear the config (disables late fees).
pub fn execute_set_late_fee_config(
    deps: DepsMut,
    info: MessageInfo,
    config: Option<LateFeeConfig>,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    if info.sender != cfg.owner {
        return Err(ContractError::Unauthorized);
    }

    if let Some(ref c) = config {
        match c {
            LateFeeConfig::Flat(flat) => {
                if flat.amount < 0 {
                    return Err(ContractError::LateFeeConfigInvalid);
                }
            }
            LateFeeConfig::AprBased(apr) => {
                if apr.surcharge_bps > 10_000 {
                    return Err(ContractError::LateFeeConfigInvalid);
                }
            }
        }
    }

    let has_config = config.is_some();
    if let Some(ref c) = config {
        LATE_FEE_CONFIG.save(deps.storage, c)?;
    } else {
        LATE_FEE_CONFIG.remove(deps.storage);
    }

    Ok(Response::default()
        .add_attribute("action", "set_late_fee_config")
        .add_attribute("has_config", has_config.to_string()))
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

/// Set or update the structured late-fee configuration (admin only).
///
/// Pass `Some(LateFeeConfig::Flat(…))` for a fixed token amount per missed
/// installment, or `Some(LateFeeConfig::AprBased(…))` for an additive
/// basis-point surcharge.  Pass `None` to remove the config.
///
/// # Errors
/// - [`ContractError::Unauthorized`] if the caller is not the contract owner.
/// - [`ContractError::InvalidAmount`] if the flat amount is zero.
/// - [`ContractError::RateTooHigh`] if the APR surcharge exceeds 10 000 bps.
fn execute_set_late_fee_config(
    deps: DepsMut,
    info: MessageInfo,
    config: Option<crate::penalties::LateFeeConfig>,
) -> Result<Response, ContractError> {
    let contract_config = CONFIG.load(deps.storage)?;
    if info.sender != contract_config.owner {
        return Err(ContractError::Unauthorized);
    }

    if let Some(ref cfg) = config {
        crate::penalties::validate_late_fee_config(cfg)?;
    }

    match config {
        Some(cfg) => LATE_FEE_CONFIG.save(deps.storage, &cfg)?,
        None => LATE_FEE_CONFIG.remove(deps.storage),
    }

    Ok(Response::default().add_attribute("action", "set_late_fee_config"))
}

/// Deposit a collateral token on behalf of a borrower (admin only).
///
/// Records a `(borrower, denom)` entry in the multi-collateral store.
/// The actual token transfer must be settled off-chain or by a separate
/// settlement contract.
///
/// # Errors
///
/// - [`ContractError::Unauthorized`] if the caller is not the contract owner.
/// - [`ContractError::InvalidAmount`] if the amount is zero.
/// - [`ContractError::CollateralTokenNotAllowed`] if `denom` is not in the
///   allowlist.
pub fn execute_deposit_collateral(
    deps: DepsMut,
    info: MessageInfo,
    borrower: String,
    denom: String,
    amount: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }
    let borrower_addr = deps.api.addr_validate(&borrower)?;
    let parsed_amount: Uint128 = amount.parse().map_err(|_| {
        ContractError::Std(cosmwasm_std::StdError::parse_err("Uint128", &amount))
    })?;
    collateral::deposit_collateral(deps, &borrower_addr, &denom, parsed_amount)
}

/// Withdraw a collateral token for a borrower (admin only).
///
/// # Errors
///
/// - [`ContractError::Unauthorized`] if the caller is not the contract owner.
/// - [`ContractError::InvalidAmount`] if the amount is zero.
/// - [`ContractError::InsufficientCollateralBalance`] if the balance is
///   insufficient.
pub fn execute_withdraw_collateral(
    deps: DepsMut,
    info: MessageInfo,
    borrower: String,
    denom: String,
    amount: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }
    let borrower_addr = deps.api.addr_validate(&borrower)?;
    let parsed_amount: Uint128 = amount.parse().map_err(|_| {
        ContractError::Std(cosmwasm_std::StdError::parse_err("Uint128", &amount))
    })?;
    collateral::withdraw_collateral(deps, &borrower_addr, &denom, parsed_amount)
}

/// Add a denomination to the collateral allowlist (admin only).
///
/// # Errors
///
/// - [`ContractError::Unauthorized`] if the caller is not the contract owner.
/// - [`ContractError::InvalidAmount`] if `risk_weight_bps > 10_000`.
/// - [`ContractError::AlreadySettled`] if `denom` is already in the allowlist.
pub fn execute_add_collateral_token(
    deps: DepsMut,
    info: MessageInfo,
    denom: String,
    risk_weight_bps: u32,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }
    collateral::add_collateral_token(deps, &denom, risk_weight_bps)
}

/// Remove a denomination from the collateral allowlist (admin only).
///
/// # Errors
///
/// - [`ContractError::Unauthorized`] if the caller is not the contract owner.
/// - [`ContractError::CollateralTokenNotAllowed`] if `denom` is not in the
///   allowlist.
pub fn execute_remove_collateral_token(
    deps: DepsMut,
    info: MessageInfo,
    denom: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }
    collateral::remove_collateral_token(deps, &denom)
}

/// Update the risk weight for an allowed collateral token (admin only).
///
/// # Errors
///
/// - [`ContractError::Unauthorized`] if the caller is not the contract owner.
/// - [`ContractError::CollateralTokenNotAllowed`] if `denom` is not in the
///   allowlist.
/// - [`ContractError::InvalidAmount`] if `risk_weight_bps > 10_000`.
pub fn execute_set_collateral_risk_weight(
    deps: DepsMut,
    info: MessageInfo,
    denom: String,
    risk_weight_bps: u32,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }
    collateral::set_collateral_risk_weight(deps, &denom, risk_weight_bps)
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
        QueryMsg::GetLateFeeConfig {} => {
            let config = LATE_FEE_CONFIG
                .may_load(deps.storage)
                .map_err(|e| StdError::generic_err(e.to_string()))?;
            let resp = crate::msg::LateFeeConfigResponse { config };
            to_json_binary(&resp)
        }
        QueryMsg::GetCollateralBalance { borrower, denom } => {
            query_collateral_balance(deps, borrower, denom)
        }
        QueryMsg::GetCollateralAllowlist {} => query_collateral_allowlist(deps),
    }
}

fn query_collateral_balance(
    deps: Deps,
    borrower: String,
    denom: Option<String>,
) -> StdResult<Binary> {
    let borrower_addr = deps.api.addr_validate(&borrower)?;

    let entries = match denom {
        Some(ref d) => {
            let amount = collateral::query_collateral_balance(deps, &borrower_addr, d);
            let risk_weight_bps = collateral::collateral_risk_weight_bps(deps, d);
            if amount.is_zero() {
                vec![]
            } else {
                vec![CollateralEntryResponse {
                    denom: d.clone(),
                    amount,
                    risk_weight_bps,
                }]
            }
        }
        None => {
            let raw = collateral::query_borrower_collateral(deps, &borrower_addr);
            raw.into_iter()
                .map(|(denom, amount)| CollateralEntryResponse {
                    denom,
                    amount,
                    risk_weight_bps: collateral::collateral_risk_weight_bps(deps, &denom),
                })
                .collect()
        }
    };

    let weighted_total = collateral::weighted_collateral_total(deps, &borrower_addr)
        .map_err(|e| StdError::generic_err(e.to_string()))?;

    let resp = CollateralBalanceResponse {
        borrower,
        entries,
        weighted_total,
    };
    to_json_binary(&resp)
}

fn query_collateral_allowlist(deps: Deps) -> StdResult<Binary> {
    let denoms = collateral::query_collateral_allowlist(deps);
    let resp = CollateralAllowlistResponse { denoms };
    to_json_binary(&resp)
}

#[entry_point]
pub fn migrate(_deps: DepsMut, _env: Env, _msg: MigrateMsg) -> StdResult<Response> {
    Ok(Response::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};
    use crate::penalties::{AprFeeConfig, FlatFeeConfig, LateFeeConfig};
    use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env};
    use cosmwasm_std::testing::{MockApi, MockQuerier, MockStorage};
    use cosmwasm_std::{from_json, Addr, OwnedDeps};

    fn creator(deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>) -> Addr {
        deps.api.addr_make("creator")
    }

    fn non_admin(deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>) -> Addr {
        deps.api.addr_make("non_admin")
    }

    fn setup(deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>) {
        let env = mock_env();
        let info = message_info(&creator(deps), &[]);
        let msg = InstantiateMsg {
            owner: creator(deps).to_string(),
        };
        instantiate(deps.as_mut(), env, info, msg).unwrap();
    }

    fn query_late_fee_config(
        deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>,
    ) -> Option<LateFeeConfig> {
        let env = mock_env();
        let msg = QueryMsg::GetLateFeeConfig {};
        let raw = query(deps.as_ref(), env, msg).unwrap();
        let resp: crate::msg::LateFeeConfigResponse = from_json(&raw).unwrap();
        resp.config
    }

    fn set_late_fee_config(
        deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
        sender: &Addr,
        config: Option<LateFeeConfig>,
    ) -> Result<Response, ContractError> {
        let env = mock_env();
        let info = message_info(sender, &[]);
        let msg = ExecuteMsg::SetLateFeeConfig { config };
        execute(deps.as_mut(), env, info, msg)
    }

    mod set_late_fee_config {
        use super::*;

        #[test]
        fn admin_can_set_flat_config() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = creator(&deps);

            let config = LateFeeConfig::Flat(FlatFeeConfig { amount: 100 });
            set_late_fee_config(&mut deps, &admin, Some(config.clone())).unwrap();

            let stored = query_late_fee_config(&deps);
            assert_eq!(stored, Some(config));
        }

        #[test]
        fn admin_can_set_apr_config() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = creator(&deps);

            let config = LateFeeConfig::AprBased(AprFeeConfig { surcharge_bps: 500 });
            set_late_fee_config(&mut deps, &admin, Some(config.clone())).unwrap();

            let stored = query_late_fee_config(&deps);
            assert_eq!(stored, Some(config));
        }

        #[test]
        fn admin_can_clear_config() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = creator(&deps);

            let config = LateFeeConfig::Flat(FlatFeeConfig { amount: 50 });
            set_late_fee_config(&mut deps, &admin, Some(config)).unwrap();
            assert!(query_late_fee_config(&deps).is_some());

            set_late_fee_config(&mut deps, &admin, None).unwrap();
            assert!(query_late_fee_config(&deps).is_none());
        }

        #[test]
        fn non_admin_cannot_set_config() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let unauth = non_admin(&deps);

            let config = LateFeeConfig::Flat(FlatFeeConfig { amount: 100 });
            let err = set_late_fee_config(&mut deps, &unauth, Some(config)).unwrap_err();
            assert_eq!(err, ContractError::Unauthorized);
        }

        #[test]
        fn negative_flat_amount_rejected() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = creator(&deps);

            let config = LateFeeConfig::Flat(FlatFeeConfig { amount: -1 });
            let err = set_late_fee_config(&mut deps, &admin, Some(config)).unwrap_err();
            assert_eq!(err, ContractError::LateFeeConfigInvalid);
        }

        #[test]
        fn apr_surcharge_exceeds_max_rejected() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = creator(&deps);

            let config = LateFeeConfig::AprBased(AprFeeConfig { surcharge_bps: 10_001 });
            let err = set_late_fee_config(&mut deps, &admin, Some(config)).unwrap_err();
            assert_eq!(err, ContractError::LateFeeConfigInvalid);
        }

        #[test]
        fn max_apr_surcharge_accepted() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = creator(&deps);

            let config = LateFeeConfig::AprBased(AprFeeConfig { surcharge_bps: 10_000 });
            set_late_fee_config(&mut deps, &admin, Some(config.clone())).unwrap();

            let stored = query_late_fee_config(&deps);
            assert_eq!(stored, Some(config));
        }

        #[test]
        fn clearing_config_when_already_clear_is_noop() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = creator(&deps);

            assert!(query_late_fee_config(&deps).is_none());
            set_late_fee_config(&mut deps, &admin, None).unwrap();
            assert!(query_late_fee_config(&deps).is_none());
        }

        #[test]
        fn set_response_has_correct_attributes() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = creator(&deps);

            let config = LateFeeConfig::Flat(FlatFeeConfig { amount: 200 });
            let resp = set_late_fee_config(&mut deps, &admin, Some(config)).unwrap();
            assert_eq!(resp.attributes[0].key, "action");
            assert_eq!(resp.attributes[0].value, "set_late_fee_config");
            assert_eq!(resp.attributes[1].key, "has_config");
            assert_eq!(resp.attributes[1].value, "true");

            let resp = set_late_fee_config(&mut deps, &admin, None).unwrap();
            assert_eq!(resp.attributes[1].value, "false");
        }

        #[test]
        fn query_default_is_none() {
            let mut deps = mock_dependencies();
            setup(&mut deps);

            assert!(query_late_fee_config(&deps).is_none());
        }

        #[test]
        fn flat_config_survives_set_overwrite() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = creator(&deps);

            let flat = LateFeeConfig::Flat(FlatFeeConfig { amount: 100 });
            let apr = LateFeeConfig::AprBased(AprFeeConfig { surcharge_bps: 200 });

            set_late_fee_config(&mut deps, &admin, Some(flat)).unwrap();
            set_late_fee_config(&mut deps, &admin, Some(apr.clone())).unwrap();

            let stored = query_late_fee_config(&deps);
            assert_eq!(stored, Some(apr));
        }

        #[test]
        fn zero_flat_amount_accepted() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = creator(&deps);

            let config = LateFeeConfig::Flat(FlatFeeConfig { amount: 0 });
            set_late_fee_config(&mut deps, &admin, Some(config.clone())).unwrap();

            let stored = query_late_fee_config(&deps);
            assert_eq!(stored, Some(config));
        }
    }

    mod collateral {
        use super::*;

        fn admin(deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>) -> Addr {
            creator(deps)
        }

        fn borrower(deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>) -> Addr {
            deps.api.addr_make("borrower")
        }

        fn deposit_collateral(
            deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
            sender: &Addr,
            borrower: &str,
            denom: &str,
            amount: &str,
        ) -> Result<Response, ContractError> {
            let env = mock_env();
            let info = message_info(sender, &[]);
            let msg = ExecuteMsg::DepositCollateral {
                borrower: borrower.to_string(),
                denom: denom.to_string(),
                amount: amount.to_string(),
            };
            execute(deps.as_mut(), env, info, msg)
        }

        fn withdraw_collateral(
            deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
            sender: &Addr,
            borrower: &str,
            denom: &str,
            amount: &str,
        ) -> Result<Response, ContractError> {
            let env = mock_env();
            let info = message_info(sender, &[]);
            let msg = ExecuteMsg::WithdrawCollateral {
                borrower: borrower.to_string(),
                denom: denom.to_string(),
                amount: amount.to_string(),
            };
            execute(deps.as_mut(), env, info, msg)
        }

        fn add_collateral_token(
            deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
            sender: &Addr,
            denom: &str,
            risk_weight_bps: u32,
        ) -> Result<Response, ContractError> {
            let env = mock_env();
            let info = message_info(sender, &[]);
            let msg = ExecuteMsg::AddCollateralToken {
                denom: denom.to_string(),
                risk_weight_bps,
            };
            execute(deps.as_mut(), env, info, msg)
        }

        fn remove_collateral_token(
            deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
            sender: &Addr,
            denom: &str,
        ) -> Result<Response, ContractError> {
            let env = mock_env();
            let info = message_info(sender, &[]);
            let msg = ExecuteMsg::RemoveCollateralToken {
                denom: denom.to_string(),
            };
            execute(deps.as_mut(), env, info, msg)
        }

        fn set_collateral_risk_weight(
            deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
            sender: &Addr,
            denom: &str,
            risk_weight_bps: u32,
        ) -> Result<Response, ContractError> {
            let env = mock_env();
            let info = message_info(sender, &[]);
            let msg = ExecuteMsg::SetCollateralRiskWeight {
                denom: denom.to_string(),
                risk_weight_bps,
            };
            execute(deps.as_mut(), env, info, msg)
        }

        fn query_collateral_balance(
            deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>,
            borrower: &str,
            denom: Option<&str>,
        ) -> crate::msg::CollateralBalanceResponse {
            let env = mock_env();
            let msg = QueryMsg::GetCollateralBalance {
                borrower: borrower.to_string(),
                denom: denom.map(|d| d.to_string()),
            };
            let raw = query(deps.as_ref(), env, msg).unwrap();
            from_json(&raw).unwrap()
        }

        fn query_collateral_allowlist(
            deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>,
        ) -> crate::msg::CollateralAllowlistResponse {
            let env = mock_env();
            let msg = QueryMsg::GetCollateralAllowlist {};
            let raw = query(deps.as_ref(), env, msg).unwrap();
            from_json(&raw).unwrap()
        }

        fn query_health(
            deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>,
            borrower: &str,
        ) -> crate::msg::BorrowerHealthFactorResponse {
            let env = mock_env();
            let msg = QueryMsg::BorrowerHealthFactor {
                borrower: borrower.to_string(),
            };
            let raw = query(deps.as_ref(), env, msg).unwrap();
            from_json(&raw).unwrap()
        }

        fn query_por(
            deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>,
            denom: Option<String>,
        ) -> crate::msg::ProofOfReserveResponse {
            let env = mock_env();
            let msg = QueryMsg::ProofOfReserve { denom };
            let raw = query(deps.as_ref(), env, msg).unwrap();
            from_json(&raw).unwrap()
        }

        fn create_credit_line(
            deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
            sender: &Addr,
            borrower: &str,
            coll_denom: &str,
            coll_amount: &str,
            credit_denom: &str,
            credit_amount: &str,
        ) -> Result<Response, ContractError> {
            let env = mock_env();
            let info = message_info(sender, &[]);
            let msg = ExecuteMsg::CreateCreditLine {
                borrower: borrower.to_string(),
                collateral_denom: coll_denom.to_string(),
                collateral_amount: coll_amount.to_string(),
                credit_denom: credit_denom.to_string(),
                credit_amount: credit_amount.to_string(),
            };
            execute(deps.as_mut(), env, info, msg)
        }

        fn create_draw(
            deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
            borrower: &Addr,
            credit_line_id: u64,
            amount: &str,
        ) -> Result<Response, ContractError> {
            let env = mock_env();
            let info = message_info(borrower, &[]);
            let msg = ExecuteMsg::CreateDraw {
                credit_line_id,
                amount: amount.to_string(),
                denom: "ucredit".to_string(),
            };
            execute(deps.as_mut(), env, info, msg)
        }

        // ── Deposit / Withdraw authorisation ──────────────────────────

        #[test]
        fn deposit_collateral_requires_admin() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let unauth = non_admin(&deps);
            let err = deposit_collateral(&mut deps, &unauth, "borrower", "uusd", "100")
                .unwrap_err();
            assert_eq!(err, ContractError::Unauthorized);
        }

        #[test]
        fn withdraw_collateral_requires_admin() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let unauth = non_admin(&deps);
            let err = withdraw_collateral(&mut deps, &unauth, "borrower", "uusd", "100")
                .unwrap_err();
            assert_eq!(err, ContractError::Unauthorized);
        }

        // ── Add / Remove / Set risk-weight authorisation ───────────────

        #[test]
        fn add_collateral_token_requires_admin() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let unauth = non_admin(&deps);
            let err = add_collateral_token(&mut deps, &unauth, "uusd", 10_000).unwrap_err();
            assert_eq!(err, ContractError::Unauthorized);
        }

        #[test]
        fn remove_collateral_token_requires_admin() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let unauth = non_admin(&deps);
            let err = remove_collateral_token(&mut deps, &unauth, "uusd").unwrap_err();
            assert_eq!(err, ContractError::Unauthorized);
        }

        #[test]
        fn set_collateral_risk_weight_requires_admin() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let unauth = non_admin(&deps);
            let err =
                set_collateral_risk_weight(&mut deps, &unauth, "uusd", 5_000).unwrap_err();
            assert_eq!(err, ContractError::Unauthorized);
        }

        // ── Deposit / Withdraw success paths ──────────────────────────

        #[test]
        fn deposit_and_query_balance() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = admin(&deps);
            let borrower_addr = borrower(&deps);
            let borrower_str = borrower_addr.to_string();

            add_collateral_token(&mut deps, &admin, "uusd", 10_000).unwrap();

            deposit_collateral(&mut deps, &admin, &borrower_str, "uusd", "500").unwrap();

            let resp = query_collateral_balance(&deps, &borrower_str, Some("uusd"));
            assert_eq!(resp.borrower, borrower_str);
            assert_eq!(resp.entries.len(), 1);
            assert_eq!(resp.entries[0].denom, "uusd");
            assert_eq!(resp.entries[0].amount, Uint128::new(500));
            assert_eq!(resp.entries[0].risk_weight_bps, 10_000);
            assert_eq!(resp.weighted_total, Uint128::new(500));
        }

        #[test]
        fn deposit_multiple_tokens_and_query_all() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = admin(&deps);
            let borrower_str = borrower(&deps).to_string();

            add_collateral_token(&mut deps, &admin, "uusd", 10_000).unwrap();
            add_collateral_token(&mut deps, &admin, "uatom", 5_000).unwrap();

            deposit_collateral(&mut deps, &admin, &borrower_str, "uusd", "1000").unwrap();
            deposit_collateral(&mut deps, &admin, &borrower_str, "uatom", "200").unwrap();

            let resp = query_collateral_balance(&deps, &borrower_str, None);
            assert_eq!(resp.entries.len(), 2);
            // weighted: 1000*10000/10000 + 200*5000/10000 = 1000 + 100 = 1100
            assert_eq!(resp.weighted_total, Uint128::new(1100));
        }

        #[test]
        fn withdraw_after_deposit() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = admin(&deps);
            let borrower_str = borrower(&deps).to_string();

            add_collateral_token(&mut deps, &admin, "uusd", 10_000).unwrap();
            deposit_collateral(&mut deps, &admin, &borrower_str, "uusd", "500").unwrap();
            withdraw_collateral(&mut deps, &admin, &borrower_str, "uusd", "200").unwrap();

            let resp = query_collateral_balance(&deps, &borrower_str, Some("uusd"));
            assert_eq!(resp.entries[0].amount, Uint128::new(300));
        }

        #[test]
        fn insufficient_balance_on_withdraw() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = admin(&deps);
            let borrower_str = borrower(&deps).to_string();

            add_collateral_token(&mut deps, &admin, "uusd", 10_000).unwrap();
            deposit_collateral(&mut deps, &admin, &borrower_str, "uusd", "100").unwrap();

            let err =
                withdraw_collateral(&mut deps, &admin, &borrower_str, "uusd", "200").unwrap_err();
            assert_eq!(err, ContractError::InsufficientCollateralBalance);
        }

        #[test]
        fn invalid_amount_on_zero_deposit() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = admin(&deps);
            let borrower_str = borrower(&deps).to_string();

            add_collateral_token(&mut deps, &admin, "uusd", 10_000).unwrap();
            let err =
                deposit_collateral(&mut deps, &admin, &borrower_str, "uusd", "0").unwrap_err();
            assert_eq!(err, ContractError::InvalidAmount);
        }

        // ── Allowlist management ──────────────────────────────────────

        #[test]
        fn add_and_remove_token() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = admin(&deps);

            add_collateral_token(&mut deps, &admin, "uusd", 10_000).unwrap();

            let list = query_collateral_allowlist(&deps);
            assert!(list.denoms.contains(&"uusd".to_string()));

            remove_collateral_token(&mut deps, &admin, "uusd").unwrap();

            let list2 = query_collateral_allowlist(&deps);
            assert!(!list2.denoms.contains(&"uusd".to_string()));
        }

        #[test]
        fn deposit_rejected_for_unlisted_token() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = admin(&deps);
            let borrower_str = borrower(&deps).to_string();

            let err =
                deposit_collateral(&mut deps, &admin, &borrower_str, "uusd", "100").unwrap_err();
            assert_eq!(err, ContractError::CollateralTokenNotAllowed);
        }

        // ── Multi-collateral health factor ────────────────────────────

        #[test]
        fn health_factor_includes_multi_collateral() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = admin(&deps);
            let borrower_addr = borrower(&deps);
            let borrower_str = borrower_addr.to_string();

            // Create a credit line: collateral 1000, credit 500
            create_credit_line(
                &mut deps,
                &admin,
                &borrower_str,
                "ucollateral",
                "1000",
                "ucredit",
                "500",
            )
            .unwrap();

            // Draw 100
            create_draw(&mut deps, &borrower_addr, 0, "100").unwrap();

            // Add multi-collateral: 200 of uatom at 50% risk weight
            add_collateral_token(&mut deps, &admin, "uatom", 5_000).unwrap();
            deposit_collateral(&mut deps, &admin, &borrower_str, "uatom", "200").unwrap();

            let resp = query_health(&deps, &borrower_str);
            assert_eq!(resp.credit_lines.len(), 1);

            // effective_collateral = 1000 (credit line) + 200*5000/10000 (multi) = 1000 + 100 = 1100
            // health = 1100 * 10_000 / 100 = 110_000
            assert_eq!(resp.credit_lines[0].health_factor_bps, 110_000);
        }

        #[test]
        fn health_factor_with_only_multi_collateral() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = admin(&deps);
            let borrower_addr = borrower(&deps);
            let borrower_str = borrower_addr.to_string();

            // Create a credit line with zero collateral
            create_credit_line(
                &mut deps,
                &admin,
                &borrower_str,
                "ucollateral",
                "0",
                "ucredit",
                "500",
            )
            .unwrap();

            // Draw 100
            create_draw(&mut deps, &borrower_addr, 0, "100").unwrap();

            // Without any multi-collateral, effective_collateral = 0 → health = 0
            let resp1 = query_health(&deps, &borrower_str);
            assert_eq!(resp1.credit_lines[0].health_factor_bps, 0);

            // Add multi-collateral: 500 uusd at 100% weight
            add_collateral_token(&mut deps, &admin, "uusd", 10_000).unwrap();
            deposit_collateral(&mut deps, &admin, &borrower_str, "uusd", "500").unwrap();

            let resp2 = query_health(&deps, &borrower_str);
            // effective_collateral = 0 + 500 = 500
            // health = 500 * 10_000 / 100 = 50_000
            assert_eq!(resp2.credit_lines[0].health_factor_bps, 50_000);
        }

        // ── Multi-collateral proof of reserve ─────────────────────────

        #[test]
        fn proof_of_reserve_includes_multi_collateral() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = admin(&deps);
            let borrower_addr = borrower(&deps);
            let borrower_str = borrower_addr.to_string();

            create_credit_line(
                &mut deps,
                &admin,
                &borrower_str,
                "ucollateral",
                "1000",
                "ucredit",
                "500",
            )
            .unwrap();

            add_collateral_token(&mut deps, &admin, "uusd", 10_000).unwrap();
            deposit_collateral(&mut deps, &admin, &borrower_str, "uusd", "200").unwrap();

            let por = query_por(&deps, None);
            // total_collateral should include both: 1000 + 200 = 1200
            assert_eq!(por.total_collateral, Uint128::new(1200));

            // Filter by multi-collateral denom
            let por_uusd = query_por(&deps, Some("uusd".to_string()));
            assert_eq!(por_uusd.reserves_by_denom.len(), 1);
            assert_eq!(por_uusd.reserves_by_denom[0].denom, "uusd");
            assert_eq!(
                por_uusd.reserves_by_denom[0].collateral_amount,
                Uint128::new(200)
            );
            assert_eq!(por_uusd.total_collateral, Uint128::new(200));
        }

        // ── Edge cases ────────────────────────────────────────────────

        #[test]
        fn deposit_zero_amount_rejected() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = admin(&deps);
            let borrower_str = borrower(&deps).to_string();

            add_collateral_token(&mut deps, &admin, "uusd", 10_000).unwrap();
            let err =
                deposit_collateral(&mut deps, &admin, &borrower_str, "uusd", "0").unwrap_err();
            assert_eq!(err, ContractError::InvalidAmount);
        }

        #[test]
        fn withdraw_zero_amount_rejected() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = admin(&deps);
            let borrower_str = borrower(&deps).to_string();

            add_collateral_token(&mut deps, &admin, "uusd", 10_000).unwrap();
            deposit_collateral(&mut deps, &admin, &borrower_str, "uusd", "100").unwrap();
            let err =
                withdraw_collateral(&mut deps, &admin, &borrower_str, "uusd", "0").unwrap_err();
            assert_eq!(err, ContractError::InvalidAmount);
        }

        #[test]
        fn add_existing_token_rejected() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = admin(&deps);

            add_collateral_token(&mut deps, &admin, "uusd", 10_000).unwrap();
            let err = add_collateral_token(&mut deps, &admin, "uusd", 10_000).unwrap_err();
            assert_eq!(err, ContractError::AlreadySettled);
        }

        #[test]
        fn remove_unlisted_token_rejected() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = admin(&deps);

            let err = remove_collateral_token(&mut deps, &admin, "uusd").unwrap_err();
            assert_eq!(err, ContractError::CollateralTokenNotAllowed);
        }

        #[test]
        fn risk_weight_exceeds_max_rejected() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = admin(&deps);

            let err = add_collateral_token(&mut deps, &admin, "uusd", 10_001).unwrap_err();
            assert_eq!(err, ContractError::InvalidAmount);
        }

        #[test]
        fn borrower_collateral_independent_from_credit_line() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = admin(&deps);
            let borrower_str = borrower(&deps).to_string();

            add_collateral_token(&mut deps, &admin, "uusd", 10_000).unwrap();
            deposit_collateral(&mut deps, &admin, &borrower_str, "uusd", "300").unwrap();

            // No credit lines exist, but collateral should still be queryable
            let resp = query_collateral_balance(&deps, &borrower_str, Some("uusd"));
            assert_eq!(resp.entries.len(), 1);
            assert_eq!(resp.entries[0].amount, Uint128::new(300));
        }

        #[test]
        fn multiple_borrowers_independent_collateral() {
            let mut deps = mock_dependencies();
            setup(&mut deps);
            let admin = admin(&deps);
            let b1 = deps.api.addr_make("borrower1").to_string();
            let b2 = deps.api.addr_make("borrower2").to_string();

            add_collateral_token(&mut deps, &admin, "uusd", 10_000).unwrap();
            deposit_collateral(&mut deps, &admin, &b1, "uusd", "500").unwrap();
            deposit_collateral(&mut deps, &admin, &b2, "uusd", "300").unwrap();

            let r1 = query_collateral_balance(&deps, &b1, Some("uusd"));
            assert_eq!(r1.entries[0].amount, Uint128::new(500));

            let r2 = query_collateral_balance(&deps, &b2, Some("uusd"));
            assert_eq!(r2.entries[0].amount, Uint128::new(300));
        }
    }
}
