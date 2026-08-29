// SPDX-License-Identifier: MIT

//! Property test: `RepayDraw` never increases utilization.
//!
//! # What
//!
//! Verifies the fundamental accounting invariant that marking a draw as repaid
//! must never increase the borrower's utilized amount. Utilization is the sum
//! of all unrepaid draw amounts on a credit line (see
//! [`creditra_credit::views::query_borrower_health_factor`]).
//!
//! # Property
//!
//! For any valid setup (open line, one or more draws), and any draw chosen
//! for repayment:
//!
//! ```text
//! utilization_after_repay <= utilization_before
//! ```
//!
//! When the chosen draw was not already repaid, utilization must also decrease
//! by exactly that draw's amount.
//!
//! # Why
//!
//! This is the core safety property of the CosmWasm credit line. If a
//! repayment could increase utilization, debt accounting would be wrong and
//! health-factor views would become inconsistent.
//!
//! # References
//!
//! - [`creditra_credit::contract::execute_repay_draw`]
//! - Issue #796

use cosmwasm_std::testing::{
    message_info, mock_dependencies, mock_env, MockApi, MockQuerier, MockStorage,
};
use cosmwasm_std::{from_json, Addr, OwnedDeps, Uint128};
use creditra_credit::contract::{execute, instantiate, query};
use creditra_credit::msg::{
    BorrowerHealthFactorResponse, ExecuteMsg, InstantiateMsg, QueryMsg,
};
use proptest::prelude::*;
use proptest::test_runner::Config as ProptestConfig;

// ── Strategies ────────────────────────────────────────────────────────────

/// Strategy for a single draw amount (non-zero, bounded to avoid overflow).
fn draw_amount() -> impl Strategy<Value = u128> {
    1_u128..=50_000_u128
}

/// Strategy for 1..=8 draw amounts on one credit line.
fn draw_amounts() -> impl Strategy<Value = Vec<u128>> {
    prop::collection::vec(draw_amount(), 1..=8)
}

