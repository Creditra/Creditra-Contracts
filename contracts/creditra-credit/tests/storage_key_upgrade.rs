// SPDX-License-Identifier: MIT

//! # Storage Key Audit & Upgrade Compatibility Test Suite
//!
//! Exhaustive verification of storage key namespaces, types, collision
//! resistance, upgrade compatibility fixtures, and migration lifecycle
//! across both empty and populated states.

use cosmwasm_std::{
    testing::{message_info, mock_dependencies, mock_env, MockApi, MockQuerier, MockStorage},
    to_json_binary, Addr, OwnedDeps, StdError, Storage, Timestamp, Uint128,
};
use std::collections::HashSet;

use creditra_credit::contract::{execute, instantiate, migrate, query};
use creditra_credit::key::{
    check_new_namespace_collision, validate_storage_key_catalog, StorageType,
    ALL_STORAGE_KEY_FAMILIES,
};
use creditra_credit::msg::{
    CreditLineSnapshotResponse, DrawAuditTrailResponse, ExecuteMsg, InstantiateMsg,
    LateFeeConfigResponse, MigrateMsg, OraclePriceResponse, ProofOfReserveResponse, QueryMsg,
};
use creditra_credit::penalties::{FlatFeeConfig, LateFeeConfig};
use creditra_credit::state::{
    Config, CreditLine, Draw, DrawAction, DrawAuditEntry, OraclePriceRecord, OracleQuorumConfig,
    OracleReportData, BORROWER_COLLATERAL_TOKENS, BORROWER_TO_ID, BOUNTY_BALANCE,
    COLLATERAL_BALANCES, COLLATERAL_RISK_WEIGHTS, COLLATERAL_TOKEN_ALLOWLIST, CONFIG, CREDIT_LINES,
    CREDIT_LINE_COUNT, DEFAULT_FEE_SHARE_BPS, DRAWS, DRAW_AUDIT, DRAW_AUDIT_COUNT, DRAW_COUNT,
    LATE_FEE_CONFIG, MARKET_FEE_SHARE_BPS, ORACLE_LIST, ORACLE_PRICE_RECORD, ORACLE_QUORUM_CONFIG,
    ORACLE_REPORT, ORACLE_WEIGHT, TREASURY_BALANCE,
};

/// Helper to generate test addresses.
fn make_addr(deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>, label: &str) -> Addr {
    deps.api.addr_make(label)
}

/// Helper to initialize a contract with a creator address.
fn setup_contract(deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>) -> Addr {
    let env = mock_env();
    let creator = make_addr(deps, "admin_owner");
    let info = message_info(&creator, &[]);
    let msg = InstantiateMsg {
        owner: creator.to_string(),
    };
    instantiate(deps.as_mut(), env, info, msg).unwrap();
    creator
}

/// Helper to compute exact binary storage keys from state definitions.
fn extract_raw_storage_key<F>(action: F) -> Vec<u8>
where
    F: FnOnce(&mut MockStorage),
{
    let mut storage = MockStorage::new();
    action(&mut storage);
    let (key, _) = storage
        .range(None, None, cosmwasm_std::Order::Ascending)
        .next()
        .expect("No storage entry created");
    key
}

#[test]
fn test_storage_key_inventory_completeness_and_documentation() {
    assert!(validate_storage_key_catalog().is_ok());
    assert_eq!(ALL_STORAGE_KEY_FAMILIES.len(), 22);

    let mut namespaces = HashSet::new();
    for family in ALL_STORAGE_KEY_FAMILIES.iter() {
        assert!(!family.namespace.is_empty());
        assert!(!family.key_type_name.is_empty());
        assert!(!family.value_type_name.is_empty());
        assert!(!family.description.is_empty());
        assert!(namespaces.insert(family.namespace));
    }

    let expected_namespaces = [
        "config",
        "clc",
        "cl",
        "dcnt",
        "dr",
        "dacnt",
        "da",
        "bid",
        "orc_qcfg",
        "orc_prc",
        "orc_lst",
        "orc_w",
        "orc_rpt",
        "lfc",
        "bct",
        "cb",
        "crw",
        "cta",
        "default_fee_share",
        "mkt_fee_share",
        "treasury_bal",
        "bounty_bal",
    ];

    for expected in expected_namespaces {
        assert!(
            namespaces.contains(expected),
            "Missing expected namespace: {}",
            expected
        );
    }
}

