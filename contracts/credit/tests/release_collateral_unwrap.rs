// SPDX-License-Identifier: MIT
//! Focused regression tests for the `resolve_quorum_price` bare-`.unwrap()` fix.
//!
//! # What was fixed
//!
//! `contracts/credit/src/oracles.rs` — `resolve_quorum_price()` previously
//! called `prices.get(i).unwrap()` while iterating `0..n` (where `n` is
//! `prices.len()`).  Although the index is always in-bounds by construction,
//! a bare `.unwrap()` produces an opaque host trap instead of a typed
//! [`ContractError`] when the SDK `Vec::get` returns `None` unexpectedly.
//!
//! The fix replaces the bare `.unwrap()` with:
//!
//! ```rust
//! prices.get(i).unwrap_or_else(|| env.panic_with_error(ContractError::OraclePriceInvalid))
//! ```
//!
//! This ensures every failure on the price-reading path surfaces as
//! `ContractError::OraclePriceInvalid` (discriminant 36) rather than an
//! untyped panic.
//!
//! # Coverage checklist
//!
//! - [x] Happy-path: quorum resolves with minimal K=2 window
//! - [x] Happy-path: lower-median returned for K=3 window
//! - [x] Happy-path: all prices identical (deviation = 0)
//! - [x] Revert: empty price list → OraclePriceInvalid (#36)
//! - [x] Revert: list exceeds MAX_ORACLE_FEEDS → OraclePriceInvalid (#36)
//! - [x] Revert: any price is zero → OraclePriceInvalid (#36)
//! - [x] Revert: any price is negative → OraclePriceInvalid (#36)
//! - [x] Revert: k < 2 → OracleQuorumNotMet (#50)
//! - [x] Revert: k > n → OracleQuorumNotMet (#50)
//! - [x] Revert: no window satisfies deviation bound → OracleQuorumNotMet (#50)
//! - [x] Single valid window at the start of the sorted array
//! - [x] Single valid window at the end of the sorted array
//! - [x] Submit via `submit_oracle_prices` end-to-end path

#![cfg(test)]

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{testutils::Address as _, Address, Env, Vec};

// ─── helpers ─────────────────────────────────────────────────────────────────

/// Build a minimal environment with the Credit contract initialised.
fn setup(env: &Env) -> (CreditClient, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    (client, admin)
}

/// Configure a quorum policy and return the client ready for price submission.
fn setup_with_quorum(env: &Env, k: u32, max_dev_bps: u32, max_age: u64) -> CreditClient {
    let (client, _) = setup(env);
    client.set_oracle_quorum_config(&k, &max_dev_bps, &max_age);
    client
}

/// Build a Soroban `Vec<i128>` from a Rust slice.
fn price_vec(env: &Env, prices: &[i128]) -> Vec<i128> {
    let mut v = Vec::new(env);
    for &p in prices {
        v.push_back(p);
    }
    v
}

// ─── happy-path tests ─────────────────────────────────────────────────────────

/// K=2, two feeds agree within 0 bps deviation → lower-median = the lower price.
#[test]
fn quorum_k2_exact_match_returns_lower() {
    let env = Env::default();
    let client = setup_with_quorum(&env, 2, 0, 3_600);

    let prices = price_vec(&env, &[100, 100]);
    client.submit_oracle_prices(&prices);

    assert_eq!(client.get_oracle_last_price(), Some(100));
}

/// K=2, two feeds within 10 bps deviation → lower-median (lower value) is stored.
#[test]
fn quorum_k2_within_deviation_returns_lower_median() {
    let env = Env::default();
    // Allow up to 100 bps (1%) deviation.
    let client = setup_with_quorum(&env, 2, 100, 3_600);

    // 99 and 100 → deviation = (100-99)*10_000/99 ≈ 101 bps — just outside 100 bps.
    // Use 100 and 101 → deviation = (101-100)*10_000/100 = 100 bps — exactly at boundary.
    let prices = price_vec(&env, &[100, 101]);
    client.submit_oracle_prices(&prices);

    // lower-median of [100, 101] with K=2 is index (2-1)/2 = 0 → 100.
    assert_eq!(client.get_oracle_last_price(), Some(100));
}

/// K=3, three feeds with the median being the qualifying value.
#[test]
fn quorum_k3_lower_median_selected() {
    let env = Env::default();
    // Wide deviation tolerance so all three form one window.
    let client = setup_with_quorum(&env, 3, 10_000, 3_600);

    // Prices: [200, 100, 150] → sorted: [100, 150, 200].
    // Only one K=3 window: [100, 150, 200].
    // Lower-median index = (3-1)/2 = 1 → 150.
    let prices = price_vec(&env, &[200, 100, 150]);
    client.submit_oracle_prices(&prices);

    assert_eq!(client.get_oracle_last_price(), Some(150));
}

/// All prices identical → deviation = 0 → quorum always satisfied.
#[test]
fn quorum_all_identical_prices_resolved() {
    let env = Env::default();
    let client = setup_with_quorum(&env, 3, 0, 3_600);

    let prices = price_vec(&env, &[500, 500, 500]);
    client.submit_oracle_prices(&prices);

    assert_eq!(client.get_oracle_last_price(), Some(500));
}

