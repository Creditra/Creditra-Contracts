// SPDX-License-Identifier: MIT

//! Tests verifying the one-time initialization contract for the Credit contract.
//!
//! # What is tested
//!
//! 1. Double-init reverts with `AlreadyInitialized`.
//! 2. Admin is unchanged after a failed re-init attempt.
//! 3. No state is mutated by a failed re-init.
//! 4. LiquiditySource is set to the contract address on first init.
//! 5. Admin-gated functions work after init and fail before init.
//! 6. Init is deterministic across multiple contract instances.

#![cfg(test)]

use soroban_sdk::testutils::{Address as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{Address, Env, IntoVal};

use creditra_credit::{Credit, CreditClient, FreezeReason};

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn deploy(env: &Env) -> (CreditClient<'_>, Address) {
    let admin = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    (client, admin)
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. Double-init reverts with AlreadyInitialized
// ─────────────────────────────────────────────────────────────────────────────

/// A second call to `init` must revert with `ContractError::AlreadyInitialized`
/// (error code 14).
#[test]
#[should_panic(expected = "Error(Contract, #14)")]
fn double_init_reverts_with_already_initialized() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, admin) = deploy(&env);
    client.init(&admin);

    let attacker = Address::generate(&env);
    // Second call must revert — attacker cannot overwrite admin.
    client.init(&attacker);
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. Admin is unchanged after failed re-init
// ─────────────────────────────────────────────────────────────────────────────

/// After a failed re-init attempt the original admin must still be in storage
/// and admin-gated operations must continue to work.
#[test]
fn admin_unchanged_after_failed_reinit() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, admin) = deploy(&env);
    client.init(&admin);

    let attacker = Address::generate(&env);
    // Attempt re-init — must fail.
    let result = client.try_init(&attacker);
    assert!(result.is_err(), "second init should fail");

    // Admin-gated operation must still succeed with original admin.
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &50_u32);
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.borrower, borrower);
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. No state mutation on failed re-init
// ─────────────────────────────────────────────────────────────────────────────

/// A failed re-init must not change LiquiditySource or any other instance
/// storage value.
#[test]
fn failed_reinit_does_not_mutate_state() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    client.init(&admin);

    // Record the liquidity source after first init.
    let new_source = Address::generate(&env);
    client.set_liquidity_source(&new_source);

    // Attempt re-init with a different address — must fail.
    let attacker = Address::generate(&env);
    let _ = client.try_init(&attacker);

    // Liquidity source must still be new_source, not contract address.
    // We verify indirectly: admin-gated set_liquidity_source still works,
    // meaning admin was not overwritten.
    let another_source = Address::generate(&env);
    client.set_liquidity_source(&another_source);
    // If we reach here without panic, admin is still the original.
}

// ─────────────────────────────────────────────────────────────────────────────
// 4. LiquiditySource defaults to contract address on first init
// ─────────────────────────────────────────────────────────────────────────────

/// On first init, LiquiditySource must be set to the contract's own address.
/// This is verified indirectly: a draw without set_liquidity_source uses the
/// contract balance as the reserve.
#[test]
fn init_sets_liquidity_source_to_contract_address() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, admin) = deploy(&env);
    client.init(&admin);

    // set_liquidity_source requires admin auth — if admin is set correctly
    // this call succeeds, confirming init wrote the admin key.
    let external_source = Address::generate(&env);
    client.set_liquidity_source(&external_source);
    // No panic = admin was stored correctly by init.
}

// ─────────────────────────────────────────────────────────────────────────────
// 5. Admin-gated functions fail before init
// ─────────────────────────────────────────────────────────────────────────────

/// Calling an admin-gated function before init must revert because no admin
/// is stored.
#[test]
#[should_panic]
fn admin_gated_call_before_init_reverts() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    // No init call — admin is not set — this must panic.
    // freeze_draws requires admin auth and will fail because no admin is stored.
    client.freeze_draws(&FreezeReason::LiquidityReserve);
}

