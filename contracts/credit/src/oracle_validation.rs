// SPDX-License-Identifier: MIT

//! Oracle input validation for price-dependent settlement.
//!
//! # What
//!
//! Provides deterministic validation of oracle prices before `settle_default_liquidation`
//! commits state mutations. Enforces three critical invariants:
//!
//! 1. **Positivity**: price > 0 (prevents silent liquidation at zero/negative prices)
//! 2. **Freshness**: now - timestamp ≤ max_age_seconds (prevents stale data)
//! 3. **Stability**: |price - last_price| / last_price ≤ max_deviation_bps (circuit breaker)
//!
//! # How
//!
//! The validation layer supports three oracle modes, selected by a fixed
//! precedence (highest first):
//!
//! - **Registry-Median Mode** (highest precedence): If the weighted-median
//!   oracle registry has a quorum threshold configured
//!   (`oracles::is_registry_configured`), the authoritative price is the
//!   weighted median of the fresh reports from the approved oracles. The
//!   caller's `oracle_price` argument is ignored and settlement fails closed
//!   (`OracleQuorumNotMet`) when the registry cannot assemble a quorum.
//!
//! - **Quorum Mode** (next): If `OracleQuorumConfig` is set, uses the stored
//!   multi-oracle quorum price. The caller's `oracle_price` argument is ignored.
//!   Requires at least K feeds within the deviation tolerance.
//!
//! - **Single-Oracle Mode** (fallback): If only `OracleConfig` is set, validates
//!   the supplied `oracle_price` argument against the circuit-breaker bounds.
//!
//! - **No-Oracle Mode** (backward compat): If none of the above is set, skips
//!   validation. The caller's price argument is ignored; settlement proceeds
//!   without oracle gating.
//!
//! All validation occurs BEFORE state mutation. Any failure panics immediately with
//! a typed error code; no partial state is committed.
//!
//! # Why
//!
//! Settlement prices directly impact the `utilized_amount` reduction. An invalid price
//! can cause:
//! - Silent data loss if price is zero (utilized → 0 instantly)
//! - Incorrect liquidation if price is stale (uses outdated exchange rate)
//! - Flash-loan attacks if deviation tolerance is too loose
//!
//! Validating BEFORE mutation ensures all-or-nothing atomicity at the Soroban host level.

use crate::events::publish_oracle_price_accepted_event;
use crate::math_utils::compute_deviation_bps;
use crate::oracles;
use crate::storage::{
    get_oracle_config, get_oracle_last_price, get_oracle_last_price_ts, get_oracle_quorum_config,
    get_oracle_quorum_price, get_oracle_quorum_price_ts, set_oracle_last_price,
};
use crate::types::{ContractError, OracleConfig, OracleQuorumConfig};
use soroban_sdk::Env;

/// Outcome of oracle validation for settlement.
///
/// Each variant represents a successful validation in a different mode.
/// Failures panic with typed [`ContractError`] before returning.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolvedOraclePrice {
    /// Neither oracle config is set; price validation is skipped.
    /// Settlement proceeds without oracle gating (backward compatible).
    /// The supplied `oracle_price` argument is ignored.
    NotConfigured,

    /// The weighted-median oracle registry is active. The weighted median of
    /// the fresh oracle reports was successfully validated and accepted. The
    /// supplied `oracle_price` argument was ignored (registry mode takes
    /// precedence over every configured feed).
    ///
    /// Contains the validated registry median for observability/logging.
    RegistryMedianMode(i128),

    /// Multi-oracle quorum config was active. The stored quorum price
    /// was successfully validated and accepted. The supplied `oracle_price`
    /// argument was ignored (quorum mode takes precedence).
    ///
    /// Contains the validated quorum price for observability/logging.
    QuorumMode(i128),

    /// Single-oracle config was active. The supplied `oracle_price` argument
    /// was successfully validated against circuit-breaker bounds.
    ///
    /// Contains the validated single-oracle price for observability/logging.
    SingleOracleMode(i128),
}

