use cosmwasm_std::{
    entry_point, to_json_binary, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdError,
    StdResult,
};

use crate::error::ContractError;
use crate::handshake::{self, ProtocolVersion};
use crate::limits;
use crate::msg::{ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg};
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
        ExecuteMsg::AddOracle { oracle, weight } => {
            execute_add_oracle(deps, info, oracle, weight)
        }
        ExecuteMsg::RemoveOracle { oracle } => execute_remove_oracle(deps, info, oracle),
        ExecuteMsg::ReportValue { value } => execute_report_value(deps, env, info, value),
        ExecuteMsg::SubmitOraclePrices { prices } => {
            execute_submit_oracle_prices(deps, env, info, prices)
        }
        ExecuteMsg::SetLateFeeConfig { config } => {
            execute_set_late_fee_config(deps, info, config)
        }
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

pub fn execute_add_oracle(
    deps: DepsMut,
    info: MessageInfo,
    oracle: String,
    weight: u32,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }
    let oracle_addr = deps.api.addr_validate(&oracle)?;
    oracles::add_oracle(deps, oracle_addr, weight)?;
    Ok(Response::default().add_attribute("action", "add_oracle").add_attribute("oracle", oracle))
}

pub fn execute_remove_oracle(
    deps: DepsMut,
    info: MessageInfo,
    oracle: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }
    let oracle_addr = deps.api.addr_validate(&oracle)?;
    oracles::remove_oracle(deps, oracle_addr)?;
    Ok(Response::default().add_attribute("action", "remove_oracle").add_attribute("oracle", oracle))
}

pub fn execute_report_value(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    value: i128,
) -> Result<Response, ContractError> {
    oracles::report_value(deps, env, info, value)?;
    Ok(Response::default().add_attribute("action", "report_value").add_attribute("value", value.to_string()))
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
    }
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
}
