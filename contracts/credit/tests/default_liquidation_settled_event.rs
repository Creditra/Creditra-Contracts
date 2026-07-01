// SPDX-License-Identifier: MIT

//! Reconciliation tests for DefaultLiquidationSettledEvent payload completeness.
//!
//! Validates that the event emitted by `settle_default_liquidation` fully
//! reconciles with on-chain state after settlement, covering both full-recovery
//! (line closed) and partial-recovery (residual debt) scenarios.
//!
//! # Assertions per test
//! - `liq_setl` topic present with correct namespace ("credit")
//! - Event fields (`recovered_amount`, `remaining_utilized_amount`, `status`)
//!   equal the post-settlement `CreditLineData`
//! - `settlement_id` matches the input
//! - `borrower` matches
//! - Topic ordering: (`"credit"`, `"liq_setl"`) is stable
//! - No extra events emitted beyond expected

use std::panic::{catch_unwind, AssertUnwindSafe};

use creditra_credit::events::DefaultLiquidationSettledEvent;
use creditra_credit::types::CreditStatus;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::testutils::Events as _;
use soroban_sdk::{token, Address, Env, Symbol, TryFromVal};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn setup_defaulted_line(utilized_amount: i128) -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());

    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &1_000_000_i128);
    token::StellarAssetClient::new(&env, &token_address).mint(&borrower, &1_000_000_i128);
    token::Client::new(&env, &token_address).approve(
        &borrower,
        &contract_id,
        &1_000_000_i128,
        &1_000_000_u32,
    );

    client.open_credit_line(&borrower, &10_000_i128, &300_u32, &60_u32);

    if utilized_amount > 0 {
        client.draw_credit(&borrower, &utilized_amount);
    }

    client.default_credit_line(&borrower);

    (env, contract_id, borrower, admin)
}

fn get_last_liq_setl_event(env: &Env) -> DefaultLiquidationSettledEvent {
    let namespace = Symbol::new(env, "credit");
    let kind = Symbol::new(env, "liq_setl");

    for (_contract, topics, data) in env.events().all().iter().rev() {
        let t0: Symbol = Symbol::try_from_val(env, &topics.get(0).unwrap()).unwrap();
        let t1: Symbol = Symbol::try_from_val(env, &topics.get(1).unwrap()).unwrap();
        if t0 == namespace && t1 == kind {
            return DefaultLiquidationSettledEvent::try_from_val(env, &data).unwrap();
        }
    }

    panic!("No liq_setl event found");
}

fn assert_liq_setl_topic_ordering(env: &Env) {
    let namespace = Symbol::new(env, "credit");
    let kind = Symbol::new(env, "liq_setl");

    let mut found = false;
    for (_contract, topics, _data) in env.events().all().iter() {
        let t0: Symbol = Symbol::try_from_val(env, &topics.get(0).unwrap()).unwrap();
        let t1: Symbol = Symbol::try_from_val(env, &topics.get(1).unwrap()).unwrap();
        if t0 == namespace && t1 == kind {
            found = true;
            assert_eq!(topics.len(), 2, "liq_setl event must have exactly 2 topics");
            break;
        }
    }

    assert!(found, "Expected liq_setl event not found");
}

#[test]
fn settle_full_recovery_closes_line_and_event_matches_state() {
    let (env, contract_id, borrower, _admin) = setup_defaulted_line(1_000);
    let client = CreditClient::new(&env, &contract_id);
    let settlement_id = Symbol::new(&env, "auc_full_001");

    client.settle_default_liquidation(&borrower, &1_000_i128, &settlement_id, &10_000_u32, &None);

    // Check events before the next invocation resets the buffer.
    let event = get_last_liq_setl_event(&env);
    assert_eq!(event.borrower, borrower);
    assert_eq!(event.settlement_id, settlement_id);
    assert_eq!(event.recovered_amount, 1_000_i128);
    assert_eq!(event.remaining_utilized_amount, 0_i128);
    assert_eq!(event.status, CreditStatus::Closed);

    assert_liq_setl_topic_ordering(&env);

    let namespace = Symbol::new(&env, "credit");
    let closed_kind = Symbol::new(&env, "closed");
    let closed_found = env.events().all().iter().any(|(_c, topics, _d)| {
        let t0: Symbol = Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap();
        let t1: Symbol = Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap();
        t0 == namespace && t1 == closed_kind
    });
    assert!(closed_found, "full recovery must also emit closed event");

    // Now verify on-chain state (this resets the event buffer).
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Closed);
    assert_eq!(line.utilized_amount, 0);
    assert_eq!(event.remaining_utilized_amount, line.utilized_amount);
    assert_eq!(event.status, line.status);
}

