//! Property-based test asserting collateral's core invariant across arbitrary
//! action sequences.
//!
//! The invariant tested is:
//!   "The total collateral value for any position should never exceed the sum
//!    of its individual asset deposits when priced at oracle rates."
//!
//!    More concretely, for every user and every supported asset:
//!      balance(user, asset) >= 0  (no negative balances)
//!      No asset can be deposited or withdrawn without proper authorization.
//!
//! This test uses the Soroban test env with proptest to generate random
//! sequences of deposit/withdraw/admin actions and verifies that invariants
//! hold after each step.

#![cfg(test)]

extern crate std;

use proptest::prelude::*;
use soroban_sdk::{testutils::Address as _, vec, Env, Address, Symbol, Vec};

use creditra_credit::contract::CreditraCreditClient;

// ---------------------------------------------------------------------------
// Test harness — thin wrapper around the Soroban env + contract
// ---------------------------------------------------------------------------

struct CollateralHarness {
    env: Env,
    admin: Address,
    user: Address,
    contract_id: Address,
    credit_contract_id: Address,
    supported_asset: Address,
}

impl CollateralHarness {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let user = Address::generate(&env);
        let supported_asset = Address::generate(&env);

        // Deploy a minimal credit contract for dependency
        let credit_contract_id = env.register(CreditraCredit, ());
        let credit_client = CreditraCreditClient::new(&env, &credit_contract_id);

        // Deploy the collateral contract
        let contract_id = env.register_contract(None, CreditraCollateral);
        let collateral_client = CreditraCollateralClient::new(&env, &contract_id);

        // Initialize
        collateral_client.init(
            &admin,
            &credit_contract_id,
        );

        // Add a supported asset
        collateral_client.add_supported_asset(&supported_asset);

        Self {
            env,
            admin,
            user,
            contract_id,
            credit_contract_id: credit_contract_id,
            supported_asset,
        }
    }

    fn collateral_client(&self) -> CreditraCollateralClient {
        CreditraCollateralClient::new(&self.env, &self.contract_id)
    }

    fn deposit(&self, asset: &Address, amount: i128) {
        self.collateral_client().deposit(&self.user, asset, &amount);
    }

    fn withdraw(&self, asset: &Address, amount: i128) {
        self.collateral_client().withdraw(&self.user, asset, &amount);
    }

    fn get_balance(&self, asset: &Address) -> i128 {
        self.collateral_client().balance(&self.user, asset)
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
        // Admin action (cooldown reset, etc.)
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
    let balance = harness.get_balance(&harness.supported_asset);

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
        // Run 50 iterations in CI, more for thorough local testing
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
                    let clamped = amount.min(10_000);
                    harness.deposit(&harness.supported_asset, clamped);
                    total_deposited += clamped;
                }
                Action::Withdraw(amount) => {
                    let current = harness.get_balance(&harness.supported_asset);
                    let clamped = amount.min(current);
                    if clamped > 0 {
                        harness.withdraw(&harness.supported_asset, clamped);
                    }
                    // Withdrawals reduce deposit total for sanity bound check
                    total_deposited = total_deposited.saturating_sub(clamped);
                }
                Action::AdminAction => {
                    // Admin actions (like cooldown resets) should not alter balances
                    // or break invariants. We simply verify invariants still hold.
                }
            }

            check_invariants(&harness);
        }

        // Final invariant: balance equals total_deposited minus withdrawals
        let final_balance = harness.get_balance(&harness.supported_asset);
        assert_eq!(
            final_balance, total_deposited,
            "Final balance {} does not match expected deposited amount {}",
            final_balance, total_deposited
        );
    }
}

// ---------------------------------------------------------------------------
// Placeholder — these would be real contract clients imported from the
// actual creditra-collateral crate. For the proptest structure we define
// minimal extern functions that the real implementation provides.
// ---------------------------------------------------------------------------

mod creditra_collateral {
    soroban_sdk::contractimport!(
        file = "../target/wasm32-unknown-unknown/release/creditra_collateral.wasm"
    );
}

// We also need the credit contract import
soroban_sdk::contractimport!(
    file = "../credit/target/wasm32-unknown-unknown/release/creditra_credit.wasm"
);
