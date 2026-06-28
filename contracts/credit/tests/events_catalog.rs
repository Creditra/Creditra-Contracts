// SPDX-License-Identifier: MIT

//! Focused tests for the events catalog.
//!
//! Verifies that every event and publisher declared in `docs/EVENTS_CATALOG.md`
//! is present in the compiled contract and emits the expected topic and payload
//! shape. Run with:
//!
//! ```bash
//! cargo test -p creditra-credit --test events_catalog
//! ```

use soroban_sdk::{symbol_short, Address, BytesN, Env, Symbol, TryFromVal};

use creditra_credit::{FreezeReason, types::CreditStatus, types::GraceWaiverMode};
use creditra_credit::events::*;
use gateway_auction::events::*;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn env_and_addresses() -> (Env, Address, Address) {
    let env = Env::default();
    let borrower = Address::generate(&env);
    let admin = Address::generate(&env);
    (env, borrower, admin)
}

// Read the first topic symbol from a published event (credit contract uses
// ("credit", "...") or ("blk_chg",) or ("br_freeze",)).
fn first_topic(env: &Env, index: u32) -> Symbol {
    let ev = env.events().all().get(index).unwrap();
    let topics = ev.1;
    Symbol::try_from_val(env, &topics.get(0).unwrap()).unwrap()
}

// Read the second topic symbol from a published event, if present.
fn second_topic(env: &Env, index: u32) -> Symbol {
    let ev = env.events().all().get(index).unwrap();
    let topics = ev.1;
    Symbol::try_from_val(env, &topics.get(1).unwrap()).unwrap()
}

// ── Credit contract event shape tests ─────────────────────────────────────────

#[test]
fn credit_line_event_shape() {
    let (env, borrower, _admin) = env_and_addresses();

    for suffix in ["opened", "suspend", "closed", "default", "reinstate"] {
        let ev = CreditLineEvent {
            borrower: borrower.clone(),
            status: CreditStatus::Active,
            credit_limit: 1_000,
            interest_rate_bps: 500,
            risk_score: 70,
        };
        publish_credit_line_event(
            &env,
            (symbol_short!("credit"), Symbol::new(&env, suffix)),
            ev,
        );
    }

    let events = env.events().all();
    assert_eq!(events.len(), 5);

    for i in 0..5 {
        assert_eq!(first_topic(&env, i as u32), symbol_short!("credit"));
    }
}

#[test]
fn drawn_event_shape() {
    let (env, borrower, _admin) = env_and_addresses();

    publish_drawn_event(
        &env,
        DrawnEvent {
            borrower: borrower.clone(),
            amount: 500,
            new_utilized_amount: 500,
        },
    );

    let ev = env.events().all().get(0).unwrap();
    assert_eq!(first_topic(&env, 0), symbol_short!("credit"));
    assert_eq!(second_topic(&env, 0), symbol_short!("drawn"));
}

#[test]
fn drawn_event_v2_shape() {
    let (env, borrower, admin) = env_and_addresses();

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

    assert_eq!(first_topic(&env, 0), symbol_short!("credit"));
    assert_eq!(second_topic(&env, 0), symbol_short!("drawn_v2"));
}

#[test]
fn repayment_event_shape() {
    let (env, borrower, _admin) = env_and_addresses();

    publish_repayment_event(
        &env,
        RepaymentEvent {
            borrower: borrower.clone(),
            amount: 100,
            new_utilized_amount: 400,
        },
    );

    assert_eq!(first_topic(&env, 0), symbol_short!("credit"));
    assert_eq!(second_topic(&env, 0), symbol_short!("repay"));
}

#[test]
fn interest_accrued_event_shape() {
    let (env, borrower, _admin) = env_and_addresses();

    publish_interest_accrued_event(
        &env,
        InterestAccruedEvent {
            borrower: borrower.clone(),
            accrued_amount: 25,
            new_utilized_amount: 425,
        },
    );

    assert_eq!(first_topic(&env, 0), symbol_short!("credit"));
    assert_eq!(second_topic(&env, 0), symbol_short!("accrue"));
}

#[test]
fn fee_accrued_event_shape() {
    let (env, borrower, _admin) = env_and_addresses();

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

    assert_eq!(first_topic(&env, 0), symbol_short!("credit"));
    assert_eq!(second_topic(&env, 0), symbol_short!("fee_accrd"));
}

