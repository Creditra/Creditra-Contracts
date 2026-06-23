// SPDX-License-Identifier: MIT

//! External default-oracle integration for liquidation valuation.
//!
//! # Issue context
//!
//! `docs/default-oracle.md` specifies the integration with an on-chain oracle
//! to inform liquidation valuation. The credit contract does *not* trust any
//! single external feed blindly — instead, it queries a configured oracle
//! contract, validates the price for freshness and basic sanity, and uses
//! the price to compute an upper-bound on the recovered amount recorded in
//! [`crate::lifecycle::settle_default_liquidation`].
//!
//! # Trust model
//!
//! - **Trusted**: the oracle contract's `latest_price` view (returns a
//!   `(price, timestamp)` pair).
//! - **Partially trusted**: the oracle operator (signers / data sources
//!   behind the oracle) — failures here can lead to stale or manipulated
//!   prices, which we bound with a freshness window and a hard upper-bound
//!   on recovered amount.
//! - **Never trusted**: the relayer of `settle_default_liquidation`. Admin
//!   authorization plus oracle attestation is required for every settlement.
//!
//! # Storage
//!
//! | Key | Type | Value | Written by |
//! |-----|------|-------|------------|
//! | `Symbol("oracle_cfg")` | Instance | `OracleConfig` (optional) | [`crate::Credit::set_default_oracle`] |
//!
//! Shared instance TTL with all other instance keys — extend alongside other
//! instance keys via `extend_ttl()`.
//!
//! # Failure mode (fail-closed)
//!
//! If `set_default_oracle` has not been called, [`crate::lifecycle::settle_default_liquidation`]
//! reverts with [`ContractError::MissingOracle`] (code 30). This is a deliberate
//! fail-closed posture: liquidations may not proceed silently without an
//! oracle-bound recovery value.

use crate::types::{ContractError, OracleConfig};

use soroban_sdk::{contracttype, Address, Env, Symbol, Val, Vec};

/// Default maximum-price-age bound (1 hour) when the admin does not configure
/// one explicitly via [`crate::Credit::set_default_oracle`].
///
/// This value is exported in the public API as the reference default; tests
/// that want a different bound pass it through the function argument.
pub const DEFAULT_MAX_PRICE_AGE_SECONDS: u64 = 3_600;

/// Hard upper bound on `max_price_age_seconds` accepted by [`crate::Credit::set_default_oracle`].
///
/// 1 year, expressed in seconds. Anything larger would invalidate the freshness
/// guarantee of [`ContractError::OraclePriceStale`] and is rejected before
/// storage mutation.
pub const MAX_PRICE_AGE_BOUND_SECONDS: u64 = 365 * 24 * 60 * 60;

/// Fixed-point scale for oracle prices.
///
/// Oracle prices are returned and stored as `i128` with this scale, so a
/// returned price equal to [`ORACLE_PRICE_SCALE`] represents parity (one unit
/// of the base token worth one unit of itself).
///
/// The recovery upper bound is computed as:
///
/// ```text
/// max_recovery_value = floor(price * utilized_amount / ORACLE_PRICE_SCALE)
/// ```
///
/// This keeps the division a single right-shift (in time) by exactly 9 decimal
/// places and avoids per-tenant precision drift between oracle operators.
pub const ORACLE_PRICE_SCALE: i128 = 1_000_000_000;

/// Latest oracle price point and its claimed emit time.
///
/// Decoded from the oracle contract's `latest_price()` invocation.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LatestPrice {
    /// Oracle price (scaled by [`ORACLE_PRICE_SCALE`]).
    pub price: i128,
    /// Ledger timestamp at which the oracle reported the price (u64 unix epoch).
    pub timestamp: u64,
}

/// Trait boundary over the default-oracle client.
///
/// Implementations are responsible for fetching the latest price from the
/// oracle contract. The production contract uses
/// [`CrossContractDefaultOracleClient`] (a real cross-contract invocation).
/// Tests can substitute alternative impls that read prices from in-memory
/// storage without incurring cross-contract budget, but the issue's required
/// integration tests use a real Soroban mock contract registered in the test
/// environment.
//
//
// `Eq, PartialEq` are derived on `LatestPrice` for tests. We do not derive
// them on the trait itself since trait methods do not produce values. Avoid
// splitting this trait into sealed types; surface it as a public trait name
// to satisfy the "oracle client trait" line item in issue #343.
pub trait DefaultOracleClient {
    /// Read the latest price point from the oracle at `oracle_addr`.
    ///
    /// Implementations MUST return a `(price, timestamp)` pair with `price`
    /// scaled by [`ORACLE_PRICE_SCALE`] and `timestamp` in unix seconds.
    ///
    /// Implementations MAY panic or trap; callers must be ready for these
    /// failures to surface as `SorobanHostError`. Validation of
    /// non-positive / stale prices is the responsibility of
    /// [`validate_oracle_price`], not this trait.
    fn latest_price(env: &Env, oracle_addr: &Address) -> LatestPrice;
}

