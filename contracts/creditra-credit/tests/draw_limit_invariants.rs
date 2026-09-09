// SPDX-License-Identifier: MIT

//! Invariant and property tests proving draw and credit-limit constraints.
//!
//! Enforces:
//! 1. Utilization never exceeds the configured credit limit.
//! 2. Failed draws leave contract storage completely unchanged.
//! 3. Repeated valid operations produce deterministic state.
//! 4. Property tests cover boundary, zero, and maximum amounts.

use cosmwasm_std::testing::{
    message_info, mock_dependencies, mock_env, MockApi, MockQuerier, MockStorage,
};
use cosmwasm_std::{from_json, Addr, Order, OwnedDeps, Storage, Uint128};
use creditra_credit::contract::{
    execute_create_credit_line, execute_create_draw, execute_repay_draw, instantiate, query,
};
use creditra_credit::error::ContractError;
use creditra_credit::limits::{
    assert_storage_utilization_invariant, assert_utilization_within_limit, can_draw,
    check_draw_limit, remaining_headroom,
};
use creditra_credit::msg::{
    CreditLineSnapshotResponse, DrawAuditTrailResponse, InstantiateMsg, ProofOfReserveResponse,
    QueryMsg,
};
use creditra_credit::state::outstanding_utilization;
use proptest::collection::vec as proptest_vec;
use proptest::prelude::*;
use proptest::test_runner::Config as ProptestConfig;

fn admin(deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>) -> Addr {
    deps.api.addr_make("admin")
}

fn borrower(deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>) -> Addr {
    deps.api.addr_make("alice")
}

fn stranger(deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>) -> Addr {
    deps.api.addr_make("mallory")
}

fn setup_test_contract(deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>) {
    let owner = admin(deps);
    let env = mock_env();
    let info = message_info(&owner, &[]);
    instantiate(
        deps.as_mut(),
        env,
        info,
        InstantiateMsg {
            owner: owner.to_string(),
        },
    )
    .unwrap();
}