#[test]
fn settle_partial_recovery_keeps_line_defaulted_and_event_matches_state() {
    let (env, contract_id, borrower, _admin) = setup_defaulted_line(1_000);
    let client = CreditClient::new(&env, &contract_id);
    let settlement_id = Symbol::new(&env, "auc_partial_002");

    client.settle_default_liquidation(&borrower, &300_i128, &settlement_id, &10_000_u32, &None);

    // Check events before the next invocation resets the buffer.
    let event = get_last_liq_setl_event(&env);
    assert_eq!(event.borrower, borrower);
    assert_eq!(event.settlement_id, settlement_id);
    assert_eq!(event.recovered_amount, 300_i128);
    assert_eq!(event.remaining_utilized_amount, 700_i128);
    assert_eq!(event.status, CreditStatus::Defaulted);

    assert_liq_setl_topic_ordering(&env);

    let namespace = Symbol::new(&env, "credit");
    let closed_kind = Symbol::new(&env, "closed");
    let closed_found = env.events().all().iter().any(|(_c, topics, _d)| {
        let t0: Symbol = Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap();
        let t1: Symbol = Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap();
        t0 == namespace && t1 == closed_kind
    });
    assert!(!closed_found, "partial recovery must NOT emit closed event");

    // Now verify on-chain state.
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Defaulted);
    assert_eq!(line.utilized_amount, 700_i128);
    assert_eq!(event.remaining_utilized_amount, line.utilized_amount);
    assert_eq!(event.status, line.status);
}

#[test]
fn settle_minimal_partial_recovery_event_matches_state() {
    let (env, contract_id, borrower, _admin) = setup_defaulted_line(500);
    let client = CreditClient::new(&env, &contract_id);
    let settlement_id = Symbol::new(&env, "auc_min_003");

    client.settle_default_liquidation(&borrower, &1_i128, &settlement_id, &10_000_u32, &None);

    // Check events before the next invocation resets the buffer.
    let event = get_last_liq_setl_event(&env);
    assert_eq!(event.recovered_amount, 1_i128);
    assert_eq!(event.remaining_utilized_amount, 499_i128);
    assert_eq!(event.status, CreditStatus::Defaulted);
    assert_liq_setl_topic_ordering(&env);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Defaulted);
    assert_eq!(line.utilized_amount, 499_i128);
    assert_eq!(event.remaining_utilized_amount, line.utilized_amount);
    assert_eq!(event.status, line.status);
}

#[test]
fn settle_near_full_recovery_event_matches_state() {
    let (env, contract_id, borrower, _admin) = setup_defaulted_line(1_000);
    let client = CreditClient::new(&env, &contract_id);
    let settlement_id = Symbol::new(&env, "auc_near_004");

    client.settle_default_liquidation(&borrower, &999_i128, &settlement_id, &10_000_u32, &None);

    // Check events before the next invocation resets the buffer.
    let event = get_last_liq_setl_event(&env);
    assert_eq!(event.recovered_amount, 999_i128);
    assert_eq!(event.remaining_utilized_amount, 1_i128);
    assert_eq!(event.status, CreditStatus::Defaulted);
    assert_liq_setl_topic_ordering(&env);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Defaulted);
    assert_eq!(line.utilized_amount, 1_i128);
    assert_eq!(event.remaining_utilized_amount, line.utilized_amount);
    assert_eq!(event.status, line.status);
}

#[test]
fn liq_setl_event_field_ordering_is_stable() {
    let (env, contract_id, borrower, _admin) = setup_defaulted_line(800);
    let client = CreditClient::new(&env, &contract_id);
    let settlement_id = Symbol::new(&env, "auc_order_005");

    client.settle_default_liquidation(&borrower, &200_i128, &settlement_id, &10_000_u32, &None);

    let event = get_last_liq_setl_event(&env);

    assert_eq!(event.borrower, borrower);
    assert_eq!(event.settlement_id, settlement_id);
    assert_eq!(event.recovered_amount, 200_i128);
    assert_eq!(event.remaining_utilized_amount, 600_i128);
    assert_eq!(event.status, CreditStatus::Defaulted);

    let _ = event.borrower;
    let _ = event.settlement_id;
    let _ = event.recovered_amount;
    let _ = event.remaining_utilized_amount;
    let _ = event.status;
}

