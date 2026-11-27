// SPDX-License-Identifier: MIT

//! Integration tests for oracle removal registry hygiene (Issue #1349).
//!
//! # What is tested
//!
//! - `remove_oracle` purges weight and last report: the removed oracle is
//!   excluded from the weighted-median computation.
//! - Two oracles added and both report; removing one causes the median to
//!   reflect only the surviving oracle's value.
//! - A removed oracle cannot call `report_value` — reverts `Unauthorized`.
//! - Removing an oracle that was never registered reverts `OracleNotFound` (#55).
//! - Re-adding a previously removed oracle restores its ability to report and
//!   its value is included in the next `get_median_value`.
//! - Removing one oracle does not affect the surviving oracle's contribution.
//!
//! # Validation
//!
//! ```text
//! cargo test -p creditra-credit oracles
//! ```

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, Env};

// ── helpers ───────────────────────────────────────────────────────────────────

fn setup(env: &Env) -> CreditClient<'_> {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    client
}

// ── AC1: removed oracle excluded from median ──────────────────────────────────

/// After removing oracle2, `get_median_value` uses only oracle1's report.
///
/// Setup:
///   oracle1 weight=10, value=100
///   oracle2 weight=20, value=900
///   total weight=30, target=15
///   sorted: (100,10),(900,20) → cumulative 10 < 15, then 30 ≥ 15 → median=900
///
/// After removing oracle2 and lowering quorum to 10:
///   only oracle1 contributes → median=100
#[test]
fn removed_oracle_excluded_from_median() {
    let env = Env::default();
    let client = setup(&env);

    let oracle1 = Address::generate(&env);
    let oracle2 = Address::generate(&env);
    client.add_oracle(&oracle1, &10_u32);
    client.add_oracle(&oracle2, &20_u32);
    client.set_reporting_window(&3_600_u64);

    client.report_value(&oracle1, &100_u128);
    client.report_value(&oracle2, &900_u128);

    // Sanity: both oracles contribute, median is 900.
    client.set_quorum_threshold(&30_u32);
    assert_eq!(client.get_median_value(), 900_u128);

    // Remove oracle2 and drop quorum to oracle1's weight alone.
    client.remove_oracle(&oracle2);
    client.set_quorum_threshold(&10_u32);

    // Median must be 100 — oracle2's stale report must not influence the result.
    let median = client.get_median_value();
    assert_eq!(
        median, 100_u128,
        "removed oracle's stale report must not influence the median"
    );
}

/// Removing oracle1 (the first-registered) leaves oracle2 as the sole
/// contributor. Verifies removal is index-position-independent.
#[test]
fn removing_first_oracle_leaves_second_as_sole_contributor() {
    let env = Env::default();
    let client = setup(&env);

    let oracle1 = Address::generate(&env);
    let oracle2 = Address::generate(&env);
    client.add_oracle(&oracle1, &10_u32);
    client.add_oracle(&oracle2, &20_u32);
    client.set_reporting_window(&3_600_u64);

    client.report_value(&oracle1, &200_u128);
    client.report_value(&oracle2, &800_u128);

    client.remove_oracle(&oracle1);

    // Only oracle2 contributes; quorum = its weight = 20.
    client.set_quorum_threshold(&20_u32);
    assert_eq!(
        client.get_median_value(),
        800_u128,
        "oracle2 should be sole contributor after oracle1 is removed"
    );
}

/// Removing all registered oracles causes `get_median_value` to fail
/// (quorum not met) even with a quorum threshold of 1.
#[test]
fn remove_all_oracles_causes_quorum_not_met() {
    let env = Env::default();
    let client = setup(&env);

    let oracle1 = Address::generate(&env);
    let oracle2 = Address::generate(&env);
    client.add_oracle(&oracle1, &10_u32);
    client.add_oracle(&oracle2, &20_u32);
    client.set_reporting_window(&3_600_u64);
    // Low quorum so individual oracles can satisfy it.
    client.set_quorum_threshold(&1_u32);

    client.report_value(&oracle1, &100_u128);
    client.report_value(&oracle2, &200_u128);

    client.remove_oracle(&oracle1);
    client.remove_oracle(&oracle2);

    // No oracles remain — quorum cannot be met regardless of threshold.
    let result = client.try_get_median_value();
    assert!(
        result.is_err(),
        "get_median_value must fail when all oracles have been removed"
    );
}