#[test]
fn late_fee_event_shape() {
    let (env, borrower, _admin) = env_and_addresses();

    publish_late_fee_charged_event(
        &env,
        LateFeeChargedEvent {
            borrower: borrower.clone(),
            fee: 50,
            installment_index: 3,
        },
    );

    assert_eq!(first_topic(&env, 0), symbol_short!("credit"));
    assert_eq!(second_topic(&env, 0), symbol_short!("late_fee"));
}

#[test]
fn risk_parameters_updated_shape() {
    let (env, borrower, _admin) = env_and_addresses();

    publish_risk_parameters_updated(&env, &borrower, 2_000, 750, 80);

    assert_eq!(first_topic(&env, 0), symbol_short!("credit"));
    assert_eq!(second_topic(&env, 0), symbol_short!("risk_upd"));
}

#[test]
fn draw_reversed_event_shape() {
    let (env, borrower, admin) = env_and_addresses();

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

    assert_eq!(first_topic(&env, 0), symbol_short!("credit"));
    assert_eq!(second_topic(&env, 0), Symbol::new(&env, "draw_rev"));
}

#[test]
fn credit_line_freeze_event_shape() {
    let (env, borrower, _admin) = env_and_addresses();

    publish_credit_line_freeze_event(&env, &borrower, FreezeReason::AdminAction, true);

    assert_eq!(first_topic(&env, 0), symbol_short!("credit"));
    assert_eq!(second_topic(&env, 0), Symbol::new(&env, "line_frz"));
}

#[test]
fn borrower_frozen_event_shape() {
    let (env, borrower, _admin) = env_and_addresses();

    publish_borrower_frozen_event(&env, &borrower, 1_000_000);

    // Published on a single-element topic tuple ("br_freeze",)
    let ev = env.events().all().get(0).unwrap();
    let topics = ev.1;
    assert_eq!(topics.len(), 1);
    assert_eq!(
        Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap(),
        Symbol::new(&env, "br_freeze")
    );
}

#[test]
fn penalty_rate_entered_shape() {
    let (env, borrower, _admin) = env_and_addresses();

    publish_penalty_rate_entered_event(&env, &borrower, 500, 200, 700);

    assert_eq!(first_topic(&env, 0), symbol_short!("credit"));
    assert_eq!(second_topic(&env, 0), Symbol::new(&env, "pen_enter"));
}

#[test]
fn penalty_rate_exited_shape() {
    let (env, borrower, _admin) = env_and_addresses();

    publish_penalty_rate_exited_event(&env, &borrower, 700, 500);

    assert_eq!(first_topic(&env, 0), symbol_short!("credit"));
    assert_eq!(second_topic(&env, 0), Symbol::new(&env, "pen_exit"));
}

#[test]
fn grace_waiver_event_shape() {
    let (env, borrower, _admin) = env_and_addresses();

    publish_grace_waiver_applied_event(
        &env,
        &borrower,
        10,
        creditra_credit::types::GraceWaiverMode::FullWaiver,
    );

    assert_eq!(first_topic(&env, 0), symbol_short!("credit"));
    assert_eq!(second_topic(&env, 0), symbol_short!("grace_wv"));
}

#[test]
fn admin_rotation_proposed_shape() {
    let (env, _borrower, admin) = env_and_addresses();

    publish_admin_rotation_proposed(&env, &admin, 200);

    assert_eq!(first_topic(&env, 0), symbol_short!("credit"));
    assert_eq!(second_topic(&env, 0), Symbol::new(&env, "admin_prop"));
}

#[test]
fn admin_rotation_accepted_shape() {
    let (env, _borrower, admin) = env_and_addresses();

    publish_admin_rotation_accepted(&env, &admin);

    assert_eq!(first_topic(&env, 0), symbol_short!("credit"));
    assert_eq!(second_topic(&env, 0), Symbol::new(&env, "admin_acc"));
}

#[test]
fn treasury_withdrawal_proposed_shape() {
    let (env, _borrower, admin) = env_and_addresses();

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

    assert_eq!(first_topic(&env, 0), symbol_short!("credit"));
    assert_eq!(second_topic(&env, 0), Symbol::new(&env, "tre_prop"));
}

