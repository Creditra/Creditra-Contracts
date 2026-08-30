// SPDX-License-Identifier: MIT

//! Issue #1169 — fee-configuration changes are rejected while a liquidation
//! auction is active.
//!
//! # State invariant
//!
//! An auction is considered **active** from the moment a credit line enters
//! [`CreditStatus::Defaulted`] until it exits the `Defaulted` pipeline through
//! one of the four terminal paths:
//!
//! 1. `settle_default_liquidation` with full recovery (`Defaulted → Closed`)
//! 2. `reinstate_credit_line` (`Defaulted → Active | Restricted`)
//! 3. `close_credit_line` admin force-close (`Defaulted → Closed`)
//! 4. `open_credit_line` admin reopen (`Defaulted → Active`)
//!
//! A **partial** settlement leaves the line in `Defaulted`, so the auction
//! stays active. While at least one auction is active, every fee-configuration
//! entrypoint — `set_protocol_fee_bps`, `set_treasury_fee_share_bps`,
//! `set_penalty_surcharge_bps`, `set_late_fee_flat`, and
//! `set_late_fee_config` — reverts with [`ContractError::AuctionActive`] (63).
//!
//! # Coverage
//!
//! - **Success paths**: all five fee setters work when no auction is active.
//! - **Forced rejections**: all five fee setters revert with `AuctionActive`
//!   while an auction is active, and stored fee values are untouched.
//! - **Boundary transitions**: full settlement, partial settlement, reinstate,
//!   admin force-close, and reopen each move the guard deterministically.
//! - **Concurrency / retry safety**: multiple simultaneous auctions keep the
//!   guard engaged until the last one exits; retried rejections are
//!   deterministic; the pending-auction counter never drifts negative.

use creditra_credit::types::{ContractError, CreditStatus};
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, Address, Env, Symbol};

/// Deploy the contract, fund a real SAC, and wire liquidity + reserve.
fn setup<'a>(env: &'a Env) -> (CreditClient<'a>, Address, Address, Address, Address) {
    env.mock_all_auths();

    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);
    client.set_liquidity_source(&contract_id);

    (client, admin, borrower, contract_id, token_address)
}

/// Open a line, deposit collateral, draw, and default it so its liquidation
/// auction becomes active with `utilized_amount == DRAW_AMOUNT`.
fn open_draw_and_default(
    env: &Env,
    client: &CreditClient,
    borrower: &Address,
    contract_id: &Address,
    token_address: &Address,
) {
    client.open_credit_line(borrower, &1_000_i128, &500_u32, &50_u32);

    // draw_credit enforces the default 150% minimum collateral ratio.
    let asset = token::StellarAssetClient::new(env, token_address);
    asset.mint(borrower, &2_000_i128);
    client.deposit_collateral(borrower, &2_000_i128);

    // Fund the reserve (liquidity source = the contract itself).
    asset.mint(contract_id, &1_000_i128);
    client.draw_credit(borrower, &800_i128);

    client.default_credit_line(borrower);
    assert_eq!(client.get_pending_auction_count(), 1);
}

/// Assert that all five fee-configuration entrypoints revert with
/// [`ContractError::AuctionActive`].
fn assert_fee_configs_rejected(client: &CreditClient) {
    for (result, what) in [
        (
            client.try_set_protocol_fee_bps(&1_000_u32),
            "set_protocol_fee_bps",
        ),
        (
            client.try_set_treasury_fee_share_bps(&5_000_u32),
            "set_treasury_fee_share_bps",
        ),
        (
            client.try_set_penalty_surcharge_bps(&500_u32),
            "set_penalty_surcharge_bps",
        ),
        (client.try_set_late_fee_flat(&100_i128), "set_late_fee_flat"),
        (client.try_set_late_fee_config(&None), "set_late_fee_config"),
    ] {
        assert!(
            result.is_err(),
            "{what} must revert while an auction is active"
        );
        assert_eq!(
            result.err().unwrap().unwrap(),
            ContractError::AuctionActive.into(),
            "{what} must revert with AuctionActive"
        );
    }
}