impl ResolvedOraclePrice {
    /// Extract the validated price, if one was resolved.
    ///
    /// Returns `None` for `NotConfigured`, `Some(price)` otherwise.
    pub fn price(&self) -> Option<i128> {
        match self {
            ResolvedOraclePrice::NotConfigured => None,
            ResolvedOraclePrice::RegistryMedianMode(p) => Some(*p),
            ResolvedOraclePrice::QuorumMode(p) => Some(*p),
            ResolvedOraclePrice::SingleOracleMode(p) => Some(*p),
        }
    }
}

/// Validate oracle price(s) for settlement before state mutation.
///
/// # Behavior
///
/// 1. **Config resolution**: Load both oracle configs from storage
/// 2. **Mode selection**: Determine active mode
///    (registry median > quorum > single-oracle > none)
/// 3. **Registry-median validation** (if applicable):
///    - Read the weighted median of the fresh oracle reports
///    - Check a quorum can be assembled (panic with `OracleQuorumNotMet`)
///    - Check the median is a representable positive price
///      (panic with `OraclePriceInvalid`)
///    - Return the validated price
/// 4. **Quorum validation** (if applicable):
///    - Load stored quorum price + timestamp
///    - Check not `None` (panic with `OracleQuorumNotMet`)
///    - Check freshness (panic with `OraclePriceStale`)
///    - Return the validated price
/// 5. **Single-oracle validation** (if applicable):
///    - Check supplied `oracle_price` is Some (panic with `OraclePriceInvalid`)
///    - Check price > 0 (panic with `OraclePriceInvalid`)
///    - Check freshness (panic with `OraclePriceStale`)
///    - Check deviation from last price (panic with `OraclePriceDeviation`)
///    - Return the validated price
/// 6. **Return**: One of four outcomes
///
/// # Parameters
/// - `env`: Soroban environment (used to read storage and panic with errors)
/// - `oracle_price`: Optional price supplied by caller (single-oracle mode only;
///   ignored in registry, quorum and no-oracle modes)
///
/// # Returns
/// `ResolvedOraclePrice` enum on success:
/// - `NotConfigured` — no oracle facility is configured (backward compatible)
/// - `RegistryMedianMode(price)` — registry median validated and accepted
/// - `QuorumMode(price)` — quorum price validated and accepted
/// - `SingleOracleMode(price)` — single-oracle price validated and accepted
///
/// # Errors
/// Panics with typed `ContractError` on validation failure (before any state mutation):
/// - `OraclePriceInvalid` (36) — price is zero, negative, or missing when required
/// - `OraclePriceStale` (37) — price timestamp exceeds max_age_seconds
/// - `OraclePriceDeviation` (38) — price deviates from last accepted price
/// - `OracleQuorumNotMet` (50) — registry quorum not met, or quorum price not
///   yet submitted
///
/// # Side effects
/// None — pure validation function. Storage is not modified.
/// (See [`record_accepted_oracle_price`] to update storage after settlement succeeds.)
pub fn validate_settlement_oracle_price(
    env: &Env,
    oracle_price: Option<i128>,
) -> ResolvedOraclePrice {
    let now = env.ledger().timestamp();

    // Stage 1: Resolve which oracle configs are active
    let single_cfg = get_oracle_config(env);
    let quorum_cfg = get_oracle_quorum_config(env);

    // Stage 2: An active oracle registry takes precedence over every other feed.
    if oracles::is_registry_configured(env) {
        return validate_registry_median_mode(env);
    }

    // Stage 3: Quorum mode takes precedence when both are set
    if let Some(qcfg) = quorum_cfg {
        return validate_quorum_mode(env, &qcfg, now);
    }

    // Stage 4: Single-oracle mode if configured
    if let Some(cfg) = single_cfg {
        let validated_price = validate_single_oracle_mode(env, oracle_price, &cfg, now);
        return ResolvedOraclePrice::SingleOracleMode(validated_price);
    }

    // Stage 5: No oracle mode (backward compatible)
    ResolvedOraclePrice::NotConfigured
}

