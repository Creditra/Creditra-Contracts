// SPDX-License-Identifier: MIT

//! Draw must not push unrepaid utilization above `CreditLine.credit_amount`.

use cosmwasm_std::testing::{
    message_info, mock_dependencies, mock_env, MockApi, MockQuerier, MockStorage,
};
use cosmwasm_std::{from_json, Addr, OwnedDeps, Uint128};
use creditra_credit::contract::{
    execute_create_credit_line, execute_create_draw, execute_repay_draw, instantiate, query,
};
use creditra_credit::error::ContractError;
use creditra_credit::msg::{CreditLineSnapshotResponse, InstantiateMsg, QueryMsg};

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

fn draw(
    deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
    cl_id: u64,
    amount: &str,
) -> Result<u64, ContractError> {
    let info = message_info(&borrower(deps), &[]);
    let res = execute_create_draw(
        deps.as_mut(),
        mock_env(),
        info,
        cl_id,
        amount.to_string(),
        "ucredit".to_string(),
    )?;
    Ok(res
        .attributes
        .iter()
        .find(|a| a.key == "draw_id")
        .unwrap()
        .value
        .parse()
        .unwrap())
}

fn snapshot(
    deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>,
    credit_line_id: u64,
) -> CreditLineSnapshotResponse {
    let raw = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::CreditLineSnapshot { credit_line_id },
    )
    .unwrap();
    let snap: Option<CreditLineSnapshotResponse> = from_json(&raw).unwrap();
    snap.expect("credit line must exist")
}

#[test]
fn draw_at_credit_amount_succeeds() {
    let mut deps = mock_dependencies();
    setup(&mut deps);
    let cl_id = open_line(&mut deps, "1000");
    draw(&mut deps, cl_id, "1000").unwrap();
    let snap = snapshot(&deps, cl_id);
    assert_eq!(snap.total_utilized, Uint128::new(1000));
}

#[test]
fn draw_above_credit_amount_returns_over_limit() {
    let mut deps = mock_dependencies();
    setup(&mut deps);
    let cl_id = open_line(&mut deps, "1000");
    let err = draw(&mut deps, cl_id, "1001").unwrap_err();
    assert_eq!(err, ContractError::OverLimit);
    let snap = snapshot(&deps, cl_id);
    assert!(snap.draws.is_empty());
    assert_eq!(snap.total_utilized, Uint128::zero());
}

#[test]
fn second_draw_that_crosses_limit_returns_over_limit() {
    let mut deps = mock_dependencies();
    setup(&mut deps);
    let cl_id = open_line(&mut deps, "1000");
    let first = draw(&mut deps, cl_id, "600").unwrap();
    assert_eq!(first, 0);
    let err = draw(&mut deps, cl_id, "401").unwrap_err();
    assert_eq!(err, ContractError::OverLimit);
    let snap = snapshot(&deps, cl_id);
    assert_eq!(snap.draws.len(), 1);
    assert_eq!(snap.total_utilized, Uint128::new(600));
}

#[test]
fn exact_headroom_draw_succeeds() {
    let mut deps = mock_dependencies();
    setup(&mut deps);
    let cl_id = open_line(&mut deps, "1000");
    draw(&mut deps, cl_id, "600").unwrap();
    draw(&mut deps, cl_id, "400").unwrap();
    let snap = snapshot(&deps, cl_id);
    assert_eq!(snap.total_utilized, Uint128::new(1000));
    assert_eq!(snap.draws.len(), 2);
}

#[test]
fn zero_amount_returns_invalid_amount() {
    let mut deps = mock_dependencies();
    setup(&mut deps);
    let cl_id = open_line(&mut deps, "1000");
    let err = draw(&mut deps, cl_id, "0").unwrap_err();
    assert_eq!(err, ContractError::InvalidAmount);
    let snap = snapshot(&deps, cl_id);
    assert!(snap.draws.is_empty());
}

#[test]
fn zero_credit_amount_rejects_positive_draw() {
    let mut deps = mock_dependencies();
    setup(&mut deps);
    let cl_id = open_line(&mut deps, "0");
    let err = draw(&mut deps, cl_id, "1").unwrap_err();
    assert_eq!(err, ContractError::OverLimit);
}

#[test]
fn repay_restores_headroom() {
    let mut deps = mock_dependencies();
    setup(&mut deps);
    let cl_id = open_line(&mut deps, "1000");
    let draw_id = draw(&mut deps, cl_id, "1000").unwrap();
    let info = message_info(&borrower(&deps), &[]);
    execute_repay_draw(deps.as_mut(), mock_env(), info, cl_id, draw_id).unwrap();
    draw(&mut deps, cl_id, "1000").unwrap();
    let snap = snapshot(&deps, cl_id);
    assert_eq!(snap.total_utilized, Uint128::new(1000));
}

#[test]
fn over_limit_does_not_write_draw_or_audit() {
    let mut deps = mock_dependencies();
    setup(&mut deps);
    let cl_id = open_line(&mut deps, "100");
    assert_eq!(
        draw(&mut deps, cl_id, "101").unwrap_err(),
        ContractError::OverLimit
    );
    let snap = snapshot(&deps, cl_id);
    assert!(snap.draws.is_empty());
    assert_eq!(snap.total_utilized, Uint128::zero());
    let raw = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::DrawAuditTrail {
            credit_line_id: cl_id,
            draw_id: None,
        },
    )
    .unwrap();
    let trail: Vec<creditra_credit::msg::DrawAuditTrailResponse> = from_json(&raw).unwrap();
    assert!(trail.is_empty());
}
