// SPDX-License-Identifier: MIT

//! Interest accrual logic for credit lines.
//!
//! This module computes and applies pro-rated interest to a [`CreditLineData`]
//! record. Interest is computed via the audited [`math_utils::prorate_interest`]
//! helper with explicit `Rounding` and is capitalised into `accrued_interest`.

#![warn(missing_docs)]

use crate::events::{publish_interest_accrued_event, InterestAccruedEvent};
use crate::storage::persist_credit_line;
use crate::types::{ContractError, CreditLineData, CreditStatus, GracePeriodConfig, GraceWaiverMode};
use crate::math_utils::{prorate_interest, Rounding};
use soroban_sdk::{Address, Env, Vec};

/// Apply interest accrual to a credit line and return the updated line.
///
/// This implementation routes all prorating math through `math_utils::prorate_interest`,
/// with explicit `Rounding::Floor`. `last_accrual_ts` is only updated when a
/// non-zero accrual has been successfully computed and applied. No rounding-up
/// is performed by default.

pub fn apply_accrual(env: &Env, mut line: CreditLineData) -> CreditLineData {
    let now = env.ledger().timestamp();

    // Do nothing if ledger time has not advanced.
    if now <= line.last_accrual_ts {
        return line;
    }

    // If there's no utilization, this is a read-only check — do not update
    // `last_accrual_ts` here per requirements.
    if line.utilized_amount == 0 {
        return line;
    }

    let accrual_start = line.last_accrual_ts;

    // Helper to convert u128 interest result back to i128 with overflow check.
    let u128_to_i128 = |v: u128| -> i128 {
        if v > (i128::MAX as u128) {
            env.panic_with_error(ContractError::Overflow);
        }
        v as i128
    };

    // Compute accrued interest using the audited prorate helper with floor rounding.
    let accrued_u: u128 = if line.status == CreditStatus::Suspended {
        let grace_cfg: Option<GracePeriodConfig> = env
            .storage()
            .instance()
            .get(&crate::storage::grace_period_key(env));

        match grace_cfg {
            Some(cfg) if cfg.grace_period_seconds > 0 => {
                let grace_end = line.suspension_ts.saturating_add(cfg.grace_period_seconds);

                if now <= grace_end {
                    // Entire period in grace window
                    match cfg.waiver_mode {
                        GraceWaiverMode::FullWaiver => 0u128,
                        GraceWaiverMode::ReducedRate => prorate_interest(
                            line.utilized_amount as u128,
                            cfg.reduced_rate_bps,
                            (now - accrual_start) as u64,
                            Rounding::Floor,
                        ),
                    }
                } else if accrual_start >= grace_end {
                    // Entire period after grace window
                    prorate_interest(
                        line.utilized_amount as u128,
                        line.interest_rate_bps,
                        (now - accrual_start) as u64,
                        Rounding::Floor,
                    )
                } else {
                    // Straddles grace boundary — prorate two sub-periods and add.
                    let in_window_secs = (grace_end - accrual_start) as u64;
                    let post_window_secs = (now - grace_end) as u64;

                    let in_window = match cfg.waiver_mode {
                        GraceWaiverMode::FullWaiver => 0u128,
                        GraceWaiverMode::ReducedRate => prorate_interest(
                            line.utilized_amount as u128,
                            cfg.reduced_rate_bps,
                            in_window_secs,
                            Rounding::Floor,
                        ),
                    };
                    let post_window = prorate_interest(
                        line.utilized_amount as u128,
                        line.interest_rate_bps,
                        post_window_secs,
                        Rounding::Floor,
                    );
                    in_window
                        .checked_add(post_window)
                        .unwrap_or_else(|| env.panic_with_error(ContractError::Overflow))
                }
            }
            _ => prorate_interest(
                line.utilized_amount as u128,
                line.interest_rate_bps,
                (now - accrual_start) as u64,
                Rounding::Floor,
            ),
        }
    } else {
        prorate_interest(
            line.utilized_amount as u128,
            line.interest_rate_bps,
            (now - accrual_start) as u64,
            Rounding::Floor,
        )
    };

    let accrued_i: i128 = u128_to_i128(accrued_u);

    if accrued_i > 0 {
        // Apply accrual to utilized and accrued_interest, revert on overflow.
        line.utilized_amount = line
            .utilized_amount
            .checked_add(accrued_i)
            .unwrap_or_else(|| env.panic_with_error(ContractError::Overflow));

        line.accrued_interest = line
            .accrued_interest
            .checked_add(accrued_i)
            .unwrap_or_else(|| env.panic_with_error(ContractError::Overflow));

        publish_interest_accrued_event(
            env,
            InterestAccruedEvent {
                borrower: line.borrower.clone(),
                accrued_amount: accrued_i,
                new_utilized_amount: line.utilized_amount,
            },
        );

        // Only update last_accrual_ts after successful, non-zero accrual.
        line.last_accrual_ts = now;
    }

    line
}

/// Materialize interest accrual for a caller-supplied batch of borrowers.
///
/// Only `Active` credit lines are processed. Missing lines and non-active lines
/// are skipped without reverting the batch. Non-zero accruals are persisted and
/// emit the standard `InterestAccruedEvent` through [`apply_accrual`].
pub fn accrue_batch(env: &Env, borrowers: Vec<Address>) {
    for borrower in borrowers.iter() {
        let Some(stored_line) = env.storage().persistent().get::<Address, CreditLineData>(&borrower) else {
            continue;
        };

        if stored_line.status != CreditStatus::Active {
            continue;
        }

        let previous_utilized = stored_line.utilized_amount;
        let updated_line = apply_accrual(env, stored_line);

        if updated_line != stored_line {
            persist_credit_line(env, &borrower, &updated_line, previous_utilized);
        }
    }
}
