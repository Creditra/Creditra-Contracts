use crate::auth::require_admin_auth;
use crate::events::{publish_risk_parameters_updated, RiskParametersUpdatedEvent};
use crate::storage::rate_cfg_key;
use crate::types::{ContractError, CreditLineData, RateChangeConfig};
use soroban_sdk::{Address, Env};

/// Maximum interest rate in basis points (100%).
pub const MAX_INTEREST_RATE_BPS: u32 = 10_000;

/// Maximum risk score (0–100 scale).
pub const MAX_RISK_SCORE: u32 = 100;

pub fn update_risk_parameters(
    env: Env,
    borrower: Address,
    credit_limit: i128,
    interest_rate_bps: u32,
    risk_score: u32,
) {
    require_admin_auth(&env);

    if credit_limit < 0 {
        env.panic_with_error(ContractError::NegativeLimit);
    }
    if interest_rate_bps > MAX_INTEREST_RATE_BPS {
        env.panic_with_error(ContractError::RateTooHigh);
    }
    if risk_score > MAX_RISK_SCORE {
        env.panic_with_error(ContractError::ScoreTooHigh);
    }

    let mut credit_line: CreditLineData = env
        .storage()
        .persistent()
        .get(&borrower)
        .expect("Credit line not found");

    if credit_limit < credit_line.utilized_amount {
        env.panic_with_error(crate::types::ContractError::LimitDecreaseRequiresRepayment);
    }

    if interest_rate_bps != credit_line.interest_rate_bps {
        if let Some(cfg) = env
            .storage()
            .instance()
            .get::<_, RateChangeConfig>(&rate_cfg_key(&env))
        {
            let old_rate = credit_line.interest_rate_bps;
            let delta = interest_rate_bps.abs_diff(old_rate);

            if delta > cfg.max_rate_change_bps {
                env.panic_with_error(crate::types::ContractError::RateTooHigh);
            }

            if cfg.rate_change_min_interval > 0 && credit_line.last_rate_update_ts != 0 {
                let now = env.ledger().timestamp();
                let elapsed = now.saturating_sub(credit_line.last_rate_update_ts);
                if elapsed < cfg.rate_change_min_interval {
                    env.panic_with_error(crate::types::ContractError::InvalidStatus);
                }
            }
        }

        credit_line.last_rate_update_ts = env.ledger().timestamp();
    }

    credit_line.credit_limit = credit_limit;
    credit_line.interest_rate_bps = interest_rate_bps;
    credit_line.risk_score = risk_score;
    env.storage().persistent().set(&borrower, &credit_line);

    publish_risk_parameters_updated(
        &env,
        RiskParametersUpdatedEvent {
            borrower: borrower.clone(),
            credit_limit,
            interest_rate_bps,
            risk_score,
        },
    );
}

/// Set rate-change limits (admin only).
///
/// Configures the maximum allowed interest-rate change per call and the
/// minimum time interval between consecutive rate changes.
pub fn set_rate_change_limits(env: Env, max_rate_change_bps: u32, rate_change_min_interval: u64) {
    require_admin_auth(&env);
    let cfg = RateChangeConfig {
        max_rate_change_bps,
        rate_change_min_interval,
    };
    env.storage().instance().set(&rate_cfg_key(&env), &cfg);
}

/// Get the current rate-change limit configuration (view function).
pub fn get_rate_change_limits(env: Env) -> Option<RateChangeConfig> {
    env.storage().instance().get(&rate_cfg_key(&env))
}
