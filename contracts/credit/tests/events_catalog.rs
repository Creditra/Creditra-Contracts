// SPDX-License-Identifier: MIT

//! Integration test: verifies every `publish_*` function in `events.rs`
//! emits the correct topic tuple and payload shape as documented in
//! `docs/EVENT_CATALOG.md`.
//!
//! Run with:
//! ```bash
//! cargo test -p creditra-credit --test events_catalog
//! ```

use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{symbol_short, Address, BytesN, Env, Symbol, TryFromVal, Val, Vec as SdkVec};

use creditra_credit::events::*;
use creditra_credit::{types::CreditStatus, Credit, FreezeReason};
use gateway_auction::events::{
    publish_auction_closed_event, publish_bid_refunded_event,
    publish_default_liquidation_settlement_event, AuctionClosedEvent, BidRefundedEvent,
    DefaultLiquidationSettlementEvent,
};

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Returns an Env with a registered credit contract and two test addresses.
/// The contract is registered so that events published via `env.as_contract`
/// are captured by `env.events().all()`.
fn setup() -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    {
        let client = creditra_credit::CreditClient::new(&env, &contract_id);
        client.init(&admin);
    }
    // Clear events emitted by init so our per-test assertions start from 0.
    env.events().all(); // consume/clear
    let borrower = Address::generate(&env);
    (env, contract_id, borrower, admin)
}

fn first_topic(env: &Env) -> Symbol {
    let all = env.events().all();
    let ev = all.get(0).unwrap();
    Symbol::try_from_val(env, &ev.1.get(0).unwrap()).unwrap()
}

fn second_topic(env: &Env) -> Symbol {
    let all = env.events().all();
    let ev = all.get(0).unwrap();
    Symbol::try_from_val(env, &ev.1.get(1).unwrap()).unwrap()
}

fn nth_first_topic(env: &Env, n: u32) -> Symbol {
    let all = env.events().all();
    let ev = all.get(n).unwrap();
    Symbol::try_from_val(env, &ev.1.get(0).unwrap()).unwrap()
}

fn nth_second_topic(env: &Env, n: u32) -> Symbol {
    let all = env.events().all();
    let ev = all.get(n).unwrap();
    Symbol::try_from_val(env, &ev.1.get(1).unwrap()).unwrap()
}

// ── Credit contract lifecycle events ─────────────────────────────────────────

#[test]
fn credit_line_event_shape() {
    let (env, contract_id, borrower, _admin) = setup();

    // Publish all 5 lifecycle variants in a single contract context so all
    // events are captured together.
    env.as_contract(&contract_id, || {
        for suffix in ["opened", "suspend", "closed", "defaulted", "reinstate"] {
            let ev = CreditLineEvent {
                borrower: borrower.clone(),
                status: CreditStatus::Active,
                credit_limit: 1_000,
                interest_rate_bps: 500,
                risk_score: 70,
            };
            let topic = (symbol_short!("credit"), Symbol::new(&env, suffix));
            publish_credit_line_event(&env, topic, ev.clone());
        }
    });

    let events = env.events().all();
    assert_eq!(events.len(), 5, "expected 5 lifecycle events");
    for i in 0..5u32 {
        assert_eq!(nth_first_topic(&env, i), symbol_short!("credit"));
    }
}

// ── Draw and repayment events ─────────────────────────────────────────────────

#[test]
fn drawn_event_shape() {
    let (env, contract_id, borrower, _admin) = setup();
    env.as_contract(&contract_id, || {
        publish_drawn_event(
            &env,
            DrawnEvent {
                borrower: borrower.clone(),
                amount: 500,
                new_utilized_amount: 500,
                timestamp: 0,
            },
        );
    });
    assert_eq!(first_topic(&env), symbol_short!("credit"));
    assert_eq!(second_topic(&env), symbol_short!("drawn"));
}

#[test]
fn drawn_event_v2_shape() {
    let (env, contract_id, borrower, admin) = setup();
    env.as_contract(&contract_id, || {
        publish_drawn_event_v2(
            &env,
            DrawnEventV2 {
                borrower: borrower.clone(),
                recipient: borrower.clone(),
                reserve_source: admin.clone(),
                amount: 500,
                new_utilized_amount: 500,
                timestamp: 100,
            },
        );
    });
    assert_eq!(first_topic(&env), symbol_short!("credit"));
    assert_eq!(second_topic(&env), symbol_short!("drawn_v2"));
}

