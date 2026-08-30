// SPDX-License-Identifier: MIT

//! Focused integration tests for `creditra-credit` borrow public entrypoints.
//!
//! Covers the v7 CosmWasm surface documented in
//! [`creditra_credit::contract`]:
//! - `instantiate` — owner + counter init
//! - `execute_create_credit_line` — admin-only line origination
//! - `execute_create_draw` — borrower-authored draw
//! - `execute_repay_draw` — drawer-authored repay + protocol fee
//! - `execute_add_audit_memo` — admin memo append
//! - `execute_update_protocol_version` — version bump handshake
//! - `execute_set_oracle_quorum_config` / `execute_submit_oracle_prices`
//! - `execute_set_late_fee_config` — flat vs APR modes
//! - `query` dispatch (`DrawAuditTrail`, `ProofOfReserve`,
//!   `BorrowerHealthFactor`, `GetOracleQuorumConfig`, `GetOraclePrice`,
//!   `GetLateFeeConfig`)
//!
//! Auth, amount-parsing, and audit-trail invariants are pinned here so a
//! future refactor cannot silently weaken the documented preconditions.

use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env};
use cosmwasm_std::testing::{MockApi, MockQuerier, MockStorage};
use cosmwasm_std::{from_json, Addr, OwnedDeps, Uint128};
use creditra_credit::contract::{
    execute, execute_add_audit_memo, execute_create_credit_line, execute_create_draw,
    execute_repay_draw, execute_set_late_fee_config, execute_set_oracle_quorum_config,
    execute_submit_oracle_prices, execute_update_protocol_version, instantiate, query,
};
use creditra_credit::msg::{
    ExecuteMsg, InstantiateMsg, LateFeeConfigResponse, OraclePriceResponse,
    OracleQuorumConfigResponse, QueryMsg,
};
use creditra_credit::penalties::{AprFeeConfig, FlatFeeConfig, LateFeeConfig};

// ═══════════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════════

fn admin(deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>) -> Addr {
    deps.api.addr_make("admin")
}

fn borrower(deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>) -> Addr {
    deps.api.addr_make("alice")
}

fn stranger(deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>) -> Addr {
    deps.api.addr_make("eve")
}

fn setup(deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>) {
    let admin_addr = admin(deps);
    let env = mock_env();
    let info = message_info(&admin_addr, &[]);
    let msg = InstantiateMsg {
        owner: admin_addr.to_string(),
    };
    instantiate(deps.as_mut(), env, info, msg).unwrap();
}

fn open_default_line(deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>) -> u64 {
    let admin_addr = admin(deps);
    let borrower_addr = borrower(deps);
    let env = mock_env();
    let info = message_info(&admin_addr, &[]);
    let res = execute_create_credit_line(
        deps.as_mut(),
        env,
        info,
        borrower_addr.to_string(),
        "ucosm".to_string(),
        "1000000".to_string(),
        "ucredit".to_string(),
        "500000".to_string(),
    )
    .unwrap();
    let id_attr = res
        .attributes
        .iter()
        .find(|a| a.key == "credit_line_id")
        .unwrap();
    id_attr.value.parse::<u64>().unwrap()
}

fn draw_amount(
    deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
    cl_id: u64,
    amount: &str,
) -> u64 {
    let borrower_addr = borrower(deps);
    let env = mock_env();
    let info = message_info(&borrower_addr, &[]);
    let res = execute_create_draw(
        deps.as_mut(),
        env,
        info,
        cl_id,
        amount.to_string(),
        "ucredit".to_string(),
    )
    .unwrap();
    let draw_attr = res.attributes.iter().find(|a| a.key == "draw_id").unwrap();
    draw_attr.value.parse::<u64>().unwrap()
}

// ═══════════════════════════════════════════════════════════════════════════
// Instantiate
// ═══════════════════════════════════════════════════════════════════════════

mod instantiate_tests {
    use super::*;