fn open_test_line(
    deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
    credit_amount: &str,
) -> u64 {
    let admin_addr = admin(deps);
    let borrower_addr = borrower(deps);
    let res = execute_create_credit_line(
        deps.as_mut(),
        mock_env(),
        message_info(&admin_addr, &[]),
        borrower_addr.to_string(),
        "ucollateral".to_string(),
        "1000000000".to_string(),
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

fn try_draw(
    deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
    caller: &Addr,
    cl_id: u64,
    amount: &str,
    denom: &str,
) -> Result<u64, ContractError> {
    let info = message_info(caller, &[]);
    let res = execute_create_draw(
        deps.as_mut(),
        mock_env(),
        info,
        cl_id,
        amount.to_string(),
        denom.to_string(),
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

fn dump_storage(storage: &MockStorage) -> Vec<(Vec<u8>, Vec<u8>)> {
    storage.range(None, None, Order::Ascending).collect()
}

fn query_snapshot(
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

fn query_por(deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>) -> ProofOfReserveResponse {
    let raw = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::ProofOfReserve { denom: None },
    )
    .unwrap();
    from_json(&raw).unwrap()
}

fn query_audit_trail(
    deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>,
    credit_line_id: u64,
) -> Vec<DrawAuditTrailResponse> {
    let raw = query(
        deps.as_ref(),
        mock_env(),
        QueryMsg::DrawAuditTrail {
            credit_line_id,
            draw_id: None,
        },
    )
    .unwrap();
    from_json(&raw).unwrap()
}

#[derive(Clone, Debug)]
enum StepAction {
    Draw(u128),
    Repay(usize),
}

fn action_sequence_strategy() -> impl Strategy<Value = Vec<StepAction>> {
    proptest_vec(
        prop_oneof![
            (1u128..=3_000u128).prop_map(StepAction::Draw),
            (0usize..=20usize).prop_map(StepAction::Repay),
        ],
        1..=64,
    )
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, .. ProptestConfig::default() })]

    #[test]
    fn utilization_never_exceeds_configured_limit_proptest(
        credit_limit_raw in 1_000u128..=100_000u128,
        actions in action_sequence_strategy(),
    ) {
        let mut deps = mock_dependencies();
        setup_test_contract(&mut deps);

        let credit_limit = Uint128::from(credit_limit_raw);
        let cl_id = open_test_line(&mut deps, &credit_limit.to_string());
        let alice = borrower(&deps);

        let mut active_draw_ids: Vec<u64> = Vec::new();

        for action in actions {
            match action {
                StepAction::Draw(amount) => {
                    let draw_amount = Uint128::from(amount);
                    let outstanding = outstanding_utilization(&deps.storage, cl_id).unwrap();
                    let headroom = remaining_headroom(credit_limit, outstanding);
                    let check_res = check_draw_limit(outstanding, draw_amount, credit_limit);
                    let should_succeed = can_draw(outstanding, draw_amount, credit_limit);
                    prop_assert_eq!(should_succeed, check_res.is_ok());
                    prop_assert_eq!(should_succeed, draw_amount <= headroom && !draw_amount.is_zero());

                    let result = try_draw(&mut deps, &alice, cl_id, &amount.to_string(), "ucredit");

                    if should_succeed {
                        let draw_id = result.expect("draw within headroom must succeed");
                        active_draw_ids.push(draw_id);
                    } else {
                        let err = result.expect_err("draw exceeding limit must fail");
                        prop_assert_eq!(err, ContractError::OverLimit);
                    }
                }
                StepAction::Repay(idx) => {
                    if !active_draw_ids.is_empty() {
                        let selected_id = active_draw_ids[idx % active_draw_ids.len()];
                        let info = message_info(&alice, &[]);
                        let _ = execute_repay_draw(
                            deps.as_mut(),
                            mock_env(),
                            info,
                            cl_id,
                            selected_id,
                        );
                    }
                }
            }

            let current_utilization = outstanding_utilization(&deps.storage, cl_id).unwrap();
            prop_assert!(
                current_utilization <= credit_limit,
                "Invariant violation: utilization {} exceeds limit {}",
                current_utilization,
                credit_limit
            );

            assert_utilization_within_limit(current_utilization, credit_limit).unwrap();
            assert_storage_utilization_invariant(&deps.storage, cl_id, credit_limit).unwrap();

            let por = query_por(&deps);
            prop_assert_eq!(por.net_outstanding, current_utilization);
            prop_assert!(por.net_outstanding <= credit_limit);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, .. ProptestConfig::default() })]

    #[test]
    fn property_tests_cover_boundary_and_maximum_amounts(
        credit_limit_raw in 100u128..=1_000_000_000u128,
        draw_ratio_permille in 0u32..=2_000u32,
    ) {
        let mut deps = mock_dependencies();
        setup_test_contract(&mut deps);

        let credit_limit = Uint128::from(credit_limit_raw);
        let cl_id = open_test_line(&mut deps, &credit_limit.to_string());
        let alice = borrower(&deps);

        let draw_amount = credit_limit.multiply_ratio(draw_ratio_permille as u128, 1_000u128);

        let result = try_draw(&mut deps, &alice, cl_id, &draw_amount.to_string(), "ucredit");

        if draw_amount.is_zero() {
            prop_assert_eq!(result.unwrap_err(), ContractError::InvalidAmount);
        } else if draw_amount <= credit_limit {
            prop_assert!(result.is_ok());
            let utilization = outstanding_utilization(&deps.storage, cl_id).unwrap();
            prop_assert_eq!(utilization, draw_amount);
            assert_storage_utilization_invariant(&deps.storage, cl_id, credit_limit).unwrap();
        } else {
            prop_assert_eq!(result.unwrap_err(), ContractError::OverLimit);
            let utilization = outstanding_utilization(&deps.storage, cl_id).unwrap();
            prop_assert_eq!(utilization, Uint128::zero());
        }
    }
}

