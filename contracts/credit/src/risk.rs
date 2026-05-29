// SPDX-License-Identifier: MIT

//! Risk parameter management for credit lines.

#![warn(missing_docs)]

use crate::auth::require_admin_auth;
use crate::events::{publish_risk_parameters_updated, RiskParametersUpdatedEvent};
use crate::storage::{
    assert_not_paused, assert_ts_monotonic, rate_cfg_key, rate_formula_key,
    set_borrower_rate_floor,
};
use crate::types::{ContractError, CreditLineData, CreditStatus, RateChangeConfig, RateFormulaConfig};
use crate::events::publish_risk_parameters_updated;
use crate::storage::{
    assert_not_paused, assert_ts_monotonic, persist_credit_line, rate_cfg_key, rate_formula_key,
};
use crate::types::{
    ContractError, CreditLineData, CreditStatus, RateChangeConfig, RateFormulaConfig,
};
use soroban_sdk::{Address, Env};

/// Maximum interest rate in basis points (100%).
pub const MAX_INTEREST_RATE_BPS: u32 = 10_000;

/// Maximum risk score on the normalized 0-100 scale.
pub const MAX_RISK_SCORE: u32 = 100;

/// Compute an interest rate in basis points from a normalised risk score.
///
/// Maps a borrower's risk score linearly onto the range
/// `[min_rate_bps, max_rate_bps]`. A score of `0` maps to `min_rate_bps`
/// (lowest risk, lowest rate) and a score of `100` maps to `max_rate_bps`
/// (highest risk, highest rate).
///
/// Formula:
/// ```text
/// rate = min_rate_bps + (max_rate_bps - min_rate_bps) * score / 100
/// ```
///
/// # Rounding
/// Truncates toward zero. For example, a spread of `999` bps over a score of
/// `1` yields `9` bps (`9.99` truncated), not `10`.
///
/// # Parameters
/// assert_eq!(compute_rate_from_score_linear(50, 200, 800), 500);
///                   Values outside this range are accepted but produce
///                   extrapolated results; callers should validate first.
/// assert_eq!(compute_rate_from_score_linear(0, 200, 800), 200);
/// - `max_rate_bps`: Rate assigned to a score of `100` (worst credit).
///
/// assert_eq!(compute_rate_from_score_linear(100, 200, 800), 800);
/// Interest rate in basis points for the given score, clamped implicitly by
/// the linear interpolation between `min_rate_bps` and `max_rate_bps`.
///
/// # Panics
/// - If `max_rate_bps < min_rate_bps` (invalid range).
///
/// Compute interest rate from risk score using piecewise-linear formula.
///
/// # Formula
/// ```text
/// raw_rate = base_rate_bps + (risk_score * slope_bps_per_score)
/// effective_rate = clamp(raw_rate, min_rate_bps, min(max_rate_bps, MAX_INTEREST_RATE_BPS))
/// ```
///
/// Uses saturating arithmetic to prevent overflow — if the multiplication
/// overflows u32, it saturates to `u32::MAX` and is then clamped by the
/// upper bound.
///
/// # Arguments
/// * `cfg` — The rate formula configuration.
/// * `risk_score` — The borrower's risk score (0–100).
///
/// # Returns
/// The computed effective interest rate in basis points.
pub fn compute_rate_from_score(cfg: &RateFormulaConfig, risk_score: u32) -> u32 {
    let raw = cfg
        .base_rate_bps
        .saturating_add(risk_score.saturating_mul(cfg.slope_bps_per_score));
    let upper = cfg.max_rate_bps.min(MAX_INTEREST_RATE_BPS);
    raw.clamp(cfg.min_rate_bps, upper)
}

/// Set optional global rate-change caps (admin only).
pub fn set_rate_change_limits_legacy(env: Env, max_rate_change_bps: u32, rate_change_min_interval: u64) {
    assert_not_paused(&env);
    require_admin_auth(&env);

    let cfg = RateChangeConfig {
        max_rate_change_bps,
        rate_change_min_interval,
    };
    env.storage().instance().set(&rate_cfg_key(&env), &cfg);
}

