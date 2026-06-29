use cosmwasm_std::{Deps, StdResult};

use crate::error::ContractError;
use crate::msg::DrawAuditTrailResponse;
use crate::state::{
    Draw, DrawAuditEntry, CREDIT_LINES, DRAW_AUDIT, DRAW_AUDIT_COUNT, DRAW_COUNT, DRAWS,
};

/// Returns the full audit trail for one or all draws on a given credit line.
///
/// When `draw_id` is `Some(id)`, returns the audit trail for that specific draw.
/// When `draw_id` is `None`, returns audit trails for *all* draws on the credit line.
///
/// # Errors
///
/// Returns `ContractError::CreditLineNotFound` if the credit line does not exist.
/// Returns `ContractError::DrawNotFound` if a specific `draw_id` is requested but not found.
pub fn query_draw_audit_trail(
    deps: Deps,
    credit_line_id: u64,
    draw_id: Option<u64>,
) -> Result<Vec<DrawAuditTrailResponse>, ContractError> {
    let _ = CREDIT_LINES
        .may_load(deps.storage, credit_line_id)?
        .ok_or(ContractError::CreditLineNotFound(credit_line_id))?;

    match draw_id {
        Some(did) => {
            ensure_draw_exists(deps, credit_line_id, did)?;
            let resp = build_response(deps, credit_line_id, did)?;
            Ok(vec![resp])
        }
        None => {
            let draw_count = DRAW_COUNT.may_load(deps.storage, credit_line_id)?.unwrap_or(0);
            let mut responses = Vec::with_capacity(draw_count as usize);
            for did in 0..draw_count {
                responses.push(build_response(deps, credit_line_id, did)?);
            }
            Ok(responses)
        }
    }
}

fn ensure_draw_exists(deps: Deps, credit_line_id: u64, draw_id: u64) -> Result<(), ContractError> {
    DRAWS
        .may_load(deps.storage, (credit_line_id, draw_id))?
        .ok_or(ContractError::DrawNotFound(draw_id, credit_line_id))?;
    Ok(())
}