#[test]
fn test_item_and_map_storage_prefixes_are_mutually_disjoint() {
    for f1 in ALL_STORAGE_KEY_FAMILIES.iter() {
        let p1 = f1.raw_prefix();
        for f2 in ALL_STORAGE_KEY_FAMILIES.iter() {
            if f1.namespace == f2.namespace {
                continue;
            }
            let p2 = f2.raw_prefix();
            assert_ne!(
                p1, p2,
                "Prefix collision between {} and {}",
                f1.namespace, f2.namespace
            );

            if f1.storage_type == StorageType::Item && f2.storage_type == StorageType::Map {
                assert!(
                    !p1.starts_with(&p2) && !p2.starts_with(&p1),
                    "Prefix overlap between Item {} and Map {}",
                    f1.namespace,
                    f2.namespace
                );
            }
        }
    }
}

#[test]
fn test_cross_namespace_same_key_isolation() {
    let mut deps = mock_dependencies();
    let sample_addr = Addr::unchecked("cosmos1sampleaddr00000000000000000000000000");
    let sample_denom = "ucredit";
    let sample_id = 42u64;

    BORROWER_TO_ID
        .save(&mut deps.storage, sample_addr.clone(), &100)
        .unwrap();
    ORACLE_WEIGHT
        .save(&mut deps.storage, sample_addr.clone(), &5000)
        .unwrap();
    ORACLE_REPORT
        .save(
            &mut deps.storage,
            sample_addr.clone(),
            &OracleReportData {
                value: 123456,
                timestamp: 9999,
            },
        )
        .unwrap();
    BORROWER_COLLATERAL_TOKENS
        .save(
            &mut deps.storage,
            &sample_addr,
            &vec![sample_denom.to_string()],
        )
        .unwrap();
    COLLATERAL_BALANCES
        .save(
            &mut deps.storage,
            (&sample_addr, sample_denom),
            &Uint128::new(777),
        )
        .unwrap();

    CREDIT_LINES
        .save(
            &mut deps.storage,
            sample_id,
            &CreditLine {
                id: sample_id,
                borrower: sample_addr.clone(),
                collateral_denom: "ucollateral".to_string(),
                collateral_amount: Uint128::new(1000),
                credit_denom: sample_denom.to_string(),
                credit_amount: Uint128::new(500),
                active: true,
            },
        )
        .unwrap();
    DRAW_COUNT.save(&mut deps.storage, sample_id, &1).unwrap();
    DRAWS
        .save(
            &mut deps.storage,
            (sample_id, 0),
            &Draw {
                id: 0,
                credit_line_id: sample_id,
                amount: Uint128::new(200),
                denom: sample_denom.to_string(),
                drawn_at: Timestamp::from_seconds(150),
                drawn_by: sample_addr.clone(),
                repaid: false,
            },
        )
        .unwrap();
    DRAW_AUDIT_COUNT
        .save(&mut deps.storage, (sample_id, 0), &1)
        .unwrap();
    DRAW_AUDIT
        .save(
            &mut deps.storage,
            (sample_id, 0, 0),
            &DrawAuditEntry {
                seq: 0,
                draw_id: 0,
                credit_line_id: sample_id,
                action: DrawAction::DrawCreated,
                timestamp: Timestamp::from_seconds(150),
                block_height: 10,
                by: sample_addr.clone(),
                memo: "initial draw".to_string(),
            },
        )
        .unwrap();

    COLLATERAL_RISK_WEIGHTS
        .save(&mut deps.storage, sample_denom, &8500)
        .unwrap();
    MARKET_FEE_SHARE_BPS
        .save(&mut deps.storage, sample_denom, &1500)
        .unwrap();
    TREASURY_BALANCE
        .save(&mut deps.storage, sample_denom, &Uint128::new(300))
        .unwrap();
    BOUNTY_BALANCE
        .save(&mut deps.storage, sample_denom, &Uint128::new(100))
        .unwrap();

    assert_eq!(
        BORROWER_TO_ID
            .load(&deps.storage, sample_addr.clone())
            .unwrap(),
        100
    );
    assert_eq!(
        ORACLE_WEIGHT
            .load(&deps.storage, sample_addr.clone())
            .unwrap(),
        5000
    );
    assert_eq!(
        ORACLE_REPORT
            .load(&deps.storage, sample_addr.clone())
            .unwrap()
            .value,
        123456
    );
    assert_eq!(
        COLLATERAL_BALANCES
            .load(&deps.storage, (&sample_addr, sample_denom))
            .unwrap(),
        Uint128::new(777)
    );
    assert_eq!(
        CREDIT_LINES
            .load(&deps.storage, sample_id)
            .unwrap()
            .borrower,
        sample_addr
    );
    assert_eq!(DRAW_COUNT.load(&deps.storage, sample_id).unwrap(), 1);
    assert_eq!(
        DRAWS.load(&deps.storage, (sample_id, 0)).unwrap().amount,
        Uint128::new(200)
    );
    assert_eq!(
        COLLATERAL_RISK_WEIGHTS
            .load(&deps.storage, sample_denom)
            .unwrap(),
        8500
    );
    assert_eq!(
        MARKET_FEE_SHARE_BPS
            .load(&deps.storage, sample_denom)
            .unwrap(),
        1500
    );
    assert_eq!(
        TREASURY_BALANCE.load(&deps.storage, sample_denom).unwrap(),
        Uint128::new(300)
    );
    assert_eq!(
        BOUNTY_BALANCE.load(&deps.storage, sample_denom).unwrap(),
        Uint128::new(100)
    );
}

