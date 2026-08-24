use cosmwasm_std::{Deps, StdError, StdResult, Uint128};
use std::collections::BTreeMap;

use crate::collateral;
use crate::error::ContractError;
use crate::msg::{
    BorrowerHealthFactorResponse, CollateralEntryResponse, CreditLineHealthResponse,
    CreditLineSnapshotResponse, DenomReserve, DrawAuditTrailResponse, DrawSnapshotEntry,
    ProofOfReserveResponse,
};
use crate::state::{
    Draw, DrawAuditEntry, CREDIT_LINES, CREDIT_LINE_COUNT, DRAWS, DRAW_AUDIT, DRAW_AUDIT_COUNT,
    DRAW_COUNT,
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
            let draw_count = DRAW_COUNT
                .may_load(deps.storage, credit_line_id)?
                .unwrap_or(0);
            let mut responses = Vec::with_capacity(draw_count as usize);
            for did in 0..draw_count {
                responses.push(build_response(deps, credit_line_id, did)?);
            }
            Ok(responses)
        }
    }
}

/// Returns the proof-of-reserve metrics for protocol reserves.
///
/// Aggregate total collateral, credit limits, drawn balances, and repaid amounts
/// across credit lines and active draws. Optionally filters by a target asset denomination.
///
/// # Errors
///
/// Returns `ContractError` if storage reads fail or arithmetic overflow occurs.
pub fn query_proof_of_reserve(
    deps: Deps,
    denom_filter: Option<String>,
) -> Result<ProofOfReserveResponse, ContractError> {
    let credit_line_count = CREDIT_LINE_COUNT.may_load(deps.storage)?.unwrap_or(0);

    let mut total_credit_lines: u64 = 0;
    let mut active_credit_lines: u64 = 0;
    let mut total_collateral = Uint128::zero();
    let mut total_credit_limit = Uint128::zero();
    let mut total_drawn = Uint128::zero();
    let mut total_repaid = Uint128::zero();

    let mut denom_map: BTreeMap<String, DenomReserve> = BTreeMap::new();
    let filter = denom_filter.as_deref();

    for id in 0..credit_line_count {
        if let Some(cl) = CREDIT_LINES.may_load(deps.storage, id)? {
            let cl_matches = match filter {
                Some(target) => cl.collateral_denom == target || cl.credit_denom == target,
                None => true,
            };

            if cl_matches {
                total_credit_lines = total_credit_lines.saturating_add(1);
            }

            if cl.active {
                if cl_matches {
                    active_credit_lines = active_credit_lines.saturating_add(1);

                    total_collateral =
                        total_collateral
                            .checked_add(cl.collateral_amount)
                            .map_err(|_| {
                                ContractError::Std(cosmwasm_std::StdError::generic_err(
                                    "Collateral overflow",
                                ))
                            })?;
                    total_credit_limit =
                        total_credit_limit
                            .checked_add(cl.credit_amount)
                            .map_err(|_| {
                                ContractError::Std(cosmwasm_std::StdError::generic_err(
                                    "Credit limit overflow",
                                ))
                            })?;

                    // Include per-credit-line collateral.
                    let cl_collateral = cl.collateral_amount;
                    if filter.is_none() || filter == Some(&cl.collateral_denom) {
                        add_to_denom_collateral(
                            &mut denom_map,
                            &cl.collateral_denom,
                            cl_collateral,
                        )?;
                    }

                    if filter.is_none() || filter == Some(&cl.credit_denom) {
                        let entry = denom_map.entry(cl.credit_denom.clone()).or_insert_with(|| {
                            DenomReserve {
                                denom: cl.credit_denom.clone(),
                                collateral_amount: Uint128::zero(),
                                credit_limit: Uint128::zero(),
                                drawn_amount: Uint128::zero(),
                                repaid_amount: Uint128::zero(),
                                net_outstanding: Uint128::zero(),
                            }
                        });
                        entry.credit_limit = entry
                            .credit_limit
                            .checked_add(cl.credit_amount)
                            .map_err(|_| {
                                ContractError::Std(cosmwasm_std::StdError::generic_err(
                                    "Credit limit overflow",
                                ))
                            })?;
                    }
                }

                // Include multi-collateral per-token balances for this borrower.
                // This runs regardless of cl_matches because multi-collateral tokens
                // may differ from the credit line's collateral_denom/credit_denom.
                let multi = collateral::query_borrower_collateral(deps, &cl.borrower);
                for (m_denom, m_amount) in &multi {
                    if filter.is_none() || filter == Some(m_denom) {
                        add_to_denom_collateral(&mut denom_map, m_denom, *m_amount)?;
                        total_collateral =
                            total_collateral.checked_add(*m_amount).map_err(|_| {
                                ContractError::Std(cosmwasm_std::StdError::generic_err(
                                    "Collateral overflow",
                                ))
                            })?;
                    }
                }
            }

            let draw_count = DRAW_COUNT.may_load(deps.storage, id)?.unwrap_or(0);
            for did in 0..draw_count {
                if let Some(draw) = DRAWS.may_load(deps.storage, (id, did))? {
                    let draw_matches = match filter {
                        Some(target) => draw.denom == target,
                        None => true,
                    };

                    if draw_matches {
                        let entry =
                            denom_map
                                .entry(draw.denom.clone())
                                .or_insert_with(|| DenomReserve {
                                    denom: draw.denom.clone(),
                                    collateral_amount: Uint128::zero(),
                                    credit_limit: Uint128::zero(),
                                    drawn_amount: Uint128::zero(),
                                    repaid_amount: Uint128::zero(),
                                    net_outstanding: Uint128::zero(),
                                });

                        if draw.repaid {
                            total_repaid = total_repaid.checked_add(draw.amount).map_err(|_| {
                                ContractError::Std(cosmwasm_std::StdError::generic_err(
                                    "Repaid overflow",
                                ))
                            })?;
                            entry.repaid_amount =
                                entry.repaid_amount.checked_add(draw.amount).map_err(|_| {
                                    ContractError::Std(cosmwasm_std::StdError::generic_err(
                                        "Repaid overflow",
                                    ))
                                })?;
                        } else {
                            total_drawn = total_drawn.checked_add(draw.amount).map_err(|_| {
                                ContractError::Std(cosmwasm_std::StdError::generic_err(
                                    "Drawn overflow",
                                ))
                            })?;
                            entry.drawn_amount =
                                entry.drawn_amount.checked_add(draw.amount).map_err(|_| {
                                    ContractError::Std(cosmwasm_std::StdError::generic_err(
                                        "Drawn overflow",
                                    ))
                                })?;
                        }
                    }
                }
            }
        }
    }

    for d_entry in denom_map.values_mut() {
        d_entry.net_outstanding = d_entry.drawn_amount;
    }

    let net_outstanding = total_drawn;
    let reserves_by_denom = denom_map.into_values().collect();

    Ok(ProofOfReserveResponse {
        total_credit_lines,
        active_credit_lines,
        total_collateral,
        total_credit_limit,
        total_drawn,
        total_repaid,
        net_outstanding,
        reserves_by_denom,
    })
}

