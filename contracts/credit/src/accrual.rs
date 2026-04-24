// SPDX-License-Identifier: MIT

use soroban_sdk::Env;
use crate::types::{CreditLineData, CreditStatus, GracePeriodConfig, GraceWaiverMode};
use crate::events::{publish_interest_accrued_event, InterestAccruedEvent};
use crate::storage::grace_period_key;

/// Seconds in a non-leap year (365 days).
const SECONDS_PER_YEAR: u64 = 31_536_000;

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

    // Initialization: if this is the first touch, establish the checkpoint and return.
    if line.last_accrual_ts == 0 {
        line.last_accrual_ts = now;
        return line;
    }

    // No time elapsed: nothing to do.
    if now <= line.last_accrual_ts {
        return line;
    }

    let elapsed = now.saturating_sub(line.last_accrual_ts);

    // If there is no debt, we just update the timestamp.
    if line.utilized_amount == 0 {
        line.last_accrual_ts = now;
        return line;
    }

    // Total denominator = 10,000 (bps conversion) * 31,536,000 (seconds per year)
    let denominator: i128 = 10_000 * (SECONDS_PER_YEAR as i128);

    // Compute accrued interest, splitting the elapsed window if a grace period
    // boundary falls within it.
    let accrued = compute_accrued_with_grace(env, &line, now, elapsed, denominator);

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
                total_accrued_interest: line.accrued_interest,
                new_utilized_amount: line.utilized_amount,
                timestamp: now,
            },
        );
    }

    line.last_accrual_ts = now;
    line
}

/// Compute the total interest accrued over `elapsed` seconds, respecting any
/// grace period boundary that falls within the window.
///
/// If the line is Suspended and a grace period is active, the elapsed window
/// may be split into:
/// 1. An in-grace portion (from `last_accrual_ts` to `grace_end`) at the waiver rate.
/// 2. A post-grace portion (from `grace_end` to `now`) at the full rate.
///
/// If no grace period applies, the full rate is used for the entire `elapsed`.
fn compute_accrued_with_grace(
    env: &Env,
    line: &CreditLineData,
    now: u64,
    elapsed: u64,
    denominator: i128,
) -> i128 {
    let utilized = line.utilized_amount;
    let last_ts = line.last_accrual_ts;

    // Check whether a grace period split is needed.
    if line.status == CreditStatus::Suspended && line.suspension_ts > 0 {
        if let Some(cfg) = env
            .storage()
            .instance()
            .get::<_, GracePeriodConfig>(&grace_period_key(env))
        {
            if cfg.grace_period_seconds > 0 {
                let grace_end = line.suspension_ts.saturating_add(cfg.grace_period_seconds);

                // Case 1: entire window is inside the grace period.
                if now <= grace_end {
                    let waiver_rate = match cfg.waiver_mode {
                        GraceWaiverMode::FullWaiver => 0i128,
                        GraceWaiverMode::ReducedRate => cfg.reduced_rate_bps as i128,
                    };
                    return accrual_amount(utilized, waiver_rate, elapsed as i128, denominator);
                }

                // Case 2: window straddles the grace boundary.
                if last_ts < grace_end {
                    let in_grace_secs = grace_end.saturating_sub(last_ts) as i128;
                    let post_grace_secs = now.saturating_sub(grace_end) as i128;

                    let waiver_rate = match cfg.waiver_mode {
                        GraceWaiverMode::FullWaiver => 0i128,
                        GraceWaiverMode::ReducedRate => cfg.reduced_rate_bps as i128,
                    };
                    let full_rate = line.interest_rate_bps as i128;

                    let in_grace_accrued =
                        accrual_amount(utilized, waiver_rate, in_grace_secs, denominator);
                    let post_grace_accrued =
                        accrual_amount(utilized, full_rate, post_grace_secs, denominator);

                    return in_grace_accrued.saturating_add(post_grace_accrued);
                }

                // Case 3: entire window is after the grace period — fall through to normal.
            }
        }
    }

    // Normal accrual: full rate for the entire elapsed window.
    let rate = line.interest_rate_bps as i128;
    accrual_amount(utilized, rate, elapsed as i128, denominator)
}

/// Compute `floor(utilized * rate_bps * seconds / denominator)`.
///
/// Returns 0 if `rate_bps` is 0 or the intermediate product overflows.
fn accrual_amount(utilized: i128, rate_bps: i128, seconds: i128, denominator: i128) -> i128 {
    if rate_bps == 0 || seconds == 0 {
        return 0;
    }
    match utilized.checked_mul(rate_bps).and_then(|v| v.checked_mul(seconds)) {
        Some(val) => val / denominator,
        None => panic!("interest calculation overflow"),
    }
}