#[test]
fn test_new_namespace_collision_detection() {
    assert!(check_new_namespace_collision("config", StorageType::Item).is_err());
    assert!(check_new_namespace_collision("cl", StorageType::Map).is_err());
    assert!(check_new_namespace_collision("dr", StorageType::Map).is_err());
    assert!(check_new_namespace_collision("treasury_bal", StorageType::Map).is_err());
    assert!(check_new_namespace_collision("", StorageType::Item).is_err());

    assert!(check_new_namespace_collision("new_timelock", StorageType::Item).is_ok());
    assert!(check_new_namespace_collision("new_liquidity_pool", StorageType::Map).is_ok());
}

#[test]
fn test_upgrade_fixture_empty_state_loads_without_data_loss() {
    let mut deps = mock_dependencies();
    let admin = make_addr(&deps, "admin_owner");

    let config_val = to_json_binary(&Config {
        owner: admin.clone(),
    })
    .unwrap();
    deps.storage.set(b"config", config_val.as_slice());

    let loaded_config = CONFIG.load(&deps.storage).unwrap();
    assert_eq!(loaded_config.owner, admin);

    let empty_clc = CREDIT_LINE_COUNT.may_load(&deps.storage).unwrap();
    assert_eq!(empty_clc, None);

    let env = mock_env();
    let res = migrate(deps.as_mut(), env.clone(), MigrateMsg {}).unwrap();
    assert_eq!(res.messages.len(), 0);

    let post_config = CONFIG.load(&deps.storage).unwrap();
    assert_eq!(post_config.owner, admin);

    let reserve_res = query(deps.as_ref(), env, QueryMsg::ProofOfReserve { denom: None }).unwrap();
    let reserve: ProofOfReserveResponse = cosmwasm_std::from_json(reserve_res).unwrap();
    assert_eq!(reserve.active_credit_lines, 0);
    assert_eq!(reserve.total_credit_limit, Uint128::zero());
    assert_eq!(reserve.total_drawn, Uint128::zero());
}