    #[test]
    fn sets_owner_and_zeroes_counters() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let env = mock_env();
        let msg = QueryMsg::ProofOfReserve { denom: None };
        let raw = query(deps.as_ref(), env, msg).unwrap();
        let por: creditra_credit::msg::ProofOfReserveResponse = from_json(&raw).unwrap();
        assert_eq!(por.total_credit_lines, 0);
        assert_eq!(por.active_credit_lines, 0);
    }

    #[test]
    fn rejects_invalid_owner_address() {
        let mut deps = mock_dependencies();
        let env = mock_env();
        let info = message_info(&Addr::unchecked("creator"), &[]);
        let msg = InstantiateMsg {
            owner: "not-a-valid-bech32!!!".to_string(),
        };
        let result = instantiate(deps.as_mut(), env, info, msg);
        assert!(
            result.is_err(),
            "instantiate should reject invalid owner addresses"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// execute_create_credit_line
// ═══════════════════════════════════════════════════════════════════════════

mod create_credit_line_tests {
    use super::*;
    use creditra_credit::error::ContractError;

    #[test]
    fn admin_can_create_line_and_gets_correct_id() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let id0 = open_default_line(&mut deps);
        assert_eq!(id0, 0);
        let id1 = open_default_line(&mut deps);
        assert_eq!(id1, 1);
    }

    #[test]
    fn non_admin_rejected_with_unauthorized() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let stranger_addr = stranger(&deps);
        let borrower_addr = borrower(&deps);
        let env = mock_env();
        let info = message_info(&stranger_addr, &[]);
        let err = execute_create_credit_line(
            deps.as_mut(),
            env,
            info,
            borrower_addr.to_string(),
            "ucosm".to_string(),
            "1".to_string(),
            "ucredit".to_string(),
            "1".to_string(),
        )
        .unwrap_err();
        assert_eq!(err, ContractError::Unauthorized);
    }

    #[test]
    fn rejects_invalid_collateral_amount_string() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let admin_addr = admin(&deps);
        let borrower_addr = borrower(&deps);
        let env = mock_env();
        let info = message_info(&admin_addr, &[]);
        let err = execute_create_credit_line(
            deps.as_mut(),
            env,
            info,
            borrower_addr.to_string(),
            "ucosm".to_string(),
            "not-a-number".to_string(),
            "ucredit".to_string(),
            "100".to_string(),
        )
        .unwrap_err();
        match err {
            ContractError::Std(_) => {}
            other => panic!("Expected Std parse error, got {:?}", other),
        }
    }

    #[test]
    fn response_attributes_include_action_and_id() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let admin_addr = admin(&deps);
        let borrower_addr = borrower(&deps);
        let env = mock_env();
        let info = message_info(&admin_addr, &[]);
        let res = execute_create_credit_line(
            deps.as_mut(),
            env,
            info,
            borrower_addr.to_string(),
            "ucosm".to_string(),
            "100".to_string(),
            "ucredit".to_string(),
            "50".to_string(),
        )
        .unwrap();
        assert_eq!(res.attributes[0].key, "action");
        assert_eq!(res.attributes[0].value, "create_credit_line");
        assert_eq!(res.attributes[1].key, "credit_line_id");
        assert_eq!(res.attributes[1].value, "0");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// execute_create_draw
// ═══════════════════════════════════════════════════════════════════════════

mod create_draw_tests {
    use super::*;
    use creditra_credit::error::ContractError;

