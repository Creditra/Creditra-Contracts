// SPDX-License-Identifier: MIT

//! # Multi-oracle quorum price resolution
//!
//! Implements the quorum-of-K algorithm for combining multiple independent
//! oracle price feeds into a single canonical price used by
//! [`crate::lib::settle_default_liquidation`].
//!
//! ## Algorithm
//!
//! Given N submitted prices and a quorum threshold K:
//!
//! 1. Validate every price is strictly positive and N ≤ [`MAX_ORACLE_FEEDS`].
//! 2. Sort prices ascending (selection sort; O(n²) but bounded by
//!    [`MAX_ORACLE_FEEDS`] ≤ 20 to keep gas predictable).
//! 3. Slide a window of K consecutive prices over the sorted array.
//! 4. For each window, check whether the highest price deviates from the
//!    lowest by no more than `max_deviation_bps` of the lowest.
//! 5. Return the **lower-median** of the first qualifying window.
//! 6. Panic with [`crate::types::ContractError::OracleQuorumNotMet`] if no
//!    window qualifies.
//!
//! ## Security properties
//!
//! - An outlier feed cannot influence the result unless it falls inside a
//!   qualifying K-wide window alongside K−1 honest feeds.
//! - Requires at least K feeds to agree, so an attacker must corrupt K
//!   independent feeds simultaneously to manipulate the canonical price.
//! - The stack buffer is bounded at compile time; gas consumption is O(n²)
//!   for sorting and O(n) for window scanning.

use soroban_sdk::{Env, Vec};

use crate::math_utils::compute_deviation_bps;
use crate::types::{ContractError, OracleQuorumConfig};

/// Maximum number of oracle price feeds accepted per `submit_oracle_prices` call.
///
/// Limits gas consumption and keeps the stack buffer within WASM limits.
/// Adjust after gas profiling if the protocol sources more feeds.
pub const MAX_ORACLE_FEEDS: u32 = 20;

