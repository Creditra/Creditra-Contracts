#[cfg(test)]
mod tests {
    use cosmwasm_std::{
        attr,
        testing::{mock_dependencies, mock_env, mock_info},
        Addr, Event,
    };

    use crate::contract::{execute, instantiate};
    use crate::msg::{ExecuteMsg, InstantiateMsg};
    use crate::state::{DrawAction, DRAW_AUDIT, DRAW_AUDIT_COUNT};

    const OWNER: &str = "owner";
    const BORROWER: &str = "borrower";
    const STRANGER: &str = "stranger";

    fn setup(deps: &mut cosmwasm_std::OwnedDeps<
        cosmwasm_std::MemoryStorage,
        cosmwasm_std::testing::MockApi,
        cosmwasm_std::testing::MockQuerier,
    >) {
        let msg = InstantiateMsg {
            owner: OWNER.to_string(),
        };
        instantiate(deps.as_mut(), mock_env(), mock_info(OWNER, &[]), msg).unwrap();
    }

    fn create_credit_line(
        deps: &mut cosmwasm_std::OwnedDeps<
            cosmwasm_std::MemoryStorage,
            cosmwasm_std::testing::MockApi,
            cosmwasm_std::testing::MockQuerier,
        >,
    ) {
        execute(
            deps.as_mut(),
            mock_env(),
            mock_info(OWNER, &[]),
            ExecuteMsg::CreateCreditLine {
                borrower: BORROWER.to_string(),
                collateral_denom: "uatom".to_string(),
                collateral_amount: "1000".to_string(),
                credit_denom: "uosmo".to_string(),
                credit_amount: "500".to_string(),
            },
        )
        .unwrap();
    }

    fn create_draw(
        deps: &mut cosmwasm_std::OwnedDeps<
            cosmwasm_std::MemoryStorage,
            cosmwasm_std::testing::MockApi,
            cosmwasm_std::testing::MockQuerier,
        >,
    ) {
        execute(
            deps.as_mut(),
            mock_env(),
            mock_info(BORROWER, &[]),
            ExecuteMsg::CreateDraw {
                credit_line_id: 0,
                amount: "100".to_string(),
                denom: "uosmo".to_string(),
            },
        )
        .unwrap();
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Happy-path: owner can waive grace period
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn waive_grace_period_emits_event() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps);
        create_draw(&mut deps);

        let res = execute(
            deps.as_mut(),
            mock_env(),
            mock_info(OWNER, &[]),
            ExecuteMsg::WaiveGracePeriod {
                credit_line_id: 0,
                draw_id: 0,
                memo: "late payment waived".to_string(),
            },
        )
        .unwrap();

        // Must contain a `grace_period_waived` typed event
        let grace_event = res
            .events
            .iter()
            .find(|e| e.ty == "grace_period_waived")
            .expect("expected grace_period_waived event");

