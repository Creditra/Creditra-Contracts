// SPDX-License-Identifier: MIT

//! Risk management module: rate formulas, rate change limits, and risk parameter updates.
//!
//! # Storage
//! Rate configuration is stored in **instance storage** (shared TTL):
//! - `rate_cfg`: Rate change limits (`RateChangeConfig`)
//! - `rate_form`: Rate formula configuration (`RateFormulaConfig`)
//!
//! These are global singleton values — one config per contract deployment.

use crate::auth::require_admin_auth;
use crate::events::{publish_risk_parameters_updated, RiskParametersUpdatedEvent};
use crate::storage::{rate_cfg_key, rate_formula_key};
use crate::types::{CreditLineData, CreditStatus, RateChangeConfig, RateFormulaConfig};
use soroban_sdk::{Address, Env};

/// Maximum interest rate in basis points (100%).
pub const MAX_INTEREST_RATE_BPS: u32 = 10_000;

/// Maximum risk score (0–100 scale).
pub const MAX_RISK_SCORE: u32 = 100;

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
pub fn set_rate_change_limits(env: Env, max_rate_change_bps: u32, rate_change_min_interval: u64) {
    assert_not_paused(&env);
    require_admin_auth(&env);
    let cfg = RateChangeConfig {
        max_rate_change_bps,
        rate_change_min_interval,
    };
    env.storage().instance().set(&rate_cfg_key(&env), &cfg);
}

/// Update risk parameters for an existing credit line (admin only).
///
/// This function handles updating the credit limit, risk score, and interest rate.
/// If a dynamic rate formula is configured, the `interest_rate_bps` parameter is
/// ignored and the rate is re-calculated based on the provided `risk_score`.
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
pub fn update_risk_parameters(
    env: Env,
    borrower: Address,
    credit_limit: i128,
    interest_rate_bps: u32,
    risk_score: u32,
) {
    assert_not_paused(&env);
    require_admin_auth(&env);

    let mut credit_line: CreditLineData = env
        .storage()
        .persistent()
        .get(&borrower)
        .unwrap_or_else(|| env.panic_with_error(ContractError::CreditLineNotFound));

    // Apply interest accrual before any mutation
    credit_line = crate::accrual::apply_accrual(&env, credit_line);

    if credit_limit < 0 {
        env.panic_with_error(ContractError::NegativeLimit);
    }
    if risk_score > MAX_RISK_SCORE {
        env.panic_with_error(ContractError::ScoreTooHigh);
    }

    // Determine the effective interest rate:
    // - If a rate formula config is stored, compute from risk_score (ignore passed rate).
    // - Otherwise, use the manually supplied interest_rate_bps (existing behavior).
    let effective_rate = if let Some(formula_cfg) = env
        .storage()
        .instance()
        .get::<_, RateFormulaConfig>(&rate_formula_key(&env))
    {
        compute_rate_from_score(&formula_cfg, risk_score)
    } else {
        interest_rate_bps
    };

    if effective_rate > MAX_INTEREST_RATE_BPS {
        env.panic_with_error(ContractError::RateTooHigh);
    }

    if effective_rate != credit_line.interest_rate_bps {
        if let Some(cfg) = env
            .storage()
            .instance()
            .get::<_, RateChangeConfig>(&rate_cfg_key(&env))
        {
            let old_rate = credit_line.interest_rate_bps;
            let delta = effective_rate.abs_diff(old_rate);

            if delta > cfg.max_rate_change_bps {
                env.panic_with_error(ContractError::RateTooHigh);
            }

            if cfg.rate_change_min_interval > 0 && credit_line.last_rate_update_ts != 0 {
                let now = env.ledger().timestamp();
                let elapsed = now.saturating_sub(credit_line.last_rate_update_ts);
                if elapsed < cfg.rate_change_min_interval {
                    env.panic_with_error(ContractError::RateTooHigh); // Or a specific RateChangeTooFrequent
                }
            }
        }

        let new_ts = env.ledger().timestamp();
        assert_ts_monotonic(&env, credit_line.last_rate_update_ts, new_ts);
        credit_line.last_rate_update_ts = new_ts;
    }

    // Handle limit decrease relative to utilization.
    // If new limit < utilized amount, transition to Restricted status.
    // This prevents new draws but allows repayments.
    if credit_limit < credit_line.utilized_amount {
        credit_line.status = CreditStatus::Restricted;
    } else if credit_line.status == CreditStatus::Restricted && credit_limit >= credit_line.utilized_amount {
        // Auto-cure: if previously Restricted and limit is now at or above utilization, return to Active.
        credit_line.status = CreditStatus::Active;
    }
    // Note: if status is Suspended, Defaulted, or Closed, we don't force a transition.
    // The admin must explicitly change status using dedicated methods.

    credit_line.credit_limit = credit_limit;
    credit_line.interest_rate_bps = effective_rate;
    env.storage().persistent().set(&borrower, &credit_line);

    publish_risk_parameters_updated(&env, &borrower, credit_limit, effective_rate, risk_score);
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