/// Validate the price in registry-median mode.
///
/// The weighted median of the fresh reports from the approved oracles is
/// authoritative and the caller-supplied `oracle_price` is deliberately
/// ignored: the circuit breaker must not be satisfiable by the same admin who
/// triggers settlement.
///
/// # Panics
/// - `OracleQuorumNotMet` if the registry cannot assemble a quorum of fresh
///   reports (fails closed rather than falling back to an admin-chosen price)
/// - `OraclePriceInvalid` if the median cannot be represented as a positive
///   `i128` settlement price
fn validate_registry_median_mode(env: &Env) -> ResolvedOraclePrice {
    // Reports are `u128` while settlement prices are `i128`; reject values that
    // cannot be represented instead of silently truncating them.
    let median = oracles::get_median_value(env.clone())
        .unwrap_or_else(|_| env.panic_with_error(ContractError::OracleQuorumNotMet));

    if median == 0 || median > i128::MAX as u128 {
        env.panic_with_error(ContractError::OraclePriceInvalid);
    }

    ResolvedOraclePrice::RegistryMedianMode(median as i128)
}

/// Validate oracle price in quorum mode.
///
/// # Panics
/// - `OracleQuorumNotMet` if no quorum price has been submitted yet
/// - `OraclePriceStale` if the stored quorum price exceeds max_age_seconds
fn validate_quorum_mode(env: &Env, cfg: &OracleQuorumConfig, now: u64) -> ResolvedOraclePrice {
    // Load stored quorum price and timestamp
    let quorum_price = get_oracle_quorum_price(env).unwrap_or_else(|| {
        env.panic_with_error(ContractError::OracleQuorumNotMet);
    });

    let quorum_ts = get_oracle_quorum_price_ts(env).unwrap_or_else(|| {
        env.panic_with_error(ContractError::OracleQuorumNotMet);
    });

    // Check freshness: now - ts <= max_age_seconds
    if now.saturating_sub(quorum_ts) > cfg.max_age_seconds {
        env.panic_with_error(ContractError::OraclePriceStale);
    }

    ResolvedOraclePrice::QuorumMode(quorum_price)
}

/// Validate oracle price in single-oracle mode.
///
/// # Panics
/// - `OraclePriceInvalid` if price is None, zero, or negative
/// - `OraclePriceStale` if price timestamp exceeds max_age_seconds
/// - `OraclePriceDeviation` if price deviates from last accepted price
fn validate_single_oracle_mode(
    env: &Env,
    oracle_price: Option<i128>,
    cfg: &OracleConfig,
    now: u64,
) -> i128 {
    // Stage 1: Check price is provided and positive
    let price = oracle_price.unwrap_or_else(|| {
        env.panic_with_error(ContractError::OraclePriceInvalid);
    });

    if price <= 0 {
        env.panic_with_error(ContractError::OraclePriceInvalid);
    }

    // Stage 2: Check freshness if a prior price was recorded
    if let Some(last_price) = get_oracle_last_price(env) {
        if let Some(last_ts) = get_oracle_last_price_ts(env) {
            // Check staleness: now - ts <= max_age_seconds
            if now.saturating_sub(last_ts) > cfg.max_age_seconds {
                env.panic_with_error(ContractError::OraclePriceStale);
            }

            // Stage 3: Check deviation from last price
            let deviation_bps = compute_deviation_bps(price, last_price).unwrap_or_else(|| {
                // compute_deviation_bps returns None only if last_price <= 0,
                // which should never happen if we stored it ourselves.
                // But if it does, treat as invalid.
                env.panic_with_error(ContractError::OraclePriceInvalid);
            });

            if deviation_bps > cfg.max_deviation_bps {
                env.panic_with_error(ContractError::OraclePriceDeviation);
            }
        }
    }
    // If no prior price: first acceptance, any positive price is allowed

    price
}