        assert!(grace_event
            .attributes
            .iter()
            .any(|a| a.key == "credit_line_id" && a.value == "0"));
        assert!(grace_event
            .attributes
            .iter()
            .any(|a| a.key == "draw_id" && a.value == "0"));
        assert!(grace_event
            .attributes
            .iter()
            .any(|a| a.key == "waived_by" && a.value == OWNER));
        assert!(grace_event
            .attributes
            .iter()
            .any(|a| a.key == "memo" && a.value == "late payment waived"));
    }

    #[test]
    fn waive_grace_period_response_attributes() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps);
        create_draw(&mut deps);

        let res = execute(
            deps.as_mut(),
            mock_env(),
            mock_info(OWNER, &[]),
            ExecuteMsg::WaiveGracePeriod {
                credit_line_id: 0,
                draw_id: 0,
                memo: String::new(),
            },
        )
        .unwrap();

        assert!(res
            .attributes
            .contains(&attr("action", "waive_grace_period")));
        assert!(res
            .attributes
            .contains(&attr("credit_line_id", "0")));
        assert!(res.attributes.contains(&attr("draw_id", "0")));
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Audit trail: GraceWaived entry is stored
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn waive_grace_period_appends_audit_entry() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps);
        create_draw(&mut deps);

        execute(
            deps.as_mut(),
            mock_env(),
            mock_info(OWNER, &[]),
            ExecuteMsg::WaiveGracePeriod {
                credit_line_id: 0,
                draw_id: 0,
                memo: "audit test".to_string(),
            },
        )
        .unwrap();

        // draw was created → seq 0 = DrawCreated; seq 1 = GraceWaived
        let count = DRAW_AUDIT_COUNT
            .load(&deps.storage, (0, 0))
            .unwrap();
        assert_eq!(count, 2, "expected 2 audit entries (DrawCreated + GraceWaived)");

        let entry = DRAW_AUDIT.load(&deps.storage, (0, 0, 1)).unwrap();
        assert!(matches!(entry.action, DrawAction::GraceWaived));
        assert_eq!(entry.memo, "audit test");
        assert_eq!(entry.by, Addr::unchecked(OWNER));
    }

    #[test]
    fn waive_grace_period_with_empty_memo() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps);
        create_draw(&mut deps);

        let res = execute(
            deps.as_mut(),
            mock_env(),
            mock_info(OWNER, &[]),
            ExecuteMsg::WaiveGracePeriod {
                credit_line_id: 0,
                draw_id: 0,
                memo: String::new(),
            },
        );
        assert!(res.is_ok(), "empty memo should be valid");

        let entry = DRAW_AUDIT.load(&deps.storage, (0, 0, 1)).unwrap();
        assert_eq!(entry.memo, "");
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Access control
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn waive_grace_period_unauthorized_stranger() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps);
        create_draw(&mut deps);

        let err = execute(
            deps.as_mut(),
            mock_env(),
            mock_info(STRANGER, &[]),
            ExecuteMsg::WaiveGracePeriod {
                credit_line_id: 0,
                draw_id: 0,
                memo: String::new(),
            },
        )
        .unwrap_err();

        assert!(
            matches!(err, crate::error::ContractError::Unauthorized),
            "stranger should be rejected"
        );
    }

    #[test]
    fn waive_grace_period_unauthorized_borrower() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps);
        create_draw(&mut deps);

        let err = execute(
            deps.as_mut(),
            mock_env(),
            mock_info(BORROWER, &[]),
            ExecuteMsg::WaiveGracePeriod {
                credit_line_id: 0,
                draw_id: 0,
                memo: String::new(),
            },
        )
        .unwrap_err();

        assert!(matches!(err, crate::error::ContractError::Unauthorized));
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Not-found guard
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn waive_grace_period_draw_not_found() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps);

        let err = execute(
            deps.as_mut(),
            mock_env(),
            mock_info(OWNER, &[]),
            ExecuteMsg::WaiveGracePeriod {
                credit_line_id: 0,
                draw_id: 99,
                memo: String::new(),
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            crate::error::ContractError::DrawNotFound(99, 0)
        ));
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Idempotency: can waive the same draw multiple times (each creates a new
    // audit entry — no state machine lock-out required by the issue)
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn waive_grace_period_multiple_times_increments_audit() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps);
        create_draw(&mut deps);

        for i in 0..3u64 {
            execute(
                deps.as_mut(),
                mock_env(),
                mock_info(OWNER, &[]),
                ExecuteMsg::WaiveGracePeriod {
                    credit_line_id: 0,
                    draw_id: 0,
                    memo: format!("waiver #{i}"),
                },
            )
            .unwrap();
        }

        // seq 0 = DrawCreated, seq 1..3 = GraceWaived × 3
        let count = DRAW_AUDIT_COUNT.load(&deps.storage, (0, 0)).unwrap();
        assert_eq!(count, 4);

        for seq in 1..=3u64 {
            let entry = DRAW_AUDIT.load(&deps.storage, (0, 0, seq)).unwrap();
            assert!(matches!(entry.action, DrawAction::GraceWaived));
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Existing behaviour unchanged
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn existing_repay_draw_still_works() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps);
        create_draw(&mut deps);

        let res = execute(
            deps.as_mut(),
            mock_env(),
            mock_info(BORROWER, &[]),
            ExecuteMsg::RepayDraw {
                credit_line_id: 0,
                draw_id: 0,
            },
        );
        assert!(res.is_ok());
    }

    #[test]
    fn waive_does_not_affect_repaid_flag() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        create_credit_line(&mut deps);
        create_draw(&mut deps);

        execute(
            deps.as_mut(),
            mock_env(),
            mock_info(OWNER, &[]),
            ExecuteMsg::WaiveGracePeriod {
                credit_line_id: 0,
                draw_id: 0,
                memo: String::new(),
            },
        )
        .unwrap();

        let draw = crate::state::DRAWS
            .load(&deps.storage, (0, 0))
            .unwrap();
        assert!(!draw.repaid, "waiving grace period must not mark the draw as repaid");
    }

    // ─────────────────────────────────────────────────────────────────────────
    // events.rs unit tests
    // ─────────────────────────────────────────────────────────────────────────

    mod events_unit {
        use cosmwasm_std::{Addr, Timestamp};

        use crate::events::GracePeriodWaivedEvent;

        #[test]
        fn into_attributes_contains_all_fields() {
            let event = GracePeriodWaivedEvent {
                credit_line_id: 7,
                draw_id: 3,
                waived_by: Addr::unchecked("operator"),
                timestamp: Timestamp::from_nanos(0),
                block_height: 42,
                memo: "test".to_string(),
            };

            let attrs = event.into_attributes();
            let find = |key: &str| {
                attrs
                    .iter()
                    .find(|a| a.key == key)
                    .map(|a| a.value.as_str())
                    .unwrap_or("")
                    .to_string()
            };

            assert_eq!(find("credit_line_id"), "7");
            assert_eq!(find("draw_id"), "3");
            assert_eq!(find("waived_by"), "operator");
            assert_eq!(find("block_height"), "42");
            assert_eq!(find("memo"), "test");
        }

        #[test]
        fn into_attributes_empty_memo() {
            let event = GracePeriodWaivedEvent {
                credit_line_id: 0,
                draw_id: 0,
                waived_by: Addr::unchecked("op"),
                timestamp: Timestamp::from_nanos(0),
                block_height: 1,
                memo: String::new(),
            };
            let attrs = event.into_attributes();
            let memo_attr = attrs.iter().find(|a| a.key == "memo").unwrap();
            assert_eq!(memo_attr.value, "");
        }
    }
}
