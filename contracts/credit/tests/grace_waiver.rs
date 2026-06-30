// SPDX-License-Identifier: MIT

//! Integration tests for the grace-period waiver receipt event (#603).
//!
//! # Coverage matrix
//!
//! | Branch | Mode         | Test |
//! |--------|--------------|------|
//! | Entirely in-grace (branch 1) | FullWaiver  | `full_waiver_in_window_emits_event_with_correct_payload` |
//! | Entirely in-grace (branch 1) | ReducedRate | `reduced_rate_in_window_emits_event_waived_amount_is_difference` |
//! | Straddles boundary (branch 3) | FullWaiver  | `full_waiver_straddle_emits_event_for_in_grace_portion` |
//! | Straddles boundary (branch 3) | ReducedRate | `reduced_rate_straddle_emits_event_with_correct_waived_amount` |
//! | Entirely post-grace (branch 2) | —           | `no_event_when_entirely_post_grace` |
//! | No config | —            | `no_event_when_no_grace_config` |
//! | Zero grace seconds | —    | `no_event_when_grace_seconds_is_zero` |
//! | Active line | —           | `no_event_for_active_line` |
//! | Zero utilization | —       | `no_event_when_utilized_amount_zero` |
//! | Correct borrower address | FullWaiver | `event_borrower_field_matches_actual_borrower` |
//! | Correct mode field | ReducedRate | `event_mode_field_matches_configured_mode` |
//! | FullWaiver waived = full-rate interest | FullWaiver | `full_waiver_waived_amount_equals_full_rate_interest` |
//! | ReducedRate waived = full − reduced | ReducedRate | `reduced_rate_waived_amount_equals_difference` |
//! | ReducedRate == full rate → no waiver | ReducedRate | `reduced_rate_equal_to_full_rate_emits_no_event` |
//! | Topic stability | FullWaiver | `event_topic_is_stable` |

use creditra_credit::events::GraceWaiverReceiptEvent;
use creditra_credit::types::GraceWaiverMode;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Events, Ledger};
use soroban_sdk::{symbol_short, token::StellarAssetClient, Address, Env, Symbol, TryFromVal};

// ── helpers ───────────────────────────────────────────────────────────────────

/// Deploy the contract, set up a token, open a credit line, draw `draw_amount`
/// at `t = 1`, then suspend at `suspend_ts`.
///
/// Returns `(env, contract_id, borrower)`.  Callers construct `CreditClient`
/// themselves so the borrow of `env` stays within the test frame.
fn setup_suspended(
    credit_limit: i128,
    draw_amount: i128,
    rate_bps: u32,
    suspend_ts: u64,
) -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    StellarAssetClient::new(&env, &token).mint(&contract_id, &1_000_000_000_000_i128);

    client.open_credit_line(&borrower, &credit_limit, &rate_bps, &50_u32);

    // Draw at t=1 to establish accrual checkpoint.
    env.ledger().set_timestamp(1);
    client.draw_credit(&borrower, &draw_amount);

    // Suspend at the specified timestamp (clamped to ≥ 1).
    let ts = suspend_ts.max(1);
    env.ledger().set_timestamp(ts);
    client.suspend_credit_line(&borrower);

    (env, contract_id, borrower)
}

/// Find the `GraceWaiverReceiptEvent` in the current event log, if any.
fn find_grace_waiver_event(env: &Env) -> Option<GraceWaiverReceiptEvent> {
    for event in env.events().all().iter() {
        let topics = event.1;
        if topics.len() < 2 {
            continue;
        }
        let t1_match = Symbol::try_from_val(env, &topics.get(1).unwrap())
            .map(|s| s == symbol_short!("grace_wv"))
            .unwrap_or(false);
        if t1_match {
            if let Ok(payload) = GraceWaiverReceiptEvent::try_from_val(env, &event.2) {
                return Some(payload);
            }
        }
    }
    None
}

// ── branch 1: entire accrual window falls inside the grace window ─────────────