fn build_response(
    deps: Deps,
    credit_line_id: u64,
    draw_id: u64,
) -> StdResult<DrawAuditTrailResponse> {
    let draw: Draw = DRAWS.load(deps.storage, (credit_line_id, draw_id))?;
    let audit_count = DRAW_AUDIT_COUNT
        .may_load(deps.storage, (credit_line_id, draw_id))?
        .unwrap_or(0);

    let mut events = Vec::with_capacity(audit_count as usize);
    for seq in 0..audit_count {
        let entry: DrawAuditEntry =
            DRAW_AUDIT.load(deps.storage, (credit_line_id, draw_id, seq))?;
        events.push(entry.into_event());
    }

    Ok(DrawAuditTrailResponse {
        credit_line_id,
        draw_id,
        draw_amount: draw.amount.to_string(),
        draw_denom: draw.denom,
        drawn_at: draw.drawn_at,
        drawn_by: draw.drawn_by,
        repaid: draw.repaid,
        events,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::query;
    use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};
    use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env, MockApi, MockQuerier, MockStorage};
    use cosmwasm_std::{from_json, Addr, OwnedDeps};

    fn creator(deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>) -> Addr {
        deps.api.addr_make("creator")
    }

    fn borrower(deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>) -> Addr {
        deps.api.addr_make("borrower")
    }

    fn setup_contract(deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>) {
        let env = mock_env();
        let creator_addr = creator(deps);
        let info = message_info(&creator_addr, &[]);
        let msg = InstantiateMsg {
            owner: creator_addr.to_string(),
        };
        crate::contract::instantiate(deps.as_mut(), env, info, msg).unwrap();
    }

    fn create_credit_line(deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>) {
        let env = mock_env();
        let creator_addr = creator(deps);
        let info = message_info(&creator_addr, &[]);
        let msg = ExecuteMsg::CreateCreditLine {
            borrower: borrower(deps).to_string(),
            collateral_denom: "ucollateral".to_string(),
            collateral_amount: "1000".to_string(),
            credit_denom: "ucredit".to_string(),
            credit_amount: "500".to_string(),
        };
        crate::contract::execute(deps.as_mut(), env, info, msg).unwrap();
    }

    fn create_draw(
        deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
        credit_line_id: u64,
        amount: &str,
    ) {
        let env = mock_env();
        let borrower_addr = borrower(deps);
        let info = message_info(&borrower_addr, &[]);
        let msg = ExecuteMsg::CreateDraw {
            credit_line_id,
            amount: amount.to_string(),
            denom: "ucredit".to_string(),
        };
        crate::contract::execute(deps.as_mut(), env, info, msg).unwrap();
    }

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
        crate::contract::execute(deps.as_mut(), env, info, msg).unwrap();
    }

    fn add_audit_memo(
        deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
        credit_line_id: u64,
        draw_id: u64,
        memo: &str,
    ) {
        let env = mock_env();
        let creator_addr = creator(deps);
        let info = message_info(&creator_addr, &[]);
        let msg = ExecuteMsg::AddAuditMemo {
            credit_line_id,
            draw_id,
            memo: memo.to_string(),
        };
        crate::contract::execute(deps.as_mut(), env, info, msg).unwrap();
    }

    fn query_audit(
        deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>,
        credit_line_id: u64,
        draw_id: Option<u64>,
    ) -> Vec<DrawAuditTrailResponse> {
        let env = mock_env();
        let msg = QueryMsg::DrawAuditTrail {
            credit_line_id,
            draw_id,
        };
        let raw = query(deps.as_ref(), env, msg).unwrap();
        from_json(&raw).unwrap()
    }

    mod query_draw_audit_trail {
        use super::*;

        #[test]
        fn returns_empty_for_credit_line_without_draws() {
            let mut deps = mock_dependencies();
            setup_contract(&mut deps);
            create_credit_line(&mut deps);

            let resp = query_audit(&deps, 0, None);
            assert!(resp.is_empty());
        }

        #[test]
        fn returns_audit_trail_for_single_draw() {
            let mut deps = mock_dependencies();
            setup_contract(&mut deps);
            create_credit_line(&mut deps);
            create_draw(&mut deps, 0, "100");

            let resp = query_audit(&deps, 0, Some(0));
            assert_eq!(resp.len(), 1);
            assert_eq!(resp[0].draw_amount, "100");
            assert_eq!(resp[0].draw_denom, "ucredit");
            assert!(!resp[0].repaid);
            assert_eq!(resp[0].draw_id, 0);
            assert_eq!(resp[0].credit_line_id, 0);
        }

        #[test]
        fn returns_all_draws_when_no_draw_id_specified() {
            let mut deps = mock_dependencies();
            setup_contract(&mut deps);
            create_credit_line(&mut deps);
            create_draw(&mut deps, 0, "100");
            create_draw(&mut deps, 0, "200");
            create_draw(&mut deps, 0, "300");

            let resp = query_audit(&deps, 0, None);
            assert_eq!(resp.len(), 3);
            assert_eq!(resp[0].draw_amount, "100");
            assert_eq!(resp[1].draw_amount, "200");
            assert_eq!(resp[2].draw_amount, "300");
        }

        #[test]
        fn includes_repaid_status() {
            let mut deps = mock_dependencies();
            setup_contract(&mut deps);
            create_credit_line(&mut deps);
            create_draw(&mut deps, 0, "100");
            assert!(!query_audit(&deps, 0, Some(0))[0].repaid);

            repay_draw(&mut deps, 0, 0);
            assert!(query_audit(&deps, 0, Some(0))[0].repaid);
        }

        #[test]
        fn includes_audit_events() {
            let mut deps = mock_dependencies();
            setup_contract(&mut deps);
            create_credit_line(&mut deps);
            create_draw(&mut deps, 0, "100");

            add_audit_memo(&mut deps, 0, 0, "First note");
            add_audit_memo(&mut deps, 0, 0, "Second note");

            let resp = query_audit(&deps, 0, Some(0));
            assert_eq!(resp[0].events.len(), 3);

            assert_eq!(resp[0].events[0].action, crate::state::DrawAction::DrawCreated);
            assert_eq!(resp[0].events[0].seq, 0);

            assert_eq!(resp[0].events[1].action, crate::state::DrawAction::MemoAdded);
            assert_eq!(resp[0].events[1].memo, "First note");
            assert_eq!(resp[0].events[1].seq, 1);

            assert_eq!(resp[0].events[2].action, crate::state::DrawAction::MemoAdded);
            assert_eq!(resp[0].events[2].memo, "Second note");
            assert_eq!(resp[0].events[2].seq, 2);
        }

        #[test]
        fn includes_repay_audit_event() {
            let mut deps = mock_dependencies();
            setup_contract(&mut deps);
            create_credit_line(&mut deps);
            create_draw(&mut deps, 0, "100");

            repay_draw(&mut deps, 0, 0);

            let resp = query_audit(&deps, 0, Some(0));
            assert_eq!(resp[0].events.len(), 2);
            assert_eq!(resp[0].events[0].action, crate::state::DrawAction::DrawCreated);
            assert_eq!(resp[0].events[1].action, crate::state::DrawAction::Repaid);
        }

        #[test]
        fn errors_on_nonexistent_credit_line() {
            let deps = mock_dependencies();
            let err = query_draw_audit_trail(deps.as_ref(), 999, None).unwrap_err();
            assert_eq!(err, ContractError::CreditLineNotFound(999));
        }

        #[test]
        fn errors_on_nonexistent_draw() {
            let mut deps = mock_dependencies();
            setup_contract(&mut deps);
            create_credit_line(&mut deps);

            let err = query_draw_audit_trail(deps.as_ref(), 0, Some(999)).unwrap_err();
            assert_eq!(err, ContractError::DrawNotFound(999, 0));
        }

        #[test]
        fn draw_has_draw_created_audit_event() {
            let mut deps = mock_dependencies();
            setup_contract(&mut deps);
            create_credit_line(&mut deps);
            create_draw(&mut deps, 0, "50");

            let resp = query_audit(&deps, 0, Some(0));
            assert_eq!(resp[0].events.len(), 1);
            assert_eq!(resp[0].events[0].action, crate::state::DrawAction::DrawCreated);
        }

        #[test]
        fn multiple_credit_lines_isolated() {
            let mut deps = mock_dependencies();
            setup_contract(&mut deps);
            create_credit_line(&mut deps);
            create_credit_line(&mut deps);
            create_draw(&mut deps, 0, "100");
            create_draw(&mut deps, 1, "200");

            let resp0 = query_audit(&deps, 0, None);
            let resp1 = query_audit(&deps, 1, None);
            assert_eq!(resp0.len(), 1);
            assert_eq!(resp1.len(), 1);
            assert_eq!(resp0[0].draw_amount, "100");
            assert_eq!(resp1[0].draw_amount, "200");
        }

        #[test]
        fn audit_events_include_correct_by_address() {
            let mut deps = mock_dependencies();
            setup_contract(&mut deps);
            create_credit_line(&mut deps);
            create_draw(&mut deps, 0, "100");
            add_audit_memo(&mut deps, 0, 0, "Owner note");

            let creator_str = creator(&deps).to_string();
            let borrower_str = borrower(&deps).to_string();

            let resp = query_audit(&deps, 0, Some(0));
            assert_eq!(resp[0].events[0].by.as_str(), borrower_str);
            assert_eq!(resp[0].events[1].by.as_str(), creator_str);
        }
    }
}
