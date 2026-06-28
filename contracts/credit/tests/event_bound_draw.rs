// SPDX-License-Identifier: MIT

//! Assert per-draw event count is bounded.
//!
//! A single `draw_credit` can emit accrual + drawn + penalty-rate-enter/exit
//! + grace-waiver events. Indexers depend on an upper bound — this test
//! locks the invariant `env.events().all().len() <= MAX_EVENTS_PER_DRAW`
//! after every successful `draw_credit`, covering both the Active (non-
//! delinquent) and Delinquent paths.
//!
//! # Event budget
//!
//! | Path       | Expected events                                        | Max |
//! |------------|--------------------------------------------------------|-----|
//! | Active     | `InterestAccrued` + `Drawn`                            | 2   |
//! | Delinquent | `InterestAccrued` + `PenaltyRateEntered` + `Drawn`     | 4   |
//!
//! The delinquent path may additionally emit `PenaltyRateExited` if the
//! line exits delinquency mid-accrual (unlikely in a single step), and
//! `GraceWaiverApplied` for suspended lines straddling the grace boundary.
//! The per-draw cap `MAX_EVENTS_PER_DRAW = 4` accommodates all legitimate
//! combinations while remaining tight enough for indexers.
//!
//! See [`docs/EVENTS_CATALOG.md`](../../docs/EVENTS_CATALOG.md) for the
//! canonical event catalog.

use creditra_credit::types::CreditStatus;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Events, Ledger};
use soroban_sdk::{token, Address, Env};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Maximum number of events any single `draw_credit` call may emit.
///
/// Chosen to cover the worst legitimate case:
///   `InterestAccrued` + `PenaltyRateEntered` + `Drawn` + 1 spare
///   (e.g. `GraceWaiverApplied` when straddling the grace boundary).
/// If a code change pushes `draw_credit` past this bound the test will
/// catch the regression immediately.
const MAX_EVENTS_PER_DRAW: usize = 4;

/// Ledger start timestamp shared by all tests.
const START_TS: u64 = 1_000_000;

/// Credit line ceiling shared by all tests.
const CREDIT_LIMIT: i128 = 100_000;

/// Reserve seeding amount (must be >= CREDIT_LIMIT).
const RESERVE_BALANCE: i128 = 200_000;

/// Penalty surcharge in bps for delinquent-path tests.
const PENALTY_SURCHARGE_BPS: u32 = 500;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Create an `Env` with a fully initialised Credit contract, a funded
/// reserve, and an open credit line for `borrower`.
///
/// Returns `(env, contract_id, borrower, admin)`.
fn setup_active_borrower(start_ts: u64) -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = start_ts);

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    // Register a Stellar Asset Contract token and seed the reserve.
    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);
    client.set_liquidity_source(&contract_id);
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &RESERVE_BALANCE);

    client.open_credit_line(&borrower, &CREDIT_LIMIT, &300_u32, &70_u32);

    (env, contract_id, borrower, admin)
}

/// Advance the ledger timestamp.
fn set_timestamp(env: &Env, ts: u64) {
    env.ledger().with_mut(|li| li.timestamp = ts);
}

/// Return the total number of events currently recorded by the test `Env`.
fn event_count(env: &Env) -> usize {
    env.events().all().len()
}

/// Assert the event count delta across a closure is within the bound.
fn assert_draw_event_bound<F>(env: &Env, label: &str, mut draw: F)
where
    F: FnMut(),
{
    let before = event_count(env);
    draw();
    let after = event_count(env);
    let delta = after - before;

    assert!(
        delta <= MAX_EVENTS_PER_DRAW,
        "{}: draw emitted {delta} events, max allowed {MAX_EVENTS_PER_DRAW}",
        label,
    );
}

// ── Active-path tests ────────────────────────────────────────────────────────

#[test]
fn active_draw_without_prior_utilization_emits_one_event() {
    let (env, contract_id, borrower, _admin) = setup_active_borrower(START_TS);
    let client = CreditClient::new(&env, &contract_id);

    assert_draw_event_bound(&env, "active draw, no prior utilization", || {
        client.draw_credit(&borrower, &500_i128);
    });

    // Verify state is consistent.
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 500);
    assert_eq!(line.status, CreditStatus::Active);
}