#[test]
fn test_upgrade_fixture_populated_state_loads_without_data_loss() {
    let mut deps = mock_dependencies();
    let admin = make_addr(&deps, "admin_owner");
    let borrower_a = make_addr(&deps, "borrower_alpha");
    let borrower_b = make_addr(&deps, "borrower_beta");
    let oracle_1 = make_addr(&deps, "oracle_primary");
    let oracle_2 = make_addr(&deps, "oracle_secondary");

    let mut fixture_entries: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();

    let config_bytes = to_json_binary(&Config {
        owner: admin.clone(),
    })
    .unwrap();
    fixture_entries.push((b"config".to_vec(), config_bytes.to_vec()));

    let clc_bytes = to_json_binary(&2u64).unwrap();
    fixture_entries.push((b"clc".to_vec(), clc_bytes.to_vec()));

    let cl0 = CreditLine {
        id: 0,
        borrower: borrower_a.clone(),
        collateral_denom: "ucollateral".to_string(),
        collateral_amount: Uint128::new(2000),
        credit_denom: "ucredit".to_string(),
        credit_amount: Uint128::new(1000),
        active: true,
    };
    let k_cl0 = extract_raw_storage_key(|s| CREDIT_LINES.save(s, 0, &cl0).unwrap());
    fixture_entries.push((k_cl0, to_json_binary(&cl0).unwrap().to_vec()));

    let cl1 = CreditLine {
        id: 1,
        borrower: borrower_b.clone(),
        collateral_denom: "ucollateral".to_string(),
        collateral_amount: Uint128::new(5000),
        credit_denom: "ucredit".to_string(),
        credit_amount: Uint128::new(2500),
        active: true,
    };
    let k_cl1 = extract_raw_storage_key(|s| CREDIT_LINES.save(s, 1, &cl1).unwrap());
    fixture_entries.push((k_cl1, to_json_binary(&cl1).unwrap().to_vec()));

    let k_bid_a =
        extract_raw_storage_key(|s| BORROWER_TO_ID.save(s, borrower_a.clone(), &0).unwrap());
    fixture_entries.push((k_bid_a, to_json_binary(&0u64).unwrap().to_vec()));

    let k_bid_b =
        extract_raw_storage_key(|s| BORROWER_TO_ID.save(s, borrower_b.clone(), &1).unwrap());
    fixture_entries.push((k_bid_b, to_json_binary(&1u64).unwrap().to_vec()));

    let k_dcnt0 = extract_raw_storage_key(|s| DRAW_COUNT.save(s, 0, &2).unwrap());
    fixture_entries.push((k_dcnt0, to_json_binary(&2u64).unwrap().to_vec()));

    let k_dcnt1 = extract_raw_storage_key(|s| DRAW_COUNT.save(s, 1, &1).unwrap());
    fixture_entries.push((k_dcnt1, to_json_binary(&1u64).unwrap().to_vec()));

    let draw0_0 = Draw {
        id: 0,
        credit_line_id: 0,
        amount: Uint128::new(400),
        denom: "ucredit".to_string(),
        drawn_at: Timestamp::from_seconds(1050),
        drawn_by: borrower_a.clone(),
        repaid: false,
    };
    let k_d0_0 = extract_raw_storage_key(|s| DRAWS.save(s, (0, 0), &draw0_0).unwrap());
    fixture_entries.push((k_d0_0, to_json_binary(&draw0_0).unwrap().to_vec()));

    let draw0_1 = Draw {
        id: 1,
        credit_line_id: 0,
        amount: Uint128::new(200),
        denom: "ucredit".to_string(),
        drawn_at: Timestamp::from_seconds(1080),
        drawn_by: borrower_a.clone(),
        repaid: true,
    };
    let k_d0_1 = extract_raw_storage_key(|s| DRAWS.save(s, (0, 1), &draw0_1).unwrap());
    fixture_entries.push((k_d0_1, to_json_binary(&draw0_1).unwrap().to_vec()));

    let draw1_0 = Draw {
        id: 0,
        credit_line_id: 1,
        amount: Uint128::new(1000),
        denom: "ucredit".to_string(),
        drawn_at: Timestamp::from_seconds(1150),
        drawn_by: borrower_b.clone(),
        repaid: false,
    };
    let k_d1_0 = extract_raw_storage_key(|s| DRAWS.save(s, (1, 0), &draw1_0).unwrap());
    fixture_entries.push((k_d1_0, to_json_binary(&draw1_0).unwrap().to_vec()));

    let qcfg = OracleQuorumConfig {
        min_quorum_k: 2,
        max_deviation_bps: 500,
        max_age_seconds: 3600,
    };
    fixture_entries.push((
        b"orc_qcfg".to_vec(),
        to_json_binary(&qcfg).unwrap().to_vec(),
    ));

    let prc = OraclePriceRecord {
        price: 1_250_000,
        timestamp: 1200,
    };
    fixture_entries.push((b"orc_prc".to_vec(), to_json_binary(&prc).unwrap().to_vec()));

    let orc_list = vec![oracle_1.clone(), oracle_2.clone()];
    fixture_entries.push((
        b"orc_lst".to_vec(),
        to_json_binary(&orc_list).unwrap().to_vec(),
    ));

    let k_ow1 =
        extract_raw_storage_key(|s| ORACLE_WEIGHT.save(s, oracle_1.clone(), &5000).unwrap());
    fixture_entries.push((k_ow1, to_json_binary(&5000u32).unwrap().to_vec()));

    let k_ow2 =
        extract_raw_storage_key(|s| ORACLE_WEIGHT.save(s, oracle_2.clone(), &5000).unwrap());
    fixture_entries.push((k_ow2, to_json_binary(&5000u32).unwrap().to_vec()));

    let lfc = LateFeeConfig::Flat(FlatFeeConfig {
        amount: Uint128::new(75),
    });
    fixture_entries.push((b"lfc".to_vec(), to_json_binary(&lfc).unwrap().to_vec()));

    let allowlist = vec!["ucollateral".to_string(), "usecondary".to_string()];
    fixture_entries.push((
        b"cta".to_vec(),
        to_json_binary(&allowlist).unwrap().to_vec(),
    ));

    let k_crw1 = extract_raw_storage_key(|s| {
        COLLATERAL_RISK_WEIGHTS
            .save(s, "ucollateral", &10_000)
            .unwrap()
    });
    fixture_entries.push((k_crw1, to_json_binary(&10_000u32).unwrap().to_vec()));

    let k_crw2 = extract_raw_storage_key(|s| {
        COLLATERAL_RISK_WEIGHTS
            .save(s, "usecondary", &8_000)
            .unwrap()
    });
    fixture_entries.push((k_crw2, to_json_binary(&8_000u32).unwrap().to_vec()));

    fixture_entries.push((
        b"default_fee_share".to_vec(),
        to_json_binary(&2500u32).unwrap().to_vec(),
    ));

    let k_mkt =
        extract_raw_storage_key(|s| MARKET_FEE_SHARE_BPS.save(s, "ucredit", &3000).unwrap());
    fixture_entries.push((k_mkt, to_json_binary(&3000u32).unwrap().to_vec()));

    let k_tb = extract_raw_storage_key(|s| {
        TREASURY_BALANCE
            .save(s, "ucredit", &Uint128::new(450))
            .unwrap()
    });
    fixture_entries.push((k_tb, to_json_binary(&Uint128::new(450)).unwrap().to_vec()));

    let k_bb = extract_raw_storage_key(|s| {
        BOUNTY_BALANCE
            .save(s, "ucredit", &Uint128::new(150))
            .unwrap()
    });
    fixture_entries.push((k_bb, to_json_binary(&Uint128::new(150)).unwrap().to_vec()));

    for (k, v) in fixture_entries.iter() {
        deps.storage.set(k, v);
    }

    assert_eq!(CONFIG.load(&deps.storage).unwrap().owner, admin);
    assert_eq!(CREDIT_LINE_COUNT.load(&deps.storage).unwrap(), 2);
    assert_eq!(CREDIT_LINES.load(&deps.storage, 0).unwrap(), cl0);
    assert_eq!(CREDIT_LINES.load(&deps.storage, 1).unwrap(), cl1);
    assert_eq!(
        BORROWER_TO_ID
            .load(&deps.storage, borrower_a.clone())
            .unwrap(),
        0
    );
    assert_eq!(
        BORROWER_TO_ID
            .load(&deps.storage, borrower_b.clone())
            .unwrap(),
        1
    );
    assert_eq!(DRAW_COUNT.load(&deps.storage, 0).unwrap(), 2);
    assert_eq!(DRAW_COUNT.load(&deps.storage, 1).unwrap(), 1);
    assert_eq!(DRAWS.load(&deps.storage, (0, 0)).unwrap(), draw0_0);
    assert_eq!(DRAWS.load(&deps.storage, (0, 1)).unwrap(), draw0_1);
    assert_eq!(DRAWS.load(&deps.storage, (1, 0)).unwrap(), draw1_0);
    assert_eq!(ORACLE_QUORUM_CONFIG.load(&deps.storage).unwrap(), qcfg);
    assert_eq!(ORACLE_PRICE_RECORD.load(&deps.storage).unwrap(), prc);
    assert_eq!(ORACLE_LIST.load(&deps.storage).unwrap(), orc_list);
    assert_eq!(
        ORACLE_WEIGHT.load(&deps.storage, oracle_1.clone()).unwrap(),
        5000
    );
    assert_eq!(LATE_FEE_CONFIG.load(&deps.storage).unwrap(), lfc);
    assert_eq!(
        COLLATERAL_TOKEN_ALLOWLIST.load(&deps.storage).unwrap(),
        allowlist
    );
    assert_eq!(
        COLLATERAL_RISK_WEIGHTS
            .load(&deps.storage, "ucollateral")
            .unwrap(),
        10_000
    );
    assert_eq!(DEFAULT_FEE_SHARE_BPS.load(&deps.storage).unwrap(), 2500);
    assert_eq!(
        MARKET_FEE_SHARE_BPS.load(&deps.storage, "ucredit").unwrap(),
        3000
    );
    assert_eq!(
        TREASURY_BALANCE.load(&deps.storage, "ucredit").unwrap(),
        Uint128::new(450)
    );
    assert_eq!(
        BOUNTY_BALANCE.load(&deps.storage, "ucredit").unwrap(),
        Uint128::new(150)
    );

    let env = mock_env();
    let q_snap_res = query(
        deps.as_ref(),
        env.clone(),
        QueryMsg::CreditLineSnapshot { credit_line_id: 0 },
    )
    .unwrap();
    let snap_opt: Option<CreditLineSnapshotResponse> = cosmwasm_std::from_json(q_snap_res).unwrap();
    assert!(snap_opt.is_some());
    let snap = snap_opt.unwrap();
    assert_eq!(snap.credit_line_id, 0);
    assert_eq!(snap.borrower, borrower_a);
    assert_eq!(snap.credit_amount, Uint128::new(1000));
    assert_eq!(snap.total_utilized, Uint128::new(400));
    assert_eq!(snap.draws.len(), 2);

    let q_late_res = query(deps.as_ref(), env.clone(), QueryMsg::GetLateFeeConfig {}).unwrap();
    let late_resp: LateFeeConfigResponse = cosmwasm_std::from_json(q_late_res).unwrap();
    assert_eq!(late_resp.config, Some(lfc));

    let q_oracle_res = query(deps.as_ref(), env.clone(), QueryMsg::GetOraclePrice {}).unwrap();
    let oracle_resp: OraclePriceResponse = cosmwasm_std::from_json(q_oracle_res).unwrap();
    assert_eq!(oracle_resp.price, Some(1_250_000));

    let q_reserve_res = query(
        deps.as_ref(),
        env.clone(),
        QueryMsg::ProofOfReserve { denom: None },
    )
    .unwrap();
    let reserve_resp: ProofOfReserveResponse = cosmwasm_std::from_json(q_reserve_res).unwrap();
    assert_eq!(reserve_resp.active_credit_lines, 2);
    assert_eq!(reserve_resp.total_credit_limit, Uint128::new(3500));
    assert_eq!(reserve_resp.total_drawn, Uint128::new(1400));
    assert_eq!(reserve_resp.total_repaid, Uint128::new(200));
    assert_eq!(reserve_resp.net_outstanding, Uint128::new(1400));

    let mig_res = migrate(deps.as_mut(), env, MigrateMsg {}).unwrap();
    assert_eq!(mig_res.messages.len(), 0);

    assert_eq!(CONFIG.load(&deps.storage).unwrap().owner, admin);
    assert_eq!(CREDIT_LINE_COUNT.load(&deps.storage).unwrap(), 2);
    assert_eq!(CREDIT_LINES.load(&deps.storage, 0).unwrap(), cl0);
    assert_eq!(CREDIT_LINES.load(&deps.storage, 1).unwrap(), cl1);
    assert_eq!(DRAWS.load(&deps.storage, (0, 0)).unwrap(), draw0_0);
    assert_eq!(DRAWS.load(&deps.storage, (0, 1)).unwrap(), draw0_1);
    assert_eq!(DRAWS.load(&deps.storage, (1, 0)).unwrap(), draw1_0);
    assert_eq!(
        TREASURY_BALANCE.load(&deps.storage, "ucredit").unwrap(),
        Uint128::new(450)
    );
    assert_eq!(
        BOUNTY_BALANCE.load(&deps.storage, "ucredit").unwrap(),
        Uint128::new(150)
    );
}