/// Production default-oracle client.
///
/// Performs a real cross-contract invocation against the oracle at
/// `oracle_address`. The oracle contract is expected to expose a public
/// function called `latest_price` returning a `(i128, u64)` tuple, decoded
/// here into [`LatestPrice`].
///
/// # Failure semantics
///
/// If the oracle contract traps or returns a value that cannot be decoded as
/// `(i128, u64)`, the entire Soroban transaction reverts. There is no
/// resilient fallback — fail-closed is documented behavior for issue #343.
///
/// # Trust
///
/// This client does not enforce any bounds on the returned price. Validation
/// of staleness, sign, and timestamp ordering is performed by
/// [`validate_oracle_price`] immediately after the call returns.
pub struct CrossContractDefaultOracleClient;

impl DefaultOracleClient for CrossContractDefaultOracleClient {
    fn latest_price(env: &Env, oracle_addr: &Address) -> LatestPrice {
        // `Vec<Val>` carries zero args to the niladic `latest_price`.
        let args: Vec<Val> = Vec::new(env);
        let pair: (i128, u64) = env.invoke_contract(
            oracle_addr,
            &Symbol::new(env, "latest_price"),
            args,
        );
        LatestPrice {
            price: pair.0,
            timestamp: pair.1,
        }
    }
}

/// Default oracle client used by the production credit contract.
///
/// Calling [`read_oracle_price`] is the canonical entry point for
/// `settle_default_liquidation` and the only place where the credit contract
/// dispatches into a cross-contract oracle read.
pub fn read_oracle_price(env: &Env, oracle_addr: &Address) -> LatestPrice {
    CrossContractDefaultOracleClient::latest_price(env, oracle_addr)
}

/// Validate the oracle price point.
///
/// # Rejection rules
///
/// | Condition                                          | Error |
/// |----------------------------------------------------|-------|
/// | `price <= 0`                                       | [`ContractError::OraclePriceInvalid`] (32) |
/// | `timestamp > env.ledger().timestamp()`            | [`ContractError::OraclePriceInvalid`] (32) |
/// | `env.ledger().timestamp() - timestamp > bound`     | [`ContractError::OraclePriceStale`] (31) |
///
/// # Arguments
///
/// - `price`: latest oracle return value.
/// - `max_age_seconds`: configured maximum allowed age. `0` is **prohibited**
///   by the setter and never reaches this function in practice; the explicit
///   branch keeps the validator robust to direct misuse from tests.
pub fn validate_oracle_price(env: &Env, price: &LatestPrice, max_age_seconds: u64) {
    if price.price <= 0 {
        env.panic_with_error(ContractError::OraclePriceInvalid);
    }
    let now = env.ledger().timestamp();
    if price.timestamp > now {
        env.panic_with_error(ContractError::OraclePriceInvalid);
    }
    if max_age_seconds == 0 {
        // No staleness check configured — fail closed to prevent misconfig.
        env.panic_with_error(ContractError::OraclePriceStale);
    }
    let age = now.saturating_sub(price.timestamp);
    if age > max_age_seconds {
        env.panic_with_error(ContractError::OraclePriceStale);
    }
}

/// Compute the oracle-derived upper bound on `recovered_amount`.
///
/// The bound is `floor(price * utilized_amount / ORACLE_PRICE_SCALE)` using
/// checked arithmetic. The caller compares this bound against the supplied
/// `recovered_amount` to detect over-recovery.
///
/// # Failure semantics
///
/// Returns `ContractError::Overflow` if the products would exceed `i128::MAX`,
/// matching the credit contract's overflow policy elsewhere.
///
/// # Argument contract
///
/// `price` MUST already be validated for sign and freshness. This function
/// performs arithmetic only.
pub fn compute_max_recovery(
    env: &Env,
    price: i128,
    utilized_amount: i128,
) -> i128 {
    price
        .checked_mul(utilized_amount)
        .and_then(|v| v.checked_div(ORACLE_PRICE_SCALE))
        .unwrap_or_else(|| env.panic_with_error(ContractError::Overflow))
}