/// Assert that all five fee-configuration entrypoints succeed and persist.
fn set_all_fee_configs(client: &CreditClient) {
    client.set_protocol_fee_bps(&1_000_u32);
    client.set_treasury_fee_share_bps(&5_000_u32);
    client.set_penalty_surcharge_bps(&500_u32);
    client.set_late_fee_flat(&100_i128);
    client.set_late_fee_config(&None);
}

fn assert_all_fee_configs_persisted(client: &CreditClient) {
    assert_eq!(client.get_protocol_fee_bps(), Some(1_000));
    assert_eq!(client.get_treasury_fee_share_bps(), Some(5_000));
    assert_eq!(client.get_penalty_surcharge_bps(), 500);
    assert_eq!(client.get_late_fee_flat(), 100);
}

// ── Success paths ────────────────────────────────────────────────────────────

#[test]
fn fee_configs_settable_when_no_auction_active() {
    let env = Env::default();
    let (client, _admin, _borrower, _contract_id, _token) = setup(&env);

    assert_eq!(client.get_pending_auction_count(), 0);
    set_all_fee_configs(&client);
    assert_all_fee_configs_persisted(&client);
}

// ── Forced rejections ────────────────────────────────────────────────────────

#[test]
fn fee_configs_rejected_while_auction_active() {
    let env = Env::default();
    let (client, _admin, borrower, contract_id, token_address) = setup(&env);

    open_draw_and_default(&env, &client, &borrower, &contract_id, &token_address);

    assert_fee_configs_rejected(&client);

    // Stored fee values must be untouched by the rejected updates.
    assert_eq!(client.get_protocol_fee_bps(), None);
    assert_eq!(client.get_treasury_fee_share_bps(), None);
    assert_eq!(client.get_penalty_surcharge_bps(), 0);
    assert_eq!(client.get_late_fee_flat(), 0);
}

#[test]
fn non_fee_admin_config_still_allowed_while_auction_active() {
    let env = Env::default();
    let (client, _admin, borrower, contract_id, token_address) = setup(&env);

    open_draw_and_default(&env, &client, &borrower, &contract_id, &token_address);

    // Scope boundary: only fee configurations are frozen (Issue #1169);
    // other admin config surfaces stay available to operators.
    client.set_max_draw_amount(&10_000_i128);
    assert_eq!(client.get_max_draw_amount(), Some(10_000));
}

// ── Boundary transitions ─────────────────────────────────────────────────────

#[test]
fn fee_configs_allowed_after_full_settlement() {
    let env = Env::default();
    let (client, _admin, borrower, contract_id, token_address) = setup(&env);

    open_draw_and_default(&env, &client, &borrower, &contract_id, &token_address);

    // Full recovery closes the line and ends the auction.
    client.settle_default_liquidation(
        &borrower,
        &800_i128,
        &Symbol::new(&env, "settle_full"),
        &10_000_u32,
        &None,
    );
    assert_eq!(client.get_pending_auction_count(), 0);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Closed
    );

    set_all_fee_configs(&client);
    assert_all_fee_configs_persisted(&client);
}

#[test]
fn fee_configs_still_blocked_after_partial_settlement() {
    let env = Env::default();
    let (client, _admin, borrower, contract_id, token_address) = setup(&env);

    open_draw_and_default(&env, &client, &borrower, &contract_id, &token_address);

    // Partial recovery keeps the line `Defaulted`, so the auction stays active.
    client.settle_default_liquidation(
        &borrower,
        &300_i128,
        &Symbol::new(&env, "settle_partial"),
        &10_000_u32,
        &None,
    );
    assert_eq!(client.get_pending_auction_count(), 1);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Defaulted
    );

    assert_fee_configs_rejected(&client);
}

#[test]
fn fee_configs_allowed_after_reinstate() {
    let env = Env::default();
    let (client, _admin, borrower, contract_id, token_address) = setup(&env);

    open_draw_and_default(&env, &client, &borrower, &contract_id, &token_address);

    client.reinstate_credit_line(&borrower, &CreditStatus::Active);
    assert_eq!(client.get_pending_auction_count(), 0);

    set_all_fee_configs(&client);
    assert_all_fee_configs_persisted(&client);
}

