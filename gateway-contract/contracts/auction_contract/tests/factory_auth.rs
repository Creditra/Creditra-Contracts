//! Tests that factory operations require cryptographically verified
//! authorization from the registered factory contract address.
//!
//! Every state-changing entrypoint gated by [`Address::require_auth`]
//! must reject invocations where the claimed authorizer does not match
//! the registered factory address.
//!
//! # Running
//!
//! ```bash
//! cargo test -p gateway-auction --test factory_auth
//! ```
//!
//! [`Address::require_auth`]: soroban_sdk::Address::require_auth

use gateway_auction::{
    Auction, AuctionClient, AuctionMode, DutchAuctionDecay,
};
use soroban_sdk::testutils::{Address as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{Address, Env, IntoVal, Symbol};

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Deploy the contract, register a factory, create and close an auction so that
/// settlement entrypoints are exercisable.
///
/// Returns `(env, contract_id, auction_id, factory, borrower, expected_recovered)`.
fn setup_settleable() -> (Env, Address, Symbol, Address, Address, i128) {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);

    let factory = Address::generate(&env);
    let bidder = Address::generate(&env);
    let borrower = Address::generate(&env);
    let auction_id = Symbol::new(&env, "fac_auth_stl");

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
        &DutchAuctionDecay::None,
        &None,
    );
    client.place_bid(&auction_id, &bidder, &420_i128);
    client.close_auction(&auction_id);

    (env, contract_id, auction_id, factory, borrower, 420_i128)
}

// ── set_factory_contract ─────────────────────────────────────────────────────

