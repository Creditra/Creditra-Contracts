// SPDX-License-Identifier: MIT

//! # Credit-line snapshot integration tests
//!
//! Covers [`QueryMsg::CreditLineSnapshot`] end-to-end through the contract
//! entrypoint.  Each sub-module targets a distinct concern:
//!
//! | Module | What it pins |
//! |---|---|
//! | `missing` | Returns `None` for unknown ids |
//! | `fresh` | Correct snapshot directly after `CreateCreditLine` |
//! | `draws` | `total_utilized` and `draws` list as draws are created / repaid |
//! | `active_flag` | Snapshot reflects `active == false` after manual deactivation |
//! | `health_factor` | `health_factor_bps` math at key utilisation points |
//! | `multi_collateral` | Multi-token collateral flows into snapshot fields |
//! | `isolation` | Multiple credit lines do not bleed into each other |

use cosmwasm_std::{
    from_json,
    testing::{message_info, mock_dependencies, mock_env, MockApi, MockQuerier, MockStorage},
    Addr, OwnedDeps, Uint128,
};
use creditra_credit::{
    contract::{execute, instantiate, query},
    msg::{CreditLineSnapshotResponse, ExecuteMsg, InstantiateMsg, QueryMsg},
    state::CREDIT_LINES,
};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn creator(deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>) -> Addr {
    deps.api.addr_make("creator")
}

fn borrower(deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>) -> Addr {
    deps.api.addr_make("borrower")
}

fn setup(deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>) {
    let env = mock_env();
    let creator_addr = creator(deps);
    let info = message_info(&creator_addr, &[]);
    let msg = InstantiateMsg {
        owner: creator_addr.to_string(),
    };
    instantiate(deps.as_mut(), env, info, msg).unwrap();
}

/// Create a credit line with the given parameters (creator-only, so `info.sender = creator`).
fn create_credit_line(
    deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
    collateral_amount: &str,
    credit_amount: &str,
) {
    let env = mock_env();
    let creator_addr = creator(deps);
    let info = message_info(&creator_addr, &[]);
    let msg = ExecuteMsg::CreateCreditLine {
        borrower: borrower(deps).to_string(),
        collateral_denom: "ucollateral".to_string(),
        collateral_amount: collateral_amount.to_string(),
        credit_denom: "ucredit".to_string(),
        credit_amount: credit_amount.to_string(),
    };
    execute(deps.as_mut(), env, info, msg).unwrap();
}

/// Create a draw against `credit_line_id` for `amount` of `denom`.
fn create_draw(
    deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
    credit_line_id: u64,
    amount: &str,
    denom: &str,
) {
    let env = mock_env();
    let borrower_addr = borrower(deps);
    let info = message_info(&borrower_addr, &[]);
    let msg = ExecuteMsg::CreateDraw {
        credit_line_id,
        amount: amount.to_string(),
        denom: denom.to_string(),
    };
    execute(deps.as_mut(), env, info, msg).unwrap();
}

/// Repay draw `draw_id` on `credit_line_id`.
fn repay_draw(
    deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
    credit_line_id: u64,
    draw_id: u64,
) {
    let env = mock_env();
    let borrower_addr = borrower(deps);
    let info = message_info(&borrower_addr, &[]);
    let msg = ExecuteMsg::RepayDraw {
        credit_line_id,
        draw_id,
    };
    execute(deps.as_mut(), env, info, msg).unwrap();
}

/// Query snapshot for `credit_line_id`; returns the deserialized Option.
fn snapshot(
    deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>,
    credit_line_id: u64,
) -> Option<CreditLineSnapshotResponse> {
    let env = mock_env();
    let msg = QueryMsg::CreditLineSnapshot { credit_line_id };
    let raw = query(deps.as_ref(), env, msg).unwrap();
    from_json(&raw).unwrap()
}

// ── Tests: missing credit line ────────────────────────────────────────────────

mod missing {
    use super::*;

    #[test]
    fn returns_none_for_unregistered_id() {
        let mut deps = mock_dependencies();
        setup(&mut deps);

        assert!(snapshot(&deps, 0).is_none());
    }

    #[test]
    fn returns_none_for_arbitrary_large_id() {
        let mut deps = mock_dependencies();
        setup(&mut deps);

        assert!(snapshot(&deps, 9999).is_none());
    }

    #[test]
    fn returns_some_after_credit_line_exists() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps, "1000", "500");

        assert!(snapshot(&deps, 0).is_some());
    }
}

// ── Tests: fresh credit line (no draws) ──────────────────────────────────────

