// SPDX-License-Identifier: MIT

//! Property test: `net_outstanding` (ProofOfReserve) equals sum of unrepaid draws.
//!
//! This test creates several credit lines, performs arbitrary draws and
//! repays, and after every step asserts the protocol's reported
//! `net_outstanding` equals the modeled sum of unrepaid draws.

use cosmwasm_std::testing::{
    message_info, mock_dependencies, mock_env, MockApi, MockQuerier, MockStorage,
};
use cosmwasm_std::{from_json, OwnedDeps, Uint128};
use creditra_credit::contract;
use creditra_credit::msg::{ExecuteMsg, InstantiateMsg, ProofOfReserveResponse, QueryMsg};
use proptest::collection::vec as proptest_vec;
use proptest::prelude::*;
use proptest::test_runner::Config as ProptestConfig;

const BORROWER_COUNT: usize = 3;
const MAX_STEPS: usize = 64;
const MAX_REQUEST: u128 = 500u128;

/// Strategy for raw steps: (borrower_index, wants_draw, amount)
#[derive(Clone, Debug)]
struct RawStep {
    borrower_index: usize,
    wants_draw: bool,
    amount: u128,
}

fn raw_steps_strategy() -> impl Strategy<Value = Vec<RawStep>> {
    proptest_vec(
        (0usize..BORROWER_COUNT, any::<bool>(), 1u128..=MAX_REQUEST),
        1..=MAX_STEPS,
    )
    .prop_map(|steps| {
        steps
            .into_iter()
            .map(|(borrower_index, wants_draw, amount)| RawStep {
                borrower_index,
                wants_draw,
                amount,
            })
            .collect()
    })
}

fn setup_contract(deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>) {
    let env = mock_env();
    let owner = deps.api.addr_make("owner");
    let info = message_info(&owner, &[]);
    let msg = InstantiateMsg {
        owner: owner.to_string(),
    };
    contract::instantiate(deps.as_mut(), env, info, msg).unwrap();
}

fn create_credit_line(
    deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
    borrower_label: &str,
    collateral_amount: &str,
    credit_amount: &str,
) -> u64 {
    let env = mock_env();
    let owner = deps.api.addr_make("owner");
    let info = message_info(&owner, &[]);
    let msg = ExecuteMsg::CreateCreditLine {
        borrower: deps.api.addr_make(borrower_label).to_string(),
        collateral_denom: "ucollateral".to_string(),
        collateral_amount: collateral_amount.to_string(),
        credit_denom: "ucredit".to_string(),
        credit_amount: credit_amount.to_string(),
    };
    contract::execute(deps.as_mut(), env, info, msg).unwrap();

    // The contract assigns sequential IDs starting at 0; the new id is
    // CREDIT_LINE_COUNT - 1. For tests we can infer id by counting existing lines
    // via scanning CREDIT_LINE_COUNT through query_proof_of_reserve or by
    // returning the next id via a side-effect-free approach. For simplicity
    // we return the next sequential id by tracking creation order in the test.
    // Callers should track IDs in creation order starting at 0.
    0
}

fn create_draw(
    deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
    credit_line_id: u64,
    amount: u128,
    borrower_label: &str,
) {
    let env = mock_env();
    let borrower = deps.api.addr_make(borrower_label);
    let info = message_info(&borrower, &[]);
    let msg = ExecuteMsg::CreateDraw {
        credit_line_id,
        amount: amount.to_string(),
        denom: "ucredit".to_string(),
    };
    contract::execute(deps.as_mut(), env, info, msg).unwrap();
}

fn repay_draw(
    deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
    credit_line_id: u64,
    draw_id: u64,
    borrower_label: &str,
) {
    let env = mock_env();
    let borrower = deps.api.addr_make(borrower_label);
    let info = message_info(&borrower, &[]);
    let msg = ExecuteMsg::RepayDraw {
        credit_line_id,
        draw_id,
    };
    contract::execute(deps.as_mut(), env, info, msg).unwrap();
}

fn query_por(deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>) -> ProofOfReserveResponse {
    let env = mock_env();
    let raw =
        contract::query(deps.as_ref(), env, QueryMsg::ProofOfReserve { denom: None }).unwrap();
    from_json(&raw).unwrap()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 128, .. ProptestConfig::default() })]

    #[test]
    fn total_utilized_conservation(steps in raw_steps_strategy()) {
        let mut deps = mock_dependencies();
        setup_contract(&mut deps);

        // Create borrowers / credit lines
        let mut borrower_labels = Vec::with_capacity(BORROWER_COUNT);
        for i in 0..BORROWER_COUNT {
            borrower_labels.push(format!("borrower_{}", i));
        }

        // Create credit lines for each borrower. The contract assigns sequential ids
        // starting at 0; we track the mapping implicitly by creation order.
        for label in &borrower_labels {
            create_credit_line(&mut deps, label, "1000", "1000000");
        }

        // Model: per-credit-line list of draws (amount, repaid)
        let mut modeled: Vec<Vec<(u128, bool)>> = vec![Vec::new(); BORROWER_COUNT];

        // Helper to compute modeled net outstanding
        fn modeled_net(modeled: &[Vec<(u128, bool)>]) -> u128 {
            modeled.iter().flat_map(|v| v.iter()).filter(|(_, r)| !*r).map(|(a, _)| *a).sum()
        }

        // Initial invariant: net outstanding should be zero
        let por0 = query_por(&deps);
        prop_assert_eq!(por0.net_outstanding, Uint128::zero());

        for step in &steps {
            let idx = step.borrower_index;
            let label = &borrower_labels[idx];

            if step.wants_draw {
                // Perform draw
                create_draw(&mut deps, idx as u64, step.amount, label);
                modeled[idx].push((step.amount, false));
            } else {
                // Try to repay an existing unrepaid draw; if none exist, perform a draw
                let mut found = None;
                for (did, (_amt, repaid)) in modeled[idx].iter().enumerate() {
                    if !*repaid {
                        found = Some(did as u64);
                        break;
                    }
                }

                if let Some(did) = found {
                    repay_draw(&mut deps, idx as u64, did, label);
                    modeled[idx][did as usize].1 = true;
                } else {
                    // fallback to draw
                    create_draw(&mut deps, idx as u64, step.amount, label);
                    modeled[idx].push((step.amount, false));
                }
            }

            // Assert invariant against contract's ProofOfReserve net_outstanding
            let por = query_por(&deps);
            let expected = modeled_net(&modeled);
            prop_assert_eq!(por.net_outstanding, Uint128::from(expected),
                "net_outstanding mismatch: expected {} got {}", expected, por.net_outstanding.u128());
        }
    }
}