#[test]
fn test_migration_empty_state_lifecycle() {
    let mut deps = mock_dependencies();
    let admin = setup_contract(&mut deps);
    let env = mock_env();

    let pre_clc = CREDIT_LINE_COUNT.load(&deps.storage).unwrap();
    assert_eq!(pre_clc, 0);

    let res = migrate(deps.as_mut(), env.clone(), MigrateMsg {}).unwrap();
    assert_eq!(res.messages.len(), 0);

    let post_clc = CREDIT_LINE_COUNT.load(&deps.storage).unwrap();
    assert_eq!(post_clc, 0);
    assert_eq!(CONFIG.load(&deps.storage).unwrap().owner, admin);

    let borrower = make_addr(&deps, "post_mig_borrower");
    let create_msg = ExecuteMsg::CreateCreditLine {
        borrower: borrower.to_string(),
        collateral_denom: "ucollateral".to_string(),
        collateral_amount: "1000".to_string(),
        credit_denom: "ucredit".to_string(),
        credit_amount: "500".to_string(),
    };
    let info = message_info(&admin, &[]);
    execute(deps.as_mut(), env, info, create_msg).unwrap();

    assert_eq!(CREDIT_LINE_COUNT.load(&deps.storage).unwrap(), 1);
    assert_eq!(
        BORROWER_TO_ID
            .load(&deps.storage, borrower.clone())
            .unwrap(),
        0
    );
}

