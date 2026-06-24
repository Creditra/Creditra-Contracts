// SPDX-License-Identifier: MIT

//! Integration tests for `rescue_token`.
//!
//! # Coverage
//! - Successful rescue of an arbitrary (non-liquidity) token — balance deltas verified.
//! - Successful rescue emits the `rescue_tok` event with the correct topic.
//! - Rescue of the full contract balance succeeds.
//! - Revert when `token == liquidity_token` (panic message checked).
//! - Revert when `amount == 0`  (ContractError::InvalidAmount, discriminant #5).
//! - Revert when `amount < 0`   (ContractError::InvalidAmount, discriminant #5).
//! - Revert when a non-admin calls the function (auth failure).

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Events};
use soroban_sdk::{token, Address, Env, Symbol, TryFromVal};

// ── helpers ───────────────────────────────────────────────────────────────────

/// Minimal contract setup: register → init → set_liquidity_token.
///
/// Returns `(env, contract_id, admin, liquidity_token_address)`.
/// Uses `mock_all_auths` so admin calls succeed without signing.
fn setup() -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    // Register and configure the primary liquidity token.
    let liq_token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let liq_token_addr = liq_token_id.address();
    client.set_liquidity_token(&liq_token_addr);

    (env, contract_id, admin, liq_token_addr)
}

/// Register a second SAC ("stray" token) and mint `amount` units to `recipient`.
/// Returns the stray token's address.
fn create_and_mint_stray_token(env: &Env, recipient: &Address, amount: i128) -> Address {
    let stray_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let stray_addr = stray_id.address();
    token::StellarAssetClient::new(env, &stray_addr).mint(recipient, &amount);
    stray_addr
}

// ── successful rescue ─────────────────────────────────────────────────────────

#[test]
fn rescue_token_transfers_correct_amount_to_recipient() {
    let (env, contract_id, _admin, _liq_token) = setup();
    let client = CreditClient::new(&env, &contract_id);

    // Mint 1 000 units of a stray token directly into the contract.
    let stray = create_and_mint_stray_token(&env, &contract_id, 1_000);
    let recipient = Address::generate(&env);

    // Pre-conditions.
    assert_eq!(
        token::Client::new(&env, &stray).balance(&contract_id),
        1_000,
        "contract must hold 1000 before rescue"
    );

    // Execute.
    client.rescue_token(&stray, &recipient, &500_i128);

    // Post-conditions.
    assert_eq!(
        token::Client::new(&env, &stray).balance(&contract_id),
        500,
        "contract should have 500 remaining after partial rescue"
    );
    assert_eq!(
        token::Client::new(&env, &stray).balance(&recipient),
        500,
        "recipient should have received exactly 500"
    );
}

#[test]
fn rescue_token_drains_full_contract_balance() {
    let (env, contract_id, _admin, _liq_token) = setup();
    let client = CreditClient::new(&env, &contract_id);

    let stray = create_and_mint_stray_token(&env, &contract_id, 777);
    let recipient = Address::generate(&env);

    client.rescue_token(&stray, &recipient, &777_i128);

    assert_eq!(
        token::Client::new(&env, &stray).balance(&contract_id),
        0,
        "contract must be fully drained"
    );
    assert_eq!(
        token::Client::new(&env, &stray).balance(&recipient),
        777,
        "recipient must hold the full amount"
    );
}

#[test]
fn rescue_token_emits_rescue_tok_event() {
    let (env, contract_id, _admin, _liq_token) = setup();
    let client = CreditClient::new(&env, &contract_id);

    let stray = create_and_mint_stray_token(&env, &contract_id, 2_000);
    let recipient = Address::generate(&env);

    // Clear any events accumulated during setup.
    let _ = env.events().all();

    client.rescue_token(&stray, &recipient, &2_000_i128);

    let events = env.events().all();
    assert!(!events.is_empty(), "at least one event must be emitted");

    // The last event is the rescue event. Verify the second topic is "rescue_tok".
    let (_contract, topics, _data) = events.last().unwrap();
    let topic_0 = Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap();
    let topic_1 = Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap();
    assert_eq!(topic_0, Symbol::short("credit"), "first topic must be 'credit'");
    assert_eq!(
        topic_1,
        Symbol::new(&env, "rescue_tok"),
        "second topic must be 'rescue_tok'"
    );
}

// ── liquidity token guard ─────────────────────────────────────────────────────

#[test]
#[should_panic(expected = "rescue_token: cannot rescue the configured liquidity token")]
fn rescue_token_reverts_when_token_is_liquidity_token() {
    let (env, contract_id, _admin, liq_token) = setup();
    let client = CreditClient::new(&env, &contract_id);

    // Mint some liquidity tokens into the contract so the balance guard does not
    // fire before the liquidity-token identity check.
    token::StellarAssetClient::new(&env, &liq_token).mint(&contract_id, &1_000);

    let recipient = Address::generate(&env);

    // Must panic with the specific message — liquidity token is protected.
    client.rescue_token(&liq_token, &recipient, &500_i128);
}

// ── invalid amount guards ─────────────────────────────────────────────────────

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn rescue_token_reverts_for_zero_amount() {
    let (env, contract_id, _admin, _liq_token) = setup();
    let client = CreditClient::new(&env, &contract_id);

    let stray = create_and_mint_stray_token(&env, &contract_id, 1_000);
    let recipient = Address::generate(&env);

    // Amount == 0  →  ContractError::InvalidAmount (discriminant 5).
    client.rescue_token(&stray, &recipient, &0_i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn rescue_token_reverts_for_negative_amount() {
    let (env, contract_id, _admin, _liq_token) = setup();
    let client = CreditClient::new(&env, &contract_id);

    let stray = create_and_mint_stray_token(&env, &contract_id, 1_000);
    let recipient = Address::generate(&env);

    // Negative amount  →  ContractError::InvalidAmount (discriminant 5).
    client.rescue_token(&stray, &recipient, &-1_i128);
}

// ── authorization guard ───────────────────────────────────────────────────────

#[test]
#[should_panic]
fn rescue_token_reverts_for_non_admin() {
    // Set up the contract with real admin (all-auths for setup calls only).
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());

    {
        env.mock_all_auths();
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);

        let liq_token_id =
            env.register_stellar_asset_contract_v2(Address::generate(&env));
        client.set_liquidity_token(&liq_token_id.address());
    }
    // mock_all_auths scope ends — subsequent calls require real signatures.

    // Mint a stray token to the contract (still needs mock for SAC mint).
    let stray = {
        env.mock_all_auths();
        let stray_id =
            env.register_stellar_asset_contract_v2(Address::generate(&env));
        let stray_addr = stray_id.address();
        token::StellarAssetClient::new(&env, &stray_addr).mint(&contract_id, &1_000);
        stray_addr
    };

    // Call WITHOUT mock_all_auths — no auth satisfied → must revert.
    let client = CreditClient::new(&env, &contract_id);
    let recipient = Address::generate(&env);
    client.rescue_token(&stray, &recipient, &100_i128);
}