mod fresh {
    use super::*;

    fn fresh_snap(
        deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>,
    ) -> CreditLineSnapshotResponse {
        snapshot(deps, 0).unwrap()
    }

    #[test]
    fn credit_line_id_is_correct() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps, "1000", "500");

        assert_eq!(fresh_snap(&deps).credit_line_id, 0);
    }

    #[test]
    fn borrower_matches() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps, "1000", "500");

        let borrower_addr = borrower(&deps);
        assert_eq!(fresh_snap(&deps).borrower, borrower_addr);
    }

    #[test]
    fn collateral_fields_match() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps, "2000", "500");

        let snap = fresh_snap(&deps);
        assert_eq!(snap.collateral_denom, "ucollateral");
        assert_eq!(snap.collateral_amount, Uint128::new(2000));
    }

    #[test]
    fn credit_fields_match() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps, "1000", "750");

        let snap = fresh_snap(&deps);
        assert_eq!(snap.credit_denom, "ucredit");
        assert_eq!(snap.credit_amount, Uint128::new(750));
    }

    #[test]
    fn active_is_true_on_creation() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps, "1000", "500");

        assert!(fresh_snap(&deps).active);
    }

    #[test]
    fn total_utilized_is_zero_with_no_draws() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps, "1000", "500");

        assert_eq!(fresh_snap(&deps).total_utilized, Uint128::zero());
    }

    #[test]
    fn draws_list_is_empty_with_no_draws() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps, "1000", "500");

        assert!(fresh_snap(&deps).draws.is_empty());
    }

    #[test]
    fn health_factor_is_u32_max_with_no_draws() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps, "1000", "500");

        assert_eq!(fresh_snap(&deps).health_factor_bps, u32::MAX);
    }

    #[test]
    fn multi_collateral_is_empty_with_no_deposits() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps, "1000", "500");

        assert!(fresh_snap(&deps).multi_collateral.is_empty());
    }

    #[test]
    fn weighted_collateral_total_is_zero_with_no_multi_collateral() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps, "1000", "500");

        assert_eq!(fresh_snap(&deps).weighted_collateral_total, Uint128::zero());
    }
}

// ── Tests: draws ─────────────────────────────────────────────────────────────

mod draws {
    use super::*;

    #[test]
    fn single_draw_appears_in_snapshot() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps, "1000", "500");
        create_draw(&mut deps, 0, "100", "ucredit");

        let snap = snapshot(&deps, 0).unwrap();
        assert_eq!(snap.draws.len(), 1);
        assert_eq!(snap.draws[0].draw_id, 0);
        assert_eq!(snap.draws[0].amount, Uint128::new(100));
        assert_eq!(snap.draws[0].denom, "ucredit");
        assert!(!snap.draws[0].repaid);
    }

    #[test]
    fn multiple_draws_all_appear() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps, "1000", "500");
        create_draw(&mut deps, 0, "100", "ucredit");
        create_draw(&mut deps, 0, "200", "ucredit");
        create_draw(&mut deps, 0, "50", "ucredit");

        let snap = snapshot(&deps, 0).unwrap();
        assert_eq!(snap.draws.len(), 3);
        assert_eq!(snap.draws[0].amount, Uint128::new(100));
        assert_eq!(snap.draws[1].amount, Uint128::new(200));
        assert_eq!(snap.draws[2].amount, Uint128::new(50));
    }

    #[test]
    fn total_utilized_sums_unrepaid_draws() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps, "1000", "500");
        create_draw(&mut deps, 0, "100", "ucredit");
        create_draw(&mut deps, 0, "200", "ucredit");

        let snap = snapshot(&deps, 0).unwrap();
        assert_eq!(snap.total_utilized, Uint128::new(300));
    }

    #[test]
    fn repaid_draw_excluded_from_total_utilized() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps, "1000", "500");
        create_draw(&mut deps, 0, "100", "ucredit");
        create_draw(&mut deps, 0, "200", "ucredit");
        repay_draw(&mut deps, 0, 0); // repay the 100-draw

        let snap = snapshot(&deps, 0).unwrap();
        // Only the 200 draw remains outstanding.
        assert_eq!(snap.total_utilized, Uint128::new(200));
    }

    #[test]
    fn repaid_draw_still_present_in_draws_list() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps, "1000", "500");
        create_draw(&mut deps, 0, "100", "ucredit");
        repay_draw(&mut deps, 0, 0);

        let snap = snapshot(&deps, 0).unwrap();
        assert_eq!(snap.draws.len(), 1);
        assert!(snap.draws[0].repaid);
    }

    #[test]
    fn total_utilized_zero_when_all_draws_repaid() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps, "1000", "500");
        create_draw(&mut deps, 0, "100", "ucredit");
        create_draw(&mut deps, 0, "200", "ucredit");
        repay_draw(&mut deps, 0, 0);
        repay_draw(&mut deps, 0, 1);

        let snap = snapshot(&deps, 0).unwrap();
        assert_eq!(snap.total_utilized, Uint128::zero());
        assert_eq!(snap.health_factor_bps, u32::MAX);
    }

    #[test]
    fn drawn_by_matches_borrower() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps, "1000", "500");
        create_draw(&mut deps, 0, "100", "ucredit");

        let borrower_addr = borrower(&deps);
        let snap = snapshot(&deps, 0).unwrap();
        assert_eq!(snap.draws[0].drawn_by, borrower_addr);
    }
}