#[test]
fn repayment_event_shape() {
    let (env, contract_id, borrower, _admin) = setup();
    env.as_contract(&contract_id, || {
        publish_repayment_event(
            &env,
            RepaymentEvent {
                borrower: borrower.clone(),
                amount: 100,
                new_utilized_amount: 400,
            },
        );
    });
    assert_eq!(first_topic(&env), symbol_short!("credit"));
    assert_eq!(second_topic(&env), symbol_short!("repay"));
}

#[test]
fn draw_reversed_event_shape() {
    let (env, contract_id, borrower, admin) = setup();
    env.as_contract(&contract_id, || {
        publish_draw_reversed_event(
            &env,
            DrawReversedEvent {
                borrower: borrower.clone(),
                amount: 100,
                original_ts: 10,
                reason_code: 1,
                new_utilized_amount: 0,
                timestamp: 20,
                admin: admin.clone(),
                accounting_only: false,
            },
        );
    });
    assert_eq!(first_topic(&env), symbol_short!("credit"));
    assert_eq!(second_topic(&env), Symbol::new(&env, "draw_rev"));
}

// ── Accrual and fee events ────────────────────────────────────────────────────

#[test]
fn interest_accrued_event_shape() {
    let (env, contract_id, borrower, _admin) = setup();
    env.as_contract(&contract_id, || {
        publish_interest_accrued_event(
            &env,
            InterestAccruedEvent {
                borrower: borrower.clone(),
                accrued_amount: 25,
                new_utilized_amount: 425,
            },
        );
    });
    assert_eq!(first_topic(&env), symbol_short!("credit"));
    assert_eq!(second_topic(&env), symbol_short!("accrue"));
}

#[test]
fn fee_accrued_event_shape() {
    let (env, contract_id, borrower, _admin) = setup();
    env.as_contract(&contract_id, || {
        publish_fee_accrued_event(
            &env,
            FeeAccruedEvent {
                borrower: borrower.clone(),
                fee_amount: 10,
                treasury_amount: 6,
                bounty_amount: 4,
                new_treasury_balance: 106,
                new_bounty_balance: 204,
            },
        );
    });
    assert_eq!(first_topic(&env), symbol_short!("credit"));
    assert_eq!(second_topic(&env), symbol_short!("fee_accrd"));
}

#[test]
fn late_fee_event_shape() {
    let (env, contract_id, borrower, _admin) = setup();
    env.as_contract(&contract_id, || {
        publish_late_fee_charged_event(
            &env,
            LateFeeChargedEvent {
                borrower: borrower.clone(),
                fee: 50,
                installment_index: 3,
            },
        );
    });
    assert_eq!(first_topic(&env), symbol_short!("credit"));
    assert_eq!(second_topic(&env), symbol_short!("late_fee"));
}

// ── Risk and parameter events ─────────────────────────────────────────────────

#[test]
fn risk_parameters_updated_shape() {
    let (env, contract_id, borrower, _admin) = setup();
    env.as_contract(&contract_id, || {
        publish_risk_parameters_updated(&env, &borrower, 2_000, 750, 80);
    });
    assert_eq!(first_topic(&env), symbol_short!("credit"));
    assert_eq!(second_topic(&env), symbol_short!("risk_upd"));
}

#[test]
fn draws_frozen_event_shape() {
    let (env, contract_id, _borrower, _admin) = setup();
    env.as_contract(&contract_id, || {
        publish_draws_frozen_event(&env, true, FreezeReason::LiquidityReserve);
    });
    assert_eq!(first_topic(&env), symbol_short!("credit"));
    assert_eq!(second_topic(&env), Symbol::new(&env, "drw_freeze"));
}

#[test]
fn credit_line_freeze_event_shape() {
    let (env, contract_id, borrower, _admin) = setup();
    env.as_contract(&contract_id, || {
        publish_credit_line_freeze_event(&env, &borrower, FreezeReason::AdminAction, true);
    });
    assert_eq!(first_topic(&env), symbol_short!("credit"));
    assert_eq!(second_topic(&env), Symbol::new(&env, "line_frz"));
}

