// SPDX-License-Identifier: MIT

use crate::events::{publish_interest_accrued_event, InterestAccruedEvent};
use crate::types::CreditLineData;
use soroban_sdk::{Address, Env};

pub(crate) const SECONDS_PER_YEAR: u64 = 31_536_000;

fn compute_interest(utilized: i128, rate_bps: i128, seconds: i128) -> i128 {
    let denominator: i128 = 10_000 * (SECONDS_PER_YEAR as i128);
    let intermediate = utilized
        .checked_mul(rate_bps)
        .and_then(|v| v.checked_mul(seconds));
    match intermediate {
        Some(val) => val / denominator,
        None => panic!("interest calculation overflow"),
    }
}

/// Apply interest accrual to a credit line and return the updated line.
///
/// Reads the optional [`GracePeriodConfig`] from instance storage to determine
/// the effective rate for Suspended lines within their grace window.
///
/// # Grace period interaction
/// - If the line is Suspended and a grace period policy is configured, the
///   effective rate is reduced (or zeroed) for the portion of `elapsed` that
///   falls within the grace window.
/// - If the grace window expires mid-period, the elapsed time is split: the
///   in-window portion uses the waiver rate and the post-window portion uses
///   the full rate.
/// - If no policy is configured, or the line is not Suspended, normal accrual
///   applies unchanged.
pub fn apply_accrual(env: &Env, mut line: CreditLineData) -> CreditLineData {
    let now = env.ledger().timestamp();

    if now <= line.last_accrual_ts {
        return line;
    }

    if line.utilized_amount == 0 {
        line.last_accrual_ts = now;
        return line;
    }

    let utilized = line.utilized_amount;
    let full_rate = line.interest_rate_bps as i128;
    let accrual_start = line.last_accrual_ts;

    let accrued = if line.status == CreditStatus::Suspended {
        let grace_cfg: Option<GracePeriodConfig> = env
            .storage()
            .instance()
            .get(&crate::storage::grace_period_key(env));

        match grace_cfg {
            Some(cfg) if cfg.grace_period_seconds > 0 => {
                let grace_end = line.suspension_ts.saturating_add(cfg.grace_period_seconds);

                if now <= grace_end {
                    // Entire period is within the grace window
                    let seconds = (now - accrual_start) as i128;
                    match cfg.waiver_mode {
                        GraceWaiverMode::FullWaiver => 0,
                        GraceWaiverMode::ReducedRate => {
                            compute_interest(utilized, cfg.reduced_rate_bps as i128, seconds)
                        }
                    }
                } else if accrual_start >= grace_end {
                    // Entire period is after grace window
                    let seconds = (now - accrual_start) as i128;
                    compute_interest(utilized, full_rate, seconds)
                } else {
                    // Period straddles the grace boundary
                    let in_window_secs = (grace_end - accrual_start) as i128;
                    let post_window_secs = (now - grace_end) as i128;

                    let in_window_interest = match cfg.waiver_mode {
                        GraceWaiverMode::FullWaiver => 0,
                        GraceWaiverMode::ReducedRate => {
                            compute_interest(utilized, cfg.reduced_rate_bps as i128, in_window_secs)
                        }
                    };
                    let post_window_interest =
                        compute_interest(utilized, full_rate, post_window_secs);
                    in_window_interest + post_window_interest
                }
            }
            _ => {
                let seconds = (now - accrual_start) as i128;
                compute_interest(utilized, full_rate, seconds)
            }
        }
    } else {
        let seconds = (now - accrual_start) as i128;
        compute_interest(utilized, full_rate, seconds)
    };

    if accrued > 0 {
        line.utilized_amount = line
            .utilized_amount
            .checked_add(accrued)
            .expect("utilized_amount overflow");
        line.accrued_interest = line
            .accrued_interest
            .checked_add(accrued)
            .expect("accrued_interest overflow");

        publish_interest_accrued_event(
            env,
            InterestAccruedEvent {
                borrower: line.borrower.clone(),
                accrued_amount: accrued,
                new_utilized_amount: line.utilized_amount,
            },
        );
    }

    line.last_accrual_ts = now;
    line
}