// ── Tests: active flag ────────────────────────────────────────────────────────

mod active_flag {
    use super::*;

    #[test]
    fn active_reflects_manual_deactivation() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps, "1000", "500");

        // Manually deactivate the credit line via storage.
        let mut cl = CREDIT_LINES.load(deps.as_ref().storage, 0).unwrap();
        cl.active = false;
        CREDIT_LINES.save(deps.as_mut().storage, 0, &cl).unwrap();

        let snap = snapshot(&deps, 0).unwrap();
        assert!(!snap.active);
    }

    #[test]
    fn active_true_for_newly_created_line() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps, "1000", "500");

        assert!(snapshot(&deps, 0).unwrap().active);
    }
}

// ── Tests: health factor ──────────────────────────────────────────────────────

mod health_factor {
    use super::*;

    #[test]
    fn u32_max_when_no_debt() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps, "1000", "500");

        assert_eq!(snapshot(&deps, 0).unwrap().health_factor_bps, u32::MAX);
    }

    #[test]
    fn u32_max_restored_after_full_repayment() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps, "1000", "500");
        create_draw(&mut deps, 0, "100", "ucredit");
        repay_draw(&mut deps, 0, 0);

        assert_eq!(snapshot(&deps, 0).unwrap().health_factor_bps, u32::MAX);
    }

    #[test]
    fn health_factor_computed_correctly() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        // collateral = 1000, credit = 500
        create_credit_line(&mut deps, "1000", "500");
        // draw 100 → utilization = 100
        create_draw(&mut deps, 0, "100", "ucredit");

        // health = 1000 * 10_000 / 100 = 100_000 bps
        let snap = snapshot(&deps, 0).unwrap();
        assert_eq!(snap.health_factor_bps, 100_000);
    }

    #[test]
    fn health_factor_at_par_when_fully_utilized() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        // collateral == credit == 500 → at-par collateralization
        create_credit_line(&mut deps, "500", "500");
        create_draw(&mut deps, 0, "500", "ucredit");

        // health = 500 * 10_000 / 500 = 10_000 bps (exactly 1× collateral)
        let snap = snapshot(&deps, 0).unwrap();
        assert_eq!(snap.health_factor_bps, 10_000);
    }

    #[test]
    fn health_factor_zero_when_collateral_zero() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps, "0", "500");
        create_draw(&mut deps, 0, "100", "ucredit");

        let snap = snapshot(&deps, 0).unwrap();
        assert_eq!(snap.health_factor_bps, 0);
    }

    #[test]
    fn health_factor_decreases_as_utilization_rises() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps, "1000", "500");
        create_draw(&mut deps, 0, "100", "ucredit");

        let hf1 = snapshot(&deps, 0).unwrap().health_factor_bps;

        create_draw(&mut deps, 0, "400", "ucredit");
        let hf2 = snapshot(&deps, 0).unwrap().health_factor_bps;

        assert!(
            hf1 > hf2,
            "hf should decrease as utilization rises: {hf1} vs {hf2}"
        );
    }

    #[test]
    fn health_factor_caps_at_u32_max_not_overflow() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        // Very large collateral, tiny utilization → result > u32::MAX → caps at u32::MAX.
        create_credit_line(&mut deps, "1000000000000", "500");
        create_draw(&mut deps, 0, "1", "ucredit");

        // 1_000_000_000_000 * 10_000 / 1 = 10^16, which overflows u32 → should be u32::MAX.
        let snap = snapshot(&deps, 0).unwrap();
        assert_eq!(snap.health_factor_bps, u32::MAX);
    }
}

// ── Tests: multi-collateral ───────────────────────────────────────────────────