#[test]
fn borrower_frozen_event_shape() {
    let (env, contract_id, borrower, _admin) = setup();
    env.as_contract(&contract_id, || {
        publish_borrower_frozen_event(&env, &borrower, 1_000_000);
    });
    let ev = env.events().all().get(0).unwrap();
    assert_eq!(
        ev.1.len(),
        1,
        "br_freeze should be a single-element topic tuple"
    );
    assert_eq!(
        Symbol::try_from_val(&env, &ev.1.get(0).unwrap()).unwrap(),
        Symbol::new(&env, "br_freeze")
    );
}

#[test]
fn penalty_rate_entered_shape() {
    let (env, contract_id, borrower, _admin) = setup();
    env.as_contract(&contract_id, || {
        publish_penalty_rate_entered_event(&env, &borrower, 500, 200, 700);
    });
    assert_eq!(first_topic(&env), symbol_short!("credit"));
    assert_eq!(second_topic(&env), Symbol::new(&env, "pen_enter"));
}

#[test]
fn penalty_rate_exited_shape() {
    let (env, contract_id, borrower, _admin) = setup();
    env.as_contract(&contract_id, || {
        publish_penalty_rate_exited_event(&env, &borrower, 700, 500);
    });
    assert_eq!(first_topic(&env), symbol_short!("credit"));
    assert_eq!(second_topic(&env), Symbol::new(&env, "pen_exit"));
}

#[test]
fn grace_waiver_event_shape() {
    let (env, contract_id, borrower, _admin) = setup();
    env.as_contract(&contract_id, || {
        publish_grace_waiver_receipt_event(
            &env,
            &borrower,
            10,
            creditra_credit::types::GraceWaiverMode::FullWaiver,
        );
    });
    assert_eq!(first_topic(&env), symbol_short!("credit"));
    assert_eq!(second_topic(&env), symbol_short!("grace_wv"));
}

// ── Admin and governance events ───────────────────────────────────────────────

#[test]
fn admin_rotation_proposed_shape() {
    let (env, contract_id, _borrower, admin) = setup();
    env.as_contract(&contract_id, || {
        publish_admin_rotation_proposed(&env, &admin, 200);
    });
    assert_eq!(first_topic(&env), symbol_short!("credit"));
    assert_eq!(second_topic(&env), Symbol::new(&env, "admin_prop"));
}

#[test]
fn admin_rotation_accepted_shape() {
    let (env, contract_id, _borrower, admin) = setup();
    env.as_contract(&contract_id, || {
        publish_admin_rotation_accepted(&env, &admin);
    });
    assert_eq!(first_topic(&env), symbol_short!("credit"));
    assert_eq!(second_topic(&env), Symbol::new(&env, "admin_acc"));
}

#[test]
fn treasury_withdrawal_proposed_shape() {
    let (env, contract_id, _borrower, admin) = setup();
    env.as_contract(&contract_id, || {
        publish_treasury_withdrawal_proposed(
            &env,
            TreasuryWithdrawalProposedEvent {
                recipient: admin.clone(),
                amount: 1_000,
                proposer: admin.clone(),
                proposed_at: 100,
                execute_after: 100 + 86_400,
            },
        );
    });
    assert_eq!(first_topic(&env), symbol_short!("credit"));
    assert_eq!(second_topic(&env), Symbol::new(&env, "tre_prop"));
}

#[test]
fn treasury_withdrawal_executed_shape() {
    let (env, contract_id, _borrower, admin) = setup();
    env.as_contract(&contract_id, || {
        publish_treasury_withdrawal_executed(
            &env,
            TreasuryWithdrawalExecutedEvent {
                recipient: admin.clone(),
                amount: 500,
                executor: admin.clone(),
                executed_at: 200,
                remaining_balance: 25,
            },
        );
    });
    assert_eq!(first_topic(&env), symbol_short!("credit"));
    assert_eq!(second_topic(&env), Symbol::new(&env, "tre_exec"));
}

#[test]
fn contract_upgraded_shape() {
    let (env, contract_id, _borrower, _admin) = setup();
    env.as_contract(&contract_id, || {
        publish_contract_upgraded_event(
            &env,
            ContractUpgradedEvent {
                old_wasm_hash: BytesN::from_array(&env, &[0xAA; 32]),
                new_wasm_hash: BytesN::from_array(&env, &[0xBB; 32]),
            },
        );
    });
    assert_eq!(first_topic(&env), symbol_short!("credit"));
    assert_eq!(second_topic(&env), Symbol::new(&env, "upgraded"));
}