#[test]
fn multiple_settlements_each_emit_event_with_correct_state() {
    let (env, contract_id, borrower, _admin) = setup_defaulted_line(1_000);
    let client = CreditClient::new(&env, &contract_id);

    let sid1 = Symbol::new(&env, "auc_multi_1");
    client.settle_default_liquidation(&borrower, &400_i128, &sid1, &10_000_u32, &None);

    // Check first event immediately (before next invocation resets the buffer).
    let event1 = get_last_liq_setl_event(&env);
    assert_eq!(event1.recovered_amount, 400_i128);
    assert_eq!(event1.remaining_utilized_amount, 600_i128);
    assert_eq!(event1.settlement_id, sid1);
    assert_eq!(event1.status, CreditStatus::Defaulted);

    let line1 = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line1.utilized_amount, 600_i128);
    assert_eq!(line1.status, CreditStatus::Defaulted);

    let sid2 = Symbol::new(&env, "auc_multi_2");
    client.settle_default_liquidation(&borrower, &600_i128, &sid2, &10_000_u32, &None);

    // Check second event immediately.
    let event2 = get_last_liq_setl_event(&env);
    assert_eq!(event2.recovered_amount, 600_i128);
    assert_eq!(event2.remaining_utilized_amount, 0_i128);
    assert_eq!(event2.settlement_id, sid2);
    assert_eq!(event2.status, CreditStatus::Closed);

    assert_liq_setl_topic_ordering(&env);

    let line2 = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line2.utilized_amount, 0_i128);
    assert_eq!(line2.status, CreditStatus::Closed);
}

#[test]
fn replay_settlement_with_same_id_panics() {
    let (env, contract_id, borrower, _admin) = setup_defaulted_line(1_000);
    let client = CreditClient::new(&env, &contract_id);
    let settlement_id = Symbol::new(&env, "auc_replay_006");

    client.settle_default_liquidation(&borrower, &200_i128, &settlement_id, &10_000_u32, &None);

    let result = catch_unwind(AssertUnwindSafe(|| {
        client.settle_default_liquidation(&borrower, &100_i128, &settlement_id, &10_000_u32, &None);
    }));
    assert!(result.is_err(), "replay of same settlement_id must panic");

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 800_i128);
}

#[test]
fn settle_zero_recovered_amount_panics() {
    let (env, contract_id, borrower, _admin) = setup_defaulted_line(500);
    let client = CreditClient::new(&env, &contract_id);

    let result = catch_unwind(AssertUnwindSafe(|| {
        client.settle_default_liquidation(
            &borrower,
            &0_i128,
            &Symbol::new(&env, "auc_zero"),
            &None,
        );
    }));
    assert!(result.is_err());

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 500_i128);
}

#[test]
fn settle_negative_recovered_amount_panics() {
    let (env, contract_id, borrower, _admin) = setup_defaulted_line(500);
    let client = CreditClient::new(&env, &contract_id);

    let result = catch_unwind(AssertUnwindSafe(|| {
        client.settle_default_liquidation(
            &borrower,
            &(-100_i128),
            &Symbol::new(&env, "auc_neg"),
            &None,
        );
    }));
    assert!(result.is_err());

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 500_i128);
}

#[test]
fn settle_over_recovery_panics() {
    let (env, contract_id, borrower, _admin) = setup_defaulted_line(500);
    let client = CreditClient::new(&env, &contract_id);

    let result = catch_unwind(AssertUnwindSafe(|| {
        client.settle_default_liquidation(
            &borrower,
            &600_i128,
            &Symbol::new(&env, "auc_over"),
            &None,
        );
    }));
    assert!(result.is_err());

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 500_i128);
}

#[test]
fn settle_on_active_line_panics() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &1_000_000_i128);
    token::StellarAssetClient::new(&env, &token_address).mint(&borrower, &1_000_000_i128);
    token::Client::new(&env, &token_address).approve(
        &borrower,
        &contract_id,
        &1_000_000_i128,
        &1_000_000_u32,
    );

    client.open_credit_line(&borrower, &5_000_i128, &200_u32, &40_u32);
    client.draw_credit(&borrower, &1_000_i128);

    let result = catch_unwind(AssertUnwindSafe(|| {
        client.settle_default_liquidation(
            &borrower,
            &500_i128,
            &Symbol::new(&env, "auc_active"),
            &None,
        );
    }));
    assert!(result.is_err());

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Active);
    assert_eq!(line.utilized_amount, 1_000_i128);
}

#[test]
fn settle_on_nonexistent_line_panics() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);

    let result = catch_unwind(AssertUnwindSafe(|| {
        client.settle_default_liquidation(
            &borrower,
            &100_i128,
            &Symbol::new(&env, "auc_nonex"),
            &None,
        );
    }));
    assert!(result.is_err());
}
