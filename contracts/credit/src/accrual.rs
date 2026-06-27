// SPDX-License-Identifier: MIT

//! Interest accrual logic for credit lines.
//!
//! # What
//!
//! Owns [`apply_accrual`], the chokepoint that every state-mutating
//! entrypoint calls at the head of its flow. Computes pro-rated interest
//! since `last_accrual_ts`, capitalizes it into both `accrued_interest`
//! and `utilized_amount`, and conditionally emits
//! [`InterestAccruedEvent`], [`PenaltyRateEnteredEvent`], and
//! [`PenaltyRateExitedEvent`].
//!
//! # How (three branches)
//!
//! The effective rate `r_eff` depends on line state and delinquency:
//!
//! 1. **Active, current**: `r_eff = interest_rate_bps`.
//! 2. **Active, delinquent** (past `next_due_ts + grace`): `r_eff =
//!    min(interest_rate_bps + penalty_surcharge_bps, 10_000)`. First
//!    delinquent accrual emits [`PenaltyRateEnteredEvent`]; first
//!    non-delinquent accrual after a delinquency period emits
//!    [`PenaltyRateExitedEvent`].
//! 3. **Suspended with grace policy**: Δt is split into in-grace
//!    `min(Δt, T_g)` and post-grace remainder. In FullWaiver mode the
//!    in-grace portion is waived; in ReducedRate mode it accrues at
//!    `reduced_rate_bps`.
//!
//! The math primitive is [`crate::math_utils::prorate_interest`] with
//! [`crate::math_utils::Rounding::Floor`], so every `ΔI` rounds **down**.
//! The denominator uses [`crate::math_utils::SECONDS_PER_YEAR`] = 31 557 600
//! (Julian year).
//!
//! # Invariants
//!
//! - If `utilized_amount == 0` or `now <= last_accrual_ts`: no-op, and
//!   crucially `last_accrual_ts` is NOT advanced (avoids silently zeroing
//!   sub-tick deltas on chains with sub-second ledger close times).
//! - `last_accrual_ts` is advanced only when `ΔI > 0`.
//! - The fold uses `checked_add` on both the new utilized amount and the
//!   new accrued-interest total; overflow translates to
//!   `ContractError::Overflow = 12`.
//!
//! # Why (capitalize-on-mutation)
//!
//! Periodic accrual via a keeper or per-block hook would either bloat
//! storage (a write per block per borrower) or require unbounded loops at
//! settlement. The capitalize-on-mutation model is O(1) per call, has no
//! liveness assumption, and is auditable in isolation — see
//! [`docs/interest-accrual.md`](../../../docs/interest-accrual.md) for the
//! normative reference and
//! [`docs/RISK_PRICING.md`](../../../docs/RISK_PRICING.md) §4 for the
//! formal derivation with worked examples.

#![warn(missing_docs)]

use crate::events::{
    publish_grace_waiver_applied_event, publish_interest_accrued_event,
    publish_penalty_rate_entered_event, publish_penalty_rate_exited_event, InterestAccruedEvent,
};
use crate::math_utils::{prorate_interest, Rounding};
use crate::storage::get_credit_line;
use crate::storage::persist_credit_line;
use crate::types::{
    ContractError, CreditLineData, CreditStatus, GracePeriodConfig, GraceWaiverMode,
};
use soroban_sdk::{Address, Env, Vec};

/// Compute and apply accrued interest to a credit line for the elapsed period.
///
/// Calculates the interest owed since `credit_line.last_accrual_ts` using
/// [`prorate_interest`], adds it to `credit_line.accrued_interest`, and
/// updates `credit_line.last_accrual_ts` to `now`.
///
/// # How interest is computed
/// ```text
/// elapsed  = now - last_accrual_ts          (seconds)
/// interest = principal * rate_bps * elapsed
///            ────────────────────────────────
///                  10_000 * 31_536_000
/// ```
/// where `principal` is `credit_line.utilized_amount` and `rate_bps` is
/// `credit_line.interest_rate_bps`.
///
/// # Rounding
/// Truncates toward zero via [`prorate_interest`]. Sub-unit interest amounts
/// accrue as `0` for that period and are not carried forward.
///
/// # Parameters
/// - `env`:         The Soroban environment; used to read the current ledger
///                  timestamp via `env.ledger().timestamp()`.
/// - `credit_line`: Mutable reference to the credit line to update. Both
///                  `accrued_interest` and `last_accrual_ts` are modified
///                  in-place. The caller is responsible for persisting the
///                  updated record to storage.
///
/// # Returns
/// The amount of interest accrued in this call (may be `0` if `elapsed == 0`,
/// `utilized_amount == 0`, or the computed amount truncates to zero).
///
/// # Panics
/// - If `principal * rate_bps * elapsed` overflows `i128`.
/// - If adding interest to `credit_line.accrued_interest` overflows `i128`.
///
/// # Example
/// ```text
/// // Credit line: 1_000_000 utilized at 500 bps (5% p.a.)
/// // last_accrual_ts = 0, now = 86_400 (1 day later)
/// // interest = 1_000_000 * 500 * 86_400 / 315_360_000_000 = 137
/// // After call: accrued_interest += 137, last_accrual_ts = 86_400
/// ```
pub(crate) const SECONDS_PER_YEAR: u64 = 31_536_000;

