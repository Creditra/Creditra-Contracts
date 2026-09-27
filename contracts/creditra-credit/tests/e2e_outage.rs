// SPDX-License-Identifier: MIT

//! # End-to-End Simulation of Oracle Price-Feed Outage and Recovery
//!
//! Simulates production oracle outage scenarios and recovery workflows for the
//! CosmWasm `creditra-credit` smart contract.
//!
//! ## Scenarios Tested
//!
//! 1. **Healthy Oracle Baseline**: Multi-oracle quorum configuration and price resolution.
//! 2. **Deviation Outage**: Price feeds diverge beyond `max_deviation_bps`, triggering `OracleQuorumNotMet`.
//! 3. **Insufficient Quorum Outage**: Fewer feeds submitted than `min_quorum_k`, triggering `OracleQuorumNotMet`.
//! 4. **Invalid Price Outage**: Non-positive prices (zero or negative) trigger `OraclePriceInvalid`.
//! 5. **Stale Price Outage**: Ledger timestamp advancing past `max_age_seconds` triggers staleness detection.
//! 6. **Authorization Enforcement**: Non-owner attempts to configure quorum or submit prices are rejected with `Unauthorized`.
//! 7. **State Isolation**: Existing credit lines, draws, audit entries, and queries remain stable during oracle outage.
//! 8. **Recovery Route A (Admin Parameter Adjustment)**: Admin widens `max_deviation_bps` to accommodate market volatility.
//! 9. **Recovery Route B (Oracle Feed Restoration)**: Feed providers restore synchronized price streams.
//! 10. **Recovery Route C (Stale Price Refresh)**: Fresh price submission clears staleness state.
//! 11. **Edge Cases**: Unconfigured quorum config, boundary feed counts, and invalid config parameters.

use cosmwasm_std::testing::{
    message_info, mock_dependencies, mock_env, MockApi, MockQuerier, MockStorage,
};
use cosmwasm_std::{from_json, Addr, OwnedDeps, Uint128};

use creditra_credit::contract;
use creditra_credit::error::ContractError;
use creditra_credit::msg::{
    ExecuteMsg, InstantiateMsg, OraclePriceResponse, OracleQuorumConfigResponse, QueryMsg,
};
use creditra_credit::oracles::is_price_stale;
use creditra_credit::state::{
    OraclePriceRecord, OracleQuorumConfig, MAX_ORACLE_FEEDS, ORACLE_PRICE_RECORD,
    ORACLE_QUORUM_CONFIG,
};

// ═══════════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════════

fn make_addr(deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>, label: &str) -> Addr {
    deps.api.addr_make(label)
}

fn setup_contract(deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>) -> Addr {
    let env = mock_env();
    let owner = make_addr(deps, "owner");
    let info = message_info(&owner, &[]);
    let msg = InstantiateMsg {
        owner: owner.to_string(),
    };
    contract::instantiate(deps.as_mut(), env, info, msg).unwrap();
    owner
}

fn set_quorum_config(
    deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
    sender: &Addr,
    k: u32,
    dev_bps: u32,
    max_age: u64,
) -> Result<(), ContractError> {
    let env = mock_env();
    let info = message_info(sender, &[]);
    let msg = ExecuteMsg::SetOracleQuorumConfig {
        min_quorum_k: k,
        max_deviation_bps: dev_bps,
        max_age_seconds: max_age,
    };
    contract::execute(deps.as_mut(), env, info, msg).map(|_| ())
}