// ── Blocklist events ──────────────────────────────────────────────────────────

#[test]
fn borrower_blocked_shape() {
    let (env, contract_id, borrower, _admin) = setup();
    env.as_contract(&contract_id, || {
        publish_borrower_blocked_event(&env, &borrower, true);
    });
    let ev = env.events().all().get(0).unwrap();
    assert_eq!(
        ev.1.len(),
        1,
        "blk_chg should be a single-element topic tuple"
    );
    assert_eq!(
        Symbol::try_from_val(&env, &ev.1.get(0).unwrap()).unwrap(),
        Symbol::new(&env, "blk_chg")
    );
}

// ── Collateral events ─────────────────────────────────────────────────────────

#[test]
fn collateral_deposited_shape() {
    let (env, contract_id, borrower, _admin) = setup();
    env.as_contract(&contract_id, || {
        publish_collateral_deposited_event(
            &env,
            CollateralDepositedEvent {
                borrower: borrower.clone(),
                amount: 1_000,
                new_balance: 1_000,
            },
        );
    });
    assert_eq!(first_topic(&env), symbol_short!("credit"));
    assert_eq!(second_topic(&env), symbol_short!("col_dep"));
}

#[test]
fn collateral_withdrawn_shape() {
    let (env, contract_id, borrower, _admin) = setup();
    env.as_contract(&contract_id, || {
        publish_collateral_withdrawn_event(
            &env,
            CollateralWithdrawnEvent {
                borrower: borrower.clone(),
                amount: 500,
                new_balance: 500,
            },
        );
    });
    assert_eq!(first_topic(&env), symbol_short!("credit"));
    assert_eq!(second_topic(&env), symbol_short!("col_wit"));
}

#[test]
fn collateral_partial_released_shape() {
    let (env, contract_id, borrower, _admin) = setup();
    env.as_contract(&contract_id, || {
        publish_collateral_partial_released_event(
            &env,
            CollateralPartialReleasedEvent {
                borrower: borrower.clone(),
                amount_released: 200,
                new_balance: 300,
                health_factor_bps: 12_000,
            },
        );
    });
    assert_eq!(first_topic(&env), symbol_short!("credit"));
    assert_eq!(second_topic(&env), Symbol::new(&env, "col_prel"));
}

// ── Default liquidation events ────────────────────────────────────────────────

#[test]
fn default_liquidation_requested_shape() {
    let (env, contract_id, borrower, _admin) = setup();
    env.as_contract(&contract_id, || {
        publish_default_liquidation_requested_event(&env, &borrower, 1_500);
    });
    assert_eq!(first_topic(&env), symbol_short!("credit"));
    assert_eq!(second_topic(&env), Symbol::new(&env, "liq_req"));
}

#[test]
fn default_liquidation_settled_shape() {
    let (env, contract_id, borrower, _admin) = setup();
    env.as_contract(&contract_id, || {
        publish_default_liquidation_settled_event(
            &env,
            DefaultLiquidationSettledEvent {
                borrower: borrower.clone(),
                settlement_id: Symbol::new(&env, "auction_1"),
                recovered_amount: 500,
                interest_recovered: 500,
                principal_recovered: 0,
                remaining_utilized_amount: 500,
                status: CreditStatus::Closed,
                close_factor_bps: 5000,
            },
        );
    });
    assert_eq!(first_topic(&env), symbol_short!("credit"));
    assert_eq!(second_topic(&env), Symbol::new(&env, "liq_setl"));
}

// ── Attestation events ────────────────────────────────────────────────────────

#[test]
fn attestation_batch_committed_shape() {
    let (env, contract_id, borrower, _admin) = setup();
    env.as_contract(&contract_id, || {
        publish_attestation_batch_committed(
            &env,
            AttestationBatchCommittedEvent {
                borrower: borrower.clone(),
                merkle_root: BytesN::from_array(&env, &[0xCC; 32]),
                count: 42,
            },
        );
    });
    assert_eq!(first_topic(&env), symbol_short!("credit"));
    assert_eq!(second_topic(&env), Symbol::new(&env, "atst_bat"));
}

// ── Rescue events ─────────────────────────────────────────────────────────────