/// Returns the health factor and associated credit line details for all active
/// credit lines of the specified borrower.
///
/// If a borrower has no credit lines, returns an empty list in the response.
///
/// # Errors
///
/// Returns `ContractError::Std` or validation errors if the borrower address is invalid.
pub fn query_borrower_health_factor(
    deps: Deps,
    borrower: String,
) -> Result<BorrowerHealthFactorResponse, ContractError> {
    let borrower_addr = deps.api.addr_validate(&borrower)?;
    let count = CREDIT_LINE_COUNT.load(deps.storage)?;

    let mut credit_lines = Vec::new();

    for id in 0..count {
        if let Some(cl) = CREDIT_LINES.may_load(deps.storage, id)? {
            if cl.active && cl.borrower == borrower_addr {
                // Compute utilized amount (sum of all draws that are not repaid)
                let draw_count = DRAW_COUNT.may_load(deps.storage, id)?.unwrap_or(0);
                let mut utilized_amount = Uint128::zero();

                for did in 0..draw_count {
                    let draw = DRAWS.load(deps.storage, (id, did))?;
                    if !draw.repaid {
                        utilized_amount = utilized_amount
                            .checked_add(draw.amount)
                            .map_err(StdError::from)?;
                    }
                }

                // Aggregate credit-line collateral + multi-collateral (risk-weighted).
                let multi_total = collateral::weighted_collateral_total(deps, &cl.borrower)?;
                let effective_collateral = cl
                    .collateral_amount
                    .checked_add(multi_total)
                    .map_err(StdError::from)?;

                // Compute health factor based on effective collateral.
                let health_factor_bps = if utilized_amount.is_zero() {
                    u32::MAX
                } else if effective_collateral.is_zero() || cl.credit_amount.is_zero() {
                    0
                } else {
                    let numerator = effective_collateral
                        .checked_mul(Uint128::from(10_000u32))
                        .map_err(StdError::from)?;
                    let result = numerator
                        .checked_div(utilized_amount)
                        .map_err(StdError::from)?;
                    u32::try_from(result.u128()).unwrap_or(u32::MAX)
                };

                credit_lines.push(CreditLineHealthResponse {
                    credit_line_id: id,
                    collateral_denom: cl.collateral_denom,
                    collateral_amount: cl.collateral_amount,
                    credit_denom: cl.credit_denom,
                    credit_amount: cl.credit_amount,
                    utilized_amount,
                    health_factor_bps,
                });
            }
        }
    }

    Ok(BorrowerHealthFactorResponse {
        borrower,
        credit_lines,
    })
}