#[test]
fn set_factory_contract_requires_claimed_address_auth() {
    let env = Env::default();
    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);

    let claimed = Address::generate(&env);
    let intruder = Address::generate(&env);

    let result = client
        .mock_auths(&[MockAuth {
            address: &intruder,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_factory_contract",
                args: (claimed.clone(),).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_set_factory_contract(&claimed);

    assert!(
        result.is_err(),
        "set_factory_contract must reject caller that is not the claimed address"
    );
}

#[test]
fn set_factory_contract_succeeds_with_claimed_auth() {
    let env = Env::default();
    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);

    let factory = Address::generate(&env);

    let result = client
        .mock_auths(&[MockAuth {
            address: &factory,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_factory_contract",
                args: (factory.clone(),).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_set_factory_contract(&factory);

    assert!(result.is_ok(), "claimed address must be able to set itself as factory");
}

// ── init_auction ─────────────────────────────────────────────────────────────

#[test]
fn init_auction_reverts_when_factory_unset() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);
    let auction_id = Symbol::new(&env, "no_fac_init");

    let result = client.try_init_auction(
        &auction_id,
        &AuctionMode::English,
        &0_u64,
        &1000,
        &50_i128,
        &0_u32,
        &None,
        &None,
        &DutchAuctionDecay::None,
        &None,
    );

    assert!(result.is_err(), "init_auction must fail when no factory is configured");
}

#[test]
fn init_auction_requires_factory_auth() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);

    let factory = Address::generate(&env);
    client.set_factory_contract(&factory);

    let intruder = Address::generate(&env);
    let auction_id = Symbol::new(&env, "fac_init_req");

    let result = client
        .mock_auths(&[MockAuth {
            address: &intruder,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "init_auction",
                args: (
                    auction_id.clone(),
                    AuctionMode::English,
                    0_u64,
                    u64::MAX,
                    50_i128,
                    0_u32,
                    None::<i128>,
                    None::<i128>,
                    DutchAuctionDecay::None,
                    None::<u32>,
                )
                    .into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_init_auction(
            &auction_id,
            &AuctionMode::English,
            &0_u64,
            &u64::MAX,
            &50_i128,
            &0_u32,
            &None,
            &None,
            &DutchAuctionDecay::None,
            &None,
        );

    assert!(
        result.is_err(),
        "init_auction must reject non-factory caller"
    );
}

#[test]
fn init_auction_succeeds_with_factory_auth() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);

    let factory = Address::generate(&env);
    client.set_factory_contract(&factory);

    let auction_id = Symbol::new(&env, "fac_init_ok");

    let result = client
        .mock_auths(&[MockAuth {
            address: &factory,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "init_auction",
                args: (
                    auction_id.clone(),
                    AuctionMode::English,
                    0_u64,
                    u64::MAX,
                    50_i128,
                    0_u32,
                    None::<i128>,
                    None::<i128>,
                    DutchAuctionDecay::None,
                    None::<u32>,
                )
                    .into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_init_auction(
            &auction_id,
            &AuctionMode::English,
            &0_u64,
            &u64::MAX,
            &50_i128,
            &0_u32,
            &None,
            &None,
            &DutchAuctionDecay::None,
            &None,
        );

    assert!(result.is_ok(), "factory-authorized init_auction must succeed");
}

// ── close_auction ────────────────────────────────────────────────────────────

#[test]
fn close_auction_reverts_when_factory_unset() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);
    let auction_id = Symbol::new(&env, "no_fac_close");

    let result = client.try_close_auction(&auction_id);

    assert!(result.is_err(), "close_auction must fail when no factory is configured");
}

#[test]
fn close_auction_requires_factory_auth() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);

    let factory = Address::generate(&env);
    client.set_factory_contract(&factory);

    let auction_id = Symbol::new(&env, "fac_close_req");
    client.init_auction(
        &auction_id,
        &AuctionMode::English,
        &0_u64,
        &u64::MAX,
        &50_i128,
        &0_u32,
        &None,
        &None,
        &DutchAuctionDecay::None,
        &None,
    );

    let intruder = Address::generate(&env);

    let result = client
        .mock_auths(&[MockAuth {
            address: &intruder,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "close_auction",
                args: (auction_id.clone(),).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_close_auction(&auction_id);

    assert!(
        result.is_err(),
        "close_auction must reject non-factory caller"
    );
}

#[test]
fn close_auction_succeeds_with_factory_auth() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);

    let factory = Address::generate(&env);
    client.set_factory_contract(&factory);

    let auction_id = Symbol::new(&env, "fac_close_ok");
    client.init_auction(
        &auction_id,
        &AuctionMode::English,
        &0_u64,
        &u64::MAX,
        &50_i128,
        &0_u32,
        &None,
        &None,
        &DutchAuctionDecay::None,
        &None,
    );

    let result = client
        .mock_auths(&[MockAuth {
            address: &factory,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "close_auction",
                args: (auction_id.clone(),).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_close_auction(&auction_id);

    assert!(result.is_ok(), "factory-authorized close_auction must succeed");
}

// ── set_liquidation_grace_window ─────────────────────────────────────────────

#[test]
fn set_liquidation_grace_window_reverts_when_factory_unset() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);

    let result = client.try_set_liquidation_grace_window(&3600_u64);

    assert!(
        result.is_err(),
        "set_liquidation_grace_window must fail when no factory is configured"
    );
}

#[test]
fn set_liquidation_grace_window_requires_factory_auth() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);

    let factory = Address::generate(&env);
    client.set_factory_contract(&factory);

    let intruder = Address::generate(&env);

    let result = client
        .mock_auths(&[MockAuth {
            address: &intruder,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_liquidation_grace_window",
                args: (3600_u64,).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_set_liquidation_grace_window(&3600_u64);

    assert!(
        result.is_err(),
        "set_liquidation_grace_window must reject non-factory caller"
    );
}

#[test]
fn set_liquidation_grace_window_succeeds_with_factory_auth() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);

    let factory = Address::generate(&env);
    client.set_factory_contract(&factory);

    let result = client
        .mock_auths(&[MockAuth {
            address: &factory,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_liquidation_grace_window",
                args: (3600_u64,).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_set_liquidation_grace_window(&3600_u64);

    assert!(
        result.is_ok(),
        "factory-authorized set_liquidation_grace_window must succeed"
    );
}

// ── settle_default_liquidation (factory auth) ────────────────────────────────

#[test]
fn settle_liquidation_rejects_non_factory_invoker() {
    let (env, contract_id, auction_id, factory, borrower, _expected) = setup_settleable();
    let client = AuctionClient::new(&env, &contract_id);

    let intruder = Address::generate(&env);

    let result = client
        .mock_auths(&[MockAuth {
            address: &intruder,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "settle_default_liquidation",
                args: (
                    auction_id.clone(),
                    factory.clone(),
                    borrower.clone(),
                )
                    .into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_settle_default_liquidation(&auction_id, &factory, &borrower);

    assert!(
        result.is_err(),
        "non-factory invoker must be rejected (require_auth prevents bypass)"
    );
}

#[test]
fn settle_liquidation_succeeds_with_factory_invoker() {
    let (env, contract_id, auction_id, factory, borrower, expected) = setup_settleable();
    let client = AuctionClient::new(&env, &contract_id);

    let result = client
        .mock_auths(&[MockAuth {
            address: &factory,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "settle_default_liquidation",
                args: (
                    auction_id.clone(),
                    factory.clone(),
                    borrower.clone(),
                )
                    .into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_settle_default_liquidation(&auction_id, &factory, &borrower);

    let recovered = result
        .expect("factory-authorized call must not encounter host error")
        .expect("factory-authorized call must not encounter contract error");
    assert_eq!(recovered, expected, "must return the highest bid");
}