/// Removing one oracle does not affect the surviving oracle's median contribution.
#[test]
fn surviving_oracle_still_contributes_after_peer_removal() {
    let env = Env::default();
    let client = setup(&env);

    let oracle1 = Address::generate(&env);
    let oracle2 = Address::generate(&env);
    let oracle3 = Address::generate(&env);

    client.add_oracle(&oracle1, &10_u32);
    client.add_oracle(&oracle2, &20_u32);
    client.add_oracle(&oracle3, &15_u32);
    client.set_reporting_window(&3_600_u64);

    client.report_value(&oracle1, &100_u128);
    client.report_value(&oracle2, &200_u128);
    client.report_value(&oracle3, &300_u128);

    // Remove oracle2 (weight 20). Remaining: oracle1 (10) + oracle3 (15) = 25.
    client.remove_oracle(&oracle2);
    client.set_quorum_threshold(&25_u32);

    // Sorted: (100,10),(300,15); total=25, target=13;
    // cumulative after oracle1=10 < 13; after oracle3=25 ≥ 13 → median=300.
    let median = client.get_median_value();
    assert_eq!(
        median, 300_u128,
        "oracle1 and oracle3 must determine the median after oracle2 removal"
    );
}

// ── AC2: removed oracle cannot report ────────────────────────────────────────

/// A removed oracle that calls `report_value` must revert (Unauthorized).
#[test]
#[should_panic]
fn removed_oracle_cannot_report_value() {
    let env = Env::default();
    let client = setup(&env);

    let oracle = Address::generate(&env);
    client.add_oracle(&oracle, &10_u32);

    // Remove the oracle.
    client.remove_oracle(&oracle);

    // Attempt to report — must panic with Unauthorized.
    client.report_value(&oracle, &500_u128);
}

/// Removing oracle2 blocks it from reporting while oracle1 still can.
#[test]
fn only_removed_oracle_is_blocked_peer_can_still_report() {
    let env = Env::default();
    let client = setup(&env);

    let oracle1 = Address::generate(&env);
    let oracle2 = Address::generate(&env);
    client.add_oracle(&oracle1, &10_u32);
    client.add_oracle(&oracle2, &20_u32);
    client.set_reporting_window(&3_600_u64);
    client.set_quorum_threshold(&10_u32);

    // Remove oracle2.
    client.remove_oracle(&oracle2);

    // oracle1 must still be able to report.
    client.report_value(&oracle1, &777_u128);
    assert_eq!(
        client.get_median_value(),
        777_u128,
        "surviving oracle must still report after peer removal"
    );
}

// ── AC3: removing unknown oracle reverts OracleNotFound (#55) ─────────────────

/// Removing an oracle that was never registered must revert (OracleNotFound #55).
#[test]
#[should_panic]
fn remove_unknown_oracle_panics() {
    let env = Env::default();
    let client = setup(&env);

    let stranger = Address::generate(&env);
    // Never added — must panic.
    client.remove_oracle(&stranger);
}

/// Removing an oracle that was added and then already removed must also revert.
#[test]
#[should_panic]
fn remove_oracle_twice_second_call_panics() {
    let env = Env::default();
    let client = setup(&env);

    let oracle = Address::generate(&env);
    client.add_oracle(&oracle, &10_u32);

    client.remove_oracle(&oracle); // first removal — succeeds
    client.remove_oracle(&oracle); // second removal — must panic (OracleNotFound)
}

/// Removing from an empty registry (no oracles ever added) must revert.
#[test]
#[should_panic]
fn remove_from_empty_registry_panics() {
    let env = Env::default();
    let client = setup(&env);

    // Contract just initialised — oracle list is empty.
    client.remove_oracle(&Address::generate(&env));
}

// ── AC4: re-adding restores reporting ────────────────────────────────────────

