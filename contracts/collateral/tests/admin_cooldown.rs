// SPDX-License-Identifier: MIT

//! Regression tests for `AdminCollateralCooldownActive` (v7 collateral admin cool-off).

use creditra_credit::Credit;
use creditra_credit::CreditClient;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env, Vec};

const START_TS: u64 = 5_000;
const COOLDOWN_SECONDS: u64 = 120;

fn setup() -> (Env, CreditClient, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = START_TS);

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    (env, client, admin)
}

fn set_timestamp(env: &Env, timestamp: u64) {
    env.ledger().with_mut(|li| li.timestamp = timestamp);
}

fn assert_admin_collateral_cooldown_active(result: std::thread::Result<()>, context: &str) {
    let err = result.expect_err(context);
    let err_str = if let Some(s) = err.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = err.downcast_ref::<&str>() {
        s.to_string()
    } else {
        format!("{err:?}")
    };

    assert!(
        err_str.contains("Error(Contract, #54)"),
        "{context}: expected AdminCollateralCooldownActive (#54), got {err_str:?}"
    );
}

#[test]
fn admin_collateral_cooldown_zero_disables_guard() {
    let (env, client, _admin) = setup();
    client.set_admin_collateral_cooldown_seconds(&0_u64);

    client.set_min_collateral_ratio_bps(&12_000_u32);
    set_timestamp(&env, START_TS);
    client.set_min_collateral_ratio_bps(&13_000_u32);

    assert_eq!(client.get_min_collateral_ratio_bps(), Some(13_000));
}

#[test]
fn admin_collateral_cooldown_rejects_before_boundary_and_allows_at_boundary() {
    let (env, client, _admin) = setup();
    let asset = Address::generate(&env);

    client.set_admin_collateral_cooldown_seconds(&COOLDOWN_SECONDS);
    client.set_min_collateral_ratio_bps(&14_000_u32);

    set_timestamp(&env, START_TS + COOLDOWN_SECONDS - 1);
    assert_admin_collateral_cooldown_active(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.set_collateral_risk_weight(&asset, &5_000_u32);
        })),
        "second critical action one second before boundary must revert",
    );

    set_timestamp(&env, START_TS + COOLDOWN_SECONDS);
    client.set_collateral_risk_weight(&asset, &5_000_u32);
    assert_eq!(
        client.get_last_admin_collateral_critical_action_ts(),
        Some(START_TS + COOLDOWN_SECONDS)
    );
}

#[test]
fn configuring_cooldown_does_not_consume_cooldown_window() {
    let (env, client, _admin) = setup();

    client.set_admin_collateral_cooldown_seconds(&COOLDOWN_SECONDS);
    client.set_min_collateral_ratio_bps(&15_000_u32);

    set_timestamp(&env, START_TS + 1);
    client.set_admin_collateral_cooldown_seconds(&COOLDOWN_SECONDS);

    assert_admin_collateral_cooldown_active(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.set_min_collateral_ratio_bps(&16_000_u32);
        })),
        "cooldown config must not reset the critical-action clock",
    );
}

#[test]
fn allowlist_update_shares_single_cooldown_clock() {
    let (env, client, _admin) = setup();
    let token = Address::generate(&env);
    let mut tokens = Vec::new(&env);
    tokens.push_back(token.clone());

    client.set_admin_collateral_cooldown_seconds(&COOLDOWN_SECONDS);
    client.set_collateral_token_allowlist(&tokens);

    set_timestamp(&env, START_TS + 30);
    assert_admin_collateral_cooldown_active(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.set_min_collateral_ratio_bps(&10_000_u32);
        })),
        "different critical actions must share the same cooldown anchor",
    );
}
