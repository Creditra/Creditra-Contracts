// SPDX-License-Identifier: MIT

//! Integration tests for `GraceWaiverReceiptEvent` — topic encoding and
//! full payload validation.
//!
//! Complements `grace_waiver.rs` (accrual-math focus) by asserting that the
//! event is correctly encoded on-chain and decodable by off-chain indexers.

use creditra_credit::events::GraceWaiverReceiptEvent;
use creditra_credit::types::GraceWaiverMode;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Events, Ledger};
use soroban_sdk::{symbol_short, token::StellarAssetClient, Address, Env, Symbol, TryFromVal};

// ── helpers ───────────────────────────────────────────────────────────────────

/// Returns `(env, contract_id, borrower)` with the borrower suspended at t=1
/// and a 1-year FullWaiver grace config already set.
fn setup_full_waiver(grace_mode: GraceWaiverMode, reduced_bps: u32) -> (Env, Address, Address) {
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

    client.open_credit_line(&borrower, &1_000_000_i128, &1000_u32, &50_u32);
    env.ledger().set_timestamp(1);
    client.draw_credit(&borrower, &100_000_i128);
    client.suspend_credit_line(&borrower);

    client.set_grace_period_config(&31_536_000_u64, &grace_mode, &reduced_bps);

    (env, contract_id, borrower)
}

/// Scan the event log for a `GraceWaiverReceiptEvent`.
fn find_grace_waiver(env: &Env) -> Option<GraceWaiverReceiptEvent> {
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

// ── FullWaiver payload ────────────────────────────────────────────────────────

/// FullWaiver inside grace window: event emitted with fully correct payload.
///
/// Principal 100_000, rate 1000 bps, elapsed 31_536_000 s (1 Julian year):
///   full_rate_interest = 10_000, actual = 0 → waived = 10_000.
#[test]
fn full_waiver_event_payload_is_correct() {
    let (env, contract_id, borrower) =
        setup_full_waiver(GraceWaiverMode::FullWaiver, 0);
    let client = CreditClient::new(&env, &contract_id);

    let _ = env.events().all(); // clear setup events
    env.ledger().set_timestamp(31_536_001);
    client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

    let evt = find_grace_waiver(&env)
        .expect("GraceWaiverReceiptEvent must be emitted for FullWaiver inside grace window");

    assert_eq!(evt.borrower, borrower, "borrower field must match");
    assert_eq!(evt.mode, GraceWaiverMode::FullWaiver, "mode must be FullWaiver");
    assert_eq!(
        evt.waived_amount, 10_000,
        "waived_amount must equal full-rate interest (10_000)"
    );
}

// ── ReducedRate payload ───────────────────────────────────────────────────────

/// ReducedRate inside grace window: event emitted with the interest difference.
///
/// Principal 100_000, full rate 1000 bps, reduced rate 200 bps, elapsed 1 year:
///   full_rate_interest    = 10_000
///   reduced_rate_interest =  2_000
///   waived_amount         =  8_000
#[test]
fn reduced_rate_event_payload_is_correct() {
    let (env, contract_id, borrower) =
        setup_full_waiver(GraceWaiverMode::ReducedRate, 200);
    let client = CreditClient::new(&env, &contract_id);

    let _ = env.events().all();
    env.ledger().set_timestamp(31_536_001);
    client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

    let evt = find_grace_waiver(&env)
        .expect("GraceWaiverReceiptEvent must be emitted for ReducedRate inside grace window");

    assert_eq!(evt.borrower, borrower, "borrower field must match");
    assert_eq!(evt.mode, GraceWaiverMode::ReducedRate, "mode must be ReducedRate");
    assert_eq!(
        evt.waived_amount, 8_000,
        "waived_amount must equal full_rate_interest - reduced_rate_interest (8_000)"
    );
}

// ── topic encoding ────────────────────────────────────────────────────────────

/// The first topic must be `"credit"` and the second must be `"grace_wv"`.
/// This guards against accidental topic renames breaking downstream indexers.
#[test]
fn event_topics_are_credit_grace_wv() {
    let (env, contract_id, borrower) =
        setup_full_waiver(GraceWaiverMode::FullWaiver, 0);
    let client = CreditClient::new(&env, &contract_id);

    let _ = env.events().all();
    env.ledger().set_timestamp(31_536_001);
    client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

    let all = env.events().all();
    let found = all.iter().any(|ev| {
        let topics = ev.1;
        topics.len() >= 2
            && Symbol::try_from_val(&env, &topics.get(0).unwrap())
                .map(|s| s == symbol_short!("credit"))
                .unwrap_or(false)
            && Symbol::try_from_val(&env, &topics.get(1).unwrap())
                .map(|s| s == symbol_short!("grace_wv"))
                .unwrap_or(false)
    });

    assert!(found, r#"event topics must be ("credit", "grace_wv")"#);
}

// ── no event for non-suspended lines ─────────────────────────────────────────

/// Active line with grace config set: no event must be emitted.
#[test]
fn no_event_for_non_suspended_line() {
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

    // Grace config set — line is Active, not suspended.
    client.set_grace_period_config(&31_536_000_u64, &GraceWaiverMode::FullWaiver, &0_u32);

    let _ = env.events().all();
    env.ledger().set_timestamp(1 + 31_536_000);
    client.update_risk_parameters(&borrower, &1_000_000, &1000, &50);

    assert!(
        find_grace_waiver(&env).is_none(),
        "GraceWaiverReceiptEvent must NOT be emitted for an Active line"
    );
}