fn submit_prices(
    deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
    sender: &Addr,
    prices: Vec<i128>,
    timestamp_sec: u64,
) -> Result<i128, ContractError> {
    let mut env = mock_env();
    env.block.time = cosmwasm_std::Timestamp::from_seconds(timestamp_sec);
    let info = message_info(sender, &[]);
    let msg = ExecuteMsg::SubmitOraclePrices { prices };

    let res = contract::execute(deps.as_mut(), env, info, msg)?;

    // Extract canonical price attribute
    let canonical = res
        .attributes
        .iter()
        .find(|attr| attr.key == "canonical_price")
        .map(|attr| attr.value.parse::<i128>().unwrap())
        .unwrap();

    Ok(canonical)
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn e2e_oracle_healthy_baseline() {
    let mut deps = mock_dependencies();
    let owner = setup_contract(&mut deps);

    // 1. Owner sets quorum configuration: K=3, max deviation 500 bps (5%), max age 3,600s
    set_quorum_config(&mut deps, &owner, 3, 500, 3600).expect("quorum config failed");

    // Query configuration to verify
    let q_res = contract::query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::GetOracleQuorumConfig {},
    )
    .unwrap();
    let cfg_resp: OracleQuorumConfigResponse = from_json(&q_res).unwrap();
    let cfg = cfg_resp.config.expect("config should exist");
    assert_eq!(cfg.min_quorum_k, 3);
    assert_eq!(cfg.max_deviation_bps, 500);
    assert_eq!(cfg.max_age_seconds, 3600);

    // 2. Submit 5 oracle prices: [1000, 1010, 1005, 1020, 2000]
    // Sorted: 1000, 1005, 1010, 1020, 2000
    // Window [1000, 1005, 1010]: spread = (1010 - 1000) / 1000 = 100 bps <= 500 bps -> qualifies!
    // Lower-median index = 0 + (3-1)/2 = 1 -> 1005
    let canonical = submit_prices(
        &mut deps,
        &owner,
        vec![1000, 1010, 1005, 1020, 2000],
        10_000,
    )
    .expect("price submission failed");
    assert_eq!(canonical, 1005);

    // 3. Query price record
    let p_res = contract::query(deps.as_ref(), mock_env(), QueryMsg::GetOraclePrice {}).unwrap();
    let price_resp: OraclePriceResponse = from_json(&p_res).unwrap();
    assert_eq!(price_resp.price, Some(1005));
    assert_eq!(price_resp.timestamp, Some(10_000));
}

#[test]
fn e2e_oracle_outage_quorum_deviation() {
    let mut deps = mock_dependencies();
    let owner = setup_contract(&mut deps);

    set_quorum_config(&mut deps, &owner, 3, 500, 3600).unwrap();

    // Initial valid submission
    submit_prices(&mut deps, &owner, vec![1000, 1010, 1005], 10_000).unwrap();

    // Market disruption: oracle feeds diverge widely: [1000, 1200, 1500, 2000]
    // 3-wide windows:
    // [1000, 1200, 1500] -> dev(1500, 1000) = 5000 bps > 500 -> fails
    // [1200, 1500, 2000] -> dev(2000, 1200) = 6667 bps > 500 -> fails
    let err = submit_prices(&mut deps, &owner, vec![1000, 1200, 1500, 2000], 10_100).unwrap_err();
    assert_eq!(err, ContractError::OracleQuorumNotMet);

    // Previous valid price record remains unmodified in storage
    let p_res = contract::query(deps.as_ref(), mock_env(), QueryMsg::GetOraclePrice {}).unwrap();
    let price_resp: OraclePriceResponse = from_json(&p_res).unwrap();
    assert_eq!(price_resp.price, Some(1005));
    assert_eq!(price_resp.timestamp, Some(10_000));
}

#[test]
fn e2e_oracle_outage_insufficient_feeds() {
    let mut deps = mock_dependencies();
    let owner = setup_contract(&mut deps);

    set_quorum_config(&mut deps, &owner, 3, 500, 3600).unwrap();

    // Submit only 2 prices when K=3
    let err = submit_prices(&mut deps, &owner, vec![1000, 1005], 10_000).unwrap_err();
    assert_eq!(err, ContractError::OracleQuorumNotMet);
}

#[test]
fn e2e_oracle_outage_invalid_prices() {
    let mut deps = mock_dependencies();
    let owner = setup_contract(&mut deps);

    set_quorum_config(&mut deps, &owner, 2, 500, 3600).unwrap();

    // Zero price feed
    let err_zero = submit_prices(&mut deps, &owner, vec![1000, 0], 10_000).unwrap_err();
    assert_eq!(err_zero, ContractError::OraclePriceInvalid);

    // Negative price feed
    let err_neg = submit_prices(&mut deps, &owner, vec![1000, -100], 10_000).unwrap_err();
    assert_eq!(err_neg, ContractError::OraclePriceInvalid);

    // Exceeding MAX_ORACLE_FEEDS (20)
    let too_many_prices = vec![1000i128; MAX_ORACLE_FEEDS + 1];
    let err_too_many = submit_prices(&mut deps, &owner, too_many_prices, 10_000).unwrap_err();
    assert_eq!(err_too_many, ContractError::OraclePriceInvalid);
}

