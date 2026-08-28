// SPDX-License-Identifier: MIT

//! Property-based test asserting collateral's core invariant across arbitrary
//! action sequences.
//!
//! The invariant tested is:
//!   "The total collateral value for any position should never exceed the sum
//!    of its individual asset deposits."
//!
//!    More concretely, for every user:
//!      balance(user) >= 0  (no negative balances)
//!      balance(user) tracks valid deposits and withdrawals.

#![cfg(test)]

extern crate std;

use creditra_collateral::{Collateral, CollateralClient};
use proptest::prelude::*;
use soroban_sdk::{testutils::Address as _, Address, Env};

// ---------------------------------------------------------------------------
// Test harness — thin wrapper around the Soroban env + contract
// ---------------------------------------------------------------------------

struct CollateralHarness {
    env: Env,
    _admin: Address,
    user: Address,
    contract_id: Address,
}

impl CollateralHarness {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let user = Address::generate(&env);

        let contract_id = env.register(Collateral, ());

        Self {
            env,
            _admin: admin,
            user,
            contract_id,
        }
    }

    fn collateral_client(&self) -> CollateralClient<'static> {
        CollateralClient::new(&self.env, &self.contract_id)
    }

    fn deposit(&self, amount: i128) {
        self.collateral_client().deposit(&self.user, &amount);
    }

    fn withdraw(&self, amount: i128) {
        self.collateral_client().withdraw(&self.user, &amount);
    }

    fn get_balance(&self) -> i128 {
        self.collateral_client().get_balance(&self.user)
    }
}

// ---------------------------------------------------------------------------
// Proptest strategies
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Action {
    /// Deposit a random amount into the vault
    Deposit(i128),
    /// Withdraw a random amount from the vault
    Withdraw(i128),
    /// No-op (pause / admin action)
    AdminAction,
}

fn action_strategy() -> impl Strategy<Value = Action> {
    prop_oneof![
        // Deposit: amounts between 1 and 10_000
        3 => (1_i128..10_000).prop_map(Action::Deposit),
        // Withdraw: amounts between 1 and 5_000
        2 => (1_i128..5_000).prop_map(Action::Withdraw),
        // Admin action
        1 => Just(Action::AdminAction),
    ]
}

fn action_sequence_strategy() -> impl Strategy<Value = std::vec::Vec<Action>> {
    prop::collection::vec(action_strategy(), 1..20)
}

// ---------------------------------------------------------------------------
// Invariant checks
// ---------------------------------------------------------------------------

fn check_invariants(harness: &CollateralHarness) {
    let balance = harness.get_balance();

    // INVARIANT 1: Balance must never be negative.
    assert!(
        balance >= 0,
        "Collateral invariant violated: negative balance {}",
        balance
    );

    // INVARIANT 2: Balance must not exceed a sane maximum
    // (10_000 is the max single deposit * 20 max actions)
    assert!(
        balance <= 200_000,
        "Collateral invariant violated: balance {} exceeds sanity bound",
        balance
    );
}

// ---------------------------------------------------------------------------
// The property test
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 50,
        .. ProptestConfig::default()
    })]

    /// Core invariant: after any sequence of deposit/withdraw/admin actions,
    /// the collateral balance must never go negative, and must never exceed
    /// the maximum possible total deposits.
    #[test]
    fn collateral_invariant_holds(actions in action_sequence_strategy()) {
        let harness = CollateralHarness::new();

        let mut total_deposited: i128 = 0;

        for action in &actions {
            match action {
                Action::Deposit(amount) => {
                    let clamped = (*amount).min(10_000);
                    harness.deposit(clamped);
                    total_deposited += clamped;
                }
                Action::Withdraw(amount) => {
                    let current = harness.get_balance();
                    let clamped = (*amount).min(current);
                    if clamped > 0 {
                        harness.withdraw(clamped);
                        total_deposited = total_deposited.saturating_sub(clamped);
                    }
                }
                Action::AdminAction => {
                    // Admin actions should not alter balances or break invariants.
                }
            }

            check_invariants(&harness);
        }

        // Final invariant: balance equals total_deposited minus withdrawals
        let final_balance = harness.get_balance();
        assert_eq!(
            final_balance, total_deposited,
            "Final balance {} does not match expected deposited amount {}",
            final_balance, total_deposited
        );
    }
}