mod multi_collateral {
    use super::*;
    use creditra_credit::contract::execute;
    use creditra_credit::msg::ExecuteMsg;

    fn add_collateral_token(
        deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
        denom: &str,
        risk_weight_bps: u32,
    ) {
        let env = mock_env();
        let creator_addr = creator(deps);
        let info = message_info(&creator_addr, &[]);
        let msg = ExecuteMsg::AddCollateralToken {
            denom: denom.to_string(),
            risk_weight_bps,
        };
        execute(deps.as_mut(), env, info, msg).unwrap();
    }

    fn deposit_collateral(
        deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
        borrower_str: &str,
        denom: &str,
        amount: &str,
    ) {
        let env = mock_env();
        let creator_addr = creator(deps);
        let info = message_info(&creator_addr, &[]);
        let msg = ExecuteMsg::DepositCollateral {
            borrower: borrower_str.to_string(),
            denom: denom.to_string(),
            amount: amount.to_string(),
        };
        execute(deps.as_mut(), env, info, msg).unwrap();
    }

    #[test]
    fn multi_collateral_appears_in_snapshot() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps, "1000", "500");

        add_collateral_token(&mut deps, "ubtc", 8_000);
        let b = borrower(&deps).to_string();
        deposit_collateral(&mut deps, &b, "ubtc", "50");

        let snap = snapshot(&deps, 0).unwrap();
        assert_eq!(snap.multi_collateral.len(), 1);
        assert_eq!(snap.multi_collateral[0].denom, "ubtc");
        assert_eq!(snap.multi_collateral[0].amount, Uint128::new(50));
        assert_eq!(snap.multi_collateral[0].risk_weight_bps, 8_000);
    }

    #[test]
    fn multi_collateral_weighted_total_correct() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps, "0", "500");

        // risk_weight = 5_000 bps = 50 %
        add_collateral_token(&mut deps, "ueth", 5_000);
        let b = borrower(&deps).to_string();
        deposit_collateral(&mut deps, &b, "ueth", "2000");

        let snap = snapshot(&deps, 0).unwrap();
        // weighted_total = 2000 * 5_000 / 10_000 = 1000
        assert_eq!(snap.weighted_collateral_total, Uint128::new(1000));
    }

    #[test]
    fn multi_collateral_contributes_to_health_factor() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        // primary collateral = 0, so health only comes from multi-collateral.
        create_credit_line(&mut deps, "0", "500");
        create_draw(&mut deps, 0, "100", "ucredit");

        // Before any multi-collateral: health = 0 (collateral zero, debt > 0).
        assert_eq!(snapshot(&deps, 0).unwrap().health_factor_bps, 0);

        add_collateral_token(&mut deps, "ueth", 10_000); // full weight
        let b = borrower(&deps).to_string();
        deposit_collateral(&mut deps, &b, "ueth", "200");

        // weighted_total = 200; health = 200 * 10_000 / 100 = 20_000 bps.
        let snap = snapshot(&deps, 0).unwrap();
        assert_eq!(snap.health_factor_bps, 20_000);
    }

    #[test]
    fn multiple_multi_collateral_tokens() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps, "1000", "500");

        add_collateral_token(&mut deps, "ubtc", 9_000);
        add_collateral_token(&mut deps, "ueth", 7_000);
        let b = borrower(&deps).to_string();
        deposit_collateral(&mut deps, &b, "ubtc", "100");
        deposit_collateral(&mut deps, &b, "ueth", "100");

        let snap = snapshot(&deps, 0).unwrap();
        assert_eq!(snap.multi_collateral.len(), 2);

        // weighted = 100 * 9_000/10_000 + 100 * 7_000/10_000 = 90 + 70 = 160
        assert_eq!(snap.weighted_collateral_total, Uint128::new(160));
    }
}

// ── Tests: isolation between credit lines ─────────────────────────────────────

mod isolation {
    use super::*;

    fn create_credit_line_for(
        deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
        collateral_amount: &str,
        credit_amount: &str,
    ) {
        let env = mock_env();
        let creator_addr = creator(deps);
        let info = message_info(&creator_addr, &[]);
        let msg = ExecuteMsg::CreateCreditLine {
            borrower: borrower(deps).to_string(),
            collateral_denom: "ucollateral".to_string(),
            collateral_amount: collateral_amount.to_string(),
            credit_denom: "ucredit".to_string(),
            credit_amount: credit_amount.to_string(),
        };
        execute(deps.as_mut(), env, info, msg).unwrap();
    }