/// Assemble a full read-only snapshot of a single credit line.
///
/// Returns `Ok(None)` when no credit line exists for `credit_line_id`.
///
/// The snapshot bundles:
///
/// - Core credit-line fields (borrower, limits, active flag).
/// - All draws associated with the line (active and repaid).
/// - Sum of un-repaid draw amounts (`total_utilized`).
/// - Per-borrower multi-token collateral breakdown and risk-weighted total.
/// - Collateral-aware health factor in basis points (`u32::MAX` when debt is zero).
///
/// This avoids the multiple separate calls a client would otherwise need:
/// `CREDIT_LINES`, `DRAWS`, multi-collateral balance queries, and manual health
/// factor computation.
///
/// # Errors
///
/// Returns `ContractError::Std` on any underlying storage or arithmetic failure.
///
/// # Security
///
/// Read-only — no storage mutations, no authentication required.
pub fn query_credit_line_snapshot(
    deps: Deps,
    credit_line_id: u64,
) -> Result<Option<CreditLineSnapshotResponse>, ContractError> {
    // Return None cleanly when the credit line does not exist.
    let cl = match CREDIT_LINES.may_load(deps.storage, credit_line_id)? {
        Some(cl) => cl,
        None => return Ok(None),
    };

    // ── Draws ────────────────────────────────────────────────────────────────
    let draw_count = DRAW_COUNT
        .may_load(deps.storage, credit_line_id)?
        .unwrap_or(0);

    let mut draws: Vec<DrawSnapshotEntry> = Vec::with_capacity(draw_count as usize);
    let mut total_utilized = Uint128::zero();

    for did in 0..draw_count {
        if let Some(draw) = DRAWS.may_load(deps.storage, (credit_line_id, did))? {
            if !draw.repaid {
                total_utilized = total_utilized
                    .checked_add(draw.amount)
                    .map_err(StdError::from)?;
            }
            draws.push(DrawSnapshotEntry {
                draw_id: did,
                amount: draw.amount,
                denom: draw.denom,
                drawn_at: draw.drawn_at,
                drawn_by: draw.drawn_by,
                repaid: draw.repaid,
            });
        }
    }

    // ── Multi-collateral breakdown ────────────────────────────────────────────
    let raw_multi = collateral::query_borrower_collateral(deps, &cl.borrower);
    let multi_collateral: Vec<CollateralEntryResponse> = raw_multi
        .iter()
        .map(|(denom, amount)| CollateralEntryResponse {
            denom: denom.clone(),
            amount: *amount,
            risk_weight_bps: collateral::collateral_risk_weight_bps(deps, denom),
        })
        .collect();

    let weighted_collateral_total = collateral::weighted_collateral_total(deps, &cl.borrower)
        .map_err(|e| ContractError::Std(cosmwasm_std::StdError::generic_err(e.to_string())))?;

    // ── Health factor ─────────────────────────────────────────────────────────
    //
    // Aggregate effective collateral: primary collateral_amount + risk-weighted
    // multi-token total.  Then:
    //
    //   health_factor_bps = effective_collateral * 10_000 / total_utilized
    //
    // Returns u32::MAX when total_utilized == 0 (no outstanding debt — infinitely
    // healthy).  Returns 0 when effective_collateral == 0 and total_utilized > 0.
    let effective_collateral = cl
        .collateral_amount
        .checked_add(weighted_collateral_total)
        .map_err(StdError::from)?;

    let health_factor_bps = if total_utilized.is_zero() {
        u32::MAX
    } else if effective_collateral.is_zero() {
        0u32
    } else {
        let numerator = effective_collateral
            .checked_mul(Uint128::from(10_000u32))
            .map_err(StdError::from)?;
        let result = numerator
            .checked_div(total_utilized)
            .map_err(StdError::from)?;
        u32::try_from(result.u128()).unwrap_or(u32::MAX)
    };

    Ok(Some(CreditLineSnapshotResponse {
        credit_line_id,
        borrower: cl.borrower,
        collateral_denom: cl.collateral_denom,
        collateral_amount: cl.collateral_amount,
        credit_denom: cl.credit_denom,
        credit_amount: cl.credit_amount,
        active: cl.active,
        total_utilized,
        multi_collateral,
        weighted_collateral_total,
        health_factor_bps,
        draws,
    }))
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

/// Add `amount` to a denomination's collateral accumulator in the denom map.
fn add_to_denom_collateral(
    denom_map: &mut BTreeMap<String, DenomReserve>,
    denom: &str,
    amount: Uint128,
) -> Result<(), ContractError> {
    let entry = denom_map
        .entry(denom.to_string())
        .or_insert_with(|| DenomReserve {
            denom: denom.to_string(),
            collateral_amount: Uint128::zero(),
            credit_limit: Uint128::zero(),
            drawn_amount: Uint128::zero(),
            repaid_amount: Uint128::zero(),
            net_outstanding: Uint128::zero(),
        });
    entry.collateral_amount = entry.collateral_amount.checked_add(amount).map_err(|_| {
        ContractError::Std(cosmwasm_std::StdError::generic_err("Collateral overflow"))
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::query;
    use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};
    use cosmwasm_std::testing::{
        message_info, mock_dependencies, mock_env, MockApi, MockQuerier, MockStorage,
    };
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

    fn query_por(
        deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>,
        denom: Option<String>,
    ) -> ProofOfReserveResponse {
        let env = mock_env();
        let msg = QueryMsg::ProofOfReserve { denom };
        let raw = query(deps.as_ref(), env, msg).unwrap();
        from_json(&raw).unwrap()
    }

    fn query_health(
        deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>,
        borrower: &str,
    ) -> BorrowerHealthFactorResponse {
        let env = mock_env();
        let msg = QueryMsg::BorrowerHealthFactor {
            borrower: borrower.to_string(),
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

            assert_eq!(
                resp[0].events[0].action,
                crate::state::DrawAction::DrawCreated
            );
            assert_eq!(resp[0].events[0].seq, 0);

            assert_eq!(
                resp[0].events[1].action,
                crate::state::DrawAction::MemoAdded
            );
            assert_eq!(resp[0].events[1].memo, "First note");
            assert_eq!(resp[0].events[1].seq, 1);

            assert_eq!(
                resp[0].events[2].action,
                crate::state::DrawAction::MemoAdded
            );
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
            assert_eq!(
                resp[0].events[0].action,
                crate::state::DrawAction::DrawCreated
            );
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
            assert_eq!(
                resp[0].events[0].action,
                crate::state::DrawAction::DrawCreated
            );
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

    mod query_proof_of_reserve {
        use super::*;

        #[test]
        fn returns_empty_reserves_when_no_credit_lines() {
            let mut deps = mock_dependencies();
            setup_contract(&mut deps);

            let por = query_por(&deps, None);
            assert_eq!(por.total_credit_lines, 0);
            assert_eq!(por.active_credit_lines, 0);
            assert_eq!(por.total_collateral, Uint128::zero());
            assert_eq!(por.total_credit_limit, Uint128::zero());
            assert_eq!(por.total_drawn, Uint128::zero());
            assert_eq!(por.total_repaid, Uint128::zero());
            assert_eq!(por.net_outstanding, Uint128::zero());
            assert!(por.reserves_by_denom.is_empty());
        }

        #[test]
        fn returns_proof_of_reserve_for_single_credit_line_and_draws() {
            let mut deps = mock_dependencies();
            setup_contract(&mut deps);
            create_credit_line(&mut deps);
            create_draw(&mut deps, 0, "200");
            create_draw(&mut deps, 0, "100");
            repay_draw(&mut deps, 0, 1);

            let por = query_por(&deps, None);
            assert_eq!(por.total_credit_lines, 1);
            assert_eq!(por.active_credit_lines, 1);
            assert_eq!(por.total_collateral, Uint128::from(1000u128));
            assert_eq!(por.total_credit_limit, Uint128::from(500u128));
            assert_eq!(por.total_drawn, Uint128::from(200u128));
            assert_eq!(por.total_repaid, Uint128::from(100u128));
            assert_eq!(por.net_outstanding, Uint128::from(200u128));

            assert_eq!(por.reserves_by_denom.len(), 2);
            let ucollateral_res = por
                .reserves_by_denom
                .iter()
                .find(|r| r.denom == "ucollateral")
                .unwrap();
            assert_eq!(ucollateral_res.collateral_amount, Uint128::from(1000u128));

            let ucredit_res = por
                .reserves_by_denom
                .iter()
                .find(|r| r.denom == "ucredit")
                .unwrap();
            assert_eq!(ucredit_res.credit_limit, Uint128::from(500u128));
            assert_eq!(ucredit_res.drawn_amount, Uint128::from(200u128));
            assert_eq!(ucredit_res.repaid_amount, Uint128::from(100u128));
            assert_eq!(ucredit_res.net_outstanding, Uint128::from(200u128));
        }

        #[test]
        fn filters_reserves_by_denom() {
            let mut deps = mock_dependencies();
            setup_contract(&mut deps);
            create_credit_line(&mut deps);
            create_draw(&mut deps, 0, "150");

            let por_col = query_por(&deps, Some("ucollateral".to_string()));
            assert_eq!(por_col.reserves_by_denom.len(), 1);
            assert_eq!(por_col.reserves_by_denom[0].denom, "ucollateral");
            assert_eq!(por_col.total_collateral, Uint128::from(1000u128));

            let por_cred = query_por(&deps, Some("ucredit".to_string()));
            assert_eq!(por_cred.reserves_by_denom.len(), 1);
            assert_eq!(por_cred.reserves_by_denom[0].denom, "ucredit");
            assert_eq!(por_cred.total_credit_limit, Uint128::from(500u128));
            assert_eq!(por_cred.total_drawn, Uint128::from(150u128));
        }

        #[test]
        fn handles_inactive_credit_line() {
            let mut deps = mock_dependencies();
            setup_contract(&mut deps);
            create_credit_line(&mut deps);

            // Deactivate credit line manually in storage
            let mut cl = CREDIT_LINES.load(deps.as_ref().storage, 0).unwrap();
            cl.active = false;
            CREDIT_LINES.save(deps.as_mut().storage, 0, &cl).unwrap();

            let por = query_por(&deps, None);
            assert_eq!(por.total_credit_lines, 1);
            assert_eq!(por.active_credit_lines, 0);
            assert_eq!(por.total_collateral, Uint128::zero());
            assert_eq!(por.total_credit_limit, Uint128::zero());
        }
    }

    mod query_borrower_health_factor {
        use super::*;

        #[test]
        fn returns_empty_for_borrower_without_credit_lines() {
            let mut deps = mock_dependencies();
            setup_contract(&mut deps);

            let borrower_str = borrower(&deps).to_string();
            let resp = query_health(&deps, &borrower_str);
            assert_eq!(resp.borrower, borrower_str);
            assert!(resp.credit_lines.is_empty());
        }

        #[test]
        fn returns_u32_max_for_zero_utilization() {
            let mut deps = mock_dependencies();
            setup_contract(&mut deps);
            create_credit_line(&mut deps);

            let borrower_str = borrower(&deps).to_string();
            let resp = query_health(&deps, &borrower_str);
            assert_eq!(resp.credit_lines.len(), 1);
            assert_eq!(resp.credit_lines[0].credit_line_id, 0);
            assert_eq!(resp.credit_lines[0].utilized_amount, Uint128::zero());
            assert_eq!(resp.credit_lines[0].health_factor_bps, u32::MAX);
        }

        #[test]
        fn computes_health_factor_correctly() {
            let mut deps = mock_dependencies();
            setup_contract(&mut deps);
            create_credit_line(&mut deps); // collateral 1000, credit 500

            // Draw 100
            create_draw(&mut deps, 0, "100");

            let borrower_str = borrower(&deps).to_string();
            let resp = query_health(&deps, &borrower_str);
            assert_eq!(resp.credit_lines.len(), 1);
            assert_eq!(resp.credit_lines[0].utilized_amount, Uint128::from(100u128));
            // health = effective_collateral * 10_000 / utilized
            //        = 1000 * 10_000 / 100 = 100_000 bps
            assert_eq!(resp.credit_lines[0].health_factor_bps, 100_000);
        }

        #[test]
        fn handles_multiple_draws_and_repayments() {
            let mut deps = mock_dependencies();
            setup_contract(&mut deps);
            create_credit_line(&mut deps); // collateral 1000, credit 500

            create_draw(&mut deps, 0, "100");
            create_draw(&mut deps, 0, "200");

            let borrower_str = borrower(&deps).to_string();
            let resp = query_health(&deps, &borrower_str);
            assert_eq!(resp.credit_lines[0].utilized_amount, Uint128::from(300u128));
            // health = collateral * 10_000 / utilized = 1000 * 10_000 / 300 = 33_333 bps
            assert_eq!(resp.credit_lines[0].health_factor_bps, 33_333);

            // Repay first draw
            repay_draw(&mut deps, 0, 0);

            let resp2 = query_health(&deps, &borrower_str);
            assert_eq!(
                resp2.credit_lines[0].utilized_amount,
                Uint128::from(200u128)
            );
            // health = 1000 * 10_000 / 200 = 50_000 bps
            assert_eq!(resp2.credit_lines[0].health_factor_bps, 50_000);
        }

        #[test]
        fn handles_multiple_credit_lines_for_same_borrower() {
            let mut deps = mock_dependencies();
            setup_contract(&mut deps);
            create_credit_line(&mut deps); // cl 0, collateral 1000
            create_credit_line(&mut deps); // cl 1, collateral 1000

            create_draw(&mut deps, 0, "100");
            create_draw(&mut deps, 1, "250");

            let borrower_str = borrower(&deps).to_string();
            let resp = query_health(&deps, &borrower_str);
            assert_eq!(resp.credit_lines.len(), 2);
            assert_eq!(resp.credit_lines[0].credit_line_id, 0);
            // health = 1000 * 10_000 / 100 = 100_000
            assert_eq!(resp.credit_lines[0].health_factor_bps, 100_000);
            assert_eq!(resp.credit_lines[1].credit_line_id, 1);
            // health = 1000 * 10_000 / 250 = 40_000
            assert_eq!(resp.credit_lines[1].health_factor_bps, 40_000);
        }

        #[test]
        fn excludes_inactive_credit_lines() {
            let mut deps = mock_dependencies();
            setup_contract(&mut deps);
            create_credit_line(&mut deps); // cl 0, collateral 1000
            create_credit_line(&mut deps); // cl 1, collateral 1000

            create_draw(&mut deps, 0, "100");
            create_draw(&mut deps, 1, "250");

            let mut inactive = CREDIT_LINES.load(deps.as_ref().storage, 1).unwrap();
            inactive.active = false;
            CREDIT_LINES
                .save(deps.as_mut().storage, 1, &inactive)
                .unwrap();

            let borrower_str = borrower(&deps).to_string();
            let resp = query_health(&deps, &borrower_str);
            assert_eq!(resp.credit_lines.len(), 1);
            assert_eq!(resp.credit_lines[0].credit_line_id, 0);
            // health = 1000 * 10_000 / 100 = 100_000
            assert_eq!(resp.credit_lines[0].health_factor_bps, 100_000);
        }

        #[test]
        fn handles_zero_collateral_or_zero_credit_amount() {
            let mut deps = mock_dependencies();
            setup_contract(&mut deps);

            // Create a custom credit line with zero collateral
            let env = mock_env();
            let creator_addr = creator(&deps);
            let info = message_info(&creator_addr, &[]);
            let msg = ExecuteMsg::CreateCreditLine {
                borrower: borrower(&deps).to_string(),
                collateral_denom: "ucollateral".to_string(),
                collateral_amount: "0".to_string(),
                credit_denom: "ucredit".to_string(),
                credit_amount: "500".to_string(),
            };
            crate::contract::execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

            // Create a custom credit line with zero credit
            let msg2 = ExecuteMsg::CreateCreditLine {
                borrower: borrower(&deps).to_string(),
                collateral_denom: "ucollateral".to_string(),
                collateral_amount: "1000".to_string(),
                credit_denom: "ucredit".to_string(),
                credit_amount: "0".to_string(),
            };
            crate::contract::execute(deps.as_mut(), env, info, msg2).unwrap();

            // Draw on credit line 0
            create_draw(&mut deps, 0, "100");

            // Check health factors
            let borrower_str = borrower(&deps).to_string();
            let resp = query_health(&deps, &borrower_str);
            assert_eq!(resp.credit_lines.len(), 2);

            // cl 0: collateral 0, credit 500, utilized 100 -> health = 0
            assert_eq!(resp.credit_lines[0].credit_line_id, 0);
            assert_eq!(resp.credit_lines[0].health_factor_bps, 0);

            // cl 1: collateral 1000, credit 0, utilized 0 -> health = u32::MAX (no debt)
            assert_eq!(resp.credit_lines[1].credit_line_id, 1);
            assert_eq!(resp.credit_lines[1].health_factor_bps, u32::MAX);

            // Now make a draw on credit line 1
            create_draw(&mut deps, 1, "10");
            let resp2 = query_health(&deps, &borrower_str);
            // cl 1: collateral 1000, credit 0, utilized 10 -> health = 0
            assert_eq!(resp2.credit_lines[1].health_factor_bps, 0);
        }
    }
}