/// Compute simple interest: `utilized * rate_bps * seconds / (10_000 * SECONDS_PER_YEAR)`.
///
/// # Overflow behavior — **revert with `ContractError::Overflow`**
/// All intermediate multiplications use `checked_mul`. If any step would exceed
/// `i128::MAX` the function returns `Err(ContractError::Overflow)` so the caller
/// can propagate it via `env.panic_with_error`. No silent wrapping or saturation
/// occurs; the contract reverts deterministically.
fn compute_interest(utilized: i128, rate_bps: i128, seconds: i128) -> Result<i128, ContractError> {
    let denominator: i128 = 10_000 * (SECONDS_PER_YEAR as i128);
    let intermediate = utilized
        .checked_mul(rate_bps)
        .and_then(|v| v.checked_mul(seconds));
    match intermediate {
        Some(val) => Ok(val / denominator),
        None => Err(ContractError::Overflow),
    }
}

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

    // Check if the borrower is delinquent to apply penalty surcharge
    let is_delinquent = crate::query::is_delinquent(env.clone(), line.borrower.clone());
    let penalty_surcharge_bps = crate::storage::get_penalty_surcharge_bps(env);

    // Track previous rate to detect penalty rate entry/exit
    let previous_effective_rate = line.interest_rate_bps;

    // Compute the effective interest rate (base rate + penalty surcharge if delinquent)
    let effective_rate_bps = if is_delinquent && penalty_surcharge_bps > 0 {
        let base_rate = line.interest_rate_bps;
        let rate_with_surcharge = base_rate.saturating_add(penalty_surcharge_bps);
        // Clamp to MAX_INTEREST_RATE_BPS to prevent overflow-safe rate caps
        rate_with_surcharge.min(crate::risk::MAX_INTEREST_RATE_BPS)
    } else {
        line.interest_rate_bps
    };

    // Emit event if entering penalty rate (non-delinquent to delinquent with surcharge)
    if is_delinquent && penalty_surcharge_bps > 0 && previous_effective_rate != effective_rate_bps {
        publish_penalty_rate_entered_event(
            env,
            &line.borrower,
            previous_effective_rate,
            penalty_surcharge_bps,
            effective_rate_bps,
        );
    }

    // Emit event if exiting penalty rate (delinquent to non-delinquent or surcharge removed)
    if !is_delinquent && previous_effective_rate > line.interest_rate_bps {
        publish_penalty_rate_exited_event(
            env,
            &line.borrower,
            previous_effective_rate,
            line.interest_rate_bps,
        );
    }

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
                    // Entire period after grace window - use effective rate (may include penalty)
                    prorate_interest(
                        line.utilized_amount as u128,
                        effective_rate_bps,
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

                    // Calculate waived amount for grace waiver event
                    let full_rate_interest = prorate_interest(
                        line.utilized_amount as u128,
                        effective_rate_bps,
                        in_window_secs,
                        Rounding::Floor,
                    ) as i128;

                    let actual_interest = match cfg.waiver_mode {
                        GraceWaiverMode::FullWaiver => 0,
                        GraceWaiverMode::ReducedRate => prorate_interest(
                            line.utilized_amount as u128,
                            cfg.reduced_rate_bps,
                            in_window_secs,
                            Rounding::Floor,
                        ) as i128,
                    };

                    let waived_amount = full_rate_interest.saturating_sub(actual_interest);
                    if waived_amount > 0 {
                        publish_grace_waiver_applied_event(
                            env,
                            &line.borrower,
                            waived_amount,
                            cfg.waiver_mode,
                        );
                    }

                    let post_window = prorate_interest(
                        line.utilized_amount as u128,
                        effective_rate_bps,
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
                effective_rate_bps,
                (now - accrual_start) as u64,
                Rounding::Floor,
            ),
        }
    } else {
        // Active, Defaulted, Restricted, or Closed status: apply effective rate (may include penalty)
        prorate_interest(
            line.utilized_amount as u128,
            effective_rate_bps,
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

        // Only update last_accrual_ts when we actually applied accrual.
        line.last_accrual_ts = now;
    }

    line
}

/// Materialize interest accrual for a bounded list of borrowers.
///
/// No auth is required: the call only updates accounting state for lines
/// that already exist and are `Active`. Missing lines and non-active lines
/// are skipped without reverting the whole batch. Only non-zero accruals
/// emit `InterestAccruedEvent`.
pub fn accrue_batch(env: &Env, borrowers: Vec<Address>) {
    for borrower in borrowers.iter() {
        if let Some(stored_line) = get_credit_line(env, &borrower) {
            if stored_line.status == CreditStatus::Active && stored_line.utilized_amount > 0 {
                let previous_utilized = stored_line.utilized_amount;
                let previous_ts = stored_line.last_accrual_ts;
                let previous_status = stored_line.status;
                let updated = apply_accrual(env, stored_line);
                // Only persist if accrual actually changed the line
                if updated.utilized_amount != previous_utilized
                    || updated.last_accrual_ts != previous_ts
                {
                    persist_credit_line(env, &borrower, &updated, previous_utilized, Some(previous_status));
                }
            }
        }
    }
}