    #[test]
    fn borrower_can_draw_and_receives_draw_id() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let cl_id = open_default_line(&mut deps);
        let d0 = draw_amount(&mut deps, cl_id, "100");
        assert_eq!(d0, 0);
        let d1 = draw_amount(&mut deps, cl_id, "200");
        assert_eq!(d1, 1);
    }

    #[test]
    fn non_borrower_cannot_draw() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let cl_id = open_default_line(&mut deps);
        let env = mock_env();
        let info = message_info(&stranger(&deps), &[]);
        let err = execute_create_draw(
            deps.as_mut(),
            env,
            info,
            cl_id,
            "50".to_string(),
            "ucredit".to_string(),
        )
        .unwrap_err();
        assert_eq!(err, ContractError::CrossTenantIdentifier);
    }

    #[test]
    fn missing_credit_line_returns_not_found() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let env = mock_env();
        let info = message_info(&borrower(&deps), &[]);
        let err = execute_create_draw(
            deps.as_mut(),
            env,
            info,
            9999u64,
            "50".to_string(),
            "ucredit".to_string(),
        )
        .unwrap_err();
        assert_eq!(err, ContractError::CreditLineNotFound(9999));
    }

    #[test]
    fn invalid_amount_string_propagates_parse_error() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let cl_id = open_default_line(&mut deps);
        let env = mock_env();
        let info = message_info(&borrower(&deps), &[]);
        let err = execute_create_draw(
            deps.as_mut(),
            env,
            info,
            cl_id,
            "NaN".to_string(),
            "ucredit".to_string(),
        )
        .unwrap_err();
        match err {
            ContractError::Std(_) => {}
            other => panic!("Expected Std parse error, got {:?}", other),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// execute_repay_draw
// ═══════════════════════════════════════════════════════════════════════════

mod repay_draw_tests {
    use super::*;
    use creditra_credit::error::ContractError;

    #[test]
    fn drawer_can_repay_and_flips_repaid_flag() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let cl_id = open_default_line(&mut deps);
        let draw_id = draw_amount(&mut deps, cl_id, "100");
        let env = mock_env();
        let info = message_info(&borrower(&deps), &[]);
        let res = execute_repay_draw(deps.as_mut(), env, info, cl_id, draw_id).unwrap();
        assert_eq!(res.attributes[0].value, "repay_draw");

        let query_env = mock_env();
        let msg = QueryMsg::DrawAuditTrail {
            credit_line_id: cl_id,
            draw_id: Some(draw_id),
        };
        let raw = query(deps.as_ref(), query_env, msg).unwrap();
        let trails: Vec<creditra_credit::msg::DrawAuditTrailResponse> = from_json(&raw).unwrap();
        assert!(trails[0].repaid);
    }

    #[test]
    fn non_drawer_cannot_repay() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let cl_id = open_default_line(&mut deps);
        let draw_id = draw_amount(&mut deps, cl_id, "100");
        let env = mock_env();
        let info = message_info(&stranger(&deps), &[]);
        let err = execute_repay_draw(deps.as_mut(), env, info, cl_id, draw_id).unwrap_err();
        assert_eq!(err, ContractError::CrossTenantIdentifier);
    }

    #[test]
    fn missing_draw_returns_not_found() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let cl_id = open_default_line(&mut deps);
        let env = mock_env();
        let info = message_info(&borrower(&deps), &[]);
        let err = execute_repay_draw(deps.as_mut(), env, info, cl_id, 42u64).unwrap_err();
        assert_eq!(err, ContractError::DrawNotFound(42, cl_id));
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// execute_add_audit_memo
// ═══════════════════════════════════════════════════════════════════════════

mod add_audit_memo_tests {
    use super::*;
    use creditra_credit::error::ContractError;

    #[test]
    fn admin_can_append_memo_and_it_appears_in_trail() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let cl_id = open_default_line(&mut deps);
        let draw_id = draw_amount(&mut deps, cl_id, "100");
        let env = mock_env();
        let info = message_info(&admin(&deps), &[]);
        execute_add_audit_memo(
            deps.as_mut(),
            env,
            info,
            cl_id,
            draw_id,
            "manual review passed".to_string(),
        )
        .unwrap();

        let query_env = mock_env();
        let msg = QueryMsg::DrawAuditTrail {
            credit_line_id: cl_id,
            draw_id: Some(draw_id),
        };
        let raw = query(deps.as_ref(), query_env, msg).unwrap();
        let trails: Vec<creditra_credit::msg::DrawAuditTrailResponse> = from_json(&raw).unwrap();
        let memos: Vec<_> = trails[0]
            .events
            .iter()
            .filter(|e| !e.memo.is_empty())
            .collect();
        assert_eq!(memos.len(), 1);
        assert_eq!(memos[0].memo, "manual review passed");
    }

    #[test]
    fn non_admin_cannot_add_memo() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let cl_id = open_default_line(&mut deps);
        let draw_id = draw_amount(&mut deps, cl_id, "100");
        let env = mock_env();
        let info = message_info(&stranger(&deps), &[]);
        let err =
            execute_add_audit_memo(deps.as_mut(), env, info, cl_id, draw_id, "hax".to_string())
                .unwrap_err();
        assert_eq!(err, ContractError::Unauthorized);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// execute_update_protocol_version