/// FullWaiver, period entirely inside grace: event emitted with
/// `waived_amount == full-rate interest` and `mode == FullWaiver`.
///
/// Setup:  100_000 principal, 1000 bps (10% p.a.), suspended at t=1.
///         grace = 1 year (31_536_000 s), grace_end = 31_536_001.
///         Accrue at t = 31_536_001 (inside window: now <= grace_end).
///
/// Expected:
///   elapsed           = 31_536_000 s
///   full_rate_interest = 100_000 * 1000 * 31_536_000 / (10_000 * 31_536_000) = 10_000
///   actual_interest   = 0  (FullWaiver)
///   waived_amount     = 10_000
#[test]
fn full_waiver_in_window_emits_event_with_correct_payload() {
    let (env, contract_id, borrower) = setup_suspended(1_000_000, 100_000, 1000, 1);
    let client = CreditClient::new(&env, &contract_id);

    client.set_grace_period_config(&31_536_000_u64, &GraceWaiverMode::FullWaiver, &0_u32);

    let _ = env.events().all(); // clear setup events
    env.ledger().set_timestamp(31_536_001);
    client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

    let evt = find_grace_waiver_event(&env)
        .expect("GraceWaiverReceiptEvent must be emitted for branch 1 FullWaiver");

    assert_eq!(evt.borrower, borrower, "borrower address must match");
    assert_eq!(evt.mode, GraceWaiverMode::FullWaiver, "mode must be FullWaiver");
    assert_eq!(
        evt.waived_amount, 10_000,
        "waived_amount must equal the full-rate interest for the elapsed period"
    );
}

/// ReducedRate, period entirely inside grace: event emitted with
/// `waived_amount == full_rate_interest − reduced_rate_interest`.
///
/// Setup:  100_000 principal, 1000 bps full / 200 bps reduced, suspended at t=1.
///         grace = 1 year. Accrue at t = 31_536_001 (inside window).
///
/// Expected:
///   full_rate_interest    = 10_000
///   reduced_rate_interest = 100_000 * 200 * 31_536_000 / (10_000 * 31_536_000) = 2_000
///   waived_amount         = 8_000
#[test]
fn reduced_rate_in_window_emits_event_waived_amount_is_difference() {
    let (env, contract_id, borrower) = setup_suspended(1_000_000, 100_000, 1000, 1);
    let client = CreditClient::new(&env, &contract_id);

    client.set_grace_period_config(&31_536_000_u64, &GraceWaiverMode::ReducedRate, &200_u32);

    let _ = env.events().all();
    env.ledger().set_timestamp(31_536_001);
    client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

    let evt = find_grace_waiver_event(&env)
        .expect("GraceWaiverReceiptEvent must be emitted for branch 1 ReducedRate");

    assert_eq!(evt.borrower, borrower);
    assert_eq!(evt.mode, GraceWaiverMode::ReducedRate);
    assert_eq!(
        evt.waived_amount, 8_000,
        "waived_amount must be full_rate_interest - reduced_rate_interest"
    );
}

// ── branch 3: accrual window straddles the grace boundary ────────────────────

/// FullWaiver, straddle: event covers only the in-grace portion.
///
/// Setup:  100_000 principal, 1000 bps, suspended at t=1.
///         grace = 1 year (grace_end = 31_536_001).
///         Accrue at t = 47_304_001 (0.5 year after grace end).
///
/// In-grace portion (1 → 31_536_001 = 31_536_000 s):
///   full_rate_interest = 10_000, actual = 0 → waived = 10_000.
#[test]
fn full_waiver_straddle_emits_event_for_in_grace_portion() {
    let (env, contract_id, borrower) = setup_suspended(1_000_000, 100_000, 1000, 1);
    let client = CreditClient::new(&env, &contract_id);

    client.set_grace_period_config(&31_536_000_u64, &GraceWaiverMode::FullWaiver, &0_u32);

    env.ledger().set_timestamp(47_304_001); // 1 + 31_536_000 + 15_768_000
    let _ = env.events().all();
    client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

    let evt = find_grace_waiver_event(&env)
        .expect("GraceWaiverReceiptEvent must be emitted in FullWaiver straddle branch");

    assert_eq!(evt.mode, GraceWaiverMode::FullWaiver);
    assert_eq!(
        evt.waived_amount, 10_000,
        "waived_amount must reflect the in-grace portion only"
    );
}

/// ReducedRate, straddle: waived_amount covers the in-grace portion only.
///
/// In-grace portion (1 → 31_536_001 = 31_536_000 s):
///   full_rate_interest = 10_000, reduced = 2_000 → waived = 8_000.
#[test]
fn reduced_rate_straddle_emits_event_with_correct_waived_amount() {
    let (env, contract_id, borrower) = setup_suspended(1_000_000, 100_000, 1000, 1);
    let client = CreditClient::new(&env, &contract_id);

    client.set_grace_period_config(&31_536_000_u64, &GraceWaiverMode::ReducedRate, &200_u32);

    env.ledger().set_timestamp(47_304_001);
    let _ = env.events().all();
    client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

    let evt = find_grace_waiver_event(&env)
        .expect("GraceWaiverReceiptEvent must be emitted in ReducedRate straddle branch");

    assert_eq!(evt.mode, GraceWaiverMode::ReducedRate);
    assert_eq!(
        evt.waived_amount, 8_000,
        "straddle: waived_amount must reflect in-grace portion (full - reduced)"
    );
}

// ── no-event cases ────────────────────────────────────────────────────────────

