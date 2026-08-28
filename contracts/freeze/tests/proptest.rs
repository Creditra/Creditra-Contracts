//! Property test asserting freeze's core invariant across arbitrary action sequences.
//!
//! The invariant tested is:
//!   "Freeze state changes deterministically according to admin actions and time,
//!    and read queries accurately reflect the expected freeze states at any given moment."

#![cfg(test)]

extern crate std;

use creditra_credit::{Credit, CreditClient, FreezeReason};
use proptest::prelude::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, IntoVal,
};

const START_TS: u64 = 10_000;

#[derive(Debug, Clone)]
enum Action {
    FreezeDraws(u32),
    UnfreezeDraws,
    FreezeCreditLine(u32),
    UnfreezeCreditLine,
    FreezeBorrower(u64), // duration to add to current time
    UnfreezeBorrower,
    AdvanceTime(u64), // seconds to advance
}

fn action_strategy() -> impl Strategy<Value = Action> {
    prop_oneof![
        1 => (0..5_u32).prop_map(Action::FreezeDraws),
        1 => Just(Action::UnfreezeDraws),
        1 => (0..5_u32).prop_map(Action::FreezeCreditLine),
        1 => Just(Action::UnfreezeCreditLine),
        2 => (1_u64..100_000).prop_map(Action::FreezeBorrower),
        1 => Just(Action::UnfreezeBorrower),
        2 => (1_u64..86400).prop_map(Action::AdvanceTime),
    ]
}

fn reason_from_u32(val: u32) -> FreezeReason {
    match val % 5 {
        0 => FreezeReason::LiquidityReserve,
        1 => FreezeReason::Compliance,
        2 => FreezeReason::RiskInvestigation,
        3 => FreezeReason::OperationalMaintenance,
        _ => FreezeReason::BorrowerRequest,
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]
    #[test]
    fn freeze_invariants(actions in prop::collection::vec(action_strategy(), 1..50)) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| li.timestamp = START_TS);
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        // initialize
        client.init(&admin);
        client.open_credit_line(&borrower, &1_000_i128, &300_u32, &50_u32);

        // model state
        let mut model_draws_frozen = false;
        let mut model_draws_reason = None;
        let mut model_credit_line_frozen = false;
        let mut model_credit_line_reason = None;
        let mut model_borrower_frozen_until: Option<u64> = None;

        for action in actions {
            match action {
                Action::FreezeDraws(r) => {
                    let reason = reason_from_u32(r);
                    client.freeze_draws(&reason);
                    model_draws_frozen = true;
                    model_draws_reason = Some(reason);
                }
                Action::UnfreezeDraws => {
                    client.unfreeze_draws();
                    model_draws_frozen = false;
                }
                Action::FreezeCreditLine(r) => {
                    let reason = reason_from_u32(r);
                    client.freeze_credit_line(&borrower, &reason);
                    model_credit_line_frozen = true;
                    model_credit_line_reason = Some(reason);
                }
                Action::UnfreezeCreditLine => {
                    client.unfreeze_credit_line(&borrower);
                    model_credit_line_frozen = false;
                }
                Action::FreezeBorrower(duration) => {
                    let expiry = env.ledger().timestamp().saturating_add(duration);
                    client.freeze_borrower_until(&admin, &borrower, &expiry);
                    model_borrower_frozen_until = Some(expiry);
                }
                Action::UnfreezeBorrower => {
                    client.unfreeze_borrower(&admin, &borrower);
                    model_borrower_frozen_until = None;
                }
                Action::AdvanceTime(secs) => {
                    env.ledger().with_mut(|li| li.timestamp = li.timestamp.saturating_add(secs));
                }
            }

            // Assert Invariants
            assert_eq!(client.is_draws_frozen(), model_draws_frozen, "Draws frozen mismatch");
            if model_draws_frozen {
                assert_eq!(client.get_draws_freeze_reason(), model_draws_reason, "Draws freeze reason mismatch");
            }

            assert_eq!(client.is_credit_line_frozen(&borrower), model_credit_line_frozen, "Credit line frozen mismatch");
            if model_credit_line_frozen {
                assert_eq!(client.get_credit_line_freeze_reason(&borrower), model_credit_line_reason, "Credit line reason mismatch");
            }

            let now = env.ledger().timestamp();
            let expected_borrower_frozen = model_borrower_frozen_until.map_or(false, |expiry| now < expiry);
            assert_eq!(client.is_borrower_frozen(&borrower), expected_borrower_frozen, "Borrower frozen mismatch");
            if let Some(expiry) = model_borrower_frozen_until {
                assert_eq!(client.get_borrower_frozen_until(&borrower), Some(expiry), "Borrower frozen until mismatch");
            } else {
                assert_eq!(client.get_borrower_frozen_until(&borrower), None, "Borrower frozen until not None");
            }
        }
    }
}