/// Strategy for which draw index to repay (filtered against draw count later).
fn repay_index() -> impl Strategy<Value = usize> {
    0_usize..=7_usize
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn setup_contract(deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>) -> Addr {
    let env = mock_env();
    let owner = deps.api.addr_make("owner");
    let info = message_info(&owner, &[]);
    let msg = InstantiateMsg {
        owner: owner.to_string(),
    };
    instantiate(deps.as_mut(), env, info, msg).unwrap();
    owner
}

fn create_credit_line(
    deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
    owner: &Addr,
    borrower: &Addr,
) -> u64 {
    let env = mock_env();
    let info = message_info(owner, &[]);
    let msg = ExecuteMsg::CreateCreditLine {
        borrower: borrower.to_string(),
        collateral_denom: "ucollateral".to_string(),
        collateral_amount: "1000000".to_string(),
        credit_denom: "ucredit".to_string(),
        credit_amount: "1000000".to_string(),
    };
    execute(deps.as_mut(), env, info, msg).unwrap();
    0
}

fn create_draw(
    deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
    borrower: &Addr,
    credit_line_id: u64,
    amount: u128,
) {
    let env = mock_env();
    let info = message_info(borrower, &[]);
    let msg = ExecuteMsg::CreateDraw {
        credit_line_id,
        amount: amount.to_string(),
        denom: "ucredit".to_string(),
    };
    execute(deps.as_mut(), env, info, msg).unwrap();
}

fn repay_draw(
    deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
    borrower: &Addr,
    credit_line_id: u64,
    draw_id: u64,
) {
    let env = mock_env();
    let info = message_info(borrower, &[]);
    let msg = ExecuteMsg::RepayDraw {
        credit_line_id,
        draw_id,
    };
    execute(deps.as_mut(), env, info, msg).unwrap();
}

fn utilized_amount(
    deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>,
    borrower: &Addr,
    credit_line_id: u64,
) -> Uint128 {
    let env = mock_env();
    let msg = QueryMsg::BorrowerHealthFactor {
        borrower: borrower.to_string(),
    };
    let raw = query(deps.as_ref(), env, msg).unwrap();
    let resp: BorrowerHealthFactorResponse = from_json(&raw).unwrap();
    resp.credit_lines
        .into_iter()
        .find(|cl| cl.credit_line_id == credit_line_id)
        .map(|cl| cl.utilized_amount)
        .unwrap_or(Uint128::zero())
}

// ── Property tests ────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, .. ProptestConfig::default() })]

    /// Utilization never increases after `RepayDraw`, for any set of draws
    /// and any repay target among them.
    ///
    /// Covers:
    /// - Partial repay (other draws remain outstanding)
    /// - Full repay of the only draw (utilization → 0)
    /// - Repaying any index among multiple draws
    #[test]
    fn utilization_never_increases_on_repay(
        amounts in draw_amounts(),
        idx in repay_index(),
    ) {
        prop_assume!(idx < amounts.len());

        let mut deps = mock_dependencies();
        let owner = setup_contract(&mut deps);
        let borrower = deps.api.addr_make("borrower");
        let credit_line_id = create_credit_line(&mut deps, &owner, &borrower);

        for &amount in &amounts {
            create_draw(&mut deps, &borrower, credit_line_id, amount);
        }

        let utilized_before = utilized_amount(&deps, &borrower, credit_line_id);
        let expected_before: u128 = amounts.iter().sum();
        prop_assert_eq!(
            utilized_before.u128(),
            expected_before,
            "precondition: utilization must equal sum of draws"
        );

        let repaid_amount = amounts[idx];
        repay_draw(&mut deps, &borrower, credit_line_id, idx as u64);

        let utilized_after = utilized_amount(&deps, &borrower, credit_line_id);

        prop_assert!(
            utilized_after <= utilized_before,
            "utilization increased after repay!\n\
             utilized_before={}, utilized_after={}, delta={}\n\
             setup: amounts={:?}, repay_idx={}",
            utilized_before,
            utilized_after,
            utilized_after.u128().saturating_sub(utilized_before.u128()),
            amounts,
            idx
        );

        // Fresh repay of an unrepaid draw must drop utilization by exactly
        // that draw's amount.
        prop_assert_eq!(
            utilized_after.u128(),
            expected_before - repaid_amount,
            "utilization must drop by the repaid draw amount"
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, .. ProptestConfig::default() })]

    /// Re-repaying an already-repaid draw must not increase utilization
    /// (idempotent repay still satisfies the invariant).
    #[test]
    fn re_repay_never_increases_utilization(
        amounts in draw_amounts(),
        idx in repay_index(),
    ) {
        prop_assume!(idx < amounts.len());

        let mut deps = mock_dependencies();
        let owner = setup_contract(&mut deps);
        let borrower = deps.api.addr_make("borrower");
        let credit_line_id = create_credit_line(&mut deps, &owner, &borrower);

        for &amount in &amounts {
            create_draw(&mut deps, &borrower, credit_line_id, amount);
        }

        repay_draw(&mut deps, &borrower, credit_line_id, idx as u64);
        let utilized_after_first = utilized_amount(&deps, &borrower, credit_line_id);

        // Second repay of the same draw
        repay_draw(&mut deps, &borrower, credit_line_id, idx as u64);
        let utilized_after_second = utilized_amount(&deps, &borrower, credit_line_id);

        prop_assert!(
            utilized_after_second <= utilized_after_first,
            "re-repay increased utilization!\n\
             after_first={}, after_second={}",
            utilized_after_first,
            utilized_after_second
        );
        prop_assert_eq!(
            utilized_after_second,
            utilized_after_first,
            "re-repay must leave utilization unchanged"
        );
    }
}

// ── Deterministic edge cases ──────────────────────────────────────────────