    #[test]
    fn draws_on_one_line_do_not_appear_in_other() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line_for(&mut deps, "1000", "500");
        create_credit_line_for(&mut deps, "2000", "1000");

        create_draw(&mut deps, 0, "100", "ucredit");
        create_draw(&mut deps, 1, "300", "ucredit");

        let snap0 = snapshot(&deps, 0).unwrap();
        let snap1 = snapshot(&deps, 1).unwrap();

        assert_eq!(snap0.draws.len(), 1);
        assert_eq!(snap0.total_utilized, Uint128::new(100));

        assert_eq!(snap1.draws.len(), 1);
        assert_eq!(snap1.total_utilized, Uint128::new(300));
    }

    #[test]
    fn credit_line_id_matches_for_each_snapshot() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line_for(&mut deps, "500", "250");
        create_credit_line_for(&mut deps, "800", "400");

        assert_eq!(snapshot(&deps, 0).unwrap().credit_line_id, 0);
        assert_eq!(snapshot(&deps, 1).unwrap().credit_line_id, 1);
    }

    #[test]
    fn repaying_draw_on_one_line_does_not_affect_other() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line_for(&mut deps, "1000", "500");
        create_credit_line_for(&mut deps, "1000", "500");

        create_draw(&mut deps, 0, "200", "ucredit");
        create_draw(&mut deps, 1, "200", "ucredit");

        repay_draw(&mut deps, 0, 0);

        let snap0 = snapshot(&deps, 0).unwrap();
        let snap1 = snapshot(&deps, 1).unwrap();

        assert_eq!(snap0.total_utilized, Uint128::zero());
        assert_eq!(snap0.health_factor_bps, u32::MAX);

        assert_eq!(snap1.total_utilized, Uint128::new(200));
        assert!(snap1.health_factor_bps < u32::MAX);
    }

    #[test]
    fn credit_amount_fields_differ_per_line() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line_for(&mut deps, "1000", "300");
        create_credit_line_for(&mut deps, "2000", "900");

        assert_eq!(snapshot(&deps, 0).unwrap().credit_amount, Uint128::new(300));
        assert_eq!(snapshot(&deps, 1).unwrap().credit_amount, Uint128::new(900));
    }
}

mod consistency {
    use super::*;

    fn add_collateral_token(
        deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
        denom: &str,
        risk_weight_bps: u32,
    ) {
        let env = mock_env();
        let creator_addr = creator(deps);
        let info = message_info(&creator_addr, &[]);
        let msg = ExecuteMsg::AddCollateralToken {
            denom: denom.to_string(),
            risk_weight_bps,
        };
        execute(deps.as_mut(), env, info, msg).unwrap();
    }

    fn deposit_collateral_for(
        deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
        borrower_str: &str,
        denom: &str,
        amount: &str,
    ) {
        let env = mock_env();
        let creator_addr = creator(deps);
        let info = message_info(&creator_addr, &[]);
        let msg = ExecuteMsg::DepositCollateral {
            borrower: borrower_str.to_string(),
            denom: denom.to_string(),
            amount: amount.to_string(),
        };
        execute(deps.as_mut(), env, info, msg).unwrap();
    }

    fn create_credit_line_for_borrower(
        deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
        borrower_addr: &Addr,
        collateral_denom: &str,
        collateral_amount: &str,
        credit_denom: &str,
        credit_amount: &str,
    ) {
        let env = mock_env();
        let creator_addr = creator(deps);
        let info = message_info(&creator_addr, &[]);
        let msg = ExecuteMsg::CreateCreditLine {
            borrower: borrower_addr.to_string(),
            collateral_denom: collateral_denom.to_string(),
            collateral_amount: collateral_amount.to_string(),
            credit_denom: credit_denom.to_string(),
            credit_amount: credit_amount.to_string(),
        };
        execute(deps.as_mut(), env, info, msg).unwrap();
    }

    fn create_draw_for_borrower(
        deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
        borrower_addr: &Addr,
        credit_line_id: u64,
        amount: &str,
        denom: &str,
    ) {
        let env = mock_env();
        let info = message_info(borrower_addr, &[]);
        let msg = ExecuteMsg::CreateDraw {
            credit_line_id,
            amount: amount.to_string(),
            denom: denom.to_string(),
        };
        execute(deps.as_mut(), env, info, msg).unwrap();
    }

    fn repay_draw_for_borrower(
        deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
        borrower_addr: &Addr,
        credit_line_id: u64,
        draw_id: u64,
    ) {
        let env = mock_env();
        let info = message_info(borrower_addr, &[]);
        let msg = ExecuteMsg::RepayDraw {
            credit_line_id,
            draw_id,
        };
        execute(deps.as_mut(), env, info, msg).unwrap();
    }

