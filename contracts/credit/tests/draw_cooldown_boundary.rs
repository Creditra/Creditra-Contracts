// SPDX-License-Identifier: MIT

//! Regression tests for `DrawCooldownActive` ledger timestamp boundaries.
//!
//! These cases lock the inequality used by `draw_credit`:
//! - `cooldown = 0` disables the guard entirely.
//! - `last_draw_ts + cooldown - 1` must still revert.
//! - `last_draw_ts + cooldown` must succeed.
//! - `last_draw_ts + cooldown + 1` must also succeed.
//!
//! The tests also prove that `LastDrawTs` only moves on successful draws by
//! chaining failed and successful attempts around exact ledger timestamps.

use creditra_credit::Credit;
use creditra_credit::CreditClient;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{token, Address, Env};

const START_TS: u64 = 1_000;
const COOLDOWN_SECONDS: u64 = 60;
const CREDIT_LIMIT: i128 = 10_000;
const RESERVE_BALANCE: i128 = 10_000;

fn setup_with_reserve(start_ts: u64) -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = start_ts);

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);
    client.set_liquidity_source(&contract_id);
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &RESERVE_BALANCE);

    client.open_credit_line(&borrower, &CREDIT_LIMIT, &300_u32, &70_u32);

    (env, contract_id, borrower)
}

fn set_timestamp(env: &Env, timestamp: u64) {
    env.ledger().with_mut(|li| li.timestamp = timestamp);
}

fn assert_draw_cooldown_active(result: std::thread::Result<()>, context: &str) {
    let err = result.expect_err(context);
    let err_str = if let Some(s) = err.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = err.downcast_ref::<&str>() {
        s.to_string()
    } else {
        format!("{err:?}")
    };

    assert!(
        err_str.contains("Error(Contract, #29)"),
        "{context}: expected DrawCooldownActive (#29), got {err_str:?}"
    );
}

#[test]
fn draw_credit_cooldown_zero_disables_guard() {
    let (env, contract_id, borrower) = setup_with_reserve(START_TS);
    let client = CreditClient::new(&env, &contract_id);

    client.set_draw_min_interval(&0_u64);

    client.draw_credit(&borrower, &200_i128);

    // Reuse the exact same ledger timestamp to prove `cooldown = 0` disables
    // the time gate instead of merely shortening it.
    set_timestamp(&env, START_TS);
    client.draw_credit(&borrower, &100_i128);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 300);
}

#[test]
fn draw_credit_cooldown_rejects_one_second_before_boundary_and_allows_exact_boundary() {
    let (env, contract_id, borrower) = setup_with_reserve(START_TS);
    let client = CreditClient::new(&env, &contract_id);

    client.set_draw_min_interval(&COOLDOWN_SECONDS);
    client.draw_credit(&borrower, &200_i128);

    set_timestamp(&env, START_TS + COOLDOWN_SECONDS - 1);
    let just_under = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.draw_credit(&borrower, &100_i128);
    }));
    assert_draw_cooldown_active(
        just_under,
        "draw one second before the cooldown boundary must revert",
    );
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().utilized_amount,
        200
    );

    set_timestamp(&env, START_TS + COOLDOWN_SECONDS);
    client.draw_credit(&borrower, &100_i128);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 300);
}

#[test]
fn draw_credit_cooldown_updates_anchor_only_after_successful_draws() {
    let (env, contract_id, borrower) = setup_with_reserve(START_TS);
    let client = CreditClient::new(&env, &contract_id);

    client.set_draw_min_interval(&COOLDOWN_SECONDS);
    client.draw_credit(&borrower, &200_i128);

    // A failed draw at t=1059 must not move the stored success anchor away from
    // the initial t=1000 draw.
    set_timestamp(&env, START_TS + COOLDOWN_SECONDS - 1);
    let just_under = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.draw_credit(&borrower, &100_i128);
    }));
    assert_draw_cooldown_active(
        just_under,
        "failed draw must preserve the previous successful cooldown anchor",
    );
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().utilized_amount,
        200
    );

    // The exact boundary still succeeds, proving the failed attempt above did
    // not overwrite `LastDrawTs`.
    set_timestamp(&env, START_TS + COOLDOWN_SECONDS);
    client.draw_credit(&borrower, &100_i128);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().utilized_amount,
        300
    );

    // After the successful draw at t=1060, the new anchor must be 1060. A draw
    // at t=1119 must therefore still fail.
    set_timestamp(&env, START_TS + (COOLDOWN_SECONDS * 2) - 1);
    let under_new_anchor = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.draw_credit(&borrower, &100_i128);
    }));
    assert_draw_cooldown_active(
        under_new_anchor,
        "successful draw must refresh the cooldown anchor for the next window",
    );
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().utilized_amount,
        300
    );

    // One second after the refreshed boundary must succeed.
    set_timestamp(&env, START_TS + (COOLDOWN_SECONDS * 2) + 1);
    client.draw_credit(&borrower, &100_i128);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 400);
}