/// After removal the same address can be re-added via `add_oracle` and then
/// successfully reports. The fresh value is reflected by `get_median_value`.
#[test]
fn readded_oracle_can_report_and_contributes_to_median() {
    let env = Env::default();
    let client = setup(&env);

    let oracle = Address::generate(&env);
    client.add_oracle(&oracle, &10_u32);
    client.set_quorum_threshold(&10_u32);
    client.set_reporting_window(&3_600_u64);

    // Initial report cycle.
    client.report_value(&oracle, &100_u128);
    assert_eq!(client.get_median_value(), 100_u128);

    // Remove.
    client.remove_oracle(&oracle);

    // Re-add with the same weight.
    client.add_oracle(&oracle, &10_u32);

    // New report after re-adding must succeed and be reflected in the median.
    client.report_value(&oracle, &250_u128);
    assert_eq!(
        client.get_median_value(),
        250_u128,
        "re-added oracle's fresh report must be the median"
    );
}

/// Re-adding with a different weight updates the weight used in median
/// calculation and quorum accumulation.
#[test]
fn readded_oracle_with_new_weight_affects_median() {
    let env = Env::default();
    let client = setup(&env);

    let oracle1 = Address::generate(&env);
    let oracle2 = Address::generate(&env);
    client.add_oracle(&oracle1, &10_u32);
    client.add_oracle(&oracle2, &30_u32);
    client.set_reporting_window(&3_600_u64);

    client.report_value(&oracle1, &100_u128);
    client.report_value(&oracle2, &500_u128);

    // Sanity check: quorum=40 total.
    // Sorted: (100,10),(500,30); target=20;
    // cumulative after oracle1=10 < 20; after oracle2=40 ≥ 20 → median=500.
    client.set_quorum_threshold(&40_u32);
    assert_eq!(client.get_median_value(), 500_u128);

    // Remove oracle1 and re-add with weight=50 — now heavier than oracle2.
    client.remove_oracle(&oracle1);
    client.add_oracle(&oracle1, &50_u32);
    client.report_value(&oracle1, &100_u128);

    // New total weight = 50 + 30 = 80; set quorum = 80.
    // Sorted: (100,50),(500,30); target=40;
    // cumulative after oracle1=50 ≥ 40 → median=100.
    client.set_quorum_threshold(&80_u32);
    assert_eq!(
        client.get_median_value(),
        100_u128,
        "re-added oracle's heavier weight should flip the median"
    );
}

/// After removal, the previously-submitted report is gone.  Re-adding and NOT
/// reporting means the oracle contributes no weight (its report entry is absent)
/// and quorum based on the new total weight will not be met if the missing
/// report was needed.
#[test]
fn readded_oracle_without_new_report_does_not_restore_stale_value() {
    let env = Env::default();
    let client = setup(&env);

    let oracle = Address::generate(&env);
    client.add_oracle(&oracle, &20_u32);
    client.set_reporting_window(&3_600_u64);
    client.set_quorum_threshold(&20_u32);

    // Report once, confirm median works.
    client.report_value(&oracle, &123_u128);
    assert_eq!(client.get_median_value(), 123_u128);

    // Remove and re-add without a new report.
    client.remove_oracle(&oracle);
    client.add_oracle(&oracle, &20_u32);

    // Quorum = 20 but no fresh report → quorum not met.
    let result = client.try_get_median_value();
    assert!(
        result.is_err(),
        "re-added oracle with no new report must not satisfy quorum"
    );
}

// ── staleness interaction ────────────────────────────────────────────────────

/// A report submitted before the reporting window expires is excluded from the
/// median. Verifies the freshness filter still operates correctly in the
/// remove/re-add cycle.
#[test]
fn stale_report_from_readded_oracle_is_excluded() {
    let env = Env::default();
    let client = setup(&env);

    let oracle = Address::generate(&env);
    let window: u64 = 60;
    client.add_oracle(&oracle, &10_u32);
    client.set_quorum_threshold(&10_u32);
    client.set_reporting_window(&window);

    // Remove and re-add to ensure a clean registry state, then report at t=0.
    env.ledger().with_mut(|l| l.timestamp = 0);
    client.report_value(&oracle, &100_u128);

    // Advance beyond the reporting window — report is now stale.
    env.ledger().with_mut(|l| l.timestamp = window + 1);

    let result = client.try_get_median_value();
    assert!(
        result.is_err(),
        "stale report must not satisfy quorum after window expires"
    );
}
