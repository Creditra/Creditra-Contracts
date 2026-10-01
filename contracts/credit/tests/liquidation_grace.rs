// SPDX-License-Identifier: MIT

//! Integration tests for the per-borrower liquidation grace window (Issue #1325).
//!
//! # What is under test
//!
//! `set_borrower_liq_grace` / `get_borrower_liq_grace` are admin-facing wrappers
//! over [`creditra_credit::lifecycle::set_per_borrower_liquidation_grace`] and
//! [`creditra_credit::lifecycle::get_per_borrower_liquidation_grace`]. When a
//! grace period is configured for a borrower, [`Credit::default_credit_line`]
//! refuses to move the line to `Defaulted` while the ledger is still inside the
//! window, reverting with [`ContractError::LiquidationGraceActive`] (59).
//!
//! The window is not anchored to a single field: the guard picks a *base
//! timestamp* from the first non-zero source in this fixed priority order:
//!
//! ```text
//! 1. suspension_ts        (set by suspend / self_suspend)
//! 2. next_due_ts          (set by set_repayment_schedule)
//! 3. last_rate_update_ts  (set by update_risk_parameters under a rate-change config)
//! 4. last_accrual_ts      (set at origination and advanced by every accrual)
//! ```
//!
//! Default is blocked while `now < base_ts + grace_seconds` and allowed once
//! `now >= base_ts + grace_seconds`, so `now == base_ts + grace_seconds` is the
//! exact boundary at which default becomes legal.
//!
//! # Acceptance-criteria map
//!
//! | Criterion | Tests |
//! |-----------|-------|
//! | Default blocked inside grace for each base source | `default_blocked_inside_grace_base_suspension_ts`, `..._base_next_due_ts`, `..._base_last_rate_update_ts`, `..._base_last_accrual_ts` |
//! | Default allowed at exact boundary (`now == base + grace`) | same four tests, second half |
//! | Setting grace on a `Closed` line reverts | `set_grace_on_closed_line_reverts` |
//! | Test documents accrual-driven window drift | `last_accrual_ts_fallback_window_drifts_with_keeper_accrual`, `last_accrual_ts_fallback_cannot_expire_while_interest_accrues` |
//!
//! Additional coverage: the setter/getter round-trip, `0` removing the config,
//! per-borrower isolation, unset-line handling, and the source precedence chain.
//!
//! # Follow-up (do not fix here)
//!
//! The `last_accrual_ts` fallback is unsound as a grace anchor: it is the same
//! field `apply_accrual` advances, and `default_credit_line` accrues *before*
//! choosing the base. Any line with non-zero utilization and a non-zero rate is
//! therefore blocked forever once it reaches fallback #4, regardless of elapsed
//! time. The two drift tests pin that behaviour down so the fix (anchor the
//! window to a source accrual cannot move, or capture the base before accrual)
//! can be validated by flipping these assertions.

use creditra_credit::types::{ContractError, CreditStatus};
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{Address, Env, Vec};

/// Timestamp used for origination in every scenario.
const OPEN_TS: u64 = 1_000;
/// Grace window used in the boundary tests.
const GRACE: u64 = 100;

// ── helpers ───────────────────────────────────────────────────────────────────

/// Deploy, initialise, and fund a fresh credit contract.
///
/// Returns `(env, contract_id, admin)`. Callers construct a
/// [`CreditClient`] from `contract_id` so the borrow of `env` stays local to the
/// test and the helper does not have to name a lifetime.
fn deploy() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    StellarAssetClient::new(&env, &token).mint(&contract_id, &10_000_000_000_i128);

    // The grace window is orthogonal to the collateral floor. Dial the floor to
    // 0 so draws succeed without a collateral deposit and each test isolates the
    // liquidation-grace behaviour it documents.
    client.set_min_collateral_ratio_bps(&0_u32);

    (env, contract_id, admin)
}

/// Open a line for `borrower` at the current ledger timestamp, then draw
/// `draw_amount` (skip the draw when it is `0`).
fn open(client: &CreditClient<'_>, borrower: &Address, rate_bps: u32, draw_amount: i128) {
    client.open_credit_line(borrower, &1_000_000_i128, &rate_bps, &50_u32);
    if draw_amount > 0 {
        client.draw_credit(borrower, &draw_amount);
    }
}

/// A one-element borrower batch for `accrue_batch`.
fn single(env: &Env, borrower: &Address) -> Vec<Address> {
    let mut v = Vec::new(env);
    v.push_back(borrower.clone());
    v
}