/// Record an accepted oracle price after settlement completes successfully.
///
/// # Parameters
/// - `env`: Soroban environment
/// - `price`: The validated price to store for next settlement's deviation check
///
/// # Storage
/// Atomically updates both `OracleLastPrice` and `OracleLastPriceTs` in instance storage
/// within the same host transaction.
///
/// # Side effects
/// Modifies instance storage and emits `("credit", "orc_price")` (not called
/// during validation, only after settlement commits). Emitting here — rather
/// than at the call site — keeps the event and the recorded price in lockstep
/// across every oracle mode, including the ones that resolve the price from
/// storage instead of the caller's argument.
pub fn record_accepted_oracle_price(env: &Env, price: i128) {
    let ts = env.ledger().timestamp();
    set_oracle_last_price(env, price, ts);
    publish_oracle_price_accepted_event(env, price, ts);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oracles::{OracleDataKey, OracleReportData};
    use crate::types::{OracleConfig, OracleQuorumConfig};
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::{Address, Env};

    // ── helpers ──────────────────────────────────────────────────────────────

    fn setup_oracle_config(env: &Env, max_dev_bps: u32, max_age_sec: u64) {
        let cfg = OracleConfig {
            max_deviation_bps: max_dev_bps,
            max_age_seconds: max_age_sec,
        };
        crate::storage::set_oracle_config(env, &cfg);
    }

    fn setup_oracle_quorum_config(env: &Env, k: u32, max_dev_bps: u32, max_age_sec: u64) {
        let cfg = OracleQuorumConfig {
            min_quorum_k: k,
            max_deviation_bps: max_dev_bps,
            max_age_seconds: max_age_sec,
        };
        crate::storage::set_oracle_quorum_config(env, &cfg);
    }

    /// Run `body` with the credit contract as the current contract.
    ///
    /// Soroban storage is only reachable from inside a contract's execution
    /// context, so these unit tests register a fresh instance of [`crate::Credit`]
    /// and enter it with [`Env::as_contract`] instead of going through the
    /// generated client.
    fn in_contract<R>(env: &Env, body: impl FnOnce() -> R) -> R {
        let contract_id = env.register(crate::Credit, ());
        env.as_contract(&contract_id, body)
    }

    // ── no-oracle mode ───────────────────────────────────────────────────────

    #[test]
    fn no_oracle_config_returns_not_configured() {
        let env = Env::default();
        env.mock_all_auths();

        in_contract(&env, || {
            let result = validate_settlement_oracle_price(&env, Some(1_000i128));
            assert_eq!(result, ResolvedOraclePrice::NotConfigured);
            assert_eq!(result.price(), None);
        });
    }

    #[test]
    fn no_oracle_config_accepts_none_price() {
        let env = Env::default();
        env.mock_all_auths();

        in_contract(&env, || {
            let result = validate_settlement_oracle_price(&env, None);
            assert_eq!(result, ResolvedOraclePrice::NotConfigured);
        });
    }

    // ── single-oracle mode — success cases ────────────────────────────────────

    #[test]
    fn single_oracle_first_price_accepted() {
        let env = Env::default();
        env.mock_all_auths();

        in_contract(&env, || {
            setup_oracle_config(&env, 500, 3600);

            let result = validate_settlement_oracle_price(&env, Some(1_000i128));
            match result {
                ResolvedOraclePrice::SingleOracleMode(p) => assert_eq!(p, 1_000),
                _ => panic!("expected SingleOracleMode"),
            }
        });
    }

    #[test]
    fn single_oracle_second_price_within_deviation() {
        let env = Env::default();
        env.mock_all_auths();

        in_contract(&env, || {
            setup_oracle_config(&env, 500, 3600); // 5% max deviation

            // First price
            env.ledger().with_mut(|l| l.timestamp = 1_000);
            let _result = validate_settlement_oracle_price(&env, Some(1_000i128));
            record_accepted_oracle_price(&env, 1_000);

            // Second price: 1_040 is 4% from 1_000 (within 5%)
            env.ledger().with_mut(|l| l.timestamp = 1_500);
            let result = validate_settlement_oracle_price(&env, Some(1_040i128));
            match result {
                ResolvedOraclePrice::SingleOracleMode(p) => assert_eq!(p, 1_040),
                _ => panic!("expected SingleOracleMode"),
            }
        });
    }

    #[test]
    fn single_oracle_price_at_exact_max_age_accepted() {
        let env = Env::default();
        env.mock_all_auths();

        in_contract(&env, || {
            setup_oracle_config(&env, 500, 3600);

            env.ledger().with_mut(|l| l.timestamp = 1_000);
            let _result = validate_settlement_oracle_price(&env, Some(1_000i128));
            record_accepted_oracle_price(&env, 1_000);

            // Advance exactly max_age_seconds (should still be fresh)
            env.ledger().with_mut(|l| l.timestamp = 1_000 + 3600);
            let result = validate_settlement_oracle_price(&env, Some(1_010i128));
            assert!(matches!(result, ResolvedOraclePrice::SingleOracleMode(_)));
        });
    }

    #[test]
    fn single_oracle_boundary_deviation_accepted() {
        let env = Env::default();
        env.mock_all_auths();

        in_contract(&env, || {
            setup_oracle_config(&env, 500, 3600);

            env.ledger().with_mut(|l| l.timestamp = 1_000);
            let _result = validate_settlement_oracle_price(&env, Some(1_000i128));
            record_accepted_oracle_price(&env, 1_000);

            // Price 1_050 is exactly 5% from 1_000 (at boundary)
            env.ledger().with_mut(|l| l.timestamp = 1_500);
            let result = validate_settlement_oracle_price(&env, Some(1_050i128));
            assert!(matches!(result, ResolvedOraclePrice::SingleOracleMode(_)));
        });
    }

    // ── single-oracle mode — error cases ─────────────────────────────────────

    #[test]
    #[should_panic(expected = "Error(Contract, #36)")]
    fn single_oracle_missing_price_panics() {
        let env = Env::default();
        env.mock_all_auths();

        in_contract(&env, || {
            setup_oracle_config(&env, 500, 3600);

            // Config is set but price is None — must panic
            let _result = validate_settlement_oracle_price(&env, None);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #36)")]
    fn single_oracle_zero_price_panics() {
        let env = Env::default();
        env.mock_all_auths();

        in_contract(&env, || {
            setup_oracle_config(&env, 500, 3600);

            let _result = validate_settlement_oracle_price(&env, Some(0i128));
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #36)")]
    fn single_oracle_negative_price_panics() {
        let env = Env::default();
        env.mock_all_auths();

        in_contract(&env, || {
            setup_oracle_config(&env, 500, 3600);

            let _result = validate_settlement_oracle_price(&env, Some(-100i128));
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #37)")]
    fn single_oracle_stale_price_panics() {
        let env = Env::default();
        env.mock_all_auths();

        in_contract(&env, || {
            setup_oracle_config(&env, 500, 3600);

            env.ledger().with_mut(|l| l.timestamp = 1_000);
            let _result = validate_settlement_oracle_price(&env, Some(1_000i128));
            record_accepted_oracle_price(&env, 1_000);

            // Advance beyond max_age_seconds
            env.ledger().with_mut(|l| l.timestamp = 1_000 + 3601);
            let _result = validate_settlement_oracle_price(&env, Some(1_010i128));
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #38)")]
    fn single_oracle_upward_deviation_exceeds_bound() {
        let env = Env::default();
        env.mock_all_auths();

        in_contract(&env, || {
            setup_oracle_config(&env, 500, 3600); // 5% max

            env.ledger().with_mut(|l| l.timestamp = 1_000);
            let _result = validate_settlement_oracle_price(&env, Some(1_000i128));
            record_accepted_oracle_price(&env, 1_000);

            // Price 1_100 is 10% from 1_000 (exceeds 5%)
            env.ledger().with_mut(|l| l.timestamp = 1_500);
            let _result = validate_settlement_oracle_price(&env, Some(1_100i128));
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #38)")]
    fn single_oracle_downward_deviation_exceeds_bound() {
        let env = Env::default();
        env.mock_all_auths();

        in_contract(&env, || {
            setup_oracle_config(&env, 500, 3600);

            env.ledger().with_mut(|l| l.timestamp = 1_000);
            let _result = validate_settlement_oracle_price(&env, Some(1_000i128));
            record_accepted_oracle_price(&env, 1_000);

            // Price 900 is 10% below 1_000 (exceeds 5%)
            env.ledger().with_mut(|l| l.timestamp = 1_500);
            let _result = validate_settlement_oracle_price(&env, Some(900i128));
        });
    }

    // ── quorum mode — success cases ───────────────────────────────────────────

    #[test]
    fn quorum_mode_takes_precedence_over_single_oracle() {
        let env = Env::default();
        env.mock_all_auths();

        in_contract(&env, || {
            setup_oracle_config(&env, 500, 3600);
            setup_oracle_quorum_config(&env, 2, 500, 3600);

            // Store quorum price with a timestamp so the value is considered fresh.
            crate::storage::set_oracle_quorum_price(&env, 2_000i128, env.ledger().timestamp());

            // Supply single-oracle price (should be ignored)
            let result = validate_settlement_oracle_price(&env, Some(1_000i128));
            match result {
                ResolvedOraclePrice::QuorumMode(p) => assert_eq!(p, 2_000),
                _ => panic!("expected QuorumMode"),
            }
        });
    }

    #[test]
    fn quorum_mode_fresh_price_accepted() {
        let env = Env::default();
        env.mock_all_auths();

        in_contract(&env, || {
            setup_oracle_quorum_config(&env, 2, 500, 3600);

            env.ledger().with_mut(|l| l.timestamp = 1_000);
            crate::storage::set_oracle_quorum_price(&env, 1_000i128, 1_000u64);

            env.ledger().with_mut(|l| l.timestamp = 2_000);
            let result = validate_settlement_oracle_price(&env, None);
            match result {
                ResolvedOraclePrice::QuorumMode(p) => assert_eq!(p, 1_000),
                _ => panic!("expected QuorumMode"),
            }
        });
    }

    #[test]
    fn quorum_mode_at_exact_max_age_accepted() {
        let env = Env::default();
        env.mock_all_auths();

        in_contract(&env, || {
            setup_oracle_quorum_config(&env, 2, 500, 3600);

            env.ledger().with_mut(|l| l.timestamp = 1_000);
            crate::storage::set_oracle_quorum_price(&env, 1_000i128, 1_000u64);

            // Advance exactly max_age_seconds
            env.ledger().with_mut(|l| l.timestamp = 1_000 + 3600);
            let result = validate_settlement_oracle_price(&env, None);
            assert!(matches!(result, ResolvedOraclePrice::QuorumMode(_)));
        });
    }

    // ── quorum mode — error cases ────────────────────────────────────────────

    #[test]
    #[should_panic(expected = "Error(Contract, #50)")]
    fn quorum_mode_missing_price_panics() {
        let env = Env::default();
        env.mock_all_auths();

        in_contract(&env, || {
            setup_oracle_quorum_config(&env, 2, 500, 3600);

            // Config is set but no quorum price submitted yet
            let _result = validate_settlement_oracle_price(&env, None);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #37)")]
    fn quorum_mode_stale_price_panics() {
        let env = Env::default();
        env.mock_all_auths();

        in_contract(&env, || {
            setup_oracle_quorum_config(&env, 2, 500, 3600);

            env.ledger().with_mut(|l| l.timestamp = 1_000);
            crate::storage::set_oracle_quorum_price(&env, 1_000i128, 1_000u64);

            // Advance beyond max_age_seconds
            env.ledger().with_mut(|l| l.timestamp = 1_000 + 3601);
            let _result = validate_settlement_oracle_price(&env, None);
        });
    }

    // ── registry-median mode — success cases ─────────────────────────────────

    /// Register each `(oracle, weight, value)` entry, store its report, and set
    /// a quorum threshold plus reporting window.
    ///
    /// The public registry helpers are admin-gated and each call re-authorizes
    /// the admin; calling them all inside a single test frame trips Soroban's
    /// "frame is already authorized" check. These tests therefore write the
    /// registry state directly, mirroring what `add_oracle` / `report_value` /
    /// `set_quorum_threshold` persist.
    fn setup_registry(env: &Env, quorum: u32, window: u64, reports: &[(Address, u32, u128)]) {
        let mut oracle_list = soroban_sdk::Vec::new(env);

        for (oracle, weight, value) in reports {
            oracle_list.push_back(oracle.clone());
            env.storage()
                .instance()
                .set(&OracleDataKey::OracleWeight(oracle.clone()), weight);
            env.storage().instance().set(
                &OracleDataKey::OracleReport(oracle.clone()),
                &OracleReportData {
                    value: *value,
                    timestamp: env.ledger().timestamp(),
                },
            );
        }

        env.storage()
            .instance()
            .set(&OracleDataKey::OracleList, &oracle_list);
        env.storage()
            .instance()
            .set(&OracleDataKey::QuorumThreshold, &quorum);
        env.storage()
            .instance()
            .set(&OracleDataKey::ReportingWindow, &window);
    }

    #[test]
    fn registry_median_is_resolved_when_registry_is_active() {
        let env = Env::default();
        env.mock_all_auths();

        in_contract(&env, || {
            env.ledger().with_mut(|l| l.timestamp = 1_000);

            let o1 = Address::generate(&env);
            let o2 = Address::generate(&env);
            let o3 = Address::generate(&env);
            setup_registry(
                &env,
                60,
                3_600,
                &[(o1, 10, 1_000), (o2, 20, 1_100), (o3, 30, 1_200)],
            );

            let result = validate_settlement_oracle_price(&env, None);
            match result {
                ResolvedOraclePrice::RegistryMedianMode(p) => assert_eq!(p, 1_100),
                _ => panic!("expected RegistryMedianMode"),
            }
        });
    }

    #[test]
    fn registry_median_takes_precedence_over_quorum_and_single_oracle() {
        let env = Env::default();
        env.mock_all_auths();

        in_contract(&env, || {
            env.ledger().with_mut(|l| l.timestamp = 1_000);

            // Both lower-precedence facilities are configured with a stored price.
            setup_oracle_config(&env, 500, 3_600);
            setup_oracle_quorum_config(&env, 2, 500, 3_600);
            crate::storage::set_oracle_quorum_price(&env, 2_000i128, 1_000u64);

            let o1 = Address::generate(&env);
            let o2 = Address::generate(&env);
            setup_registry(&env, 30, 3_600, &[(o1, 10, 1_000), (o2, 20, 1_100)]);

            // Sorted: (1_000, w10), (1_100, w20); target = ceil(30 / 2) = 15, so
            // the cumulative weight first reaches the target at 1_100.
            // The caller-supplied price and the stored quorum price are both ignored.
            let result = validate_settlement_oracle_price(&env, Some(9_999i128));
            match result {
                ResolvedOraclePrice::RegistryMedianMode(p) => assert_eq!(p, 1_100),
                _ => panic!("expected RegistryMedianMode"),
            }
        });
    }

    // ── registry-median mode — error cases ───────────────────────────────────

    #[test]
    #[should_panic(expected = "Error(Contract, #50)")]
    fn registry_median_below_quorum_panics() {
        let env = Env::default();
        env.mock_all_auths();

        in_contract(&env, || {
            env.ledger().with_mut(|l| l.timestamp = 1_000);

            let o1 = Address::generate(&env);
            setup_registry(&env, 60, 3_600, &[(o1, 10, 1_000)]);

            let _result = validate_settlement_oracle_price(&env, None);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #50)")]
    fn registry_median_without_reports_panics() {
        let env = Env::default();
        env.mock_all_auths();

        in_contract(&env, || {
            // Threshold configured, but no oracle has ever reported.
            setup_registry(&env, 10, 3_600, &[]);

            let _result = validate_settlement_oracle_price(&env, None);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #50)")]
    fn registry_median_ignores_stale_reports() {
        let env = Env::default();
        env.mock_all_auths();

        in_contract(&env, || {
            env.ledger().with_mut(|l| l.timestamp = 1_000);

            let o1 = Address::generate(&env);
            setup_registry(&env, 10, 60, &[(o1, 10, 1_000)]);

            // Age the only report out of the freshness window.
            env.ledger().with_mut(|l| l.timestamp = 1_061);

            let _result = validate_settlement_oracle_price(&env, None);
        });
    }

    // ── price extraction ──────────────────────────────────────────────────────

    #[test]
    fn resolved_price_extraction() {
        let env = Env::default();

        let not_cfg = ResolvedOraclePrice::NotConfigured;
        assert_eq!(not_cfg.price(), None);

        let registry = ResolvedOraclePrice::RegistryMedianMode(1_100i128);
        assert_eq!(registry.price(), Some(1_100));

        let quorum = ResolvedOraclePrice::QuorumMode(1_500i128);
        assert_eq!(quorum.price(), Some(1_500));

        let single = ResolvedOraclePrice::SingleOracleMode(2_000i128);
        assert_eq!(single.price(), Some(2_000));
    }
}
