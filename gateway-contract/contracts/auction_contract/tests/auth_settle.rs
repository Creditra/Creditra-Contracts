//! Tests for `require_auth` coverage on `settle_default_liquidation` factory caller.
//!
//! The auction-side [`settle_default_liquidation`] checks the caller against the
//! registered factory address but must also cryptographically verify the factory's
//! authorization via [`Address::require_auth`]. These tests prove that a forged
//! invoker with mocked auth entries cannot bypass the check.
//!
//! # Tests
//!
//! | Test | Auth mock | `credit_contract` | Expected |
//! |------|-----------|-------------------|----------|
//! | `non_factory_invoker_reverts` | intruder only | factory address | revert |
//! | `factory_invoker_succeeds` | factory only | factory address | `Ok(420)` |
//! | `replay_settle_reverts` | factory (twice) | factory address | `Err(AlreadySettled)` |
//!
//! # Running
//!
//! ```bash
//! cargo test -p gateway-auction --test auth_settle
//! ```
//!
//! [`settle_default_liquidation`]: ../src/lib.rs
use gateway_auction::DutchAuctionDecay;
use gateway_auction::{Auction, AuctionClient, AuctionMode};
use soroban_sdk::testutils::{Address as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{Address, Env, IntoVal, Symbol};

/// Deploy the auction contract, register the factory, create and close an
/// auction so that `settle_default_liquidation` can be exercised.
///
/// Returns `(env, contract_id, auction_id, factory, borrower, expected_recovered)`.
/// Callers create the `AuctionClient` locally to keep borrow lifetimes simple.
fn setup_auction() -> (Env, Address, Symbol, Address, Address, i128) {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);

    let factory = Address::generate(&env);
    let bidder = Address::generate(&env);
    let borrower = Address::generate(&env);
    let auction_id = Symbol::new(&env, "auth_stl");

    client.set_factory_contract(&factory);
    client.init_auction(
        &auction_id,
        &AuctionMode::English,
        &0_u64,
        &u64::MAX,
        &50_i128,
        &0_u32,
        &None,
&None,
&None,
&None,
    );
    client.place_bid(&auction_id, &bidder, &420_i128);
    client.close_auction(&auction_id);

    (env, contract_id, auction_id, factory, borrower, 420_i128)
}

// ── Negative ────────────────────────────────────────────────────────────────

#[test]
fn non_factory_invoker_reverts() {
    let (env, contract_id, auction_id, factory, borrower, _expected) = setup_auction();
    let client = AuctionClient::new(&env, &contract_id);

    let intruder = Address::generate(&env);

    let result = client
        .mock_auths(&[MockAuth {
            address: &intruder,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "settle_default_liquidation",
                args: (auction_id.clone(), factory.clone(), borrower.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_settle_default_liquidation(&auction_id, &factory, &borrower);

    assert!(
        result.is_err(),
        "non-factory invoker must be rejected (require_auth prevents bypass)"
    );
}

// ── Positive ────────────────────────────────────────────────────────────────

#[test]
fn factory_invoker_succeeds() {
    let (env, contract_id, auction_id, factory, borrower, expected) = setup_auction();
    let client = AuctionClient::new(&env, &contract_id);

    let result = client
        .mock_auths(&[MockAuth {
            address: &factory,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "settle_default_liquidation",
                args: (auction_id.clone(), factory.clone(), borrower.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_settle_default_liquidation(&auction_id, &factory, &borrower);

    let recovered = result
        .expect("factory-authorized call must not encounter host error")
        .expect("factory-authorized call must not encounter contract error");
    assert_eq!(recovered, expected, "must return the highest bid");
}

// ── Replay ──────────────────────────────────────────────────────────────────

#[test]
fn replay_settle_reverts() {
    let (env, contract_id, auction_id, factory, borrower, _expected) = setup_auction();
    let client = AuctionClient::new(&env, &contract_id);
    // First settlement — succeeds
    let first = client
        .mock_auths(&[MockAuth {
            address: &factory,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "settle_default_liquidation",
                args: (auction_id.clone(), factory.clone(), borrower.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_settle_default_liquidation(&auction_id, &factory, &borrower);
    assert!(first.is_ok(), "first settlement must succeed");

    // Second settlement — must revert with AlreadySettled
    let second = client
        .mock_auths(&[MockAuth {
            address: &factory,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "settle_default_liquidation",
                args: (auction_id.clone(), factory.clone(), borrower.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_settle_default_liquidation(&auction_id, &factory, &borrower);
    assert!(second.is_err(), "second settlement must revert");
}