/// Persist [`OracleConfig`] in instance storage under the canonical
/// `Symbol("oracle_cfg")` key.
///
/// Centralizing the key here keeps the instance-storage audit table in
/// `docs/credit.md` accurate and prevents accidental key drift across modules.
///
/// # Storage
///
/// - **Type**: Instance storage (shared TTL with all instance keys)
/// - **Key**: `Symbol("oracle_cfg")`
pub fn oracle_config_key(env: &Env) -> Symbol {
    Symbol::new(env, "oracle_cfg")
}

/// Read the configured [`OracleConfig`] from instance storage.
pub fn get_oracle_config(env: &Env) -> Option<OracleConfig> {
    env.storage().instance().get(&oracle_config_key(env))
}

/// Persist [`OracleConfig`] to instance storage. Overwrites any previous value.
#[allow(dead_code)]
pub fn set_oracle_config(env: &Env, cfg: &OracleConfig) {
    env.storage()
        .instance()
        .set(&oracle_config_key(env), cfg);
}

/// Remove the configured [`OracleConfig`] from instance storage.
///
/// Used by admin tooling that wants to opt out of oracle-bound settlements.
/// Note that [`crate::lifecycle::settle_default_liquidation`] reacts to the
/// absence of a config with [`ContractError::MissingOracle`], so this function
/// is intended for explicit "un-wire the oracle" workflows, not for routine
/// operation.
#[allow(dead_code)]
pub fn clear_oracle_config(env: &Env) {
    env.storage().instance().remove(&oracle_config_key(env));
}

#[cfg(test)]
mod test_default_oracle {
    use super::*;

    fn fixture_env() -> Env {
        Env::default()
    }

    #[test]
    fn constant_default_in_one_hour() {
        assert_eq!(DEFAULT_MAX_PRICE_AGE_SECONDS, 3_600);
    }

    #[test]
    fn bound_is_one_year_in_seconds() {
        assert_eq!(MAX_PRICE_AGE_BOUND_SECONDS, 31_536_000);
    }

    #[test]
    fn scale_is_one_billion() {
        assert_eq!(ORACLE_PRICE_SCALE, 1_000_000_000);
    }

    #[test]
    fn validate_rejects_zero_price() {
        let env = fixture_env();
        let price = LatestPrice {
            price: 0,
            timestamp: 1_000,
        };
        // The panic is what we want — check the discriminant via a catch.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            validate_oracle_price(&env, &price, 60);
        }));
        assert!(result.is_err(), "zero price must be rejected");
    }

    #[test]
    fn validate_rejects_negative_price() {
        let env = fixture_env();
        let price = LatestPrice {
            price: -1,
            timestamp: 1_000,
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            validate_oracle_price(&env, &price, 60);
        }));
        assert!(result.is_err(), "negative price must be rejected");
    }

    #[test]
    fn validate_rejects_future_timestamp() {
        let env = fixture_env();
        env.ledger().with_mut(|li| li.timestamp = 1_000);
        let price = LatestPrice {
            price: ORACLE_PRICE_SCALE,
            timestamp: 2_000, // future
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            validate_oracle_price(&env, &price, 60);
        }));
        assert!(result.is_err(), "future timestamp must be rejected");
    }

    #[test]
    fn validate_rejects_stale_price() {
        let env = fixture_env();
        env.ledger().with_mut(|li| li.timestamp = 10_000);
        let price = LatestPrice {
            price: ORACLE_PRICE_SCALE,
            timestamp: 1_000, // age = 9000 > 60
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            validate_oracle_price(&env, &price, 60);
        }));
        assert!(result.is_err(), "stale price must be rejected");
    }

    #[test]
    fn validate_accepts_fresh_price_at_boundary() {
        let env = fixture_env();
        env.ledger().with_mut(|li| li.timestamp = 1_000);
        let price = LatestPrice {
            price: ORACLE_PRICE_SCALE,
            timestamp: 1_000, // age = 0
        };
        validate_oracle_price(&env, &price, 60); // must not panic
    }

    #[test]
    fn compute_max_recovery_parity_returns_utilized() {
        let env = fixture_env();
        let max = compute_max_recovery(&env, ORACLE_PRICE_SCALE, 1_000);
        assert_eq!(max, 1_000);
    }

    #[test]
    fn compute_max_recovery_half_price_halves_ceiling() {
        let env = fixture_env();
        let max = compute_max_recovery(&env, ORACLE_PRICE_SCALE / 2, 1_000);
        assert_eq!(max, 500);
    }

    #[test]
    fn compute_max_recovery_overflows_to_overflow_error() {
        let env = fixture_env();
        // i128::MAX * 2 / 1e9 overflows on multiplication.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            compute_max_recovery(&env, ORACLE_PRICE_SCALE, i128::MAX);
        }));
        assert!(result.is_err(), "overflow must panic");
    }
}