#[test]
fn treasury_withdrawal_executed_shape() {
    let (env, _borrower, admin) = env_and_addresses();

    publish_treasury_withdrawal_executed(
        &env,
        TreasuryWithdrawalExecutedEvent {
            recipient: admin.clone(),
            amount: 500,
            executor: admin.clone(),
            executed_at: 200,
        },
    );

    assert_eq!(first_topic(&env, 0), symbol_short!("credit"));
    assert_eq!(second_topic(&env, 0), Symbol::new(&env, "tre_exec"));
}

#[test]
fn borrower_blocked_shape() {
    let (env, borrower, _admin) = env_and_addresses();

    publish_borrower_blocked_event(&env, &borrower, true);

    let ev = env.events().all().get(0).unwrap();
    let topics = ev.1;
    assert_eq!(topics.len(), 1);
    assert_eq!(
        Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap(),
        Symbol::new(&env, "blk_chg")
    );
}

#[test]
fn collateral_deposited_shape() {
    let (env, borrower, _admin) = env_and_addresses();

    publish_collateral_deposited_event(
        &env,
        CollateralDepositedEvent {
            borrower: borrower.clone(),
            amount: 1_000,
            new_balance: 1_000,
        },
    );

    assert_eq!(first_topic(&env, 0), symbol_short!("credit"));
    assert_eq!(second_topic(&env, 0), symbol_short!("col_dep"));
}

#[test]
fn collateral_withdrawn_shape() {
    let (env, borrower, _admin) = env_and_addresses();

    publish_collateral_withdrawn_event(
        &env,
        CollateralWithdrawnEvent {
            borrower: borrower.clone(),
            amount: 500,
            new_balance: 500,
        },
    );

    assert_eq!(first_topic(&env, 0), symbol_short!("credit"));
    assert_eq!(second_topic(&env, 0), symbol_short!("col_wit"));
}

#[test]
fn token_rescued_shape() {
    let (env, _borrower, admin) = env_and_addresses();

    publish_token_rescued_event(
        &env,
        TokenRescuedEvent {
            token: admin.clone(),
            recipient: admin.clone(),
            amount: 100,
        },
    );

    assert_eq!(first_topic(&env, 0), symbol_short!("credit"));
    assert_eq!(second_topic(&env, 0), Symbol::new(&env, "tok_resc"));
}

#[test]
fn contract_upgraded_shape() {
    let (env, _borrower, admin) = env_and_addresses();

    publish_contract_upgraded_event(
        &env,
        ContractUpgradedEvent {
            old_wasm_hash: BytesN::new(&env, &[0xAA; 32]),
            new_wasm_hash: BytesN::new(&env, &[0xBB; 32]),
        },
    );

    assert_eq!(first_topic(&env, 0), symbol_short!("credit"));
    assert_eq!(second_topic(&env, 0), Symbol::new(&env, "upgraded"));
}

#[test]
fn default_liquidation_settled_shape() {
    let (env, borrower, _admin) = env_and_addresses();

    publish_default_liquidation_settled_event(
        &env,
        DefaultLiquidationSettledEvent {
            borrower: borrower.clone(),
            settlement_id: Symbol::new(&env, "auction-1"),
            recovered_amount: 500,
            remaining_utilized_amount: 500,
            status: CreditStatus::Closed,
            close_factor_bps: 5000,
        },
    );

    assert_eq!(first_topic(&env, 0), symbol_short!("credit"));
    assert_eq!(second_topic(&env, 0), Symbol::new(&env, "liq_setl"));
}

#[test]
fn default_liquidation_requested_shape() {
    let (env, borrower, _admin) = env_and_addresses();

    publish_default_liquidation_requested_event(&env, &borrower, 1_500);

    assert_eq!(first_topic(&env, 0), symbol_short!("credit"));
    assert_eq!(second_topic(&env, 0), Symbol::new(&env, "liq_req"));
}

#[test]
fn attestation_batch_committed_shape() {
    let (env, borrower, _admin) = env_and_addresses();

    publish_attestation_batch_committed(
        &env,
        AttestationBatchCommittedEvent {
            borrower: borrower.clone(),
            merkle_root: BytesN::new(&env, &[0xCC; 32]),
            count: 42,
        },
    );

    assert_eq!(first_topic(&env, 0), symbol_short!("credit"));
    assert_eq!(second_topic(&env, 0), Symbol::new(&env, "atst_bat"));
}