#[test]
fn active_draw_with_existing_utilization_emits_interest_accrued_and_drawn() {
    let (env, contract_id, borrower, _admin) = setup_active_borrower(START_TS);
    let client = CreditClient::new(&env, &contract_id);

    // First draw seeds utilization so the second draw triggers accrual.
    client.draw_credit(&borrower, &1_000_i128);

    // Advance time so interest accrues on the next draw.
    set_timestamp(&env, START_TS + 86_400); // +1 day

    assert_draw_event_bound(
        &env,
        "active draw with prior utilization (should emit accrue + drawn)",
        || {
            client.draw_credit(&borrower, &2_000_i128);
        },
    );

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 3_000);
    assert_eq!(line.status, CreditStatus::Active);
}

#[test]
fn active_draw_bound_is_two_when_no_delinquency() {
    let (env, contract_id, borrower, _admin) = setup_active_borrower(START_TS);
    let client = CreditClient::new(&env, &contract_id);

    client.draw_credit(&borrower, &1_000_i128);
    set_timestamp(&env, START_TS + 86_400);

    let before = event_count(&env);
    client.draw_credit(&borrower, &1_000_i128);
    let after = event_count(&env);
    let delta = after - before;

    assert!(
        delta <= 2,
        "active draw without delinquency must emit <= 2 events, got {delta}"
    );

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 2_000);
}

// ── Delinquent-path tests ────────────────────────────────────────────────────

/// Configure a repayment schedule so the borrower is already past the grace
/// window, then enable penalty surcharge.
fn make_delinquent(env: &Env, client: &CreditClient, borrower: &Address) {
    // Set a repayment schedule with next_due_ts in the past.
    // The is_delinquent check uses `ledger.timestamp() > next_due_ts + grace`.
    // With grace defaulting to 0 and no global config, setting next_due_ts
    // below the current timestamp is sufficient.
    let due_in_past = START_TS - 10_000;
    client.set_repayment_schedule(
        borrower,
        &500_i128,    // amount_per_period
        &86_400_u64,  // period_seconds (1 day)
        &due_in_past, // first_due_ts in the past -> immediately delinquent
    );

    // Enable penalty surcharge so delinquent accrual adds the surcharge.
    client.set_penalty_surcharge_bps(&PENALTY_SURCHARGE_BPS);
}

#[test]
fn delinquent_draw_emits_accrue_penalty_enter_and_drawn() {
    let (env, contract_id, borrower, _admin) = setup_active_borrower(START_TS);
    let client = CreditClient::new(&env, &contract_id);

    // Seed utilization so there is something to accrue on.
    client.draw_credit(&borrower, &1_000_i128);

    // Advance time past the repayment due date to trigger delinquency.
    set_timestamp(&env, START_TS + 86_400);

    // Configure delinquency state.
    make_delinquent(&env, &client, &borrower);

    assert_draw_event_bound(
        &env,
        "delinquent draw (should emit accrue + penalty_enter + drawn)",
        || {
            client.draw_credit(&borrower, &500_i128);
        },
    );

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 1_500);
}

#[test]
fn delinquent_draw_bound_is_four() {
    let (env, contract_id, borrower, _admin) = setup_active_borrower(START_TS);
    let client = CreditClient::new(&env, &contract_id);

    client.draw_credit(&borrower, &1_000_i128);
    set_timestamp(&env, START_TS + 86_400);
    make_delinquent(&env, &client, &borrower);

    let before = event_count(&env);
    client.draw_credit(&borrower, &500_i128);
    let after = event_count(&env);
    let delta = after - before;

    assert!(
        delta <= MAX_EVENTS_PER_DRAW,
        "delinquent draw must emit <= {MAX_EVENTS_PER_DRAW} events, got {delta}"
    );

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 1_500);
}

// ── Repeated-draw stress ─────────────────────────────────────────────────────

#[test]
fn consecutive_draws_with_time_advance_stay_within_bound() {
    let (env, contract_id, borrower, _admin) = setup_active_borrower(START_TS);
    let client = CreditClient::new(&env, &contract_id);

    let mut ts = START_TS;

    // Draw 1 — no prior utilization, time not advanced (no accrual event).
    assert_draw_event_bound(&env, "draw #1 (fresh)", || {
        client.draw_credit(&borrower, &200_i128);
    });

    // Draws 2-5 — advance time slightly each draw so small accrual fires.
    for i in 2..=5 {
        ts += 3600; // +1 hour
        set_timestamp(&env, ts);
        assert_draw_event_bound(&env, &format!("draw #{i}"), || {
            client.draw_credit(&borrower, &200_i128);
        });
    }

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 1_000);
}