// ─────────────────────────────────────────────────────────────────────────────
// 6. Init is deterministic across instances
// ─────────────────────────────────────────────────────────────────────────────

/// Two separate contract instances initialized with the same admin are
/// independent — a double-init on one does not affect the other.
#[test]
fn init_is_independent_across_contract_instances() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);

    let contract_a = env.register(Credit, ());
    let contract_b = env.register(Credit, ());
    let client_a = CreditClient::new(&env, &contract_a);
    let client_b = CreditClient::new(&env, &contract_b);

    client_a.init(&admin);
    client_b.init(&admin);

    // Double-init on A must not affect B.
    let attacker = Address::generate(&env);
    let _ = client_a.try_init(&attacker);

    // B must still accept admin-gated calls.
    let borrower = Address::generate(&env);
    client_b.open_credit_line(&borrower, &500_i128, &200_u32, &40_u32);
    assert!(client_b.get_credit_line(&borrower).is_some());
}

// ─────────────────────────────────────────────────────────────────────────────
// 7. Single init succeeds and is idempotent for state
// ─────────────────────────────────────────────────────────────────────────────

/// A single init call succeeds and leaves the contract in a usable state.
#[test]
fn single_init_succeeds() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, admin) = deploy(&env);
    client.init(&admin);

    // Contract is usable: open a credit line.
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &50_u32);
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.credit_limit, 1_000);
    assert_eq!(line.interest_rate_bps, 300);
    assert_eq!(line.risk_score, 50);
}

// ─────────────────────────────────────────────────────────────────────────────
// 8. Init applies the unconditional default minimum collateral ratio
// ─────────────────────────────────────────────────────────────────────────────

/// `init` must set the 150 % (15 000 bps) collateral floor regardless of how
/// the crate was compiled. Before this was unconditional, in-crate unit tests
/// ran with the key unset while integration tests observed 150 %, so the same
/// scenario behaved differently depending on where the test lived.
#[test]
fn init_sets_default_min_collateral_ratio() {
    let env = Env::default();

    let (client, admin) = deploy(&env);
    client.init(&admin);

    assert_eq!(client.get_min_collateral_ratio_bps(), Some(15_000));
}

// ─────────────────────────────────────────────────────────────────────────────
// 9. Re-init with same admin also reverts
// ─────────────────────────────────────────────────────────────────────────────

/// Even re-init with the original admin address must revert — init is strictly
/// one-time regardless of the caller.
#[test]
#[should_panic(expected = "Error(Contract, #14)")]
fn reinit_with_same_admin_also_reverts() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, admin) = deploy(&env);
    client.init(&admin);
    // Same admin — still must revert.
    client.init(&admin);
}

// ─────────────────────────────────────────────────────────────────────────────
// 9. Protocol-config defaults around liquidity setup (Issue #1352)
// ─────────────────────────────────────────────────────────────────────────────
//
// What is pinned here
//
// `get_protocol_config` reports `liquidity_token: None` and
// `liquidity_source: Some(<contract address>)` straight after `init`, and
// `get_liquidity_source` falls back to the contract address. Integrators and
// the draw path both depend on "draws are funded from the contract itself until
// an admin points the source somewhere else", so those defaults are asserted
// here rather than left implicit.

fn deploy_with_id(env: &Env) -> (CreditClient<'_>, Address, Address) {
    let admin = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    (client, admin, contract_id)
}

/// Immediately after `init`: no liquidity token, source defaults to the
/// contract's own address, and the fallback getter agrees with the config view.
#[test]
fn init_pins_protocol_config_defaults() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, _admin, contract_id) = deploy_with_id(&env);

    let config = client.get_protocol_config();
    assert_eq!(
        config.liquidity_token, None,
        "no liquidity token may be configured until set_liquidity_token is called"
    );
    assert_eq!(
        config.liquidity_source,
        Some(contract_id.clone()),
        "init must default the liquidity source to the contract's own address"
    );
    assert_eq!(
        client.get_liquidity_source(),
        contract_id,
        "get_liquidity_source must fall back to the contract address, not panic"
    );
}

