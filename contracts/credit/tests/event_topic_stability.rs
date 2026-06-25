// SPDX-License-Identifier: MIT

use creditra_credit::events::{
    publish_admin_rotation_accepted, publish_admin_rotation_proposed,
    publish_borrower_blocked_event, publish_default_liquidation_settled_event,
    publish_draw_reversed_event, publish_drawn_event, publish_draws_frozen_event,
    publish_interest_accrued_event, publish_rate_formula_config_event, publish_repayment_event,
    publish_risk_parameters_updated, publish_treasury_withdrawn_event,
    DefaultLiquidationSettledEvent, DrawReversedEvent, InterestAccruedEvent, RepaymentEvent,
    TreasuryWithdrawnEvent,
};
use creditra_credit::types::CreditStatus;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Events};
use soroban_sdk::{symbol_short, Address, Env, Symbol, TryFromVal};

fn setup(env: &Env) -> (CreditClient<'_>, Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    (client, admin, contract_id)
}

#[test]
fn test_event_topics_stability() {
    let env = Env::default();
    let (_client, admin, contract_id) = setup(&env);
    let borrower = Address::generate(&env);

    // Trigger all events
    env.as_contract(&contract_id, || {
        publish_drawn_event(
            &env,
            creditra_credit::events::DrawnEvent {
                borrower: borrower.clone(),
                amount: 100,
                new_utilized_amount: 100,
            },
        );
        publish_repayment_event(
            &env,
            RepaymentEvent {
                borrower: borrower.clone(),
                amount: 50,
                new_utilized_amount: 50,
            },
        );
        publish_interest_accrued_event(
            &env,
            InterestAccruedEvent {
                borrower: borrower.clone(),
                accrued_amount: 5,
                new_utilized_amount: 55,
            },
        );
        publish_default_liquidation_settled_event(
            &env,
            DefaultLiquidationSettledEvent {
                borrower: borrower.clone(),
                settlement_id: Symbol::new(&env, "setl1"),
                recovered_amount: 20,
                remaining_utilized_amount: 35,
                status: CreditStatus::Active,
            },
        );
        publish_admin_rotation_proposed(&env, &admin, 100);
        publish_admin_rotation_accepted(&env, &admin);
        publish_risk_parameters_updated(&env, &borrower, 1000, 500, 10);
        publish_draw_reversed_event(
            &env,
            DrawReversedEvent {
                borrower: borrower.clone(),
                amount: 10,
                original_ts: 10,
                reason_code: 1,
                new_utilized_amount: 45,
                timestamp: 20,
                admin: admin.clone(),
                accounting_only: false,
            },
        );
        publish_draws_frozen_event(&env, true);
        publish_borrower_blocked_event(&env, &borrower, true);
        publish_treasury_withdrawn_event(
            &env,
            TreasuryWithdrawnEvent {
                treasury: borrower.clone(),
                amount: 25,
                admin: admin.clone(),
                timestamp: 100,
            },
        );
        publish_rate_formula_config_event(&env, true);
    });

    let all_events = env.events().all();

    let has_credit_topic = |expected_t1: &str| {
        (0..all_events.len()).any(|index| {
            let topics = all_events.get(index).unwrap().1;
            if topics.len() != 2 {
                return false;
            }
            let t0 = Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap();
            let t1 = Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap();
            t0 == symbol_short!("credit") && t1 == Symbol::new(&env, expected_t1)
        })
    };

    for topic in [
        "drawn",
        "repay",
        "accrue",
        "liq_setl",
        "admin_prop",
        "admin_acc",
        "risk_upd",
        "draw_rev",
        "drw_freeze",
        "trs_wdraw",
        "rate_form",
    ] {
        assert!(has_credit_topic(topic), "missing topic credit/{topic}");
    }

    assert!(
        (0..all_events.len()).any(|index| {
            let topics = all_events.get(index).unwrap().1;
            topics.len() == 1
                && Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap()
                    == Symbol::new(&env, "blk_chg")
        }),
        "missing topic blk_chg"
    );
}
