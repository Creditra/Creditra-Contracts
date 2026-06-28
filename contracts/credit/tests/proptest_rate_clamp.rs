// SPDX-License-Identifier: MIT

//! Property tests verifying per-borrower rate clamping is order-independent.
//!
//! # What
//!
//! The contract applies up to three layers of clamping when computing a
//! borrower's effective interest rate:
//!
//! 1. Formula-internal clamp to `[min_rate_bps, min(max_rate_bps, 10_000)]`.
//! 2. Per-borrower rate floor via [`set_borrower_rate_floor`].
//! 3. Per-borrower rate ceiling via [`set_borrower_rate_ceiling`].
//!
//! These tests verify that the order of application does not affect the final
//! result — i.e., the clamping operations *commute* and *compose* correctly.
//!
//! # Property
//!
//! For any interest rate `r`, per-borrower floor `f`, and per-borrower
//! ceiling `c` such that `0 ≤ f ≤ c ≤ 10_000`:
//!
//! ```text
//! clamp(clamp(r, f), c) == clamp(clamp(r, c), f)
//! ```
//!
//! Equivalently, by the modularity law for bounded distributive lattices:
//!
//! ```text
//! max(min(r, c), f) == min(max(r, f), c)
//! ```
//!
//! # Why
//!
//! The contract applies floor before ceiling in
//! [`crate::risk::update_risk_parameters`]:
//!
//! 1. `effective_rate.max(floor)` — floor is applied first.
//! 2. `result.min(ceiling)` — ceiling is applied second.
//!
//! If these operations did *not* commute, an attacker or admin who could
//! influence the order of bound application could manipulate the final rate.
//! The modularity identity guarantees safety: it does not matter whether the
//! floor or the ceiling is applied first — the final clamped value is always
//! the same.
//!
//! # Tests
//!
//! | Test | Scope | What it verifies |
//! |------|-------|------------------|
//! | `prop_clamp_modularity` | Pure function (1024 cases) | `max(min(r,c), f) == min(max(r,f), c)` for all valid `f ≤ c` |
//! | `prop_clamp_contract_integration` | Full contract (256 cases) | `update_risk_parameters` with floor+ceiling set gives `r.max(f).min(c)` |
//! | `prop_clamp_floor_ceiling_set_order` | Full contract (128 cases) | Setting floor then ceiling vs ceiling then floor yields same final rate |
//! | `clamp_zero_bounds` | Deterministic edge case | Floor = ceiling = 0 |
//! | `clamp_global_cap_boundary` | Deterministic edge case | Bounds at 10_000 bps |
//! | `clamp_floor_exceeds_ceiling` | Deterministic edge case | Floor > ceiling degenerate case |
//! | `clamp_no_bounds` | Deterministic edge case | No floor or ceiling configured |
//!
//! # References
//!
//! - [`crate::risk::update_risk_parameters`]
//! - [`crate::risk::set_borrower_rate_floor`]
//! - [`crate::risk::set_borrower_rate_ceiling`]
//! - [`crate::risk::compute_rate_from_score`]
//! - Issue #585

use creditra_credit::{Credit, CreditClient};
use proptest::prelude::*;
use proptest::test_runner::Config as ProptestConfig;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

/// Protocol-wide interest rate ceiling (100 % = 10_000 bps).
const MAX_INTEREST_RATE_BPS: u32 = 10_000;

// ── Strategies ──────────────────────────────────────────────────────────────

/// Strategy for `(floor, ceiling)` pairs with `0 ≤ floor ≤ ceiling ≤ 10_000`.
fn floor_ceiling() -> impl Strategy<Value = (u32, u32)> {
    (0_u32..=MAX_INTEREST_RATE_BPS)
        .prop_flat_map(|floor| (Just(floor), floor..=MAX_INTEREST_RATE_BPS))
}

/// Strategy for an interest rate in a valid range.
fn rate() -> impl Strategy<Value = u32> {
    0_u32..=MAX_INTEREST_RATE_BPS
}

// ── Helper ──────────────────────────────────────────────────────────────────

/// Deploy a fresh contract and open a credit line for a random borrower.
///
/// Rate-change limits are intentionally left unset so that the clamp tests
/// are not perturbed by delta/interval guardrails.
fn setup(env: &Env) -> (CreditClient<'_>, Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    // Open a credit line with a mid-range rate and score.
    client.open_credit_line(&borrower, &10_000_i128, &500_u32, &50_u32);
    (client, admin, borrower)
}

// ── Property test 1: pure-function modularity law ───────────────────────────

proptest! {
    #![proptest_config(ProptestConfig { cases: 1024, .. ProptestConfig::default() })]
    /// Verifies the modularity identity for 1024 random `(rate, floor, ceiling)`
    /// triples with `0 ≤ floor ≤ ceiling ≤ 10_000`:
    ///
    /// ```text
    /// max(min(r, c), f) == min(max(r, f), c)
    /// ```
    #[test]
    fn prop_clamp_modularity(
        r in rate(),
        (f, c) in floor_ceiling(),
    ) {
        let floor_then_ceiling = r.max(f).min(c);
        let ceiling_then_floor = r.min(c).max(f);

        prop_assert_eq!(
            floor_then_ceiling, ceiling_then_floor,
            "clamp modularity violated:\n\
             rate = {}, floor = {}, ceiling = {}\n\
             floor-then-ceiling = {}\n\
             ceiling-then-floor = {}",
            r, f, c,
            floor_then_ceiling, ceiling_then_floor,
        );
    }
}