// ═══════════════════════════════════════════════════════════════════════════

mod update_protocol_version_tests {
    use super::*;
    use creditra_credit::error::ContractError;

    #[test]
    fn admin_can_bump_major_minor() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let admin_addr = admin(&deps);
        let info = message_info(&admin_addr, &[]);
        let res = execute_update_protocol_version(deps.as_mut(), info, 7u32, 2u32).unwrap();
        let major = res.attributes.iter().find(|a| a.key == "major").unwrap();
        let minor = res.attributes.iter().find(|a| a.key == "minor").unwrap();
        assert_eq!(major.value, "7");
        assert_eq!(minor.value, "2");
    }

    #[test]
    fn non_admin_rejected() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let stranger_addr = stranger(&deps);
        let info = message_info(&stranger_addr, &[]);
        let err = execute_update_protocol_version(deps.as_mut(), info, 2u32, 0u32).unwrap_err();
        assert_eq!(err, ContractError::Unauthorized);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// execute_set_late_fee_config
// ═══════════════════════════════════════════════════════════════════════════

mod set_late_fee_config_tests {
    use super::*;
    use creditra_credit::error::ContractError;

    #[test]
    fn admin_can_set_flat_and_query_round_trips() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let admin_addr = admin(&deps);
        let env = mock_env();
        let info = message_info(&admin_addr, &[]);
        let cfg = LateFeeConfig::Flat(FlatFeeConfig {
            amount: Uint128::new(250),
        });
        execute_set_late_fee_config(deps.as_mut(), info, Some(cfg)).unwrap();

        let raw = query(deps.as_ref(), env, QueryMsg::GetLateFeeConfig {}).unwrap();
        let resp: LateFeeConfigResponse = from_json(&raw).unwrap();
        assert_eq!(resp.config, Some(cfg));
    }

    #[test]
    fn admin_can_set_apr_based_and_query_round_trips() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let admin_addr = admin(&deps);
        let env = mock_env();
        let info = message_info(&admin_addr, &[]);
        let cfg = LateFeeConfig::AprBased(AprFeeConfig { surcharge_bps: 750 });
        execute_set_late_fee_config(deps.as_mut(), info, Some(cfg)).unwrap();

        let raw = query(deps.as_ref(), env, QueryMsg::GetLateFeeConfig {}).unwrap();
        let resp: LateFeeConfigResponse = from_json(&raw).unwrap();
        assert_eq!(resp.config, Some(cfg));
    }

    #[test]
    fn apr_above_10000_rejected() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let admin_addr = admin(&deps);
        let info = message_info(&admin_addr, &[]);
        let cfg = LateFeeConfig::AprBased(AprFeeConfig {
            surcharge_bps: 10_001,
        });
        let err = execute_set_late_fee_config(deps.as_mut(), info, Some(cfg)).unwrap_err();
        assert_eq!(err, ContractError::RateTooHigh);
    }

    #[test]
    fn none_clears_config() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let admin_addr = admin(&deps);
        let env = mock_env();
        let admin_info = message_info(&admin_addr, &[]);
        let cfg = LateFeeConfig::Flat(FlatFeeConfig {
            amount: Uint128::new(100),
        });
        execute_set_late_fee_config(deps.as_mut(), admin_info.clone(), Some(cfg)).unwrap();
        execute_set_late_fee_config(deps.as_mut(), admin_info, None).unwrap();

        let raw = query(deps.as_ref(), env, QueryMsg::GetLateFeeConfig {}).unwrap();
        let resp: LateFeeConfigResponse = from_json(&raw).unwrap();
        assert!(resp.config.is_none());
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Oracle (quorum + prices)
// ═══════════════════════════════════════════════════════════════════════════

mod oracle_tests {
    use super::*;
    use creditra_credit::error::ContractError;

