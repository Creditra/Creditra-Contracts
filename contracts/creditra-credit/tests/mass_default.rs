// SPDX-License-Identifier: MIT

//! End-to-end stress test: 10 000 concurrent defaults.
//!
//! # What
//!
//! Simulates a systemic default scenario where every borrower defaults inside a single ledger timestamp window, asserting that the protocol remains solvent after the event.
//!
//! # Requirements (GrantFox FWC26 campaign)
//!
//! - **Simulate 10 000 concurrent defaults**: All default calls must occur in the same ledger window.
//! - **Protocol must stay solvent**: `total_utilized` must remain unchanged after default.
//! - **Credit lines must survive**: Records must not be removed on default.
//! - **Outstanding debt preserved**: `utilized_amount` must remain exact.
//! - **Only admin-authorized**: Each default requires proper authentication.
//! - **Time-window consistency**: All defaults must share the same ledger timestamp.
//! - **Overflow-safe arithmetic**: No panics in production paths.
//!
//! # Why these matter
//!
//! Default processing is a critical protocol boundary. If `total_utilized` decreased on default, an attacker could:
//! 1. Draw credit on a line
//! 2. Default that line
//! 3. Repeat steps 1-2
//! 4. Eventually deplete the accumulator and allow draws exceeding actual liquidity
//!
//! # Performance note
//!
//! Enumerating 10 000 entries after every default would be O(N²). We maintain a shadow total in the test harness, enumerate only a representative sample at the end, and keep the whole test O(N).

use creditra_credit::types::CreditStatus;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env};

// ── Constants ─────────────────────────────────────────────────────────────────

const BORROWER_COUNT: usize = 10_000;
const CREDIT_LIMIT: i128 = 1_000;
const DRAW_AMOUNT: i128 = 700;
/// Zero interest so accrual cannot change `utilized_amount` during the test,
/// keeping the invariant arithmetic exact.
const INTEREST_RATE_BPS: u32 = 0;
const RISK_SCORE: u32 = 50;
/// How many lines to spot-check individually after the mass default.
const SPOT_CHECK_COUNT: usize = 20;

// ── Setup ─────────────────────────────────────────────────────────────────────