#[test]
fn token_rescued_shape() {
    let (env, contract_id, _borrower, admin) = setup();
    env.as_contract(&contract_id, || {
        publish_token_rescued_event(
            &env,
            TokenRescuedEvent {
                token: admin.clone(),
                recipient: admin.clone(),
                amount: 100,
            },
        );
    });
    assert_eq!(first_topic(&env), symbol_short!("credit"));
    assert_eq!(second_topic(&env), Symbol::new(&env, "tok_resc"));
}

// ── Raw-value events ──────────────────────────────────────────────────────────

#[test]
fn raw_value_events_shape() {
    let (env, contract_id, _borrower, _admin) = setup();

    let expected: &[(&str, &str)] = &[
        ("credit", "rate_form"),
        ("credit", "paused"),
        ("credit", "unpaused"),
        ("credit", "fee_bps"),
        ("credit", "fee_bnds"),
        ("credit", "clsfctr"),
        ("credit", "orc_cfg"),
        ("credit", "orc_qcfg"),
        ("credit", "orc_qprc"),
        ("credit", "orc_price"),
    ];

    env.as_contract(&contract_id, || {
        publish_rate_formula_config_event(&env, true);
        publish_paused_event(&env, true);
        publish_paused_event(&env, false);
        publish_protocol_fee_bps_set_event(&env, 500);
        publish_protocol_fee_bounds_set_event(&env, 100, 2_000);
        publish_close_factor_bps_set_event(&env, 5_000);
        publish_oracle_config_set_event(&env, 500, 3_600);
        publish_oracle_quorum_config_set_event(&env, 3, 500, 3_600);
        publish_oracle_quorum_price_set_event(&env, 1_000_000, 3, 1_000);
        publish_oracle_price_accepted_event(&env, 1_000_000, 1_000);
    });

    let events = env.events().all();
    assert_eq!(
        events.len(),
        expected.len() as u32,
        "expected {} raw events, got {}",
        expected.len(),
        events.len()
    );

    for (i, (t0, t1)) in expected.iter().enumerate() {
        assert_eq!(
            nth_first_topic(&env, i as u32),
            Symbol::new(&env, t0),
            "raw event[{}] first topic: expected '{}'",
            i,
            t0
        );
        assert_eq!(
            nth_second_topic(&env, i as u32),
            Symbol::new(&env, t1),
            "raw event[{}] second topic: expected '{}'",
            i,
            t1
        );
    }
}

// ── Oracle registry event shape tests ─────────────────────────────────────────

#[test]
fn oracle_added_shape() {
    let (env, contract_id, _borrower, admin) = setup();
    let _ = &admin;
    env.as_contract(&contract_id, || {
        publish_oracle_added_event(&env, &admin, 100);
    });
    assert_eq!(first_topic(&env), symbol_short!("credit"));
    assert_eq!(second_topic(&env), Symbol::new(&env, "orc_add"));
}

#[test]
fn oracle_removed_shape() {
    let (env, contract_id, _borrower, admin) = setup();
    let _ = &admin;
    env.as_contract(&contract_id, || {
        publish_oracle_removed_event(&env, &admin);
    });
    assert_eq!(first_topic(&env), symbol_short!("credit"));
    assert_eq!(second_topic(&env), Symbol::new(&env, "orc_rmv"));
}

#[test]
fn oracle_quorum_threshold_set_shape() {
    let (env, contract_id, _borrower, admin) = setup();
    let _ = &admin;
    env.as_contract(&contract_id, || {
        publish_oracle_quorum_threshold_set_event(&env, 50);
    });
    assert_eq!(first_topic(&env), symbol_short!("credit"));
    assert_eq!(second_topic(&env), Symbol::new(&env, "orc_qthrs"));
}

#[test]
fn oracle_reporting_window_set_shape() {
    let (env, contract_id, _borrower, admin) = setup();
    let _ = &admin;
    env.as_contract(&contract_id, || {
        publish_oracle_reporting_window_set_event(&env, 3600);
    });
    assert_eq!(first_topic(&env), symbol_short!("credit"));
    assert_eq!(second_topic(&env), Symbol::new(&env, "orc_win"));
}

#[test]
fn oracle_value_reported_shape() {
    let (env, contract_id, _borrower, admin) = setup();
    let _ = &admin;
    env.as_contract(&contract_id, || {
        publish_oracle_value_reported_event(&env, &admin, 1_000_000);
    });
    assert_eq!(first_topic(&env), symbol_short!("credit"));
    assert_eq!(second_topic(&env), Symbol::new(&env, "orc_rpt"));
}

