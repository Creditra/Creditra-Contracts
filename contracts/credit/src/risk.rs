use crate::auth::require_admin_auth;
use crate::events::{publish_risk_parameters_updated, RiskParametersUpdatedEvent};
use crate::storage::{rate_cfg_key, rate_formula_key};
use crate::types::{CreditLineData, RateChangeConfig, RateFormulaConfig};
use soroban_sdk::{Address, Env};

/// Maximum interest rate in basis points (100%).
pub const MAX_INTEREST_RATE_BPS: u32 = 10_000;

/// Maximum risk score (0–100 scale).
pub const MAX_RISK_SCORE: u32 = 100;

/// Retrieve the rate formula config from instance storage, if set.
pub fn get_rate_formula_config(env: Env) -> Option<RateFormulaConfig> {
    env.storage()
        .instance()
        .get::<_, RateFormulaConfig>(&rate_formula_key(&env))
}

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

/// Update risk parameters for an existing credit line (admin only).
///
/// This function handles updating the credit limit, risk score, and interest rate.
/// If a dynamic rate formula is configured, the `interest_rate_bps` parameter is
/// ignored and the rate is re-calculated based on the provided `risk_score`.
///
/// # Arguments
/// * `env` - The Soroban environment.
/// * `borrower` - The address of the borrower.
/// * `credit_limit` - The new credit limit (must be >= 0 and >= current utilization).
/// * `interest_rate_bps` - The manual interest rate (ignored if formula is enabled).
/// * `risk_score` - The new risk score (0-100).
///
/// # Panics
/// * If caller is not admin.
/// * If credit line does not exist.
/// * If validation fails (limit < utilization, score > 100, etc.).
/// * If rate change exceeds configured limits.
pub fn update_risk_parameters(
    env: Env,
    borrower: Address,
    credit_limit: i128,
    interest_rate_bps: u32,
    risk_score: u32,
) {
    require_admin_auth(&env);

    let mut credit_line: CreditLineData = env
        .storage()
        .persistent()
        .get(&borrower)
        .expect("Credit line not found");

    // Apply interest accrual before any mutation
    credit_line = crate::accrual::apply_accrual(&env, credit_line);

    if credit_limit < 0 {
        panic!("credit_limit must be non-negative");
    }
    if credit_limit < credit_line.utilized_amount {
        panic!("credit_limit cannot be less than utilized amount");
    }
    if risk_score > MAX_RISK_SCORE {
        panic!("risk_score exceeds maximum");
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
        panic!("interest_rate_bps exceeds maximum");
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
                panic!("rate change exceeds maximum allowed delta");
            }

            if cfg.rate_change_min_interval > 0 && credit_line.last_rate_update_ts != 0 {
                let now = env.ledger().timestamp();
                let elapsed = now.saturating_sub(credit_line.last_rate_update_ts);
                if elapsed < cfg.rate_change_min_interval {
                    panic!("rate change too soon: minimum interval not elapsed");
                }
            }
        }

        credit_line.last_rate_update_ts = env.ledger().timestamp();
    }

    credit_line.credit_limit = credit_limit;
    credit_line.interest_rate_bps = effective_rate;
    credit_line.risk_score = risk_score;
    env.storage().persistent().set(&borrower, &credit_line);

    publish_risk_parameters_updated(
        &env,
        RiskParametersUpdatedEvent {
            borrower: borrower.clone(),
            credit_limit,
            interest_rate_bps: effective_rate,
            risk_score,
        },
    );
}

/// Retrieve the rate formula configuration from instance storage, if set.
pub fn get_rate_formula_config(env: Env) -> Option<RateFormulaConfig> {
    env.storage()
        .instance()
        .get::<_, RateFormulaConfig>(&rate_formula_key(&env))
}