#[test]
fn e2e_oracle_outage_stale_price() {
    let mut deps = mock_dependencies();
    let owner = setup_contract(&mut deps);

    set_quorum_config(&mut deps, &owner, 2, 500, 3600).unwrap();
    submit_prices(&mut deps, &owner, vec![1000, 1010], 10_000).unwrap();

    let record = OraclePriceRecord {
        price: 1005,
        timestamp: 10_000,
    };
    let qcfg = OracleQuorumConfig {
        min_quorum_k: 2,
        max_deviation_bps: 500,
        max_age_seconds: 3600,
    };

    // Within max age (1,000s after timestamp) -> Fresh
    assert!(!is_price_stale(&record, &qcfg, 11_000));

    // Beyond max age (5,000s after timestamp > 3,600s) -> Stale
    assert!(is_price_stale(&record, &qcfg, 15_000));
}

#[test]
fn e2e_oracle_outage_auth_enforcement() {
    let mut deps = mock_dependencies();
    let owner = setup_contract(&mut deps);
    let attacker = make_addr(&deps, "attacker");

    // Non-owner trying to set quorum config -> Unauthorized
    let err_cfg = set_quorum_config(&mut deps, &attacker, 2, 500, 3600).unwrap_err();
    assert_eq!(err_cfg, ContractError::Unauthorized);

    // Owner sets config
    set_quorum_config(&mut deps, &owner, 2, 500, 3600).unwrap();

    // Non-owner trying to submit oracle prices -> Unauthorized
    let err_sub = submit_prices(&mut deps, &attacker, vec![1000, 1005], 10_000).unwrap_err();
    assert_eq!(err_sub, ContractError::Unauthorized);
}

#[test]
fn e2e_oracle_outage_state_isolation() {
    let mut deps = mock_dependencies();
    let owner = setup_contract(&mut deps);
    let borrower = make_addr(&deps, "borrower");

    // Setup credit line and draw prior to oracle outage
    let env = mock_env();
    let info = message_info(&owner, &[]);
    let msg_cl = ExecuteMsg::CreateCreditLine {
        borrower: borrower.to_string(),
        collateral_denom: "ucollateral".to_string(),
        collateral_amount: "2000".to_string(),
        credit_denom: "ucredit".to_string(),
        credit_amount: "1000".to_string(),
    };
    contract::execute(deps.as_mut(), env, info, msg_cl).unwrap();

    let info_b = message_info(&borrower, &[]);
    let msg_draw = ExecuteMsg::CreateDraw {
        credit_line_id: 0,
        amount: "300".to_string(),
        denom: "ucredit".to_string(),
    };
    contract::execute(deps.as_mut(), mock_env(), info_b, msg_draw).unwrap();

    // Trigger an oracle outage via quorum deviation failure
    set_quorum_config(&mut deps, &owner, 3, 500, 3600).unwrap();
    let outage_err = submit_prices(&mut deps, &owner, vec![1000, 2000, 3000], 10_000).unwrap_err();
    assert_eq!(outage_err, ContractError::OracleQuorumNotMet);

    // Verify existing credit line state, proof of reserves, and health factor remain completely intact
    let por_binary = contract::query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::ProofOfReserve { denom: None },
    )
    .unwrap();
    let por: creditra_credit::msg::ProofOfReserveResponse = from_json(&por_binary).unwrap();
    assert_eq!(por.total_credit_lines, 1);
    assert_eq!(por.total_drawn, Uint128::new(300));

    let hf_binary = contract::query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::BorrowerHealthFactor {
            borrower: borrower.to_string(),
        },
    )
    .unwrap();
    let hf: creditra_credit::msg::BorrowerHealthFactorResponse = from_json(&hf_binary).unwrap();
    assert_eq!(hf.credit_lines.len(), 1);
    assert_eq!(hf.credit_lines[0].utilized_amount, Uint128::new(300));
}

#[test]
fn e2e_oracle_outage_recovery_admin_reconfig() {
    let mut deps = mock_dependencies();
    let owner = setup_contract(&mut deps);

    // 1. Initial config: 5% deviation limit
    set_quorum_config(&mut deps, &owner, 3, 500, 3600).unwrap();

    // 2. Oracle feeds diverge due to high market volatility (15% spread): [1000, 1100, 1150]
    // Spread: (1150 - 1000) / 1000 = 1500 bps (15%) > 500 bps -> Outage!
    let outage_err = submit_prices(&mut deps, &owner, vec![1000, 1100, 1150], 10_000).unwrap_err();
    assert_eq!(outage_err, ContractError::OracleQuorumNotMet);

    // 3. Admin Recovery Route A: Admin widens max_deviation_bps from 500 (5%) to 2000 (20%)
    set_quorum_config(&mut deps, &owner, 3, 2000, 3600).expect("admin reconfig failed");

    // 4. Re-submitting prices now succeeds under widened parameters!
    let canonical = submit_prices(&mut deps, &owner, vec![1000, 1100, 1150], 10_050)
        .expect("recovery submission failed");
    assert_eq!(canonical, 1100); // lower median of [1000, 1100, 1150]

    let p_res = contract::query(deps.as_ref(), mock_env(), QueryMsg::GetOraclePrice {}).unwrap();
    let price_resp: OraclePriceResponse = from_json(&p_res).unwrap();
    assert_eq!(price_resp.price, Some(1100));
    assert_eq!(price_resp.timestamp, Some(10_050));
}