/// Branch 2 (entirely post-grace): no waiver occurred, no event expected.
///
/// Force `last_accrual_ts` past grace_end first, then accrue entirely post-grace.
#[test]
fn no_event_when_entirely_post_grace() {
    let (env, contract_id, borrower) = setup_suspended(1_000_000, 100_000, 1000, 1);
    let client = CreditClient::new(&env, &contract_id);

    client.set_grace_period_config(&31_536_000_u64, &GraceWaiverMode::FullWaiver, &0_u32);

    // First accrual straddles grace_end — advances last_accrual_ts past grace_end.
    env.ledger().set_timestamp(31_536_002);
    client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

    // Second accrual is entirely post-grace.
    let _ = env.events().all();
    env.ledger().set_timestamp(63_072_002);
    client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

    assert!(
        find_grace_waiver_event(&env).is_none(),
        "No GraceWaiverReceiptEvent when accrual is entirely post-grace"
    );
}

/// No grace config at all: accrues at full rate, no event.
#[test]
fn no_event_when_no_grace_config() {
    let (env, contract_id, borrower) = setup_suspended(1_000_000, 100_000, 1000, 1);
    let client = CreditClient::new(&env, &contract_id);

    // No call to set_grace_period_config.
    let _ = env.events().all();
    env.ledger().set_timestamp(1 + 31_536_000);
    client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

    assert!(
        find_grace_waiver_event(&env).is_none(),
        "No GraceWaiverReceiptEvent when no grace config is set"
    );
}

/// Grace config with `grace_period_seconds == 0`: treated as disabled, no event.
#[test]
fn no_event_when_grace_seconds_is_zero() {
    let (env, contract_id, borrower) = setup_suspended(1_000_000, 100_000, 1000, 1);
    let client = CreditClient::new(&env, &contract_id);

    client.set_grace_period_config(&0_u64, &GraceWaiverMode::FullWaiver, &0_u32);

    let _ = env.events().all();
    env.ledger().set_timestamp(1 + 31_536_000);
    client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

    assert!(
        find_grace_waiver_event(&env).is_none(),
        "No GraceWaiverReceiptEvent when grace_period_seconds == 0"
    );
}

/// Active lines (not suspended) must never emit a grace waiver event.
#[test]
fn no_event_for_active_line() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    StellarAssetClient::new(&env, &token).mint(&contract_id, &1_000_000_000_i128);

    client.open_credit_line(&borrower, &1_000_000, &1000, &50);

    env.ledger().set_timestamp(1);
    client.draw_credit(&borrower, &100_000);

    // Grace config set, but line is still Active (not suspended).
    client.set_grace_period_config(&31_536_000_u64, &GraceWaiverMode::FullWaiver, &0_u32);

    let _ = env.events().all();
    env.ledger().set_timestamp(1 + 31_536_000);
    client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

    assert!(
        find_grace_waiver_event(&env).is_none(),
        "No GraceWaiverReceiptEvent for an Active (not suspended) line"
    );
}

/// Zero utilized amount: no interest to compute, no event.
#[test]
fn no_event_when_utilized_amount_zero() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    StellarAssetClient::new(&env, &token).mint(&contract_id, &1_000_000_000_i128);

    // Open and suspend with zero draw — utilized_amount stays at 0.
    client.open_credit_line(&borrower, &1_000_000, &1000, &50);
    env.ledger().set_timestamp(1);
    client.suspend_credit_line(&borrower);

    client.set_grace_period_config(&31_536_000_u64, &GraceWaiverMode::FullWaiver, &0_u32);

    let _ = env.events().all();
    env.ledger().set_timestamp(1 + 31_536_000);
    client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

    assert!(
        find_grace_waiver_event(&env).is_none(),
        "No GraceWaiverReceiptEvent when utilized_amount == 0"
    );
}

// ── payload field correctness ─────────────────────────────────────────────────

/// The `borrower` field in the event matches the actual borrower address.
#[test]
fn event_borrower_field_matches_actual_borrower() {
    let (env, contract_id, borrower) = setup_suspended(1_000_000, 100_000, 1000, 1);
    let client = CreditClient::new(&env, &contract_id);

    client.set_grace_period_config(&31_536_000_u64, &GraceWaiverMode::FullWaiver, &0_u32);

    let _ = env.events().all();
    env.ledger().set_timestamp(31_536_001);
    client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

    let evt = find_grace_waiver_event(&env).expect("event must be emitted");
    assert_eq!(
        evt.borrower, borrower,
        "borrower field must match the actual borrower address"
    );
}