    fn deactivate_credit_line(
        deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
        credit_line_id: u64,
    ) {
        let mut cl = CREDIT_LINES
            .load(deps.as_ref().storage, credit_line_id)
            .unwrap();
        cl.active = false;
        CREDIT_LINES
            .save(deps.as_mut().storage, credit_line_id, &cl)
            .unwrap();
    }

    fn assert_snapshot_internally_consistent(snap: &CreditLineSnapshotResponse) {
        assert!(snap.is_internally_consistent());

        let expected_utilized: Uint128 = snap
            .draws
            .iter()
            .filter(|d| !d.repaid)
            .map(|d| d.amount)
            .fold(Uint128::zero(), |acc, x| acc + x);
        assert_eq!(snap.total_utilized, expected_utilized);

        let expected_weighted: Uint128 = snap
            .multi_collateral
            .iter()
            .map(|e| e.amount * Uint128::from(e.risk_weight_bps) / Uint128::from(10_000u32))
            .fold(Uint128::zero(), |acc, x| acc + x);
        assert_eq!(snap.weighted_collateral_total, expected_weighted);

        let total_drawn = snap.total_drawn().unwrap();
        let total_repaid = snap.total_repaid().unwrap();
        assert_eq!(total_drawn, snap.total_utilized + total_repaid);

        let effective_collateral = snap.collateral_amount + snap.weighted_collateral_total;
        let expected_hf = creditra_credit::math_utils::compute_health_factor_bps(
            effective_collateral,
            snap.total_utilized,
        );
        assert_eq!(snap.health_factor_bps, expected_hf);
    }

    #[test]
    fn totals_equal_sum_of_included_draw_and_collateral_records() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let b = borrower(&deps);
        create_credit_line_for_borrower(&mut deps, &b, "ucollateral", "1000", "ucredit", "1000");

        create_draw_for_borrower(&mut deps, &b, 0, "150", "ucredit");
        create_draw_for_borrower(&mut deps, &b, 0, "250", "ucredit");
        create_draw_for_borrower(&mut deps, &b, 0, "300", "ucredit");

        add_collateral_token(&mut deps, "ubtc", 8_000);
        add_collateral_token(&mut deps, "ueth", 5_000);
        deposit_collateral_for(&mut deps, b.as_ref(), "ubtc", "100");
        deposit_collateral_for(&mut deps, b.as_ref(), "ueth", "200");

        let snap = snapshot(&deps, 0).unwrap();
        assert_eq!(snap.draws.len(), 3);
        assert_eq!(snap.multi_collateral.len(), 2);
        assert_snapshot_internally_consistent(&snap);
        assert_eq!(snap.total_utilized, Uint128::new(700));
        assert_eq!(snap.weighted_collateral_total, Uint128::new(180));

