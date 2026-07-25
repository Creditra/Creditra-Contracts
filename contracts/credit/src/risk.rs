// SPDX-License-Identifier: MIT

//! Risk parameter management for credit lines.
//!
//! # What
//!
//! Owns the rate-formula primitives and the `update_risk_parameters`
//! entrypoint:
//!
//! - [`compute_rate_from_score`] — the piecewise-linear formula
//!   `r(k) = clamp(b + k * s, r_min, min(r_max, 10_000))` documented in
//!   [`docs/RISK_PRICING.md`](../../../docs/RISK_PRICING.md) §2.1.
//! - [`update_risk_parameters`] — the admin path that applies a new
//!   `(credit_limit, interest_rate_bps, risk_score)` triple, with the rate
//!   either supplied or formula-derived, then bounded by:
//!     - the per-borrower [`set_borrower_rate_floor`] floor,
//!     - the magnitude+cadence cap encoded by [`RateChangeConfig`] in
//!       `Symbol("rate_cfg")` instance storage,
//!     - the global ceiling [`MAX_INTEREST_RATE_BPS`] = 10_000.
//! - [`set_rate_change_limits`] — configure the magnitude+cadence
//!   cap.
//! - [`set_penalty_surcharge_bps`] — configure the additive surcharge
//!   applied to delinquent lines during accrual (see [`crate::accrual`]).
//! - [`set_borrower_rate_floor`] — per-borrower minimum.
//!
//! # How
//!
//! All rate arithmetic uses saturating `u32` multiplication; a misconfigured
//! `b + 100 * s` saturates rather than overflowing, then the clamp brings
//! it back into the declared `[r_min, r_max]` range. The clamp upper bound
//! is the minimum of the configured `max_rate_bps` and the protocol-wide
//! `MAX_INTEREST_RATE_BPS`, so even a misconfigured formula cannot exceed
//! 10_000 bps.
//!
//! `update_risk_parameters` invokes [`crate::accrual::apply_accrual`]
//! before mutating, so the rate change is applied against capitalized
//! interest — the borrower is never charged the new rate on debt accrued
//! under the old rate.
//!
//! # Why
//!
//! The clamp + saturating combo is the contract's defense against:
//!
//! 1. **Admin compromise leading to 1000 % APR.** The 10_000 bps ceiling is
//!    hard-coded; even admin cannot bypass it.
//! 2. **Per-step rate shock.** `RateChangeConfig` bounds the size of each
//!    increment AND the minimum interval between increments. A compromised
//!    admin can at most raise rates by `max_rate_change_bps` per
//!    `rate_change_min_interval` window, giving borrowers time to repay.
//! 3. **Formula misconfiguration.** `min_rate_bps > max_rate_bps` is
//!    rejected at config-set time; runtime evaluation cannot produce a
//!    rate outside the `[r_min, r_max]` ∩ `[0, 10_000]` interval.
//!
//! See [`docs/risk-based-rate-formula.md`](../../../docs/risk-based-rate-formula.md)
//! for the normative formula spec and
//! [`docs/SECURITY.md`](../../../docs/SECURITY.md) §2 (threats T6, T8) for
//! the threat-model justification.

#![warn(missing_docs)]

use crate::auth::require_admin_auth;
use crate::events::publish_risk_parameters_updated;
use crate::storage::{assert_not_paused, persist_credit_line, rate_cfg_key, rate_formula_key};
use crate::types::{
    ContractError, CreditLineData, CreditStatus, RateChangeConfig, RateFormulaConfig,
};
use soroban_sdk::{Address, Env};

/// Maximum interest rate in basis points (100%).
pub const MAX_INTEREST_RATE_BPS: u32 = 10_000;

/// Maximum risk score on the normalized 0-100 scale.
pub const MAX_RISK_SCORE: u32 = 100;

/// Compute interest rate from risk score using the piecewise-linear formula.
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
pub fn compute_rate_from_score(cfg: &RateFormulaConfig, risk_score: u32) -> u32 {
    let raw = cfg
        .base_rate_bps
        .saturating_add(risk_score.saturating_mul(cfg.slope_bps_per_score));
    let upper = cfg.max_rate_bps.min(MAX_INTEREST_RATE_BPS);
    raw.clamp(cfg.min_rate_bps, upper)
}