/// The `mode` field reflects the configured `GraceWaiverMode`.
#[test]
fn event_mode_field_matches_configured_mode() {
    let (env, contract_id, borrower) = setup_suspended(1_000_000, 100_000, 1000, 1);
    let client = CreditClient::new(&env, &contract_id);

    client.set_grace_period_config(&31_536_000_u64, &GraceWaiverMode::ReducedRate, &100_u32);

    let _ = env.events().all();
    env.ledger().set_timestamp(31_536_001);
    client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

    let evt = find_grace_waiver_event(&env).expect("event must be emitted");
    assert_eq!(
        evt.mode,
        GraceWaiverMode::ReducedRate,
        "mode field must reflect the configured GraceWaiverMode"
    );
}

/// FullWaiver: `waived_amount` equals the full-rate interest for the elapsed period.
///
///   100_000 * 1000 bps * 31_536_000 s / (10_000 * 31_536_000) = 10_000
#[test]
fn full_waiver_waived_amount_equals_full_rate_interest() {
    let (env, contract_id, borrower) = setup_suspended(1_000_000, 100_000, 1000, 1);
    let client = CreditClient::new(&env, &contract_id);

    client.set_grace_period_config(&31_536_000_u64, &GraceWaiverMode::FullWaiver, &0_u32);

    let _ = env.events().all();
    env.ledger().set_timestamp(31_536_001);
    client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

    let evt = find_grace_waiver_event(&env).expect("event must be emitted");
    assert_eq!(
        evt.waived_amount, 10_000,
        "FullWaiver: waived_amount must equal what the full rate would have accrued"
    );
}

/// ReducedRate: `waived_amount` equals `full_rate_interest − reduced_rate_interest`.
///
///   full_rate_interest    = 100_000 * 1000 * 31_536_000 / (10_000 * 31_536_000) = 10_000
///   reduced_rate_interest = 100_000 *  500 * 31_536_000 / (10_000 * 31_536_000) =  5_000
///   waived_amount         = 5_000
#[test]
fn reduced_rate_waived_amount_equals_difference() {
    let (env, contract_id, borrower) = setup_suspended(1_000_000, 100_000, 1000, 1);
    let client = CreditClient::new(&env, &contract_id);

    client.set_grace_period_config(&31_536_000_u64, &GraceWaiverMode::ReducedRate, &500_u32);

    let _ = env.events().all();
    env.ledger().set_timestamp(31_536_001);
    client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

    let evt = find_grace_waiver_event(&env).expect("event must be emitted");
    assert_eq!(
        evt.waived_amount, 5_000,
        "ReducedRate: waived_amount must equal full_rate_interest - reduced_rate_interest"
    );
}

// ── topic stability ───────────────────────────────────────────────────────────

/// The topic pair must be exactly `("credit", "grace_wv")` — stable for indexers.
#[test]
fn event_topic_is_stable() {
    let (env, contract_id, borrower) = setup_suspended(1_000_000, 100_000, 1000, 1);
    let client = CreditClient::new(&env, &contract_id);

    client.set_grace_period_config(&31_536_000_u64, &GraceWaiverMode::FullWaiver, &0_u32);

    let _ = env.events().all();
    env.ledger().set_timestamp(31_536_001);
    client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

    let all = env.events().all();
    let grace_events: Vec<_> = all
        .iter()
        .filter(|ev| {
            let topics = ev.1.clone();
            if topics.len() < 2 {
                return false;
            }
            let t0 = Symbol::try_from_val(&env, &topics.get(0).unwrap())
                .map(|s| s == symbol_short!("credit"))
                .unwrap_or(false);
            let t1 = Symbol::try_from_val(&env, &topics.get(1).unwrap())
                .map(|s| s == symbol_short!("grace_wv"))
                .unwrap_or(false);
            t0 && t1
        })
        .collect();

    assert_eq!(
        grace_events.len(), 1,
        "exactly one grace waiver event per accrual window; topics must be (credit, grace_wv)"
    );
}

// ── ReducedRate == full rate → waived_amount == 0 → no event ─────────────────

/// When `reduced_rate_bps` equals `interest_rate_bps`, no interest is waived
/// so no event should be emitted.
#[test]
fn reduced_rate_equal_to_full_rate_emits_no_event() {
    let (env, contract_id, borrower) = setup_suspended(1_000_000, 100_000, 1000, 1);
    let client = CreditClient::new(&env, &contract_id);

    // reduced_rate_bps == interest_rate_bps (1000) → waived_amount == 0
    client.set_grace_period_config(&31_536_000_u64, &GraceWaiverMode::ReducedRate, &1000_u32);

    let _ = env.events().all();
    env.ledger().set_timestamp(31_536_001);
    client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

    assert!(
        find_grace_waiver_event(&env).is_none(),
        "No event when ReducedRate equals full rate (waived_amount == 0)"
    );
}