        repay_draw_for_borrower(&mut deps, &b, 0, 1);
        let snap_after = snapshot(&deps, 0).unwrap();
        assert_eq!(snap_after.draws.len(), 3);
        assert_snapshot_internally_consistent(&snap_after);
        assert_eq!(snap_after.total_utilized, Uint128::new(450));
        assert_eq!(snap_after.total_repaid().unwrap(), Uint128::new(250));
        assert_eq!(snap_after.total_drawn().unwrap(), Uint128::new(700));
    }

    #[test]
    fn repaid_draws_represented_consistently_with_flags_and_totals() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let b = borrower(&deps);
        create_credit_line_for_borrower(&mut deps, &b, "ucollateral", "2000", "ucredit", "1000");

        create_draw_for_borrower(&mut deps, &b, 0, "200", "ucredit");
        create_draw_for_borrower(&mut deps, &b, 0, "300", "ucredit");

        let snap0 = snapshot(&deps, 0).unwrap();
        assert!(!snap0.draws[0].repaid);
        assert!(!snap0.draws[1].repaid);
        assert_eq!(snap0.total_utilized, Uint128::new(500));
        assert_snapshot_internally_consistent(&snap0);

        repay_draw_for_borrower(&mut deps, &b, 0, 0);
        let snap1 = snapshot(&deps, 0).unwrap();
        assert!(snap1.draws[0].repaid);
        assert!(!snap1.draws[1].repaid);
        assert_eq!(snap1.total_utilized, Uint128::new(300));
        assert_eq!(snap1.total_repaid().unwrap(), Uint128::new(200));
        assert_snapshot_internally_consistent(&snap1);

        repay_draw_for_borrower(&mut deps, &b, 0, 1);
        let snap2 = snapshot(&deps, 0).unwrap();
        assert!(snap2.draws[0].repaid);
        assert!(snap2.draws[1].repaid);
        assert_eq!(snap2.total_utilized, Uint128::zero());
        assert_eq!(snap2.total_repaid().unwrap(), Uint128::new(500));
        assert_eq!(snap2.health_factor_bps, u32::MAX);
        assert_snapshot_internally_consistent(&snap2);
    }

    #[test]
    fn health_factor_boundaries_are_deterministic() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let b = borrower(&deps);

        create_credit_line_for_borrower(&mut deps, &b, "ucollateral", "0", "ucredit", "1000");
        let snap_zero_debt_zero_collateral = snapshot(&deps, 0).unwrap();
        assert_eq!(snap_zero_debt_zero_collateral.health_factor_bps, u32::MAX);
        assert_snapshot_internally_consistent(&snap_zero_debt_zero_collateral);

        create_draw_for_borrower(&mut deps, &b, 0, "500", "ucredit");
        let snap_zero_collateral_with_debt = snapshot(&deps, 0).unwrap();
        assert_eq!(snap_zero_collateral_with_debt.health_factor_bps, 0);
        assert_snapshot_internally_consistent(&snap_zero_collateral_with_debt);

        repay_draw_for_borrower(&mut deps, &b, 0, 0);

        create_credit_line_for_borrower(&mut deps, &b, "ucollateral", "1000", "ucredit", "1000");
        create_draw_for_borrower(&mut deps, &b, 1, "1000", "ucredit");
        let snap_par = snapshot(&deps, 1).unwrap();
        assert_eq!(snap_par.health_factor_bps, 10_000);
        assert_snapshot_internally_consistent(&snap_par);

        create_credit_line_for_borrower(&mut deps, &b, "ucollateral", "999", "ucredit", "1000");
        create_draw_for_borrower(&mut deps, &b, 2, "1000", "ucredit");
        let snap_under = snapshot(&deps, 2).unwrap();
        assert_eq!(snap_under.health_factor_bps, 9_990);
        assert_snapshot_internally_consistent(&snap_under);

        create_credit_line_for_borrower(&mut deps, &b, "ucollateral", "1001", "ucredit", "1000");
        create_draw_for_borrower(&mut deps, &b, 3, "1000", "ucredit");
        let snap_over = snapshot(&deps, 3).unwrap();
        assert_eq!(snap_over.health_factor_bps, 10_010);
        assert_snapshot_internally_consistent(&snap_over);

        create_credit_line_for_borrower(
            &mut deps,
            &b,
            "ucollateral",
            "1000000000000000",
            "ucredit",
            "1000",
        );
        create_draw_for_borrower(&mut deps, &b, 4, "1", "ucredit");
        let snap_huge = snapshot(&deps, 4).unwrap();
        assert_eq!(snap_huge.health_factor_bps, u32::MAX);
        assert_snapshot_internally_consistent(&snap_huge);
    }

    #[test]
    fn empty_credit_line_cases() {
        let mut deps = mock_dependencies();
        setup(&mut deps);

        assert!(snapshot(&deps, 0).is_none());
        assert!(snapshot(&deps, 42).is_none());

        create_credit_line(&mut deps, "0", "0");
        let snap = snapshot(&deps, 0).unwrap();
        assert_eq!(snap.credit_line_id, 0);
        assert_eq!(snap.collateral_amount, Uint128::zero());
        assert_eq!(snap.credit_amount, Uint128::zero());
        assert_eq!(snap.total_utilized, Uint128::zero());
        assert!(snap.draws.is_empty());
        assert!(snap.multi_collateral.is_empty());
        assert_eq!(snap.weighted_collateral_total, Uint128::zero());
        assert_eq!(snap.health_factor_bps, u32::MAX);
        assert_snapshot_internally_consistent(&snap);
    }

    #[test]
    fn active_credit_line_lifecycle() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let b = borrower(&deps);
        create_credit_line_for_borrower(&mut deps, &b, "ucollateral", "1000", "ucredit", "800");

        create_draw_for_borrower(&mut deps, &b, 0, "200", "ucredit");
        let snap1 = snapshot(&deps, 0).unwrap();
        assert!(snap1.active);
        assert_eq!(snap1.total_utilized, Uint128::new(200));
        assert_snapshot_internally_consistent(&snap1);

        create_draw_for_borrower(&mut deps, &b, 0, "300", "ucredit");
        let snap2 = snapshot(&deps, 0).unwrap();
        assert!(snap2.active);
        assert_eq!(snap2.total_utilized, Uint128::new(500));
        assert_snapshot_internally_consistent(&snap2);
    }

    #[test]
    fn inactive_credit_line_retains_internal_consistency() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let b = borrower(&deps);
        create_credit_line_for_borrower(&mut deps, &b, "ucollateral", "1000", "ucredit", "500");
        create_draw_for_borrower(&mut deps, &b, 0, "250", "ucredit");

        deactivate_credit_line(&mut deps, 0);

        let snap_inactive_debt = snapshot(&deps, 0).unwrap();
        assert!(!snap_inactive_debt.active);
        assert_eq!(snap_inactive_debt.draws.len(), 1);
        assert_eq!(snap_inactive_debt.total_utilized, Uint128::new(250));
        assert_snapshot_internally_consistent(&snap_inactive_debt);

        repay_draw_for_borrower(&mut deps, &b, 0, 0);
        let snap_inactive_repaid = snapshot(&deps, 0).unwrap();
        assert!(!snap_inactive_repaid.active);
        assert_eq!(snap_inactive_repaid.total_utilized, Uint128::zero());
        assert_eq!(snap_inactive_repaid.health_factor_bps, u32::MAX);
        assert_snapshot_internally_consistent(&snap_inactive_repaid);
    }

    #[test]
    fn concurrent_updates_and_interleaved_operations() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let b = borrower(&deps);
        create_credit_line_for_borrower(&mut deps, &b, "ucollateral", "2000", "ucredit", "1000");

        add_collateral_token(&mut deps, "ubtc", 7_500);

        let steps = [
            ("draw", "100"),
            ("deposit", "50"),
            ("draw", "200"),
            ("repay", "0"),
            ("draw", "150"),
            ("deposit", "50"),
            ("repay", "1"),
        ];

        for (action, arg) in steps {
            match action {
                "draw" => create_draw_for_borrower(&mut deps, &b, 0, arg, "ucredit"),
                "deposit" => deposit_collateral_for(&mut deps, b.as_ref(), "ubtc", arg),
                "repay" => {
                    let did: u64 = arg.parse().unwrap();
                    repay_draw_for_borrower(&mut deps, &b, 0, did);
                }
                _ => unreachable!(),
            }

            let snap = snapshot(&deps, 0).unwrap();
            assert_snapshot_internally_consistent(&snap);
        }

        deactivate_credit_line(&mut deps, 0);
        let snap_final = snapshot(&deps, 0).unwrap();
        assert!(!snap_final.active);
        assert_snapshot_internally_consistent(&snap_final);
    }

    #[test]
    fn multi_borrower_concurrent_isolation() {
        let mut deps = mock_dependencies();
        setup(&mut deps);

        let b1 = deps.api.addr_make("borrower_alpha");
        let b2 = deps.api.addr_make("borrower_beta");

        create_credit_line_for_borrower(&mut deps, &b1, "ucollateral", "1000", "ucredit", "500");
        create_credit_line_for_borrower(&mut deps, &b2, "ucollateral", "2000", "ucredit", "1000");

        create_draw_for_borrower(&mut deps, &b1, 0, "100", "ucredit");
        create_draw_for_borrower(&mut deps, &b2, 1, "400", "ucredit");
        create_draw_for_borrower(&mut deps, &b1, 0, "200", "ucredit");

        let snap1 = snapshot(&deps, 0).unwrap();
        let snap2 = snapshot(&deps, 1).unwrap();

        assert_snapshot_internally_consistent(&snap1);
        assert_snapshot_internally_consistent(&snap2);

        assert_eq!(snap1.credit_line_id, 0);
        assert_eq!(snap1.borrower, b1);
        assert_eq!(snap1.draws.len(), 2);
        assert_eq!(snap1.total_utilized, Uint128::new(300));

        assert_eq!(snap2.credit_line_id, 1);
        assert_eq!(snap2.borrower, b2);
        assert_eq!(snap2.draws.len(), 1);
        assert_eq!(snap2.total_utilized, Uint128::new(400));

        repay_draw_for_borrower(&mut deps, &b1, 0, 0);

        let snap1_after = snapshot(&deps, 0).unwrap();
        let snap2_after = snapshot(&deps, 1).unwrap();

        assert_snapshot_internally_consistent(&snap1_after);
        assert_snapshot_internally_consistent(&snap2_after);

        assert_eq!(snap1_after.total_utilized, Uint128::new(200));
        assert_eq!(snap2_after.total_utilized, Uint128::new(400));
    }
}