#[test]
fn boundary_conditions_around_exact_headroom() {
    let mut deps = mock_dependencies();
    setup_test_contract(&mut deps);
    let cl_id = open_test_line(&mut deps, "1000");
    let alice = borrower(&deps);

    try_draw(&mut deps, &alice, cl_id, "600", "ucredit").unwrap();
    let outstanding = outstanding_utilization(&deps.storage, cl_id).unwrap();
    assert_eq!(outstanding, Uint128::new(600));

    let headroom = remaining_headroom(Uint128::new(1000), outstanding);
    assert_eq!(headroom, Uint128::new(400));

    let err_over = try_draw(&mut deps, &alice, cl_id, "401", "ucredit").unwrap_err();
    assert_eq!(err_over, ContractError::OverLimit);

    let draw_exact = try_draw(&mut deps, &alice, cl_id, "400", "ucredit").unwrap();
    assert_eq!(draw_exact, 1);

    let full_utilization = outstanding_utilization(&deps.storage, cl_id).unwrap();
    assert_eq!(full_utilization, Uint128::new(1000));
    assert_storage_utilization_invariant(&deps.storage, cl_id, Uint128::new(1000)).unwrap();

    let err_full = try_draw(&mut deps, &alice, cl_id, "1", "ucredit").unwrap_err();
    assert_eq!(err_full, ContractError::OverLimit);

    let err_max = try_draw(
        &mut deps,
        &alice,
        cl_id,
        &Uint128::MAX.to_string(),
        "ucredit",
    )
    .unwrap_err();
    assert_eq!(err_max, ContractError::Overflow);

    let fresh_cl_id = open_test_line(&mut deps, "1000");
    let err_over_limit_from_zero =
        try_draw(&mut deps, &alice, fresh_cl_id, "1001", "ucredit").unwrap_err();
    assert_eq!(err_over_limit_from_zero, ContractError::OverLimit);
}

#[test]
fn boundary_conditions_zero_and_max_credit_limit() {
    let mut deps = mock_dependencies();
    setup_test_contract(&mut deps);
    let alice = borrower(&deps);

    let zero_line_id = open_test_line(&mut deps, "0");
    let err_zero = try_draw(&mut deps, &alice, zero_line_id, "1", "ucredit").unwrap_err();
    assert_eq!(err_zero, ContractError::OverLimit);

    let max_line_id = open_test_line(&mut deps, &Uint128::MAX.to_string());
    assert_eq!(
        remaining_headroom(Uint128::MAX, Uint128::zero()),
        Uint128::MAX
    );

    let draw_max_id = try_draw(
        &mut deps,
        &alice,
        max_line_id,
        &Uint128::MAX.to_string(),
        "ucredit",
    )
    .unwrap();
    assert_eq!(draw_max_id, 0);

    let max_utilization = outstanding_utilization(&deps.storage, max_line_id).unwrap();
    assert_eq!(max_utilization, Uint128::MAX);
    assert_storage_utilization_invariant(&deps.storage, max_line_id, Uint128::MAX).unwrap();

    let err_overflow = try_draw(&mut deps, &alice, max_line_id, "1", "ucredit").unwrap_err();
    assert_eq!(err_overflow, ContractError::Overflow);
}

#[test]
fn failed_draws_leave_storage_unchanged_across_all_failure_modes() {
    let mut deps = mock_dependencies();
    setup_test_contract(&mut deps);
    let cl_id = open_test_line(&mut deps, "1000");
    let alice = borrower(&deps);
    let mallory = stranger(&deps);

    try_draw(&mut deps, &alice, cl_id, "300", "ucredit").unwrap();

    let baseline_storage = dump_storage(&deps.storage);
    let baseline_snapshot = query_snapshot(&deps, cl_id);
    let baseline_por = query_por(&deps);
    let baseline_audit = query_audit_trail(&deps, cl_id);

    let over_limit_err = try_draw(&mut deps, &alice, cl_id, "701", "ucredit").unwrap_err();
    assert_eq!(over_limit_err, ContractError::OverLimit);
    assert_eq!(dump_storage(&deps.storage), baseline_storage);
    assert_eq!(query_snapshot(&deps, cl_id), baseline_snapshot);
    assert_eq!(query_por(&deps), baseline_por);
    assert_eq!(query_audit_trail(&deps, cl_id), baseline_audit);

    let zero_amount_err = try_draw(&mut deps, &alice, cl_id, "0", "ucredit").unwrap_err();
    assert_eq!(zero_amount_err, ContractError::InvalidAmount);
    assert_eq!(dump_storage(&deps.storage), baseline_storage);
    assert_eq!(query_snapshot(&deps, cl_id), baseline_snapshot);
    assert_eq!(query_por(&deps), baseline_por);
    assert_eq!(query_audit_trail(&deps, cl_id), baseline_audit);

    let unparseable_err = try_draw(
        &mut deps,
        &alice,
        cl_id,
        "340282366920938463463374607431768211456",
        "ucredit",
    )
    .unwrap_err();
    assert!(matches!(unparseable_err, ContractError::Std(_)));
    assert_eq!(dump_storage(&deps.storage), baseline_storage);
    assert_eq!(query_snapshot(&deps, cl_id), baseline_snapshot);
    assert_eq!(query_por(&deps), baseline_por);
    assert_eq!(query_audit_trail(&deps, cl_id), baseline_audit);

    let wrong_caller_err = try_draw(&mut deps, &mallory, cl_id, "100", "ucredit").unwrap_err();
    assert_eq!(wrong_caller_err, ContractError::CrossTenantIdentifier);
    assert_eq!(dump_storage(&deps.storage), baseline_storage);
    assert_eq!(query_snapshot(&deps, cl_id), baseline_snapshot);
    assert_eq!(query_por(&deps), baseline_por);
    assert_eq!(query_audit_trail(&deps, cl_id), baseline_audit);

    let missing_line_err = try_draw(&mut deps, &alice, 9999, "100", "ucredit").unwrap_err();
    assert_eq!(missing_line_err, ContractError::CreditLineNotFound(9999));
    assert_eq!(dump_storage(&deps.storage), baseline_storage);
    assert_eq!(query_snapshot(&deps, cl_id), baseline_snapshot);
    assert_eq!(query_por(&deps), baseline_por);
    assert_eq!(query_audit_trail(&deps, cl_id), baseline_audit);
}