/// Resolve a single canonical price from N submitted oracle prices using
/// the quorum-of-K sliding-window algorithm.
///
/// # Parameters
/// - `env`: Soroban host environment (used to panic with typed errors).
/// - `prices`: N submitted prices in any order, one per oracle feed.
/// - `cfg`: Quorum configuration supplying K, max deviation, and max age.
///
/// # Returns
/// The lower-median price of the first K-wide consecutive window (in sorted
/// ascending order) whose highest-to-lowest spread is within
/// `cfg.max_deviation_bps`.
///
/// # Errors
///
/// Panics with [`ContractError::OraclePriceInvalid`] when:
/// - The price list is empty.
/// - The price list exceeds [`MAX_ORACLE_FEEDS`].
/// - Any individual price is ≤ 0.
///
/// Panics with [`ContractError::OracleQuorumNotMet`] when:
/// - `min_quorum_k < 2` (a single feed is not a meaningful quorum).
/// - `min_quorum_k > n` (cannot form a window larger than the input).
/// - No K-wide window in the sorted array satisfies the deviation bound.
pub fn resolve_quorum_price(env: &Env, prices: &Vec<i128>, cfg: &OracleQuorumConfig) -> i128 {
    let n = prices.len();

    if n == 0 || n > MAX_ORACLE_FEEDS {
        env.panic_with_error(ContractError::OraclePriceInvalid);
    }

    let k = cfg.min_quorum_k;
    if k < 2 || k > n {
        env.panic_with_error(ContractError::OracleQuorumNotMet);
    }

    // Copy prices into a fixed stack buffer and validate positivity.
    let mut buf = [0i128; MAX_ORACLE_FEEDS as usize];
    for i in 0..n {
        let p = prices.get(i).unwrap();
        if p <= 0 {
            env.panic_with_error(ContractError::OraclePriceInvalid);
        }
        buf[i as usize] = p;
    }
    let slice = &mut buf[..n as usize];

    // Selection sort — O(n²), safe and predictable for n ≤ MAX_ORACLE_FEEDS.
    let len = slice.len();
    for i in 0..len {
        let mut min_idx = i;
        for j in (i + 1)..len {
            if slice[j] < slice[min_idx] {
                min_idx = j;
            }
        }
        slice.swap(i, min_idx);
    }

    // Scan every consecutive K-wide window in sorted order.
    // A window qualifies when the deviation of its highest element from its
    // lowest is within cfg.max_deviation_bps. Return the lower-median of the
    // first qualifying window.
    let kk = k as usize;
    for i in 0..=(len - kk) {
        let lo = slice[i];
        let hi = slice[i + kk - 1];
        // lo > 0 is guaranteed; compute_deviation_bps returns None only for
        // non-positive last_price, which cannot happen here.
        let dev = compute_deviation_bps(hi, lo).unwrap_or(u32::MAX);
        if dev <= cfg.max_deviation_bps {
            // Lower-median: index (kk-1)/2 within the window.
            let median_idx = i + (kk - 1) / 2;
            return slice[median_idx];
        }
    }

    env.panic_with_error(ContractError::OracleQuorumNotMet)
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::OracleQuorumConfig;
    use soroban_sdk::{vec, Env, Vec};

    fn cfg(k: u32, dev: u32) -> OracleQuorumConfig {
        OracleQuorumConfig {
            min_quorum_k: k,
            max_deviation_bps: dev,
            max_age_seconds: 3_600,
        }
    }

    // ── happy-path ────────────────────────────────────────────────────────────

    #[test]
    fn two_of_two_exact_match_returns_lower() {
        let env = Env::default();
        let prices = vec![&env, 1_000i128, 1_000i128];
        assert_eq!(resolve_quorum_price(&env, &prices, &cfg(2, 0)), 1_000);
    }

    #[test]
    fn two_of_three_outlier_ignored() {
        // Sorted: 1_000, 1_040, 2_000 — k=2, dev=500 bps (5%)
        // Window [1_000, 1_040]: dev=400 bps ≤ 500 → qualifies
        // Lower-median of size-2 window at index 0: index 0 → 1_000
        let env = Env::default();
        let prices = vec![&env, 2_000i128, 1_000i128, 1_040i128];
        assert_eq!(resolve_quorum_price(&env, &prices, &cfg(2, 500)), 1_000);
    }

    #[test]
    fn three_of_five_returns_median_of_window() {
        // Sorted: 980, 990, 1_000, 1_010, 5_000 — k=3, dev=500 bps
        // Window [980, 990, 1_000]: dev(1_000, 980)=204 bps ≤ 500 → qualifies
        // Lower-median idx = 0+(3-1)/2 = 1 → 990
        let env = Env::default();
        let prices = vec![&env, 1_000i128, 5_000i128, 980i128, 990i128, 1_010i128];
        assert_eq!(resolve_quorum_price(&env, &prices, &cfg(3, 500)), 990);
    }

    #[test]
    fn all_identical_prices_zero_deviation() {
        let env = Env::default();
        let prices = vec![&env, 500i128, 500i128, 500i128];
        assert_eq!(resolve_quorum_price(&env, &prices, &cfg(3, 0)), 500);
    }

    #[test]
    fn window_at_end_of_sorted_array() {
        // Sorted: 1_000, 2_000, 2_010 — k=2, dev=100 bps (1%)
        // Window [1_000, 2_000]: dev=10_000 bps > 100 → skip
        // Window [2_000, 2_010]: dev=50 bps ≤ 100 → qualifies → 2_000
        let env = Env::default();
        let prices = vec![&env, 2_010i128, 1_000i128, 2_000i128];
        assert_eq!(resolve_quorum_price(&env, &prices, &cfg(2, 100)), 2_000);
    }

    #[test]
    fn four_of_four_returns_lower_median() {
        // Sorted: 100, 110, 120, 130 — k=4, dev=5_000 bps (50%)
        // Single window; lower-median idx = 0+(4-1)/2 = 1 → 110
        let env = Env::default();
        let prices = vec![&env, 130i128, 100i128, 120i128, 110i128];
        assert_eq!(resolve_quorum_price(&env, &prices, &cfg(4, 5_000)), 110);
    }

    #[test]
    fn two_of_two_within_boundary_bps() {
        // Sorted: 1_000, 1_050 — dev = 500 bps == max_deviation_bps → qualifies
        let env = Env::default();
        let prices = vec![&env, 1_050i128, 1_000i128];
        assert_eq!(resolve_quorum_price(&env, &prices, &cfg(2, 500)), 1_000);
    }

    // ── error paths ───────────────────────────────────────────────────────────

    #[test]
    #[should_panic]
    fn empty_prices_panics() {
        let env = Env::default();
        let empty: Vec<i128> = Vec::new(&env);
        resolve_quorum_price(&env, &empty, &cfg(2, 500));
    }

    #[test]
    #[should_panic]
    fn negative_price_panics() {
        let env = Env::default();
        let prices = vec![&env, 1_000i128, -1i128, 1_010i128];
        resolve_quorum_price(&env, &prices, &cfg(2, 500));
    }

    #[test]
    #[should_panic]
    fn zero_price_panics() {
        let env = Env::default();
        let prices = vec![&env, 1_000i128, 0i128];
        resolve_quorum_price(&env, &prices, &cfg(2, 500));
    }

    #[test]
    #[should_panic]
    fn k_greater_than_n_panics() {
        let env = Env::default();
        let prices = vec![&env, 1_000i128, 1_010i128];
        resolve_quorum_price(&env, &prices, &cfg(3, 500));
    }

    #[test]
    #[should_panic]
    fn k_equals_one_panics() {
        let env = Env::default();
        let prices = vec![&env, 1_000i128, 1_010i128];
        resolve_quorum_price(&env, &prices, &cfg(1, 500));
    }

    #[test]
    #[should_panic]
    fn k_equals_zero_panics() {
        let env = Env::default();
        let prices = vec![&env, 1_000i128, 1_010i128];
        resolve_quorum_price(&env, &prices, &cfg(0, 500));
    }

    #[test]
    #[should_panic]
    fn no_qualifying_window_panics() {
        // All prices more than 5% apart: no 2-wide window qualifies
        let env = Env::default();
        let prices = vec![&env, 1_000i128, 2_000i128, 4_000i128];
        resolve_quorum_price(&env, &prices, &cfg(2, 500));
    }

    #[test]
    #[should_panic]
    fn just_over_deviation_bound_panics() {
        // 1_000 and 1_051 → dev = 510 bps > 500
        let env = Env::default();
        let prices = vec![&env, 1_051i128, 1_000i128];
        resolve_quorum_price(&env, &prices, &cfg(2, 500));
    }
}