/// Set a per-borrower interest rate floor (admin only).
pub fn set_borrower_rate_floor(env: Env, borrower: Address, floor_bps: Option<u32>) {
    require_admin_auth(&env);
    if let Some(floor) = floor_bps {
        assert!(floor <= MAX_INTEREST_RATE_BPS, "floor exceeds max rate");
    }
    crate::storage::set_borrower_rate_floor(&env, &borrower, floor_bps);
}

/// Update risk parameters for an existing credit line (admin only).
///
/// Loads the borrower's [`CreditLineData`], validates all inputs, applies
/// optional rate-change guardrails from [`RateChangeConfig`], then persists
/// the updated record and emits a [`RiskParametersUpdatedEvent`].
///
/// # Parameters
/// - `env`:              The Soroban environment.
/// - `borrower`:         Address of the borrower whose credit line to update.
/// - `credit_limit`:     New maximum borrowable amount. Must be `>= 0` and
///                       `>= credit_line.utilized_amount`.
/// - `interest_rate_bps`: New annual interest rate in basis points
///                       (`0 ..= 10_000`).
/// - `risk_score`:       New risk score (`0 ..= 100`).
///
/// # Panics
/// - If the caller is not the contract admin.
/// - If no credit line exists for `borrower`.
/// - If `credit_limit < 0`.
/// - If `credit_limit < credit_line.utilized_amount` (would strand debt above limit).
/// - If `interest_rate_bps > 10_000` (exceeds 100%).
/// - If `risk_score > 100`.
/// - If a [`RateChangeConfig`] is active and the absolute rate delta
///   `|new_rate - old_rate|` exceeds `max_rate_change_bps`.
/// - If a [`RateChangeConfig`] is active with `rate_change_min_interval > 0`,
///   a prior rate change exists, and the elapsed time since the last change
///   is less than `rate_change_min_interval`.
///
/// # Rate-change guardrails
/// When [`set_rate_change_limits`] has been called, every rate change is
/// subject to two additional checks:
///
/// 1. **Delta cap** — `|new_rate - old_rate| <= max_rate_change_bps`.
/// 2. **Interval floor** — seconds since `last_rate_update_ts` must be
///    `>= rate_change_min_interval` (skipped when `rate_change_min_interval`
///    is `0` or when no prior rate change has been recorded).
///
/// If the new rate equals the old rate, neither check is evaluated.
///
/// # Events
/// Emits [`RiskParametersUpdatedEvent`] on success.
/// This function handles updating the credit limit, risk score, and interest rate.
/// If a dynamic rate formula is configured, the `interest_rate_bps` parameter is
/// ignored and the rate is re-calculated based on the provided `risk_score`.
///
/// When [`RateChangeConfig`] is present, successful rate changes must stay
/// within the configured per-call delta and minimum elapsed interval. The
/// `last_rate_update_ts` field is refreshed only after a successful rate change.
///
/// ## Limit Decrease Behavior
///
/// When the new `credit_limit` is below the current `utilized_amount`:
/// - The credit line transitions to `Restricted` status.
/// - The borrower **cannot draw additional credit** until the utilization is reduced.
/// - **Repayments are still allowed**, enabling the borrower to reduce utilization back below the new limit.
/// - This avoids forced liquidation and gives the borrower a grace period to cure.
///
/// # Arguments
/// * `env` - The Soroban environment.
/// * `borrower` - The address of the borrower.
/// * `credit_limit` - The new credit limit (must be >= 0).
/// * `interest_rate_bps` - The manual interest rate (ignored if formula is enabled).
/// * `risk_score` - The new risk score (0-100).
///
/// # Panics
/// * If caller is not admin.
/// * If credit line does not exist.
/// * If validation fails (score > 100, etc.).
/// * If rate change exceeds configured limits.
/// * If the protocol is paused.
#[allow(clippy::doc_overindented_list_items)]
pub fn update_risk_parameters(
    env: Env,
    borrower: Address,
    credit_limit: i128,
    interest_rate_bps: u32,
    risk_score: u32,
) {
    assert_not_paused(&env);
    require_admin_auth(&env);

    let stored_line: CreditLineData = crate::storage::get_credit_line(&env, &borrower)
        .unwrap_or_else(|| env.panic_with_error(ContractError::CreditLineNotFound));
    let previous_utilized = stored_line.utilized_amount;

    let mut credit_line = crate::accrual::apply_accrual(&env, stored_line);

    if credit_limit < 0 {
        env.panic_with_error(ContractError::NegativeLimit);
    }
    if risk_score > MAX_RISK_SCORE {
        env.panic_with_error(ContractError::ScoreTooHigh);
    }

    // Validate credit limit is within configured bounds
    crate::lifecycle::validate_credit_limit_bounds(&env, credit_limit);

    let effective_rate = if let Some(formula_cfg) = get_rate_formula_config(env.clone()) {
        compute_rate_from_score(&formula_cfg, risk_score)
    } else {
        interest_rate_bps
    };

    // Apply per-borrower rate floor, if set.
    if let Some(floor_bps) = crate::storage::get_borrower_rate_floor(&env, &borrower) {
        effective_rate = effective_rate.max(floor_bps);
    }

    if effective_rate > MAX_INTEREST_RATE_BPS {
        env.panic_with_error(ContractError::RateTooHigh);
    }

    if effective_rate != credit_line.interest_rate_bps {
        if let Some(cfg) = get_rate_change_limits(env.clone()) {
            let delta = effective_rate.abs_diff(credit_line.interest_rate_bps);
            if delta > cfg.max_rate_change_bps {
                env.panic_with_error(ContractError::RateTooHigh);
            }

            if cfg.rate_change_min_interval > 0 && credit_line.last_rate_update_ts != 0 {
                let now = env.ledger().timestamp();
                let elapsed = now.saturating_sub(credit_line.last_rate_update_ts);
                if elapsed < cfg.rate_change_min_interval {
                    env.panic_with_error(ContractError::RateTooHigh);
                }
            }
        }

        let new_ts = env.ledger().timestamp();
        assert_ts_monotonic(&env, credit_line.last_rate_update_ts, new_ts);
        credit_line.last_rate_update_ts = new_ts;
    }

    if credit_limit < credit_line.utilized_amount {
        credit_line.status = CreditStatus::Restricted;
    } else if credit_line.status == CreditStatus::Restricted {
        credit_line.status = CreditStatus::Active;
    }

    credit_line.credit_limit = credit_limit;
    credit_line.interest_rate_bps = effective_rate;
    credit_line.risk_score = risk_score;

    persist_credit_line(&env, &borrower, &credit_line, previous_utilized);
    publish_risk_parameters_updated(&env, &borrower, credit_limit, effective_rate, risk_score);
}

/// Return the current rate-change guardrail configuration, if any.
///
/// # Parameters
/// - `env`: The Soroban environment.
///
/// # Returns
/// `Some(RateChangeConfig)` if guardrails have been configured via
/// [`set_rate_change_limits`], or `None` if no configuration exists (meaning
/// rate changes are unconstrained).
#[allow(dead_code)]
pub fn get_rate_change_limits(env: Env) -> Option<RateChangeConfig> {
    env.storage().instance().get(&rate_cfg_key(&env))
}

/// Retrieve the rate formula configuration from instance storage, if set.
///
/// # Storage
/// - **Type**: Instance storage (shared TTL with all instance keys)
/// - **Key**: `Symbol("rate_form")`
/// - **TTL Note**: Shares instance TTL — extend alongside other instance keys.
pub fn get_rate_formula_config(env: Env) -> Option<RateFormulaConfig> {
    env.storage()
        .instance()
        .get::<_, RateFormulaConfig>(&rate_formula_key(&env))
}