#[test]
fn raw_value_events_shape() {
    let (env, _borrower, _admin) = env_and_addresses();

    publish_rate_formula_config_event(&env, true);
    publish_paused_event(&env, true);
    publish_paused_event(&env, false);
    publish_protocol_fee_bps_set_event(&env, 500);
    publish_protocol_fee_bounds_set_event(&env, 100, 2_000);
    publish_close_factor_bps_set_event(&env, 5_000);
    publish_oracle_config_set_event(&env, 500, 3_600);
    publish_oracle_price_accepted_event(&env, 1_000_000, 1_000);

    let events = env.events().all();
    assert_eq!(events.len(), 8);

    let topics = [
        ("credit", "rate_form"),
        ("credit", "paused"),
        ("credit", "unpaused"),
        ("credit", "fee_set"),
        ("credit", "fee_bnd"),
        ("credit", "clsfctr"),
        ("credit", "orc_cfg"),
        ("credit", "orc_price"),
    ];

    for (i, (t0, t1)) in topics.iter().enumerate() {
        assert_eq!(
            first_topic(&env, i as u32),
            symbol_short!(t0),
            "topic[{}] first symbol mismatch",
            i
        );
        assert_eq!(
            second_topic(&env, i as u32),
            Symbol::new(&env, t1),
            "topic[{}] second symbol mismatch",
            i
        );
    }
}

// ── Auction contract event shape tests ────────────────────────────────────────

#[test]
fn auction_bid_refunded_shape() {
    let (env, _borrower, admin) = env_and_addresses();

    publish_bid_refunded_event(&env, admin.clone(), 1_000);

    assert_eq!(
        first_topic(&env, 0),
        Symbol::new(&env, "BID_RFDN"),
        "first topic mismatch for BidRefundedEvent"
    );
    assert_eq!(
        second_topic(&env, 0),
        symbol_short!("auction"),
        "second topic mismatch for BidRefundedEvent"
    );
}

#[test]
fn auction_closed_shape() {
    let (env, _borrower, admin) = env_and_addresses();

    publish_auction_closed_event(
        &env,
        Symbol::new(&env, "auc-1"),
        Some(admin.clone()),
        5_000,
    );

    assert_eq!(
        first_topic(&env, 0),
        Symbol::new(&env, "AUC_CLOSE"),
        "first topic mismatch for AuctionClosedEvent"
    );
    assert_eq!(
        second_topic(&env, 0),
        symbol_short!("auction"),
        "second topic mismatch for AuctionClosedEvent"
    );
}

#[test]
fn auction_default_liquidation_settlement_shape() {
    let (env, borrower, admin) = env_and_addresses();

    publish_default_liquidation_settlement_event(
        &env,
        Symbol::new(&env, "auction-1"),
        admin.clone(),
        borrower.clone(),
        admin.clone(),
        3_000,
    );

    assert_eq!(
        first_topic(&env, 0),
        Symbol::new(&env, "LIQ_SETL"),
        "first topic mismatch for DefaultLiquidationSettlementEvent"
    );
    assert_eq!(
        second_topic(&env, 0),
        symbol_short!("auction"),
        "second topic mismatch for DefaultLiquidationSettlementEvent"
    );
}

// ── Struct instantiation tests ────────────────────────────────────────────────

#[test]
fn all_credit_event_structs_instantiate() {
    let (env, borrower, admin) = env_and_addresses();

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
    let _ = GraceWaiverAppliedEvent {
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
    let _ = TokenRescuedEvent {
        token: admin.clone(),
        recipient: admin.clone(),
        amount: 100,
    };
    let _ = ContractUpgradedEvent {
        old_wasm_hash: BytesN::new(&env, &[0x11; 32]),
        new_wasm_hash: BytesN::new(&env, &[0x22; 32]),
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
    };
    let _ = AttestationBatchCommittedEvent {
        borrower: borrower.clone(),
        merkle_root: BytesN::new(&env, &[0x33; 32]),
        count: 10,
    };
}

#[test]
fn all_auction_event_structs_instantiate() {
    let (_env, _borrower, admin) = env_and_addresses();

    let _ = BidRefundedEvent {
        prev_bidder: admin.clone(),
        amount: 500,
    };
    let _ = AuctionClosedEvent {
        auction_id: Symbol::new(&_env, "auc-1"),
        winner: Some(admin.clone()),
        amount: 5_000,
    };
    let _ = DefaultLiquidationSettlementEvent {
        auction_id: Symbol::new(&_env, "auc-1"),
        credit_contract: admin.clone(),
        borrower: admin.clone(),
        winner: admin.clone(),
        recovered_amount: 3_000,
    };
}
