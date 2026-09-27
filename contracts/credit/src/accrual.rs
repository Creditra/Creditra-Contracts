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
use crate::math_utils::{checked_prorate_interest, Rounding};
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

/// Apply interest accrual to a credit line and return the updated line record.
///
/// # Overview
///
/// `apply_accrual` is the central interest capitalization chokepoint. It computes pro-rated
/// interest since `line.last_accrual_ts` using [`crate::math_utils::prorate_interest`] with
/// [`Rounding::Floor`], capitalizes non-zero interest into both `line.accrued_interest` and
/// `line.utilized_amount`, and advances `line.last_accrual_ts` to the current ledger timestamp.
///
/// # Parameters
///
/// * `env` — The Soroban environment reference (`&Env`); used to retrieve current ledger timestamp.
/// * `line` — The [`CreditLineData`] record to accrue interest for.
///
/// # Returns
///
/// Returns the updated [`CreditLineData`] struct. If no time has elapsed or utilization is zero,
/// the line is returned unmodified.
///
/// # Interest Calculation & Rate Branches
///
/// 1. **Standard Active**: Effective rate is `line.interest_rate_bps`.
/// 2. **Delinquent Active**: When delinquent (`crate::query::is_delinquent`), penalty surcharge BPS is added
///    (clamped to [`crate::risk::MAX_INTEREST_RATE_BPS`]). Transitions emit [`PenaltyRateEnteredEvent`] or [`PenaltyRateExitedEvent`].
/// 3. **Suspended with Grace Policy**: If status is `Suspended` and a [`GracePeriodConfig`] exists:
///    - In-grace window uses `GraceWaiverMode::FullWaiver` (0 interest) or `GraceWaiverMode::ReducedRate` (`reduced_rate_bps`).
///    - Post-grace window accrues at standard effective rate.
///    - Emits [`GraceWaiverReceiptEvent`] when interest is waived.
///
/// # Mathematical Principles & Invariants
///
/// * **Floor Rounding**: All interest deltas round down (`Rounding::Floor`). Sub-unit fractional interest is not carried forward.
/// * **Julian Year Denominator**: Uses [`SECONDS_PER_YEAR`] = 31,536,000 seconds.
/// * **Timestamp Invariant**: `last_accrual_ts` is advanced **only** when non-zero interest (`accrued_i > 0`) is applied,
///   preventing zero-delta timestamp burn on fast ledgers.
/// * **Zero Utilization**: Returns `line` unmodified without advancing `last_accrual_ts`.
///
/// # Panics & Overflow Safety
///
/// Reverts with [`ContractError::Overflow`] if:
/// * Prorated interest conversion from `u128` exceeds `i128::MAX`.
/// * Capitalizing interest into `utilized_amount` or `accrued_interest` overflows `i128::MAX`.
///
/// # Example
///
/// ```ignore
/// let updated_line = apply_accrual(&env, credit_line);
/// assert!(updated_line.utilized_amount >= original_utilized);
/// ```
pub fn apply_accrual(env: &Env, mut line: CreditLineData) -> CreditLineData {

    let now = env.ledger().timestamp();

    // Do nothing if ledger time has not advanced.
    if now <= line.last_accrual_ts {
        return line;
    }

    // If there's no utilization, we update the checkpoint to prevent retroactive interest accrual
    // but do not compute any interest.
    if line.utilized_amount == 0 {
        line.last_accrual_ts = now;
        return line;
    }

    let accrual_start = line.last_accrual_ts;

    // Bound the interest accrual: compute floor-prorated interest via the
    // overflow-checked primitive and revert deterministically with
    // `ContractError::Overflow` when a rate or timestamp extreme would push
    // the intermediate product past `u128::MAX`. A bare `prorate_interest`
    // panic would be an unhandled string abort rather than an auditable
    // contract error, so extremes are always surfaced as `Overflow`.
    let prorate = |principal: u128, rate_bps: u32, secs: u64| -> u128 {
        checked_prorate_interest(principal, rate_bps, secs, Rounding::Floor)
            .unwrap_or_else(|| env.panic_with_error(ContractError::Overflow))
    };

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
    // Both admin `Suspended` and borrower `SelfSuspended` share the same grace
    // semantics: the suspension timestamp marks the start of the waiver window.
    // Treating them together keeps the rate economics identical while the
    // status remains distinct for authorization and audit.
    let is_suspended = matches!(
        line.status,
        CreditStatus::Suspended | CreditStatus::SelfSuspended
    );
    let accrued_u: u128 = if is_suspended {
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
                        GraceWaiverMode::ReducedRate => prorate(
                            line.utilized_amount as u128,
                            cfg.reduced_rate_bps,
                            (now - accrual_start) as u64,
                        ),
                    }
                } else if accrual_start >= grace_end {
                    // Entire period after grace window - use effective rate (may include penalty)
                    prorate(
                        line.utilized_amount as u128,
                        effective_rate_bps,
                        (now - accrual_start) as u64,
                    )
                } else {
                    // Straddles grace boundary — prorate two sub-periods and add.
                    let in_window_secs = (grace_end - accrual_start) as u64;
                    let post_window_secs = (now - grace_end) as u64;

                    let in_window = match cfg.waiver_mode {
                        GraceWaiverMode::FullWaiver => 0u128,
                        GraceWaiverMode::ReducedRate => prorate(
                            line.utilized_amount as u128,
                            cfg.reduced_rate_bps,
                            in_window_secs,
                        ),
                    };

                    // Calculate waived amount for grace waiver event
                    let full_rate_interest =
                        prorate(line.utilized_amount as u128, effective_rate_bps, in_window_secs)
                            as i128;

                    let actual_interest = match cfg.waiver_mode {
                        GraceWaiverMode::FullWaiver => 0,
                        GraceWaiverMode::ReducedRate => prorate(
                            line.utilized_amount as u128,
                            cfg.reduced_rate_bps,
                            in_window_secs,
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

                    let post_window =
                        prorate(line.utilized_amount as u128, effective_rate_bps, post_window_secs);
                    in_window
                        .checked_add(post_window)
                        .unwrap_or_else(|| env.panic_with_error(ContractError::Overflow))
                }
            }
            _ => prorate(
                line.utilized_amount as u128,
                effective_rate_bps,
                (now - accrual_start) as u64,
            ),
        }
    } else {
        // Active, Defaulted, Restricted, or Closed status: apply effective rate (may include penalty)
        prorate(
            line.utilized_amount as u128,
            effective_rate_bps,
            (now - accrual_start) as u64,
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

/// Materialize pending interest accrual across a bounded batch of borrower addresses.
///
/// # Overview
///
/// `accrue_batch` provides off-chain keepers and automated protocol maintenance routines
/// with a single batched entrypoint to materialize interest accrual on multiple active credit lines.
/// It iterates through `borrowers`, loads each line from storage, applies interest capitalization
/// via [`apply_accrual`], and persists updated records if state changed.
///
/// # Parameters
///
/// * `env` — The Soroban contract environment reference (`&Env`).
/// * `borrowers` — Soroban [`Vec<Address>`] containing borrower account addresses to process.
///
/// # Behavior
///
/// 1. Iterates through each address in `borrowers`.
/// 2. Fetches credit line from storage using [`get_credit_line`].
/// 3. Filters for active lines with positive utilization (`status == Active` and `utilized_amount > 0`).
/// 4. Executes [`apply_accrual`] to prorate interest up to the current ledger timestamp.
/// 5. If `utilized_amount` or `last_accrual_ts` modified, persists the updated record via [`persist_credit_line`].
/// 6. **Fault Tolerance**: Non-existent borrower addresses and non-active credit lines are silently skipped
///    without reverting the remainder of the batch.
///
/// # Authorization Rationale
///
/// * **No Auth Required**: Anyone may invoke batch accrual. Because accrual only capitalizes deterministically computed
///   interest based on on-chain rates and elapsed time, caller identity cannot manipulate calculations or extract funds.
///
/// # Gas & Batch Constraints
///
/// * Maximum batch size is enforced at the top-level contract entrypoint (`borrowers.len() <= ACCRUE_BATCH_MAX`, cap = 50).
/// * Storage writes are optimized: persistent storage is mutated **only** when accrual yields a non-zero interest delta.
///
/// # Events
///
/// * Emits per-borrower [`crate::events::InterestAccruedEvent`] for each line where `accrued_amount > 0`.
///
/// # Example
///
/// ```ignore
/// let mut borrowers = Vec::new(&env);
/// borrowers.push_back(alice_address);
/// borrowers.push_back(bob_address);
/// accrue_batch(&env, borrowers);
/// ```
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
                    persist_credit_line(
                        env,
                        &borrower,
                        &updated,
                        previous_utilized,
                        Some(previous_status),
                    );
                }
            }
        }
    }
}
