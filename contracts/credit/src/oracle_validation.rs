// SPDX-License-Identifier: MIT

//! Oracle price validation for `Credit::settle_default_liquidation`.
//!
//! # Price-source precedence
//!
//! `settle_default_liquidation` takes an optional, caller-supplied
//! `oracle_price`. The protocol supports three independent oracle facilities
//! and decides which of them is authoritative using a fixed precedence. The
//! order, highest first, is:
//!
//! 1. **Registry median** — when the weighted-median oracle registry has a
//!    quorum threshold configured (`oracles::is_registry_configured`), the
//!    authoritative price is the weighted median returned by
//!    `oracles::get_median_value`. The caller-supplied `oracle_price` is
//!    ignored. If the registry cannot reach quorum, the settlement is rejected
//!    with [`ContractError::OracleQuorumNotMet`].
//! 2. **Quorum-of-K price** — when [`OracleQuorumConfig`] is set, the price
//!    resolved and stored by `submit_oracle_prices` is authoritative. The
//!    caller-supplied `oracle_price` is ignored and only staleness is
//!    re-checked here.
//! 3. **Single-oracle circuit breaker** — when only [`OracleConfig`] is set,
//!    the caller-supplied `oracle_price` is validated for positivity,
//!    staleness and deviation from the last accepted price.
//! 4. **Unconfigured** — with no oracle configuration at all no validation
//!    runs and the caller-supplied price is ignored (legacy behaviour).
//!
//! Because the registry short-circuits the chain, an operator can upgrade a
//! deployment to an independent, multi-source price without any migration:
//! registering oracles and setting a quorum threshold is enough.

use soroban_sdk::Env;

use crate::events::publish_oracle_price_accepted_event;
use crate::math_utils::compute_deviation_bps;
use crate::oracles;
use crate::storage;
use crate::types::{ContractError, OracleConfig, OracleQuorumConfig};

/// Validate the price used by `Credit::settle_default_liquidation` and return
/// the price settlement should treat as authoritative.
///
/// See the [module documentation](self) for the precedence between the
/// weighted-median registry, the quorum-of-K price feed and the single-oracle
/// circuit breaker.
///
/// # Returns
/// - `Some(price)` when an oracle facility is configured and the price passed
///   validation. The accepted price is also persisted as the new "last
///   accepted price" and an `orc_price` event is emitted.
/// - `None` when no oracle facility is configured; no validation runs and
///   settlement proceeds exactly as it did before oracle support existed.
///
/// # Panics
/// - [`ContractError::OracleQuorumNotMet`] — registry active, quorum not met.
/// - [`ContractError::OraclePriceInvalid`] — missing, non-positive, or
///   out-of-range price.
/// - [`ContractError::OraclePriceStale`] — price older than `max_age_seconds`.
/// - [`ContractError::OraclePriceDeviation`] — single-oracle price deviates
///   from the last accepted price by more than `max_deviation_bps`.
pub fn validate_settlement_oracle_price(env: &Env, oracle_price: Option<i128>) -> Option<i128> {
    if oracles::is_registry_configured(env) {
        return Some(registry_median_price(env));
    }

    if let Some(cfg) = storage::get_oracle_quorum_config(env) {
        return Some(quorum_price(env, &cfg));
    }

    if let Some(cfg) = storage::get_oracle_config(env) {
        return Some(single_oracle_price(env, oracle_price, &cfg));
    }

    // No oracle configuration: legacy behaviour, nothing to validate.
    None
}

/// Registry mode: the weighted median of the fresh reports from the approved
/// oracles is authoritative and the caller-supplied `oracle_price` is ignored.
fn registry_median_price(env: &Env) -> i128 {
    let median = oracles::get_median_value(env.clone()).unwrap_or_else(|_| {
        // Registry active but quorum not met — fail closed rather than fall
        // back to an admin-supplied price.
        env.panic_with_error(ContractError::OracleQuorumNotMet)
    });

    // The registry stores `u128` values while settlement prices are `i128`.
    // Reject anything that cannot be represented, or that is not a usable
    // positive price, instead of silently truncating.
    if median == 0 || median > i128::MAX as u128 {
        env.panic_with_error(ContractError::OraclePriceInvalid);
    }
    let price = median as i128;

    let now = env.ledger().timestamp();
    storage::set_oracle_last_price(env, price, now);
    publish_oracle_price_accepted_event(env, price, now);
    price
}

/// Quorum-of-K mode: the price stored by `submit_oracle_prices` is
/// authoritative; only its age is re-checked here. The caller-supplied
/// `oracle_price` is ignored.
fn quorum_price(env: &Env, cfg: &OracleQuorumConfig) -> i128 {
    let last_ts = storage::get_oracle_last_price_ts(env)
        .unwrap_or_else(|| env.panic_with_error(ContractError::OraclePriceInvalid));
    let now = env.ledger().timestamp();
    if now.saturating_sub(last_ts) > cfg.max_age_seconds {
        env.panic_with_error(ContractError::OraclePriceStale);
    }
    let price = storage::get_oracle_last_price(env)
        .unwrap_or_else(|| env.panic_with_error(ContractError::OraclePriceInvalid));
    publish_oracle_price_accepted_event(env, price, now);
    price
}

/// Single-oracle mode: validate the caller-supplied price for positivity,
/// staleness and deviation from the last accepted price.
fn single_oracle_price(env: &Env, oracle_price: Option<i128>, cfg: &OracleConfig) -> i128 {
    let price =
        oracle_price.unwrap_or_else(|| env.panic_with_error(ContractError::OraclePriceInvalid));
    if price <= 0 {
        env.panic_with_error(ContractError::OraclePriceInvalid);
    }

    let now = env.ledger().timestamp();
    if let Some(last_ts) = storage::get_oracle_last_price_ts(env) {
        let age = now.saturating_sub(last_ts);
        if age > cfg.max_age_seconds {
            env.panic_with_error(ContractError::OraclePriceStale);
        }

        if let Some(last_price) = storage::get_oracle_last_price(env) {
            let deviation = compute_deviation_bps(price, last_price)
                .unwrap_or_else(|| env.panic_with_error(ContractError::OraclePriceInvalid));
            if deviation > cfg.max_deviation_bps {
                env.panic_with_error(ContractError::OraclePriceDeviation);
            }
        }
    }

    storage::set_oracle_last_price(env, price, now);
    publish_oracle_price_accepted_event(env, price, now);
    price
}