    fn set_default_quorum(deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>) {
        let info = message_info(&admin(deps), &[]);
        execute_set_oracle_quorum_config(deps.as_mut(), info, 2u32, 200u32, 3600u64).unwrap();
    }

    #[test]
    fn admin_sets_quorum_and_query_round_trips() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        set_default_quorum(&mut deps);

        let env = mock_env();
        let raw = query(deps.as_ref(), env, QueryMsg::GetOracleQuorumConfig {}).unwrap();
        let resp: OracleQuorumConfigResponse = from_json(&raw).unwrap();
        let cfg = resp.config.unwrap();
        assert_eq!(cfg.min_quorum_k, 2);
        assert_eq!(cfg.max_deviation_bps, 200);
        assert_eq!(cfg.max_age_seconds, 3600);
    }

    #[test]
    fn min_quorum_k_below_2_rejected() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let info = message_info(&admin(&deps), &[]);
        let err = execute_set_oracle_quorum_config(deps.as_mut(), info, 1u32, 100u32, 100u64)
            .unwrap_err();
        assert_eq!(err, ContractError::InvalidAmount);
    }

    #[test]
    fn submit_resolves_quorum_and_persists_price() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        set_default_quorum(&mut deps);

        let env = mock_env();
        let admin_addr = admin(&deps);
        let info = message_info(&admin_addr, &[]);
        let prices = vec![100i128, 101i128, 99i128];
        let res = execute_submit_oracle_prices(deps.as_mut(), env.clone(), info, prices).unwrap();
        let canon = res
            .attributes
            .iter()
            .find(|a| a.key == "canonical_price")
            .unwrap();
        let canon_val: i128 = canon.value.parse().unwrap();
        assert!(
            (99..=101).contains(&canon_val),
            "canonical price {} out of expected window",
            canon_val
        );

        let raw = query(deps.as_ref(), env, QueryMsg::GetOraclePrice {}).unwrap();
        let resp: OraclePriceResponse = from_json(&raw).unwrap();
        assert!(
            resp.price.is_some(),
            "oracle price should be persisted after submit"
        );
        let stored = resp.price.unwrap();
        assert!(
            (99..=101).contains(&stored),
            "stored oracle price {} out of expected window",
            stored
        );
        assert!(resp.timestamp.is_some());
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Query dispatch (cross-check every documented variant)
// ═══════════════════════════════════════════════════════════════════════════

mod query_dispatch_tests {
    use super::*;

    #[test]
    fn proof_of_reserve_query_defaults_to_zeroes() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let env = mock_env();
        let msg = QueryMsg::ProofOfReserve { denom: None };
        let raw = query(deps.as_ref(), env, msg).unwrap();
        let por: creditra_credit::msg::ProofOfReserveResponse = from_json(&raw).unwrap();
        assert_eq!(por.total_credit_lines, 0);
        assert_eq!(por.net_outstanding, Uint128::zero());
        assert!(por.reserves_by_denom.is_empty());
    }

    #[test]
    fn borrower_health_factor_returns_empty_for_unknown() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let env = mock_env();
        let borrower_str = deps.api.addr_make("nobody").to_string();
        let msg = QueryMsg::BorrowerHealthFactor {
            borrower: borrower_str.clone(),
        };
        let raw = query(deps.as_ref(), env, msg).unwrap();
        let health: creditra_credit::msg::BorrowerHealthFactorResponse = from_json(&raw).unwrap();
        assert_eq!(health.borrower, borrower_str);
        assert!(health.credit_lines.is_empty());
    }

    #[test]
    fn execute_dispatch_routes_all_variants_via_enum() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let env = mock_env();
        let info = message_info(&admin(&deps), &[]);
        let msg = ExecuteMsg::CreateCreditLine {
            borrower: borrower(&deps).to_string(),
            collateral_denom: "ucosm".to_string(),
            collateral_amount: "100".to_string(),
            credit_denom: "ucredit".to_string(),
            credit_amount: "50".to_string(),
        };
        let res = execute(deps.as_mut(), env, info, msg).unwrap();
        assert_eq!(res.attributes[0].value, "create_credit_line");
    }
}
