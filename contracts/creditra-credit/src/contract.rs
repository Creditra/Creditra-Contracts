use cosmwasm_std::{
    entry_point, to_json_binary, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdError,
    StdResult,
};

use crate::error::ContractError;
use crate::handshake::{self, ProtocolVersion};
use crate::limits;
use crate::msg::{ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg};
use crate::oracles;
use crate::state::{
    Config, CreditLine, Draw, DrawAction, DrawAuditEntry, OraclePriceRecord,
    BORROWER_RATE_CEILING_BPS, BORROWER_TO_ID, CONFIG, CREDIT_LINES, CREDIT_LINE_COUNT,
    DEFAULT_RATE_CEILING_BPS, DRAWS, DRAW_AUDIT, DRAW_AUDIT_COUNT, DRAW_COUNT, ORACLE_PRICE_RECORD,
    ORACLE_QUORUM_CONFIG,
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
        ExecuteMsg::SetDefaultRateCeiling { max_rate_bps } => {
            execute_set_default_rate_ceiling(deps, info, max_rate_bps)
        }
        ExecuteMsg::SetBorrowerRateCeiling {
            borrower,
            max_rate_bps,
        } => execute_set_borrower_rate_ceiling(deps, info, borrower, max_rate_bps),
        ExecuteMsg::ClearBorrowerRateCeiling { borrower } => {
            execute_clear_borrower_rate_ceiling(deps, info, borrower)
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

/// Set the protocol-wide default per-borrower rate ceiling (owner only).
///
/// The value is validated against [`limits::MAX_RATE_BPS`] before it is
/// persisted, so an out-of-range ceiling can never be stored.
pub fn execute_set_default_rate_ceiling(
    deps: DepsMut,
    info: MessageInfo,
    max_rate_bps: u32,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }

    let bps = limits::validate_ceiling_bps(max_rate_bps)?;
    DEFAULT_RATE_CEILING_BPS.save(deps.storage, &bps)?;

    Ok(Response::default()
        .add_attribute("action", "set_default_rate_ceiling")
        .add_attribute("max_rate_bps", bps.to_string()))
}

/// Set a per-borrower rate-ceiling override (owner only).
///
/// The override replaces the default for the given borrower. It is validated
/// against [`limits::MAX_RATE_BPS`] before it is persisted.
pub fn execute_set_borrower_rate_ceiling(
    deps: DepsMut,
    info: MessageInfo,
    borrower: String,
    max_rate_bps: u32,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }

    let borrower_addr = deps.api.addr_validate(&borrower)?;
    let bps = limits::validate_ceiling_bps(max_rate_bps)?;
    BORROWER_RATE_CEILING_BPS.save(deps.storage, borrower_addr.clone(), &bps)?;

    Ok(Response::default()
        .add_attribute("action", "set_borrower_rate_ceiling")
        .add_attribute("borrower", borrower_addr.to_string())
        .add_attribute("max_rate_bps", bps.to_string()))
}

/// Remove a per-borrower rate-ceiling override (owner only).
///
/// After removal the borrower is governed by the protocol-wide default again.
/// Clearing a borrower that has no override is a no-op that still succeeds.
pub fn execute_clear_borrower_rate_ceiling(
    deps: DepsMut,
    info: MessageInfo,
    borrower: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }

    let borrower_addr = deps.api.addr_validate(&borrower)?;
    BORROWER_RATE_CEILING_BPS.remove(deps.storage, borrower_addr.clone());

    Ok(Response::default()
        .add_attribute("action", "clear_borrower_rate_ceiling")
        .add_attribute("borrower", borrower_addr.to_string()))
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
        QueryMsg::GetBorrowerRateCeiling { borrower } => {
            let borrower_addr = deps.api.addr_validate(&borrower)?;
            let default_bps = DEFAULT_RATE_CEILING_BPS.may_load(deps.storage)?;
            let override_bps = BORROWER_RATE_CEILING_BPS.may_load(deps.storage, borrower_addr)?;
            // Effective ceiling: override wins, else default, else None.
            let effective_ceiling_bps = override_bps.or(default_bps);
            let resp = crate::msg::BorrowerRateCeilingResponse {
                borrower,
                effective_ceiling_bps,
                override_bps,
                default_bps,
            };
            to_json_binary(&resp)
        }
    }
}

#[entry_point]
pub fn migrate(_deps: DepsMut, _env: Env, _msg: MigrateMsg) -> StdResult<Response> {
    Ok(Response::default())
}

