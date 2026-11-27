// SPDX-License-Identifier: MIT

use cosmwasm_std::testing::{
    message_info, mock_dependencies, mock_env, MockApi, MockQuerier, MockStorage,
};
use cosmwasm_std::{from_json, Addr, OwnedDeps};
use creditra_credit::contract::{
    execute_add_audit_memo, execute_create_credit_line, execute_create_draw, instantiate, query,
};
use creditra_credit::msg::{DrawAuditTrailResponse, InstantiateMsg, QueryMsg};

fn admin(deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>) -> Addr {
    deps.api.addr_make("admin")
}

fn borrower(deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>) -> Addr {
    deps.api.addr_make("alice")
}

fn setup(deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>) {
    let admin_addr = admin(deps);
    let env = mock_env();
    let info = message_info(&admin_addr, &[]);
    instantiate(
        deps.as_mut(),
        env,
        info,
        InstantiateMsg {
            owner: admin_addr.to_string(),
        },
    )
    .unwrap();
}

fn open_line(deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>, credit_amount: &str) -> u64 {
    let admin_addr = admin(deps);
    let borrower_addr = borrower(deps);
    let res = execute_create_credit_line(
        deps.as_mut(),
        mock_env(),
        message_info(&admin_addr, &[]),
        borrower_addr.to_string(),
        "ucollateral".to_string(),
        "1000000".to_string(),
        "ucredit".to_string(),
        credit_amount.to_string(),
    )
    .unwrap();
    res.attributes
        .iter()
        .find(|a| a.key == "credit_line_id")
        .unwrap()
        .value
        .parse()
        .unwrap()
}

fn draw(deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>, cl_id: u64, amount: &str) -> u64 {
    let info = message_info(&borrower(deps), &[]);
    let res = execute_create_draw(
        deps.as_mut(),
        mock_env(),
        info,
        cl_id,
        amount.to_string(),
        "ucredit".to_string(),
    )
    .unwrap();
    res.attributes
        .iter()
        .find(|a| a.key == "draw_id")
        .unwrap()
        .value
        .parse()
        .unwrap()
}

fn add_memo(
    deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
    cl_id: u64,
    draw_id: u64,
    memo: &str,
) {
    let info = message_info(&admin(deps), &[]);
    execute_add_audit_memo(
        deps.as_mut(),
        mock_env(),
        info,
        cl_id,
        draw_id,
        memo.to_string(),
    )
    .unwrap();
}

// Snapshotting utility
use std::fs;
use std::path::PathBuf;

fn snapshot_path(name: &str) -> PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    PathBuf::from(manifest_dir)
        .join("tests")
        .join("snapshots")
        .join(format!("{}.json", name))
}

fn assert_json_snapshot<T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug>(name: &str, data: &T) {
    let path = snapshot_path(name);
    let json = serde_json::to_string_pretty(data).unwrap();
    
    if std::env::var("UPDATE_EXPECT").is_ok() || !path.exists() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, json).unwrap();
    } else {
        let stored_json = fs::read_to_string(&path).unwrap();
        let stored_data: T = serde_json::from_str(&stored_json).unwrap();
        assert_eq!(*data, stored_data, "Snapshot mismatch for {}", name);
    }
}

#[test]
fn test_audit_trail_pagination_and_memos() {
    let mut deps = mock_dependencies();
    setup(&mut deps);
    let cl_id = open_line(&mut deps, "1000000");

    // Create 5 draws
    for i in 0..5 {
        let did = draw(&mut deps, cl_id, "100");
        assert_eq!(did, i);
        // Add memos to each draw to test memo ordering
        add_memo(&mut deps, cl_id, did, &format!("Memo A for {}", i));
        add_memo(&mut deps, cl_id, did, &format!("Memo B for {}", i));
    }

    // Query without pagination
    let raw = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::DrawAuditTrail {
            credit_line_id: cl_id,
            draw_id: None,
            start_after: None,
            limit: None,
        },
    )
    .unwrap();
    let trail: Vec<DrawAuditTrailResponse> = from_json(&raw).unwrap();
    assert_json_snapshot("audit_trail_full", &trail);

    // Query with limit
    let raw_limit = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::DrawAuditTrail {
            credit_line_id: cl_id,
            draw_id: None,
            start_after: None,
            limit: Some(2),
        },
    )
    .unwrap();
    let trail_limit: Vec<DrawAuditTrailResponse> = from_json(&raw_limit).unwrap();
    assert_json_snapshot("audit_trail_limit_2", &trail_limit);

    // Query with start_after and limit
    let raw_paginated = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::DrawAuditTrail {
            credit_line_id: cl_id,
            draw_id: None,
            start_after: Some(1),
            limit: Some(2),
        },
    )
    .unwrap();
    let trail_paginated: Vec<DrawAuditTrailResponse> = from_json(&raw_paginated).unwrap();
    assert_json_snapshot("audit_trail_start_after_1_limit_2", &trail_paginated);
}