/// Overpaying in the sense of repaying every draw zeros utilization and
/// never increases it along the way.
#[test]
fn full_repay_of_all_draws_zeros_utilization() {
    let mut deps = mock_dependencies();
    let owner = setup_contract(&mut deps);
    let borrower = deps.api.addr_make("borrower");
    let credit_line_id = create_credit_line(&mut deps, &owner, &borrower);

    create_draw(&mut deps, &borrower, credit_line_id, 1_000);
    create_draw(&mut deps, &borrower, credit_line_id, 2_500);
    create_draw(&mut deps, &borrower, credit_line_id, 500);

    let mut prev = utilized_amount(&deps, &borrower, credit_line_id);
    assert_eq!(prev.u128(), 4_000);

    for draw_id in 0..3 {
        repay_draw(&mut deps, &borrower, credit_line_id, draw_id);
        let now = utilized_amount(&deps, &borrower, credit_line_id);
        assert!(
            now <= prev,
            "utilization must not increase when repaying draw {draw_id}"
        );
        prev = now;
    }

    assert_eq!(
        utilized_amount(&deps, &borrower, credit_line_id),
        Uint128::zero(),
        "full repay of all draws must zero utilization"
    );
}

/// Single-draw full repay decreases utilization to zero.
#[test]
fn single_draw_full_repay_decreases_utilization() {
    let mut deps = mock_dependencies();
    let owner = setup_contract(&mut deps);
    let borrower = deps.api.addr_make("borrower");
    let credit_line_id = create_credit_line(&mut deps, &owner, &borrower);

    create_draw(&mut deps, &borrower, credit_line_id, 10_000);
    let before = utilized_amount(&deps, &borrower, credit_line_id);
    assert_eq!(before.u128(), 10_000);

    repay_draw(&mut deps, &borrower, credit_line_id, 0);

    let after = utilized_amount(&deps, &borrower, credit_line_id);
    assert!(after <= before);
    assert_eq!(after, Uint128::zero());
}

/// Repaying borrower A's draw must not change borrower B's utilization.
#[test]
fn repay_one_borrower_does_not_affect_another() {
    let mut deps = mock_dependencies();
    let owner = setup_contract(&mut deps);
    let borrower_a = deps.api.addr_make("borrower_a");
    let borrower_b = deps.api.addr_make("borrower_b");

    // Two credit lines, different borrowers
    let env = mock_env();
    let info = message_info(&owner, &[]);
    execute(
        deps.as_mut(),
        env.clone(),
        info.clone(),
        ExecuteMsg::CreateCreditLine {
            borrower: borrower_a.to_string(),
            collateral_denom: "ucollateral".to_string(),
            collateral_amount: "1000".to_string(),
            credit_denom: "ucredit".to_string(),
            credit_amount: "10000".to_string(),
        },
    )
    .unwrap();
    execute(
        deps.as_mut(),
        env,
        info,
        ExecuteMsg::CreateCreditLine {
            borrower: borrower_b.to_string(),
            collateral_denom: "ucollateral".to_string(),
            collateral_amount: "1000".to_string(),
            credit_denom: "ucredit".to_string(),
            credit_amount: "10000".to_string(),
        },
    )
    .unwrap();

    create_draw(&mut deps, &borrower_a, 0, 5000);
    create_draw(&mut deps, &borrower_b, 1, 3000);

    let b_before = utilized_amount(&deps, &borrower_b, 1);
    assert_eq!(b_before.u128(), 3_000);

    repay_draw(&mut deps, &borrower_a, 0, 0);

    let b_after = utilized_amount(&deps, &borrower_b, 1);
    assert_eq!(
        b_before, b_after,
        "repaying borrower A must not change borrower B's utilization"
    );
    assert_eq!(
        utilized_amount(&deps, &borrower_a, 0),
        Uint128::zero(),
        "borrower A utilization must be zero after full repay"
    );
}