// ── Property test 2: contract integration ───────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, .. ProptestConfig::default() })]
    /// Verifies that the contract's `update_risk_parameters` entrypoint, with a
    /// per-borrower floor and ceiling configured, computes the same result as the
    /// mathematical clamp.
    #[test]
    fn prop_clamp_contract_integration(
        r in rate(),
        (f, c) in floor_ceiling(),
    ) {
        let env = Env::default();
        let (client, _admin, borrower) = setup(&env);

        // Set per-borrower bounds.
        client.set_borrower_rate_floor(&borrower, &Some(f));
        client.set_borrower_rate_ceiling(&borrower, &Some(c));

        // Update risk parameters with the chosen rate (no rate-change limits active).
        client.update_risk_parameters(&borrower, &10_000_i128, &r, &50_u32);

        let line = client.get_credit_line(&borrower).unwrap();
        let expected = r.max(f).min(c);
        prop_assert_eq!(
            line.interest_rate_bps, expected,
            "contract rate does not match mathematical clamp:\n\
             rate = {}, floor = {}, ceiling = {}\n\
             expected = {}, got = {}",
            r, f, c, expected, line.interest_rate_bps,
        );
    }
}

// ── Property test 3: floor/ceiling set-order independence ───────────────────

proptest! {
    #![proptest_config(ProptestConfig { cases: 128, .. ProptestConfig::default() })]
    /// Verifies that the order in which per-borrower floor and ceiling are
    /// written to storage does not affect the final clamped rate.
    ///
    /// In contract A the floor is set first, then the ceiling; in contract B
    /// the ceiling is set first, then the floor. Both contracts then execute
    /// the same `update_risk_parameters` call and must produce the same rate.
    #[test]
    fn prop_clamp_floor_ceiling_set_order(
        r in rate(),
        (f, c) in floor_ceiling(),
    ) {
        // Contract A: floor first, then ceiling.
        let env_a = Env::default();
        let (client_a, _admin_a, borrower_a) = setup(&env_a);

        client_a.set_borrower_rate_floor(&borrower_a, &Some(f));
        client_a.set_borrower_rate_ceiling(&borrower_a, &Some(c));
        client_a.update_risk_parameters(&borrower_a, &10_000_i128, &r, &50_u32);

        let rate_a = client_a
            .get_credit_line(&borrower_a)
            .unwrap()
            .interest_rate_bps;

        // Contract B: ceiling first, then floor.
        let env_b = Env::default();
        let (client_b, _admin_b, borrower_b) = setup(&env_b);

        client_b.set_borrower_rate_ceiling(&borrower_b, &Some(c));
        client_b.set_borrower_rate_floor(&borrower_b, &Some(f));
        client_b.update_risk_parameters(&borrower_b, &10_000_i128, &r, &50_u32);

        let rate_b = client_b
            .get_credit_line(&borrower_b)
            .unwrap()
            .interest_rate_bps;

        prop_assert_eq!(
            rate_a, rate_b,
            "floor/ceiling set-order independence violated:\n\
             rate = {}, floor = {}, ceiling = {}\n\
             floor-first rate = {}, ceiling-first rate = {}",
            r, f, c, rate_a, rate_b,
        );
    }
}

// ── Edge-case tests ─────────────────────────────────────────────────────────

/// Verifies that the identity holds when both bounds are zero.
#[test]
fn clamp_zero_bounds() {
    let env = Env::default();
    let (client, _admin, borrower) = setup(&env);

    // Floor = ceiling = 0.
    client.set_borrower_rate_floor(&borrower, &Some(0_u32));
    client.set_borrower_rate_ceiling(&borrower, &Some(0_u32));

    // Any rate should be clamped to 0.
    for r in [0_u32, 1, 500, 10_000] {
        client.update_risk_parameters(&borrower, &10_000_i128, &r, &50_u32);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(
            line.interest_rate_bps, 0,
            "rate clamped to 0: rate={}",
            r
        );
    }
}

/// Verifies that the global cap boundary (10_000 bps) is respected.
#[test]
fn clamp_global_cap_boundary() {
    let env = Env::default();
    let (client, _admin, borrower) = setup(&env);

    // Set floor to 9_000 and ceiling to 10_000.
    client.set_borrower_rate_floor(&borrower, &Some(9_000_u32));
    client.set_borrower_rate_ceiling(&borrower, &Some(10_000_u32));

    // Rate above ceiling should be capped.
    client.update_risk_parameters(&borrower, &10_000_i128, &10_500_u32, &50_u32);
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.interest_rate_bps, 10_000, "capped to 10_000");

    // Rate below floor should be raised.
    client.update_risk_parameters(&borrower, &10_000_i128, &8_000_u32, &50_u32);
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.interest_rate_bps, 9_000, "raised to floor");
}