/// Assert that `default_credit_line` is rejected *specifically* because the
/// liquidation grace window is still active (not for any other lifecycle or
/// authorization reason).
fn assert_default_blocked(client: &CreditClient<'_>, borrower: &Address) {
    let result = client.try_default_credit_line(borrower);
    assert!(
        result.is_err(),
        "default_credit_line must revert while the grace window is active"
    );
    assert_eq!(
        result.err().unwrap().unwrap(),
        ContractError::LiquidationGraceActive.into(),
        "revert must be LiquidationGraceActive (59), not a different failure"
    );
    assert_ne!(
        client.get_credit_line(borrower).unwrap().status,
        CreditStatus::Defaulted,
        "a blocked default must leave the line in its prior status"
    );
}

// ── setter / getter semantics ─────────────────────────────────────────────────

/// `set_borrower_liq_grace` stores the value and `get_borrower_liq_grace`
/// returns it; the default for an unconfigured borrower is `0`.
#[test]
fn set_and_get_grace_round_trip() {
    let (env, contract_id, _admin) = deploy();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    env.ledger().set_timestamp(OPEN_TS);
    open(&client, &borrower, 1_000, 100_000);

    assert_eq!(
        client.get_borrower_liq_grace(&borrower),
        0,
        "unconfigured borrower must read back as 0"
    );

    client.set_borrower_liq_grace(&borrower, &3_600_u64);
    assert_eq!(client.get_borrower_liq_grace(&borrower), 3_600);

    // Overwriting is allowed (unlike the VRF commitment, this is not
    // write-once).
    client.set_borrower_liq_grace(&borrower, &7_200_u64);
    assert_eq!(client.get_borrower_liq_grace(&borrower), 7_200);
}

/// Passing `0` removes the configured window rather than storing a zero.
#[test]
fn zero_grace_removes_configuration() {
    let (env, contract_id, _admin) = deploy();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    env.ledger().set_timestamp(OPEN_TS);
    open(&client, &borrower, 1_000, 100_000);

    client.set_borrower_liq_grace(&borrower, &600_u64);
    assert_eq!(client.get_borrower_liq_grace(&borrower), 600);

    client.set_borrower_liq_grace(&borrower, &0_u64);
    assert_eq!(client.get_borrower_liq_grace(&borrower), 0);

    // With the window removed, default is immediately available.
    client.default_credit_line(&borrower);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Defaulted
    );
}

/// The grace period is keyed per borrower and never leaks across lines.
#[test]
fn grace_is_isolated_per_borrower() {
    let (env, contract_id, _admin) = deploy();
    let client = CreditClient::new(&env, &contract_id);
    let configured = Address::generate(&env);
    let untouched = Address::generate(&env);

    env.ledger().set_timestamp(OPEN_TS);
    open(&client, &configured, 1_000, 100_000);
    open(&client, &untouched, 1_000, 100_000);

    client.set_borrower_liq_grace(&configured, &900_u64);

    assert_eq!(client.get_borrower_liq_grace(&configured), 900);
    assert_eq!(
        client.get_borrower_liq_grace(&untouched),
        0,
        "configuring one borrower must not affect another"
    );
}

/// Configuring a grace period for an address with no credit line reverts.
#[test]
fn set_grace_for_unknown_borrower_reverts() {
    let (env, contract_id, _admin) = deploy();
    let client = CreditClient::new(&env, &contract_id);
    let ghost = Address::generate(&env);

    let result = client.try_set_borrower_liq_grace(&ghost, &60_u64);
    assert!(result.is_err(), "unknown borrower must be rejected");
    assert_eq!(
        result.err().unwrap().unwrap(),
        ContractError::CreditLineNotFound.into()
    );
}

/// Setting a grace period on a `Closed` line reverts with `CreditLineClosed`
/// and does not mutate the stored configuration.
#[test]
fn set_grace_on_closed_line_reverts() {
    let (env, contract_id, admin) = deploy();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    env.ledger().set_timestamp(OPEN_TS);
    // No draw: an admin can always close, and this keeps the scenario focused on
    // the grace guard rather than utilization rules.
    open(&client, &borrower, 1_000, 0);
    client.close_credit_line(&borrower, &admin);

    let result = client.try_set_borrower_liq_grace(&borrower, &60_u64);
    assert!(
        result.is_err(),
        "setting grace on a Closed line must revert"
    );
    assert_eq!(
        result.err().unwrap().unwrap(),
        ContractError::CreditLineClosed.into()
    );
    assert_eq!(
        client.get_borrower_liq_grace(&borrower),
        0,
        "a rejected set must not persist a value"
    );
}

/// Control: with no grace configured, default is never blocked.
#[test]
fn default_allowed_when_no_grace_configured() {
    let (env, contract_id, _admin) = deploy();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    env.ledger().set_timestamp(OPEN_TS);
    open(&client, &borrower, 1_000, 100_000);

    // Immediately defaultable at t == OPEN_TS.
    client.default_credit_line(&borrower);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Defaulted
    );
}

// ── base source 1: suspension_ts ──────────────────────────────────────────────

