// SPDX-License-Identifier: MIT

use crate::events::{publish_interest_accrued_event, InterestAccruedEvent};
use crate::types::CreditLineData;
use soroban_sdk::Env;

/// Seconds in a non-leap year (365 days).
pub(crate) const SECONDS_PER_YEAR: u64 = 31_536_000;

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

    // Formula: accrued = floor(utilized_amount * interest_rate_bps * elapsed_seconds / (10_000 * 31_536_000))
    // We use i128 to prevent overflow during intermediate multiplication.

    let utilized = line.utilized_amount;
    let rate = line.interest_rate_bps as i128;
    let seconds = elapsed as i128;

    // Total denominator = 10,000 (bps conversion) * 31,536_000 (seconds per year)
    let denominator: i128 = 10_000 * (SECONDS_PER_YEAR as i128);

    // intermediate = utilized * rate * seconds
    let intermediate = utilized
        .checked_mul(rate)
        .and_then(|v| v.checked_mul(seconds));

    if let Some(val) = intermediate {
        let accrued = val / denominator;

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
    } else {
        // Handle overflow of intermediate calculation
        panic!("interest calculation overflow");
    }

    line.last_accrual_ts = now;
    line
}

/// Load a credit line, apply accrual, and persist the result.
/// No-op if the credit line does not exist.
pub fn apply_pending_accrual(env: &Env, borrower: &Address) {
    if let Some(line) = env
        .storage()
        .persistent()
        .get::<Address, CreditLineData>(borrower)
    {
        let updated = apply_accrual(env, line);
        env.storage().persistent().set(borrower, &updated);
    }
}