/// Set optional global rate-change caps (admin only).
pub fn set_rate_change_limits(env: Env, max_rate_change_bps: u32, rate_change_min_interval: u64) {
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

/// Set a per-borrower interest rate ceiling (admin only).
///
/// # Panics
/// - If caller is not admin.
/// - If `ceiling_bps` exceeds `MAX_INTEREST_RATE_BPS` (10_000).
/// - If `ceiling_bps` is less than the configured floor for this borrower.
pub fn set_borrower_rate_ceiling(env: Env, borrower: Address, ceiling_bps: Option<u32>) {
    require_admin_auth(&env);
    if let Some(ceiling) = ceiling_bps {
        assert!(ceiling <= MAX_INTEREST_RATE_BPS, "ceiling exceeds max rate");
        // Reject ceiling < floor at config-set time
        if let Some(floor) = crate::storage::get_borrower_rate_floor(&env, &borrower) {
            if ceiling < floor {
                env.panic_with_error(ContractError::RateTooHigh);
            }
        }
    }
    crate::storage::set_borrower_rate_ceiling(&env, &borrower, ceiling_bps);
}

/// Set the penalty surcharge in basis points for delinquent lines (admin only).
///
/// # Arguments
/// * `env` - The Soroban environment.
/// * `bps` - The penalty surcharge in basis points (0..=MAX_INTEREST_RATE_BPS).
///
/// # Panics
/// * If caller is not admin.
/// * If protocol is paused.
/// * If bps exceeds MAX_INTEREST_RATE_BPS (10_000 = 100%).
pub fn set_penalty_surcharge_bps(env: Env, bps: u32) {
    assert_not_paused(&env);
    require_admin_auth(&env);
    assert!(
        bps <= MAX_INTEREST_RATE_BPS,
        "penalty surcharge exceeds max rate"
    );
    crate::storage::set_penalty_surcharge_bps(&env, bps);
}

/// Get the configured penalty surcharge in basis points.
///
/// Returns 0 if not configured (no penalty surcharge).
///
/// # Arguments
/// * `env` - The Soroban environment.
///
/// # Returns
/// The penalty surcharge in basis points.
pub fn get_penalty_surcharge_bps(env: Env) -> u32 {
    crate::storage::get_penalty_surcharge_bps(&env)
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

    // Verify VRF commitment if score is changing
    if risk_score != credit_line.risk_score {
        if let Some(_commitment) = crate::scoring::get_vrf_commitment(&env, &borrower) {
            // VRF commitment exists - verify the score matches
            if !crate::scoring::verify_vrf_commitment(&env, &borrower, risk_score) {
                env.panic_with_error(ContractError::Unauthorized);
            }
        }
        // If no commitment exists, allow the update for backward compatibility
        // (existing credit lines without VRF commitments)
    }

    // Validate credit limit is within configured bounds
    crate::lifecycle::validate_credit_limit_bounds(&env, credit_limit);

    let effective_rate = if let Some(formula_cfg) = get_rate_formula_config(env.clone()) {
        compute_rate_from_score(&formula_cfg, risk_score)
    } else {
        interest_rate_bps
    };

    // Apply per-borrower rate floor if configured
    let mut final_rate =
        if let Some(floor) = crate::storage::get_borrower_rate_floor(&env, &borrower) {
            effective_rate.max(floor)
        } else {
            effective_rate
        };

    // Apply per-borrower rate ceiling if configured
    if let Some(ceiling) = crate::storage::get_borrower_rate_ceiling(&env, &borrower) {
        final_rate = final_rate.min(ceiling);
    }

    // Rate-change guardrails (if configured)
    if let Some(cfg) = get_rate_change_limits(env.clone()) {
        if final_rate != credit_line.interest_rate_bps {
            let delta = final_rate.abs_diff(credit_line.interest_rate_bps);
            if delta > cfg.max_rate_change_bps {
                env.panic_with_error(ContractError::RateTooHigh);
            }

            if cfg.rate_change_min_interval > 0 && credit_line.last_rate_update_ts > 0 {
                let elapsed = env
                    .ledger()
                    .timestamp()
                    .saturating_sub(credit_line.last_rate_update_ts);
                if elapsed < cfg.rate_change_min_interval {
                    env.panic_with_error(ContractError::TimestampRegression);
                }
            }

            credit_line.last_rate_update_ts = env.ledger().timestamp();
        }
    }

    // Enforce global max rate
    if final_rate > MAX_INTEREST_RATE_BPS {
        env.panic_with_error(ContractError::RateTooHigh);
    }

    credit_line.interest_rate_bps = final_rate;
    credit_line.risk_score = risk_score;

    let previous_status = credit_line.status;
    // Handle limit decrease: transition to Restricted if utilization exceeds new limit
    if credit_line.utilized_amount > credit_limit {
        credit_line.status = CreditStatus::Restricted;
    }

    credit_line.credit_limit = credit_limit;

    persist_credit_line(
        &env,
        &borrower,
        &credit_line,
        previous_utilized,
        Some(previous_status),
    );

    publish_risk_parameters_updated(
        &env,
        &borrower,
        credit_line.credit_limit,
        credit_line.interest_rate_bps,
        credit_line.risk_score,
    );
}

/// Get the configured rate-change limits, if any.
pub fn get_rate_change_limits(env: Env) -> Option<RateChangeConfig> {
    env.storage().instance().get(&rate_cfg_key(&env))
}

/// Get the configured rate formula, if any.
pub fn get_rate_formula_config(env: Env) -> Option<RateFormulaConfig> {
    env.storage().instance().get(&rate_formula_key(&env))
}