#[test]
fn fee_configs_allowed_after_admin_force_close() {
    let env = Env::default();
    let (client, admin, borrower, contract_id, token_address) = setup(&env);

    open_draw_and_default(&env, &client, &borrower, &contract_id, &token_address);

    // Admin force-close from `Defaulted` abandons the auction.
    client.close_credit_line(&borrower, &admin);
    assert_eq!(client.get_pending_auction_count(), 0);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Closed
    );

    set_all_fee_configs(&client);
    assert_all_fee_configs_persisted(&client);
}

#[test]
fn fee_configs_allowed_after_reopen_of_defaulted_line() {
    let env = Env::default();
    let (client, _admin, borrower, contract_id, token_address) = setup(&env);

    open_draw_and_default(&env, &client, &borrower, &contract_id, &token_address);

    // Reopening a `Defaulted` line replaces it with a fresh `Active` line.
    client.open_credit_line(&borrower, &1_000_i128, &500_u32, &50_u32);
    assert_eq!(client.get_pending_auction_count(), 0);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Active
    );

    set_all_fee_configs(&client);
    assert_all_fee_configs_persisted(&client);
}

// ── Concurrency / retry safety ───────────────────────────────────────────────

#[test]
fn multiple_auctions_lock_until_last_one_exits() {
    let env = Env::default();
    let (client, _admin, borrower, contract_id, token_address) = setup(&env);
    let borrower2 = Address::generate(&env);

    // Second borrower with its own collateral and reserve funding.
    let asset = token::StellarAssetClient::new(&env, &token_address);
    asset.mint(&borrower2, &2_000_i128);
    asset.mint(&contract_id, &1_000_i128);

    open_draw_and_default(&env, &client, &borrower, &contract_id, &token_address);
    client.open_credit_line(&borrower2, &1_000_i128, &500_u32, &50_u32);
    client.deposit_collateral(&borrower2, &2_000_i128);
    client.draw_credit(&borrower2, &800_i128);
    client.default_credit_line(&borrower2);
    assert_eq!(client.get_pending_auction_count(), 2);

    // Settling only one borrower must keep the guard engaged.
    client.settle_default_liquidation(
        &borrower,
        &800_i128,
        &Symbol::new(&env, "settle_b1"),
        &10_000_u32,
        &None,
    );
    assert_eq!(client.get_pending_auction_count(), 1);
    assert_fee_configs_rejected(&client);

    // The last active auction exiting releases the lock.
    client.settle_default_liquidation(
        &borrower2,
        &800_i128,
        &Symbol::new(&env, "settle_b2"),
        &10_000_u32,
        &None,
    );
    assert_eq!(client.get_pending_auction_count(), 0);
    set_all_fee_configs(&client);
    assert_all_fee_configs_persisted(&client);
}

#[test]
fn rejected_updates_are_deterministic_and_counter_never_drifts() {
    let env = Env::default();
    let (client, _admin, borrower, contract_id, token_address) = setup(&env);

    open_draw_and_default(&env, &client, &borrower, &contract_id, &token_address);

    // Repeated rejected updates neither mutate state nor move the counter.
    for _ in 0..3 {
        assert_fee_configs_rejected(&client);
        assert_eq!(client.get_pending_auction_count(), 1);
    }

    // Full settlement exits the auction; the counter lands exactly on zero.
    let settlement_id = Symbol::new(&env, "settle_once");
    client.settle_default_liquidation(&borrower, &800_i128, &settlement_id, &10_000_u32, &None);
    assert_eq!(client.get_pending_auction_count(), 0);

    // Retrying the settlement with the same id is replay-protected and cannot
    // decrement the counter a second time (it is already zero).
    let replay = client.try_settle_default_liquidation(
        &borrower,
        &100_i128,
        &settlement_id,
        &10_000_u32,
        &None,
    );
    assert!(replay.is_err());
    assert_eq!(client.get_pending_auction_count(), 0);

    // The guard is fully released; fee configs are settable again.
    set_all_fee_configs(&client);
    assert_all_fee_configs_persisted(&client);
}
