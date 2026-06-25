// SPDX-License-Identifier: MIT

use creditra_credit::events::{
    publish_admin_rotation_accepted, publish_admin_rotation_proposed,
    publish_borrower_blocked_event, publish_default_liquidation_settled_event,
    publish_draw_reversed_event, publish_draws_frozen_event, publish_drawn_event,
    publish_interest_accrued_event, publish_paused_event, publish_rate_formula_config_event,
    publish_repayment_event, publish_risk_parameters_updated, AdminRotationAcceptedEvent,
    AdminRotationProposedEvent, BorrowerBlockedEvent, DefaultLiquidationSettledEvent,
    DrawReversedEvent, InterestAccruedEvent, RepaymentEvent, RiskParametersUpdatedEvent,
};
use creditra_credit::types::CreditStatus;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Events};
use soroban_sdk::{symbol_short, Address, Env, Symbol};

fn setup(env: &Env) -> (CreditClient, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    (client, admin)
}

#[test]
fn test_event_topics_stability() {
    let env = Env::default();
    let (client, admin) = setup(&env);
    let borrower = Address::generate(&env);
    let recipient = Address::generate(&env);

    // Trigger all events
    publish_drawn_event(&env, creditra_credit::events::DrawnEvent {
        borrower: borrower.clone(),
        amount: 100,
        new_utilized_amount: 100,
    });
    publish_repayment_event(&env, RepaymentEvent {
        borrower: borrower.clone(),
        amount: 50,
        new_utilized_amount: 50,
    });
    publish_interest_accrued_event(&env, InterestAccruedEvent {
        borrower: borrower.clone(),
        accrued_amount: 5,
        new_utilized_amount: 55,
    });
    publish_default_liquidation_settled_event(&env, DefaultLiquidationSettledEvent {
        borrower: borrower.clone(),
        settlement_id: Symbol::new(&env, "setl1"),
        recovered_amount: 20,
        remaining_utilized_amount: 35,
        status: CreditStatus::Active,
    });
    publish_admin_rotation_proposed(&env, &admin, 100);
    publish_admin_rotation_accepted(&env, &admin);
    publish_risk_parameters_updated(&env, &borrower, 1000, 500, 10);
    publish_draw_reversed_event(&env, DrawReversedEvent {
        borrower: borrower.clone(),
        amount: 10,
        original_ts: 10,
        reason_code: 1,
        new_utilized_amount: 45,
        timestamp: 20,
        admin: admin.clone(),
        accounting_only: false,
    });
    publish_draws_frozen_event(&env, true);
    publish_borrower_blocked_event(&env, &borrower, true);
    publish_rate_formula_config_event(&env, true);

    let all_events = env.events().all();
    
    // Assert topic pairs
    let assert_topic = |index: usize, _expected_t0: &str, expected_t1: &str| {
        let ev = all_events.get(index as u32).unwrap();
        let topics = ev.1;
        let t0 = Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap();
        let t1 = Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap();
        
        assert_eq!(t0, symbol_short!("credit"));
        assert_eq!(t1, Symbol::new(&env, expected_t1));
    };

    assert_topic(0, "credit", "drawn");
    assert_topic(1, "credit", "repay");
    assert_topic(2, "credit", "accrue");
    assert_topic(3, "credit", "liq_setl");
    assert_topic(4, "credit", "admin_prop");
    assert_topic(5, "credit", "admin_acc");
    assert_topic(6, "credit", "risk_upd");
    assert_topic(7, "credit", "draw_rev");
    assert_topic(8, "credit", "drw_freeze");
    assert_topic(9, "credit", "blk_chg");
    assert_topic(10, "credit", "rate_form");
}