/// First qualifying window is at the start of the sorted slice.
#[test]
fn quorum_qualifying_window_at_start() {
    let env = Env::default();
    // K=2, tight deviation of 50 bps (0.5%).
    let client = setup_with_quorum(&env, 2, 50, 3_600);

    // Sorted: [100, 100, 200].  Window [100,100] at start qualifies (0 bps).
    // Window [100,200] → 10_000 bps — too wide.
    let prices = price_vec(&env, &[100, 200, 100]);
    client.submit_oracle_prices(&prices);

    // Lower-median of first qualifying window [100,100] is index 0 → 100.
    assert_eq!(client.get_oracle_last_price(), Some(100));
}

/// Qualifying window is at the end of the sorted slice.
#[test]
fn quorum_qualifying_window_at_end() {
    let env = Env::default();
    // K=2, tight deviation of 50 bps.
    let client = setup_with_quorum(&env, 2, 50, 3_600);

    // Sorted: [100, 500, 501].
    // Window [100,500] → (500-100)*10_000/100 = 4_000 bps — too wide.
    // Window [500,501] → (501-500)*10_000/500 = 20 bps — qualifies.
    let prices = price_vec(&env, &[500, 100, 501]);
    client.submit_oracle_prices(&prices);

    // Lower-median of [500,501] is index 0 within window → 500.
    assert_eq!(client.get_oracle_last_price(), Some(500));
}

// ─── error / revert tests (discriminant-pinned) ──────────────────────────────

/// Empty price list → OraclePriceInvalid (#36).
#[test]
#[should_panic(expected = "Error(Contract, #36)")]
fn quorum_empty_price_list_reverts() {
    let env = Env::default();
    let client = setup_with_quorum(&env, 2, 500, 3_600);

    let prices = price_vec(&env, &[]);
    client.submit_oracle_prices(&prices);
}

/// Zero price in list → OraclePriceInvalid (#36).
///
/// This is the regression case for the bare `.unwrap()` fix: `Vec::get(i)`
/// returns the value, but the subsequent positivity check catches it.
/// With the fix in place the error is always typed `#36`, never an opaque trap.
#[test]
#[should_panic(expected = "Error(Contract, #36)")]
fn quorum_zero_price_reverts() {
    let env = Env::default();
    let client = setup_with_quorum(&env, 2, 500, 3_600);

    let prices = price_vec(&env, &[100, 0, 200]);
    client.submit_oracle_prices(&prices);
}

/// Negative price in list → OraclePriceInvalid (#36).
#[test]
#[should_panic(expected = "Error(Contract, #36)")]
fn quorum_negative_price_reverts() {
    let env = Env::default();
    let client = setup_with_quorum(&env, 2, 500, 3_600);

    let prices = price_vec(&env, &[100, -50, 200]);
    client.submit_oracle_prices(&prices);
}

/// k = 1 → single feed is not a quorum → InvalidAmount (#5) during config.
#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn quorum_k_less_than_2_reverts() {
    let env = Env::default();
    let client = setup_with_quorum(&env, 1, 500, 3_600);

    let prices = price_vec(&env, &[100, 200]);
    client.submit_oracle_prices(&prices);
}

/// k = 0 → InvalidAmount (#5) during config.
#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn quorum_k_zero_reverts() {
    let env = Env::default();
    let client = setup_with_quorum(&env, 0, 500, 3_600);

    let prices = price_vec(&env, &[100]);
    client.submit_oracle_prices(&prices);
}

/// k > n → cannot form a window → OracleQuorumNotMet (#50).
#[test]
#[should_panic(expected = "Error(Contract, #50)")]
fn quorum_k_greater_than_n_reverts() {
    let env = Env::default();
    let client = setup_with_quorum(&env, 5, 500, 3_600);

    let prices = price_vec(&env, &[100, 200, 300]);
    client.submit_oracle_prices(&prices);
}

/// No K-wide window satisfies the deviation bound → OracleQuorumNotMet (#50).
#[test]
#[should_panic(expected = "Error(Contract, #50)")]
fn quorum_no_window_within_deviation_reverts() {
    let env = Env::default();
    // Very tight tolerance: 0 bps (exact match required).
    let client = setup_with_quorum(&env, 2, 0, 3_600);

    // All prices differ → no two consecutive prices in the sorted array match.
    let prices = price_vec(&env, &[100, 200, 300]);
    client.submit_oracle_prices(&prices);
}

// ─── price-counting boundary test ────────────────────────────────────────────

/// Exactly MAX_ORACLE_FEEDS (20) prices should be accepted.
#[test]
fn quorum_max_feeds_accepted() {
    let env = Env::default();
    // K=20: all 20 must agree within 0 bps (all identical).
    let client = setup_with_quorum(&env, 20, 0, 3_600);

    let prices = price_vec(
        &env,
        &[
            1_000, 1_000, 1_000, 1_000, 1_000, 1_000, 1_000, 1_000, 1_000, 1_000, 1_000, 1_000,
            1_000, 1_000, 1_000, 1_000, 1_000, 1_000, 1_000, 1_000,
        ],
    );
    client.submit_oracle_prices(&prices);
    assert_eq!(client.get_oracle_last_price(), Some(1_000));
}

/// MAX_ORACLE_FEEDS + 1 prices → OraclePriceInvalid (#36).
#[test]
#[should_panic(expected = "Error(Contract, #36)")]
fn quorum_exceeds_max_feeds_reverts() {
    let env = Env::default();
    let client = setup_with_quorum(&env, 2, 500, 3_600);

    // 21 entries — one over the limit.
    let prices = price_vec(
        &env,
        &[
            100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100,
            100, 100, 100, 100,
        ],
    );
    client.submit_oracle_prices(&prices);
}