#[test]
fn test_migration_populated_state_lifecycle_and_state_preservation() {
    let mut deps = mock_dependencies();
    let admin = setup_contract(&mut deps);
    let env = mock_env();

    let borrower_1 = make_addr(&deps, "borrower_one");
    let borrower_2 = make_addr(&deps, "borrower_two");

    let info_admin = message_info(&admin, &[]);
    execute(
        deps.as_mut(),
        env.clone(),
        info_admin.clone(),
        ExecuteMsg::CreateCreditLine {
            borrower: borrower_1.to_string(),
            collateral_denom: "ucollateral".to_string(),
            collateral_amount: "3000".to_string(),
            credit_denom: "ucredit".to_string(),
            credit_amount: "1500".to_string(),
        },
    )
    .unwrap();

    execute(
        deps.as_mut(),
        env.clone(),
        info_admin.clone(),
        ExecuteMsg::CreateCreditLine {
            borrower: borrower_2.to_string(),
            collateral_denom: "ucollateral".to_string(),
            collateral_amount: "4000".to_string(),
            credit_denom: "ucredit".to_string(),
            credit_amount: "2000".to_string(),
        },
    )
    .unwrap();

    let info_b1 = message_info(&borrower_1, &[]);
    execute(
        deps.as_mut(),
        env.clone(),
        info_b1.clone(),
        ExecuteMsg::CreateDraw {
            credit_line_id: 0,
            amount: "500".to_string(),
            denom: "ucredit".to_string(),
        },
    )
    .unwrap();

    execute(
        deps.as_mut(),
        env.clone(),
        info_b1.clone(),
        ExecuteMsg::CreateDraw {
            credit_line_id: 0,
            amount: "300".to_string(),
            denom: "ucredit".to_string(),
        },
    )
    .unwrap();

    let info_b2 = message_info(&borrower_2, &[]);
    execute(
        deps.as_mut(),
        env.clone(),
        info_b2.clone(),
        ExecuteMsg::CreateDraw {
            credit_line_id: 1,
            amount: "800".to_string(),
            denom: "ucredit".to_string(),
        },
    )
    .unwrap();

    execute(
        deps.as_mut(),
        env.clone(),
        info_admin.clone(),
        ExecuteMsg::SetLateFeeConfig {
            config: Some(LateFeeConfig::Flat(FlatFeeConfig {
                amount: Uint128::new(25),
            })),
        },
    )
    .unwrap();

    let pre_clc = CREDIT_LINE_COUNT.load(&deps.storage).unwrap();
    let pre_cl0 = CREDIT_LINES.load(&deps.storage, 0).unwrap();
    let pre_cl1 = CREDIT_LINES.load(&deps.storage, 1).unwrap();
    let pre_dcnt0 = DRAW_COUNT.load(&deps.storage, 0).unwrap();
    let pre_dcnt1 = DRAW_COUNT.load(&deps.storage, 1).unwrap();
    let pre_d0_0 = DRAWS.load(&deps.storage, (0, 0)).unwrap();
    let pre_d0_1 = DRAWS.load(&deps.storage, (0, 1)).unwrap();
    let pre_d1_0 = DRAWS.load(&deps.storage, (1, 0)).unwrap();

    let mig_res = migrate(deps.as_mut(), env.clone(), MigrateMsg {}).unwrap();
    assert_eq!(mig_res.messages.len(), 0);

    assert_eq!(CREDIT_LINE_COUNT.load(&deps.storage).unwrap(), pre_clc);
    assert_eq!(CREDIT_LINES.load(&deps.storage, 0).unwrap(), pre_cl0);
    assert_eq!(CREDIT_LINES.load(&deps.storage, 1).unwrap(), pre_cl1);
    assert_eq!(DRAW_COUNT.load(&deps.storage, 0).unwrap(), pre_dcnt0);
    assert_eq!(DRAW_COUNT.load(&deps.storage, 1).unwrap(), pre_dcnt1);
    assert_eq!(DRAWS.load(&deps.storage, (0, 0)).unwrap(), pre_d0_0);
    assert_eq!(DRAWS.load(&deps.storage, (0, 1)).unwrap(), pre_d0_1);
    assert_eq!(DRAWS.load(&deps.storage, (1, 0)).unwrap(), pre_d1_0);

    execute(
        deps.as_mut(),
        env.clone(),
        info_b1.clone(),
        ExecuteMsg::RepayDraw {
            credit_line_id: 0,
            draw_id: 0,
        },
    )
    .unwrap();

    let post_repay_draw = DRAWS.load(&deps.storage, (0, 0)).unwrap();
    assert!(post_repay_draw.repaid);

    execute(
        deps.as_mut(),
        env.clone(),
        info_b2.clone(),
        ExecuteMsg::CreateDraw {
            credit_line_id: 1,
            amount: "400".to_string(),
            denom: "ucredit".to_string(),
        },
    )
    .unwrap();

    assert_eq!(DRAW_COUNT.load(&deps.storage, 1).unwrap(), 2);
    let new_draw = DRAWS.load(&deps.storage, (1, 1)).unwrap();
    assert_eq!(new_draw.amount, Uint128::new(400));
    assert!(!new_draw.repaid);

    let trail_res = query(
        deps.as_ref(),
        env,
        QueryMsg::DrawAuditTrail {
            credit_line_id: 0,
            draw_id: Some(0),
        },
    )
    .unwrap();
    let trail_list: Vec<DrawAuditTrailResponse> = cosmwasm_std::from_json(trail_res).unwrap();
    assert_eq!(trail_list.len(), 1);
    assert_eq!(trail_list[0].events.len(), 2);
    assert_eq!(trail_list[0].events[0].action, DrawAction::DrawCreated);
    assert_eq!(trail_list[0].events[1].action, DrawAction::Repaid);
}

#[test]
fn test_migration_corrupted_key_failure_modes() {
    let mut deps = mock_dependencies();
    deps.storage.set(b"config", b"{not_valid_json");

    let res = CONFIG.load(&deps.storage);
    assert!(res.is_err());
    match res.unwrap_err() {
        StdError::ParseErr { .. } => {}
        other => panic!("Expected ParseErr, got {:?}", other),
    }

    let corrupt_u64_key = extract_raw_storage_key(|s| {
        CREDIT_LINES
            .save(
                s,
                999,
                &CreditLine {
                    id: 999,
                    borrower: Addr::unchecked("dummy"),
                    collateral_denom: "u".to_string(),
                    collateral_amount: Uint128::zero(),
                    credit_denom: "u".to_string(),
                    credit_amount: Uint128::zero(),
                    active: false,
                },
            )
            .unwrap()
    });
    deps.storage.set(&corrupt_u64_key, b"corrupted_bytes");
    let cl_res = CREDIT_LINES.load(&deps.storage, 999);
    assert!(cl_res.is_err());
    match cl_res.unwrap_err() {
        StdError::ParseErr { .. } => {}
        other => panic!("Expected ParseErr, got {:?}", other),
    }
}