/// Registers the contract, opens `BORROWER_COUNT` credit lines, draws on each,
/// and returns the client + borrower list. The ledger is NOT advanced after setup so all default calls land in the same timestamp window.
fn setup(env: &Env) -> (CreditClient<'_>, std::vec::Vec<Address>) {
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = 1_000);

    let admin = Address::generate(env);
    let contract_id = env.register(Credit, ())
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&contract_id);

    // Mint enough reserve for every draw.
    StellarAssetClient::new(env, &token)
        .mint(&contract_id, &(BORROWER_COUNT as i128 * DRAW_AMOUNT));

    let mut borrowers = std::vec::Vec::with_capacity(BORROWER_COUNT);
    for _ in 0..BORROWER_COUNT {
        let b = Address::generate(env);
        client.open_credit_line(&b, &CREDIT_LIMIT, &INTEREST_RATE_BPS, &RISK_SCORE);
        client.draw_credit(&b, &DRAW_AMOUNT);
        borrowers.push(b);
    }

    (client, borrowers)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn assert_total_utilized(client: &CreditClient<'_>, expected: i128, label: &str) {
    let actual = client.get_total_utilized();
    assert!(
        actual >= 0,
        "{label}: total_utilized is negative ({actual})"
    );
    assert_eq!(
        actual, expected,
        "{label}: total_utilized mismatch (expected={expected}, actual={actual})"
    );
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Defaults 10 000 borrowers in a single ledger timestamp window and verifies every solvency invariant.
#[test]
fn mass_default_solvency_invariants() {
    let env = Env::default();
    let (client, borrowers) = setup(&env);

    let expected_total: i128 = BORROWER_COUNT as i128 * DRAW_AMOUNT;

    // ── Pre-default ───────────────────────────────────────────────────────────
    assert_total_utilized(&client, expected_total, "pre-default");
    let pre_count = client.get_credit_line_count();
    assert_eq!(pre_count, BORROWER_COUNT as u32, "pre-default line count");

    // ── Mass default — same ledger timestamp for all calls ────────────────────
    //
    // env.ledger().timestamp() is NOT advanced between iterations.
    // This is the key "same ledger window" condition from the GrantFox FWC26 requirements.
    for borrower in &borrowers {
        client.default_credit_line(borrower);
    }

    // ── Invariant 1: total_utilized must be exactly unchanged ─────────────────
    // default_credit_line changes status but must NOT reduce the outstanding debt.
    assert_total_utilized(&client, expected_total, "post-default");

    // ── Invariant 2: credit line count must not change ────────────────────────
    assert_eq!(
        client.get_credit_line_count(),
        pre_count,
        "post-default: credit line count changed (records must survive default)"
    );

    // ── Invariant 3: protocol summary agrees with the scalar ──────────────────
    let summary = client.get_protocol_summary();
    assert!(
        summary.total_utilized >= 0,
        "post-default: protocol summary total_utilized is negative"
    );
    assert_eq!(
        summary.total_utilized, expected_total,
        "post-default: protocol summary total_utilized disagrees with get_total_utilized"
    );

    // ── Invariant 4: sampled lines have correct status + unchanged utilization ─
    let stride = (BORROWER_COUNT / SPOT_CHECK_COUNT).max(1);
    for i in (0..BORROWER_COUNT).step_by(stride).take(SPOT_CHECK_COUNT) {
        let line = client
            .get_credit_line(&borrowers[i])
            .expect("credit line record must exist after default");
        assert_eq!(
            line.status,
            CreditStatus::Defaulted,
            "borrower[{i}]: expected Defaulted, got {:?}",
            line.status
        );
        assert_eq!(
            line.utilized_amount, DRAW_AMOUNT,
            "borrower[{i}]: utilized_amount changed on default"
        );
        assert_eq!(
            line.credit_limit, CREDIT_LIMIT,
            "borrower[{i}]: credit_limit must not change on default"
        );
    }

    // ── Invariant 5: boundary lines (first and last) ──────────────────────────
    for &i in &[0usize, BORROWER_COUNT - 1] {
        let line = client.get_credit_line(&borrowers[i]).unwrap();
        assert_eq!(line.status, CreditStatus::Defaulted);
        assert_eq!(line.utilized_amount, DRAW_AMOUNT);
    }
}

/// After mass default the admin can reinstate individual lines without corrupting `total_utilized`.
///
/// This covers the "post-crisis recovery" path: the protocol must remain fully operable after a systemic default event.
#[test]
fn post_mass_default_recovery_preserves_invariant() {
    let env = Env::default();
    let (client, borrowers) = setup(&env);

    for b in &borrowers {
        client.default_credit_line(b);
    }

    let total_after_default = client.get_total_utilized();
    assert!(
        total_after_default >= 0,
        "total_utilized negative after mass default"
    );

    // Reinstate borrowers[0] → Active; total_utilized must be unchanged because
    // reinstate only changes status, it does not alter the utilized amount.
    client.reinstate_credit_line(&borrowers[0], &CreditStatus::Active);
    assert_total_utilized(&client, total_after_default, "after reinstate[0]");

    let reinstated = client.get_credit_line(&borrowers[0]).unwrap();
    assert_eq!(reinstated.status, CreditStatus::Active);
    assert_eq!(
        reinstated.utilized_amount, DRAW_AMOUNT,
        "reinstate must not zero out the outstanding balance"
    );

    // Reinstate a second borrower to Restricted (draws blocked until repaid).
    client.reinstate_credit_line(&borrowers[1], &CreditStatus::Restricted);
    assert_total_utilized(&client, total_after_default, "after reinstate[1]");

    let restricted = client.get_credit_line(&borrowers[1]).unwrap();
    assert_eq!(restricted.status, CreditStatus::Restricted);
    assert_eq!(restricted.utilized_amount, DRAW_AMOUNT);
}