// ── Auction contract events ───────────────────────────────────────────────────

#[test]
fn auction_bid_refunded_shape() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(gateway_auction::Auction, ());
    env.as_contract(&contract_id, || {
        publish_bid_refunded_event(&env, admin.clone(), 1_000);
    });
    let ev = env.events().all().get(0).unwrap();
    assert_eq!(
        Symbol::try_from_val(&env, &ev.1.get(0).unwrap()).unwrap(),
        Symbol::new(&env, "BID_RFDN"),
    );
    assert_eq!(
        Symbol::try_from_val(&env, &ev.1.get(1).unwrap()).unwrap(),
        symbol_short!("auction"),
    );
}

#[test]
fn auction_closed_shape() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(gateway_auction::Auction, ());
    env.as_contract(&contract_id, || {
        publish_auction_closed_event(&env, Symbol::new(&env, "auc_1"), Some(admin.clone()), 5_000);
    });
    let ev = env.events().all().get(0).unwrap();
    assert_eq!(
        Symbol::try_from_val(&env, &ev.1.get(0).unwrap()).unwrap(),
        Symbol::new(&env, "AUC_CLOSE"),
    );
    assert_eq!(
        Symbol::try_from_val(&env, &ev.1.get(1).unwrap()).unwrap(),
        symbol_short!("auction"),
    );
}

#[test]
fn auction_default_liquidation_settlement_shape() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(gateway_auction::Auction, ());
    env.as_contract(&contract_id, || {
        publish_default_liquidation_settlement_event(
            &env,
            Symbol::new(&env, "auction_1"),
            admin.clone(),
            borrower.clone(),
            admin.clone(),
            3_000,
        );
    });
    let ev = env.events().all().get(0).unwrap();
    assert_eq!(
        Symbol::try_from_val(&env, &ev.1.get(0).unwrap()).unwrap(),
        Symbol::new(&env, "LIQ_SETL"),
    );
    assert_eq!(
        Symbol::try_from_val(&env, &ev.1.get(1).unwrap()).unwrap(),
        symbol_short!("auction"),
    );
}

// ── Struct field completeness tests ──────────────────────────────────────────
// Ensures every documented event struct compiles with all required fields.