/// Verifies that when floor exceeds ceiling, the ceiling wins (degenerate case).
#[test]
fn clamp_floor_exceeds_ceiling() {
    // This is a degenerate case: floor > ceiling.
    // The contract allows setting floor above ceiling (no cross-check in
    // set_borrower_rate_floor). In this case, `max(r, f).min(c)` with f > c
    // always yields `c` regardless of r, because `max(r, f) >= f > c`, so
    // `min(anything >= f, c) = c`.

    // Set ceiling first (succeeds) then floor above it (also succeeds since
    // set_borrower_rate_floor does not cross-check against ceiling).
    let env = Env::default();
    let (client, _admin, borrower) = setup(&env);

    client.set_borrower_rate_ceiling(&borrower, &Some(3_000_u32));
    client.set_borrower_rate_floor(&borrower, &Some(5_000_u32));

    // Any rate should result in the ceiling value (3_000).
    for r in [0_u32, 1_000, 5_000, 10_000] {
        client.update_risk_parameters(&borrower, &10_000_i128, &r, &50_u32);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(
            line.interest_rate_bps, 3_000,
            "ceiling wins when floor > ceiling: rate={}",
            r
        );
    }
}

/// Verifies that with no floor or ceiling configured, the rate passes through
/// unchanged (within global cap).
#[test]
fn clamp_no_bounds() {
    let env = Env::default();
    let (client, _admin, borrower) = setup(&env);

    for r in [0_u32, 1, 500, 10_000] {
        client.update_risk_parameters(&borrower, &10_000_i128, &r, &50_u32);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(
            line.interest_rate_bps, r,
            "rate unchanged without bounds: rate={}",
            r
        );
    }
}

/// Verifies that floor-only (no ceiling) correctly raises rates below the floor.
#[test]
fn clamp_floor_only() {
    let env = Env::default();
    let (client, _admin, borrower) = setup(&env);

    client.set_borrower_rate_floor(&borrower, &Some(4_000_u32));

    // Rate below floor should be raised.
    client.update_risk_parameters(&borrower, &10_000_i128, &2_000_u32, &50_u32);
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.interest_rate_bps, 4_000);

    // Rate above floor should pass through.
    client.update_risk_parameters(&borrower, &10_000_i128, &6_000_u32, &50_u32);
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.interest_rate_bps, 6_000);
}

/// Verifies that ceiling-only (no floor) correctly caps rates above the ceiling.
#[test]
fn clamp_ceiling_only() {
    let env = Env::default();
    let (client, _admin, borrower) = setup(&env);

    client.set_borrower_rate_ceiling(&borrower, &Some(6_000_u32));

    // Rate above ceiling should be capped.
    client.update_risk_parameters(&borrower, &10_000_i128, &8_000_u32, &50_u32);
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.interest_rate_bps, 6_000);

    // Rate below ceiling should pass through.
    client.update_risk_parameters(&borrower, &10_000_i128, &4_000_u32, &50_u32);
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.interest_rate_bps, 4_000);
}

/// Verifies the identity `max(min(r, c), f) == min(max(r, f), c)` for a
/// representative set of deterministic values.
#[test]
fn clamp_modularity_deterministic() {
    // Representative triples covering the key regions:
    // r < f (below floor), r in [f, c] (within range), r > c (above ceiling),
    // and edge cases at the boundaries.
    let cases: [(u32, u32, u32); 12] = [
        (0, 0, 0),           // all zero
        (0, 0, 10_000),      // zero floor, max ceiling
        (5_000, 0, 10_000),  // mid rate, no constraints
        (10_000, 0, 10_000), // max rate
        (100, 500, 1_000),   // r < f: below floor
        (500, 500, 1_000),   // r == f: at floor
        (750, 500, 1_000),   // r in [f, c]: within range
        (1_000, 500, 1_000), // r == c: at ceiling
        (2_000, 500, 1_000), // r > c: above ceiling
        (0, 5_000, 5_000),   // r < f == c: below degenerate
        (5_000, 5_000, 5_000), // r == f == c: at degenerate
        (10_000, 5_000, 5_000), // r > f == c: above degenerate
    ];

    for &(r, f, c) in &cases {
        let floor_then_ceiling = r.max(f).min(c);
        let ceiling_then_floor = r.min(c).max(f);
        assert_eq!(
            floor_then_ceiling, ceiling_then_floor,
            "modularity violated for r={}, f={}, c={}",
            r, f, c,
        );
    }
}

/// Verifies the identity holds at every boundary where one of the three
/// parameters is at 0 or 10_000.
#[test]
fn clamp_modularity_boundary_sweep() {
    let bounds = [0_u32, 1, 10_000];
    for &r in &bounds {
        for &f in &bounds {
            for &c in bounds.iter().filter(|&&c| c >= f) {
                let floor_then_ceiling = r.max(f).min(c);
                let ceiling_then_floor = r.min(c).max(f);
                assert_eq!(
                    floor_then_ceiling, ceiling_then_floor,
                    "modularity violated at r={}, f={}, c={}",
                    r, f, c,
                );
            }
        }
    }
}