#[cfg(test)]
mod rate_ceiling_tests {
    use super::*;
    use crate::limits::MAX_RATE_BPS;
    use crate::msg::BorrowerRateCeilingResponse;
    use cosmwasm_std::from_json;
    use cosmwasm_std::testing::{
        message_info, mock_dependencies, mock_env, MockApi, MockQuerier, MockStorage,
    };
    use cosmwasm_std::{Addr, OwnedDeps};

    /// Instantiate the contract with a known owner and return that owner.
    fn setup(deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>) -> Addr {
        let owner = deps.api.addr_make("owner");
        let info = message_info(&owner, &[]);
        let msg = InstantiateMsg {
            owner: owner.to_string(),
        };
        instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();
        owner
    }

    fn exec(
        deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
        sender: &Addr,
        msg: ExecuteMsg,
    ) -> Result<Response, ContractError> {
        let info = message_info(sender, &[]);
        execute(deps.as_mut(), mock_env(), info, msg)
    }

    fn query_ceiling(
        deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>,
        borrower: &str,
    ) -> BorrowerRateCeilingResponse {
        let bin = query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::GetBorrowerRateCeiling {
                borrower: borrower.to_string(),
            },
        )
        .unwrap();
        from_json(bin).unwrap()
    }

    // ── set_default_rate_ceiling ────────────────────────────────────────────

    #[test]
    fn owner_sets_default_ceiling() {
        let mut deps = mock_dependencies();
        let owner = setup(&mut deps);

        exec(
            &mut deps,
            &owner,
            ExecuteMsg::SetDefaultRateCeiling {
                max_rate_bps: 1_500,
            },
        )
        .unwrap();

        assert_eq!(
            DEFAULT_RATE_CEILING_BPS
                .load(deps.as_ref().storage)
                .unwrap(),
            1_500
        );
    }

    #[test]
    fn non_owner_cannot_set_default_ceiling() {
        let mut deps = mock_dependencies();
        let _owner = setup(&mut deps);
        let stranger = deps.api.addr_make("stranger");

        let err = exec(
            &mut deps,
            &stranger,
            ExecuteMsg::SetDefaultRateCeiling {
                max_rate_bps: 1_500,
            },
        )
        .unwrap_err();
        assert_eq!(err, ContractError::Unauthorized);
    }

    #[test]
    fn default_ceiling_above_max_is_rejected() {
        let mut deps = mock_dependencies();
        let owner = setup(&mut deps);

        let err = exec(
            &mut deps,
            &owner,
            ExecuteMsg::SetDefaultRateCeiling {
                max_rate_bps: MAX_RATE_BPS + 1,
            },
        )
        .unwrap_err();
        assert_eq!(err, ContractError::InvalidAmount);
    }

    // ── set_borrower_rate_ceiling ───────────────────────────────────────────

    #[test]
    fn owner_sets_borrower_override() {
        let mut deps = mock_dependencies();
        let owner = setup(&mut deps);
        let borrower = deps.api.addr_make("borrower");

        exec(
            &mut deps,
            &owner,
            ExecuteMsg::SetBorrowerRateCeiling {
                borrower: borrower.to_string(),
                max_rate_bps: 800,
            },
        )
        .unwrap();

        let stored = BORROWER_RATE_CEILING_BPS
            .load(deps.as_ref().storage, borrower)
            .unwrap();
        assert_eq!(stored, 800);
    }

    #[test]
    fn non_owner_cannot_set_borrower_override() {
        let mut deps = mock_dependencies();
        let _owner = setup(&mut deps);
        let stranger = deps.api.addr_make("stranger");
        let borrower = deps.api.addr_make("borrower");

        let err = exec(
            &mut deps,
            &stranger,
            ExecuteMsg::SetBorrowerRateCeiling {
                borrower: borrower.to_string(),
                max_rate_bps: 800,
            },
        )
        .unwrap_err();
        assert_eq!(err, ContractError::Unauthorized);
    }

    #[test]
    fn borrower_override_above_max_is_rejected() {
        let mut deps = mock_dependencies();
        let owner = setup(&mut deps);
        let borrower = deps.api.addr_make("borrower");

        let err = exec(
            &mut deps,
            &owner,
            ExecuteMsg::SetBorrowerRateCeiling {
                borrower: borrower.to_string(),
                max_rate_bps: MAX_RATE_BPS + 1,
            },
        )
        .unwrap_err();
        assert_eq!(err, ContractError::InvalidAmount);
    }

    // ── clear_borrower_rate_ceiling ─────────────────────────────────────────

    #[test]
    fn owner_clears_borrower_override() {
        let mut deps = mock_dependencies();
        let owner = setup(&mut deps);
        let borrower = deps.api.addr_make("borrower");

        exec(
            &mut deps,
            &owner,
            ExecuteMsg::SetBorrowerRateCeiling {
                borrower: borrower.to_string(),
                max_rate_bps: 800,
            },
        )
        .unwrap();
        exec(
            &mut deps,
            &owner,
            ExecuteMsg::ClearBorrowerRateCeiling {
                borrower: borrower.to_string(),
            },
        )
        .unwrap();

        assert!(BORROWER_RATE_CEILING_BPS
            .may_load(deps.as_ref().storage, borrower)
            .unwrap()
            .is_none());
    }

    #[test]
    fn clearing_absent_override_is_ok() {
        let mut deps = mock_dependencies();
        let owner = setup(&mut deps);
        let borrower = deps.api.addr_make("borrower");

        // No override set; clearing should still succeed as a no-op.
        exec(
            &mut deps,
            &owner,
            ExecuteMsg::ClearBorrowerRateCeiling {
                borrower: borrower.to_string(),
            },
        )
        .unwrap();
    }

    #[test]
    fn non_owner_cannot_clear_borrower_override() {
        let mut deps = mock_dependencies();
        let _owner = setup(&mut deps);
        let stranger = deps.api.addr_make("stranger");
        let borrower = deps.api.addr_make("borrower");

        let err = exec(
            &mut deps,
            &stranger,
            ExecuteMsg::ClearBorrowerRateCeiling {
                borrower: borrower.to_string(),
            },
        )
        .unwrap_err();
        assert_eq!(err, ContractError::Unauthorized);
    }

    // ── query GetBorrowerRateCeiling ────────────────────────────────────────

    #[test]
    fn query_returns_none_when_unconfigured() {
        let mut deps = mock_dependencies();
        let _owner = setup(&mut deps);
        let borrower = deps.api.addr_make("borrower");

        let resp = query_ceiling(&deps, borrower.as_str());
        assert_eq!(resp.effective_ceiling_bps, None);
        assert_eq!(resp.override_bps, None);
        assert_eq!(resp.default_bps, None);
    }

    #[test]
    fn query_falls_back_to_default() {
        let mut deps = mock_dependencies();
        let owner = setup(&mut deps);
        let borrower = deps.api.addr_make("borrower");

        exec(
            &mut deps,
            &owner,
            ExecuteMsg::SetDefaultRateCeiling {
                max_rate_bps: 1_200,
            },
        )
        .unwrap();

        let resp = query_ceiling(&deps, borrower.as_str());
        assert_eq!(resp.effective_ceiling_bps, Some(1_200));
        assert_eq!(resp.override_bps, None);
        assert_eq!(resp.default_bps, Some(1_200));
    }

    #[test]
    fn query_override_takes_precedence_over_default() {
        let mut deps = mock_dependencies();
        let owner = setup(&mut deps);
        let borrower = deps.api.addr_make("borrower");

        exec(
            &mut deps,
            &owner,
            ExecuteMsg::SetDefaultRateCeiling {
                max_rate_bps: 1_200,
            },
        )
        .unwrap();
        exec(
            &mut deps,
            &owner,
            ExecuteMsg::SetBorrowerRateCeiling {
                borrower: borrower.to_string(),
                max_rate_bps: 500,
            },
        )
        .unwrap();

        let resp = query_ceiling(&deps, borrower.as_str());
        assert_eq!(resp.effective_ceiling_bps, Some(500));
        assert_eq!(resp.override_bps, Some(500));
        assert_eq!(resp.default_bps, Some(1_200));
    }

    #[test]
    fn query_reflects_override_after_clear() {
        let mut deps = mock_dependencies();
        let owner = setup(&mut deps);
        let borrower = deps.api.addr_make("borrower");

        exec(
            &mut deps,
            &owner,
            ExecuteMsg::SetDefaultRateCeiling {
                max_rate_bps: 1_200,
            },
        )
        .unwrap();
        exec(
            &mut deps,
            &owner,
            ExecuteMsg::SetBorrowerRateCeiling {
                borrower: borrower.to_string(),
                max_rate_bps: 500,
            },
        )
        .unwrap();
        exec(
            &mut deps,
            &owner,
            ExecuteMsg::ClearBorrowerRateCeiling {
                borrower: borrower.to_string(),
            },
        )
        .unwrap();

        // After clearing, the borrower reverts to the default ceiling.
        let resp = query_ceiling(&deps, borrower.as_str());
        assert_eq!(resp.effective_ceiling_bps, Some(1_200));
        assert_eq!(resp.override_bps, None);
        assert_eq!(resp.default_bps, Some(1_200));
    }
}