#[test]
fn repeated_valid_operations_produce_deterministic_state_cross_instance() {
    let mut deps_a = mock_dependencies();
    let mut deps_b = mock_dependencies();

    setup_test_contract(&mut deps_a);
    setup_test_contract(&mut deps_b);

    let cl_a = open_test_line(&mut deps_a, "5000");
    let cl_b = open_test_line(&mut deps_b, "5000");
    assert_eq!(cl_a, cl_b);

    let alice_a = borrower(&deps_a);
    let alice_b = borrower(&deps_b);

    let operations = vec![
        ("draw", "1000"),
        ("draw", "1500"),
        ("repay", "0"),
        ("draw", "2000"),
        ("repay", "1"),
        ("draw", "500"),
    ];

    for (op, arg) in operations {
        match op {
            "draw" => {
                let id_a = try_draw(&mut deps_a, &alice_a, cl_a, arg, "ucredit").unwrap();
                let id_b = try_draw(&mut deps_b, &alice_b, cl_b, arg, "ucredit").unwrap();
                assert_eq!(id_a, id_b);
            }
            "repay" => {
                let draw_id: u64 = arg.parse().unwrap();
                let info_a = message_info(&alice_a, &[]);
                let info_b = message_info(&alice_b, &[]);
                execute_repay_draw(deps_a.as_mut(), mock_env(), info_a, cl_a, draw_id).unwrap();
                execute_repay_draw(deps_b.as_mut(), mock_env(), info_b, cl_b, draw_id).unwrap();
            }
            _ => unreachable!(),
        }

        assert_eq!(dump_storage(&deps_a.storage), dump_storage(&deps_b.storage));
        assert_eq!(query_snapshot(&deps_a, cl_a), query_snapshot(&deps_b, cl_b));
        assert_eq!(query_por(&deps_a), query_por(&deps_b));
        assert_eq!(
            query_audit_trail(&deps_a, cl_a),
            query_audit_trail(&deps_b, cl_b)
        );
    }
}

#[test]
fn repeated_draw_repay_cycles_restore_utilization_deterministically() {
    let mut deps = mock_dependencies();
    setup_test_contract(&mut deps);
    let cl_id = open_test_line(&mut deps, "10000");
    let alice = borrower(&deps);

    let initial_draw = try_draw(&mut deps, &alice, cl_id, "2500", "ucredit").unwrap();
    assert_eq!(initial_draw, 0);

    let baseline_utilization = outstanding_utilization(&deps.storage, cl_id).unwrap();
    assert_eq!(baseline_utilization, Uint128::new(2500));

    for cycle in 0..10 {
        let draw_id = try_draw(&mut deps, &alice, cl_id, "1000", "ucredit").unwrap();
        assert_eq!(draw_id, (cycle + 1) as u64);

        let active_utilization = outstanding_utilization(&deps.storage, cl_id).unwrap();
        assert_eq!(active_utilization, Uint128::new(3500));

        let info = message_info(&alice, &[]);
        execute_repay_draw(deps.as_mut(), mock_env(), info, cl_id, draw_id).unwrap();

        let restored_utilization = outstanding_utilization(&deps.storage, cl_id).unwrap();
        assert_eq!(restored_utilization, baseline_utilization);
        assert_storage_utilization_invariant(&deps.storage, cl_id, Uint128::new(10000)).unwrap();
    }
}