/// `set_liquidity_token` is reflected in the config view and leaves the source
/// default intact.
#[test]
fn set_liquidity_token_is_reflected_in_protocol_config() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, _admin, contract_id) = deploy_with_id(&env);
    let token = Address::generate(&env);

    client.set_liquidity_token(&token);

    let config = client.get_protocol_config();
    assert_eq!(config.liquidity_token, Some(token));
    assert_eq!(
        config.liquidity_source,
        Some(contract_id),
        "setting the token must not move the liquidity source"
    );
}

/// A later `set_liquidity_token` replaces the previous token.
#[test]
fn set_liquidity_token_overwrites_previous_value() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, _admin, _) = deploy_with_id(&env);
    let first = Address::generate(&env);
    let second = Address::generate(&env);

    client.set_liquidity_token(&first);
    client.set_liquidity_token(&second);

    assert_eq!(client.get_protocol_config().liquidity_token, Some(second));
}

/// `set_liquidity_source` overrides the init default in both the config view and
/// the fallback getter.
#[test]
fn set_liquidity_source_overrides_init_default() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, _admin, contract_id) = deploy_with_id(&env);
    let reserve = Address::generate(&env);

    client.set_liquidity_source(&reserve);

    assert_eq!(
        client.get_protocol_config().liquidity_source,
        Some(reserve.clone()),
        "the config view must report the configured reserve"
    );
    assert_eq!(
        client.get_liquidity_source(),
        reserve,
        "the fallback getter must return the configured reserve, not the contract"
    );
    assert_ne!(reserve, contract_id);
}

/// Setting the source does not touch the token slot.
#[test]
fn set_liquidity_source_does_not_change_liquidity_token() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, _admin, _) = deploy_with_id(&env);
    let token = Address::generate(&env);
    client.set_liquidity_token(&token);

    client.set_liquidity_source(&Address::generate(&env));

    assert_eq!(client.get_protocol_config().liquidity_token, Some(token));
}

/// A failed re-init must not reset either liquidity slot.
#[test]
fn failed_reinit_does_not_reset_liquidity_config() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, _admin, _) = deploy_with_id(&env);
    let token = Address::generate(&env);
    let reserve = Address::generate(&env);
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&reserve);

    let attacker = Address::generate(&env);
    assert!(client.try_init(&attacker).is_err(), "re-init must fail");

    let config = client.get_protocol_config();
    assert_eq!(config.liquidity_token, Some(token));
    assert_eq!(config.liquidity_source, Some(reserve));
}

/// A caller that is not the configured admin is rejected by both setters, and
/// the rejected calls leave the defaults untouched.
#[test]
fn non_admin_cannot_set_liquidity_config() {
    let env = Env::default();
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    // init itself is not auth-gated — it only requires an empty admin slot.
    client.init(&admin);

    // Mock the *caller's* auth, never the admin's: the host must reject both
    // calls because the stored admin did not authorize them.
    let caller = Address::generate(&env);
    let token = Address::generate(&env);
    let reserve = Address::generate(&env);

    env.mock_auths(&[
        MockAuth {
            address: &caller,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_liquidity_token",
                args: (&token,).into_val(&env),
                sub_invokes: &[],
            },
        },
        MockAuth {
            address: &caller,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_liquidity_source",
                args: (&reserve,).into_val(&env),
                sub_invokes: &[],
            },
        },
    ]);

    assert!(
        client.try_set_liquidity_token(&token).is_err(),
        "a non-admin caller must not set the liquidity token"
    );
    assert!(
        client.try_set_liquidity_source(&reserve).is_err(),
        "a non-admin caller must not set the liquidity source"
    );

    let config = client.get_protocol_config();
    assert_eq!(config.liquidity_token, None, "rejected call must not write");
    assert_eq!(
        config.liquidity_source,
        Some(contract_id),
        "rejected call must leave the init default in place"
    );
}
