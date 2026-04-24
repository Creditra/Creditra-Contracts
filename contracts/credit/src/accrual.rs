// SPDX-License-Identifier: MIT

use soroban_sdk::{Env};
use crate::types::{CreditLineData};
use crate::events::{publish_interest_accrued_event, InterestAccruedEvent};

/// Seconds in a non-leap year (365 days).
const SECONDS_PER_YEAR: u64 = 31_536_000;

/// Internal function to apply interest accrual to a credit line.
///
/// This function calculates the interest accrued since the last checkpoint,
/// updates the utilized amount and accrued interest, and emits an event
/// if interest was materialized.
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
            line.utilized_amount = line.utilized_amount.checked_add(accrued).expect("utilized_amount overflow");
            line.accrued_interest = line.accrued_interest.checked_add(accrued).expect("accrued_interest overflow");
            
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