#[test]
fn e2e_oracle_outage_recovery_feed_restoration() {
    let mut deps = mock_dependencies();
    let owner = setup_contract(&mut deps);

    set_quorum_config(&mut deps, &owner, 3, 500, 3600).unwrap();

    // Outage: Feeds corrupted / offline -> [1000, 5000, 9000]
    let outage_err = submit_prices(&mut deps, &owner, vec![1000, 5000, 9000], 10_000).unwrap_err();
    assert_eq!(outage_err, ContractError::OracleQuorumNotMet);

    // Recovery Route B: Feed providers recover and report synchronized prices: [1040, 1045, 1050]
    // Window [1040, 1045, 1050]: dev(1050, 1040) = 96 bps <= 500 bps -> Restored!
    let canonical = submit_prices(&mut deps, &owner, vec![1040, 1045, 1050], 10_200)
        .expect("feed restoration submission failed");
    assert_eq!(canonical, 1045);

    let p_res = contract::query(deps.as_ref(), mock_env(), QueryMsg::GetOraclePrice {}).unwrap();
    let price_resp: OraclePriceResponse = from_json(&p_res).unwrap();
    assert_eq!(price_resp.price, Some(1045));
    assert_eq!(price_resp.timestamp, Some(10_200));
}

#[test]
fn e2e_oracle_outage_recovery_stale_refresh() {
    let mut deps = mock_dependencies();
    let owner = setup_contract(&mut deps);

    set_quorum_config(&mut deps, &owner, 2, 500, 3600).unwrap();

    // Initial submission at t=10,000
    submit_prices(&mut deps, &owner, vec![1000, 1010], 10_000).unwrap();

    // At t=15,000 (> 3600s elapsed), price is stale
    let record = ORACLE_PRICE_RECORD.load(deps.as_ref().storage).unwrap();
    let qcfg = ORACLE_QUORUM_CONFIG.load(deps.as_ref().storage).unwrap();
    assert!(is_price_stale(&record, &qcfg, 15_000));

    // Recovery Route C: Feed operator submits fresh prices at t=15,000
    let fresh_canonical = submit_prices(&mut deps, &owner, vec![1020, 1025], 15_000).unwrap();
    assert_eq!(fresh_canonical, 1020);

    // Verify staleness check passes now at t=15,000
    let updated_record = ORACLE_PRICE_RECORD.load(deps.as_ref().storage).unwrap();
    assert!(!is_price_stale(&updated_record, &qcfg, 15_000));
}

#[test]
fn e2e_oracle_outage_unconfigured_quorum() {
    let mut deps = mock_dependencies();
    let owner = setup_contract(&mut deps);

    // Attempt to submit prices without setting quorum config first
    let err = submit_prices(&mut deps, &owner, vec![1000, 1010], 10_000).unwrap_err();
    assert_eq!(err, ContractError::OraclePriceInvalid);
}

#[test]
fn e2e_oracle_outage_invalid_config_parameters() {
    let mut deps = mock_dependencies();
    let owner = setup_contract(&mut deps);

    // K < 2 -> InvalidAmount
    let err_k = set_quorum_config(&mut deps, &owner, 1, 500, 3600).unwrap_err();
    assert_eq!(err_k, ContractError::InvalidAmount);

    // max_deviation_bps > 10,000 -> InvalidAmount
    let err_dev = set_quorum_config(&mut deps, &owner, 2, 10_001, 3600).unwrap_err();
    assert_eq!(err_dev, ContractError::InvalidAmount);

    // max_age_seconds == 0 -> InvalidAmount
    let err_age = set_quorum_config(&mut deps, &owner, 2, 500, 0).unwrap_err();
    assert_eq!(err_age, ContractError::InvalidAmount);
}