#[test]
fn all_credit_event_structs_instantiate() {
    let (env, _contract_id, borrower, admin) = setup();

    let _ = CreditLineEvent {
        borrower: borrower.clone(),
        status: CreditStatus::Active,
        credit_limit: 100,
        interest_rate_bps: 100,
        risk_score: 10,
    };
    let _ = RepaymentEvent {
        borrower: borrower.clone(),
        amount: 50,
        new_utilized_amount: 50,
    };
    let _ = DrawnEvent {
        borrower: borrower.clone(),
        amount: 100,
        new_utilized_amount: 100,
        timestamp: 0,
    };
    let _ = DrawnEventV2 {
        borrower: borrower.clone(),
        recipient: borrower.clone(),
        reserve_source: admin.clone(),
        amount: 100,
        new_utilized_amount: 100,
        timestamp: 50,
    };
    let _ = InterestAccruedEvent {
        borrower: borrower.clone(),
        accrued_amount: 5,
        new_utilized_amount: 105,
    };
    let _ = DefaultLiquidationSettledEvent {
        borrower: borrower.clone(),
        settlement_id: Symbol::new(&env, "s1"),
        recovered_amount: 20,
        interest_recovered: 10,
        principal_recovered: 10,
        remaining_utilized_amount: 80,
        status: CreditStatus::Defaulted,
        close_factor_bps: 5000,
    };
    let _ = AdminRotationProposedEvent {
        proposed_admin: admin.clone(),
        accept_after: 200,
    };
    let _ = AdminRotationAcceptedEvent {
        new_admin: admin.clone(),
    };
    let _ = RiskParametersUpdatedEvent {
        borrower: borrower.clone(),
        credit_limit: 1_000,
        interest_rate_bps: 300,
        risk_score: 50,
    };
    let _ = DrawReversedEvent {
        borrower: borrower.clone(),
        amount: 100,
        original_ts: 10,
        reason_code: 1,
        new_utilized_amount: 0,
        timestamp: 20,
        admin: admin.clone(),
        accounting_only: false,
    };
    let _ = DrawsFrozenEvent {
        frozen: true,
        reason: FreezeReason::LiquidityReserve,
    };
    let _ = CreditLineFreezeEvent {
        borrower: borrower.clone(),
        reason: FreezeReason::AdminAction,
        frozen: true,
        ledger: 100,
    };
    let _ = BorrowerBlockedEvent {
        borrower: borrower.clone(),
        blocked: true,
        ledger: 100,
    };
    let _ = BorrowerFrozenEvent {
        borrower: borrower.clone(),
        frozen_until: 1_000_000,
        ledger: 100,
    };
    let _ = FeeAccruedEvent {
        borrower: borrower.clone(),
        fee_amount: 10,
        treasury_amount: 6,
        bounty_amount: 4,
        new_treasury_balance: 106,
        new_bounty_balance: 204,
    };
    let _ = PenaltyRateEnteredEvent {
        borrower: borrower.clone(),
        base_rate_bps: 500,
        penalty_surcharge_bps: 200,
        effective_rate_bps: 700,
    };
    let _ = PenaltyRateExitedEvent {
        borrower: borrower.clone(),
        previous_rate_bps: 700,
        new_rate_bps: 500,
    };
    let _ = GraceWaiverReceiptEvent {
        borrower: borrower.clone(),
        waived_amount: 5,
        mode: creditra_credit::types::GraceWaiverMode::FullWaiver,
    };
    let _ = CollateralDepositedEvent {
        borrower: borrower.clone(),
        amount: 500,
        new_balance: 500,
    };
    let _ = CollateralWithdrawnEvent {
        borrower: borrower.clone(),
        amount: 200,
        new_balance: 300,
    };
    let _ = CollateralPartialReleasedEvent {
        borrower: borrower.clone(),
        amount_released: 200,
        new_balance: 300,
        health_factor_bps: 12_000,
    };
    let _ = TokenRescuedEvent {
        token: admin.clone(),
        recipient: admin.clone(),
        amount: 100,
    };
    let _ = ContractUpgradedEvent {
        old_wasm_hash: BytesN::from_array(&env, &[0x11; 32]),
        new_wasm_hash: BytesN::from_array(&env, &[0x22; 32]),
    };
    let _ = LateFeeChargedEvent {
        borrower: borrower.clone(),
        fee: 50,
        installment_index: 3,
    };
    let _ = TreasuryWithdrawalProposedEvent {
        recipient: admin.clone(),
        amount: 1_000,
        proposer: admin.clone(),
        proposed_at: 100,
        execute_after: 1_000,
    };
    let _ = TreasuryWithdrawalExecutedEvent {
        recipient: admin.clone(),
        amount: 500,
        executor: admin.clone(),
        executed_at: 200,
        remaining_balance: 25,
    };
    let _ = AttestationBatchCommittedEvent {
        borrower: borrower.clone(),
        merkle_root: BytesN::from_array(&env, &[0x33; 32]),
        count: 10,
    };
    let _ = OracleAddedEvent {
        oracle: admin.clone(),
        weight: 100,
        timestamp: 1000,
    };
    let _ = OracleRemovedEvent {
        oracle: admin.clone(),
        timestamp: 1000,
    };
    let _ = OracleQuorumThresholdSetEvent {
        threshold: 50,
        timestamp: 1000,
    };
    let _ = OracleReportingWindowSetEvent {
        window_seconds: 3600,
        timestamp: 1000,
    };
    let _ = OracleValueReportedEvent {
        oracle: admin.clone(),
        value: 1_000_000,
        timestamp: 1000,
    };
}

#[test]
fn all_auction_event_structs_instantiate() {
    let env = Env::default();
    let admin = Address::generate(&env);

    let _ = BidRefundedEvent {
        prev_bidder: admin.clone(),
        amount: 500,
    };
    let _ = AuctionClosedEvent {
        auction_id: Symbol::new(&env, "auc_1"),
        winner: Some(admin.clone()),
        amount: 5_000,
    };
    let _ = DefaultLiquidationSettlementEvent {
        auction_id: Symbol::new(&env, "auc_1"),
        credit_contract: admin.clone(),
        borrower: admin.clone(),
        winner: admin.clone(),
        recovered_amount: 3_000,
    };
}