/// Base = `suspension_ts`. Blocked at `suspension_ts + grace - 1`, allowed at
/// the exact boundary `suspension_ts + grace`.
#[test]
fn default_blocked_inside_grace_base_suspension_ts() {
    let (env, contract_id, _admin) = deploy();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    env.ledger().set_timestamp(OPEN_TS);
    open(&client, &borrower, 1_000, 100_000);

    let suspension_ts = OPEN_TS + 500;
    env.ledger().set_timestamp(suspension_ts);
    client.suspend_credit_line(&borrower);
    client.set_borrower_liq_grace(&borrower, &GRACE);

    // The line is Suspended with suspension_ts set: window_end = 1_600.
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.suspension_ts, suspension_ts);
    assert_eq!(line.status, CreditStatus::Suspended);

    env.ledger().set_timestamp(suspension_ts + GRACE - 1);
    assert_default_blocked(&client, &borrower);

    env.ledger().set_timestamp(suspension_ts + GRACE);
    client.default_credit_line(&borrower);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Defaulted
    );
}

// ── base source 2: next_due_ts ────────────────────────────────────────────────

/// Base = `next_due_ts`. `suspension_ts` and `last_rate_update_ts` are zero, so
/// the schedule's first due date anchors the window.
#[test]
fn default_blocked_inside_grace_base_next_due_ts() {
    let (env, contract_id, _admin) = deploy();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    env.ledger().set_timestamp(OPEN_TS);
    open(&client, &borrower, 1_000, 100_000);

    let first_due_ts = OPEN_TS + 1_000;
    client.set_repayment_schedule(&borrower, &1_000_i128, &1_000_u64, &first_due_ts);
    client.set_borrower_liq_grace(&borrower, &GRACE);

    // Confirm the higher-priority sources really are unset, so this test
    // exercises fallback #2 and nothing else.
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.suspension_ts, 0);
    assert_eq!(line.last_rate_update_ts, 0);

    env.ledger().set_timestamp(first_due_ts + GRACE - 1);
    assert_default_blocked(&client, &borrower);

    env.ledger().set_timestamp(first_due_ts + GRACE);
    client.default_credit_line(&borrower);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Defaulted
    );
}

// ── base source 3: last_rate_update_ts ────────────────────────────────────────

/// Base = `last_rate_update_ts`. There is no suspension and no schedule, so a
/// successful rate change under a `RateChangeConfig` is the only anchor.
#[test]
fn default_blocked_inside_grace_base_last_rate_update_ts() {
    let (env, contract_id, _admin) = deploy();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    env.ledger().set_timestamp(OPEN_TS);
    open(&client, &borrower, 1_000, 100_000);

    // A rate change only refreshes last_rate_update_ts while a rate-change
    // config is active.
    client.set_rate_change_limits(&500_u32, &0_u64);

    let rate_update_ts = OPEN_TS + 500;
    env.ledger().set_timestamp(rate_update_ts);
    client.update_risk_parameters(&borrower, &1_000_000_i128, &1_200_u32, &50_u32);

    client.set_borrower_liq_grace(&borrower, &GRACE);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.suspension_ts, 0, "fallback #1 must stay unset");
    assert_eq!(
        line.last_rate_update_ts, rate_update_ts,
        "the rate change must stamp last_rate_update_ts (fallback #3)"
    );

    env.ledger().set_timestamp(rate_update_ts + GRACE - 1);
    assert_default_blocked(&client, &borrower);

    env.ledger().set_timestamp(rate_update_ts + GRACE);
    client.default_credit_line(&borrower);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Defaulted
    );
}

// ── base source 4: last_accrual_ts (zero-accrual case) ────────────────────────

/// Base = `last_accrual_ts`. A `0`-bps line accrues nothing, so the checkpoint
/// seeded at origination survives and the window behaves as a true fixed window:
/// blocked one tick before the boundary, allowed exactly on it.
///
/// A non-zero rate is deliberately *not* used here: `apply_accrual` advances
/// `last_accrual_ts` on any non-zero interest, which changes the base inside the
/// default call itself. That failure mode is pinned separately by the two
/// drift tests below.
#[test]
fn default_blocked_inside_grace_base_last_accrual_ts() {
    let (env, contract_id, _admin) = deploy();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    env.ledger().set_timestamp(OPEN_TS);
    open(&client, &borrower, 0, 100_000);
    client.set_borrower_liq_grace(&borrower, &GRACE);

    // All three higher-priority sources are unset, so the chain bottoms out at
    // last_accrual_ts == OPEN_TS.
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.suspension_ts, 0);
    assert_eq!(line.last_rate_update_ts, 0);
    assert_eq!(line.last_accrual_ts, OPEN_TS);

    env.ledger().set_timestamp(OPEN_TS + GRACE - 1);
    assert_default_blocked(&client, &borrower);

    env.ledger().set_timestamp(OPEN_TS + GRACE);
    client.default_credit_line(&borrower);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Defaulted
    );
}

// ── precedence of the fallback chain ──────────────────────────────────────────

/// The chain is priority-ordered, not recency-ordered: `suspension_ts` wins even
/// when `next_due_ts` and `last_rate_update_ts` are both later (and non-zero).
#[test]
fn suspension_ts_outranks_next_due_and_rate_update_ts() {
    let (env, contract_id, _admin) = deploy();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    env.ledger().set_timestamp(OPEN_TS);
    open(&client, &borrower, 1_000, 100_000);

    let suspension_ts = OPEN_TS + 500;
    env.ledger().set_timestamp(suspension_ts);
    client.suspend_credit_line(&borrower);

    // Both lower-priority sources are set *later* than suspension_ts; if the
    // guard used the most recent timestamp the window would end at 5_100
    // instead of 1_600.
    client.set_repayment_schedule(&borrower, &1_000_i128, &1_000_u64, &(OPEN_TS + 4_000));
    client.set_rate_change_limits(&500_u32, &0_u64);
    env.ledger().set_timestamp(suspension_ts + 100);
    client.update_risk_parameters(&borrower, &1_000_000_i128, &1_200_u32, &50_u32);

    client.set_borrower_liq_grace(&borrower, &GRACE);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.suspension_ts, suspension_ts);
    assert!(line.last_rate_update_ts > suspension_ts);
    assert!(line.last_accrual_ts > 0);

    // Window is anchored to suspension_ts: blocked at 1_599, open at 1_600.
    env.ledger().set_timestamp(suspension_ts + GRACE - 1);
    assert_default_blocked(&client, &borrower);

    env.ledger().set_timestamp(suspension_ts + GRACE);
    client.default_credit_line(&borrower);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Defaulted
    );
}

// ── accrual-driven window drift (documents the follow-up fix) ─────────────────

/// Keeper activity extends the `last_accrual_ts` fallback window.
///
/// `accrue_batch` is permissionless and moves `last_accrual_ts` forward on every
/// non-zero accrual. Because that field *is* the fallback base, a caller can
/// keep a line inside its "grace" period indefinitely by accruing regularly —
/// the window is only as long as the time since the last accrual, not the
/// configured duration measured from a fixed event.
#[test]
fn last_accrual_ts_fallback_window_drifts_with_keeper_accrual() {
    let (env, contract_id, _admin) = deploy();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    env.ledger().set_timestamp(OPEN_TS);
    open(&client, &borrower, 1_000, 1_000_000);
    client.set_borrower_liq_grace(&borrower, &1_000_u64);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.suspension_ts, 0);
    assert_eq!(line.last_rate_update_ts, 0);
    assert_eq!(
        line.last_accrual_ts, OPEN_TS,
        "the fallback base starts at the origination timestamp"
    );

    let batch = single(&env, &borrower);

    // Keeper accrual before the nominal boundary moves the anchor forward.
    env.ledger().set_timestamp(OPEN_TS + 500);
    client.accrue_batch(&batch);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().last_accrual_ts,
        OPEN_TS + 500,
        "accrue_batch advances the field used as the grace base"
    );

    // The window that should have ended at OPEN_TS + 1_000 now ends later, so
    // default is still blocked at the original boundary.
    env.ledger().set_timestamp(OPEN_TS + 1_000);
    assert_default_blocked(&client, &borrower);

    // A second accrual pushes it out again.
    env.ledger().set_timestamp(OPEN_TS + 1_200);
    client.accrue_batch(&batch);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().last_accrual_ts,
        OPEN_TS + 1_200
    );

    env.ledger().set_timestamp(OPEN_TS + 2_000);
    assert_default_blocked(&client, &borrower);
}

/// The `last_accrual_ts` fallback never expires on its own.
///
/// `default_credit_line` calls `apply_accrual` *before* selecting the base, so
/// an attempt on a line with non-zero utilization and a non-zero rate rewrites
/// `last_accrual_ts` to `now` and then tests `now < now + grace`, which is always
/// true. No amount of elapsed time can satisfy the guard, with or without a
/// keeper.
#[test]
fn last_accrual_ts_fallback_cannot_expire_while_interest_accrues() {
    let (env, contract_id, _admin) = deploy();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    env.ledger().set_timestamp(OPEN_TS);
    open(&client, &borrower, 1_000, 1_000_000);
    client.set_borrower_liq_grace(&borrower, &1_000_u64);

    // Each attempt accrues interest, moving the base to the current timestamp,
    // so the window is re-armed inside the same call — even far past the
    // nominal end.
    for offset in [1_000_u64, 5_000, 50_000] {
        env.ledger().set_timestamp(OPEN_TS + offset);
        assert_default_blocked(&client, &borrower);
    }
}
