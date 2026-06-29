// SPDX-License-Identifier: MIT

//! Read-only query views for the Creditra credit contract.
//!
//! Each function is a pure storage read — no state mutations, no token CPIs,
//! no authentication required. TTL may be bumped by `get_credit_line` via the
//! storage layer when the persistent entry nears expiry.

use crate::storage::DataKey;
use crate::types::{
    CreditLineSnapshot, CreditStatus, GracePeriodConfig, ProofOfReserve, ProtocolSummaryView,
    RepaymentSchedule,
};
use soroban_sdk::{Address, Env};

// ── Protocol-level views ─────────────────────────────────────────────────────

/// Return protocol-level dashboard aggregates including `active_line_count`.
///
/// Reads aggregate instance-storage slots only; does not touch per-borrower
/// records and does not bump persistent-entry TTL.
pub fn get_protocol_summary_view(env: Env) -> ProtocolSummaryView {
    ProtocolSummaryView {
        total_utilized: crate::storage::get_total_utilized(&env),
        total_collateral: crate::storage::get_total_collateral(&env),
        active_line_count: crate::storage::get_active_line_count(&env),
    }
}

/// Return proof-of-reserve balances for the protocol treasury.
///
/// Exposes the accumulated treasury and bounty pool reserves held in the
/// contract as a result of protocol fee collection. A pure storage read —
/// no token CPIs or borrower records are touched.
///
/// Callers can compare `treasury_balance + bounty_balance` against the
/// on-chain token balance of the contract to verify reserve integrity.
pub fn get_proof_of_reserve(env: Env) -> ProofOfReserve {
    ProofOfReserve {
        treasury_balance: crate::storage::get_treasury_balance(&env),
        bounty_balance: crate::storage::get_bounty_balance(&env),
    }
}

// ── Per-borrower snapshot view ────────────────────────────────────────────────

/// Return a full snapshot of `borrower`'s credit line, or `None` if no line exists.
///
/// Assembles [`CreditLineSnapshot`] in a single entrypoint call, avoiding the
/// multiple round-trips that callers would otherwise need to issue for
/// `get_credit_line` + `get_collateral` + `get_health_factor` +
/// `get_repayment_schedule` + `is_delinquent`.
///
/// # Authentication
/// None — this is a pure read with no state mutations or trust boundary.
///
/// # Laziness
/// Interest accrual is lazy: `line.accrued_interest` and `line.utilized_amount`
/// reflect the last mutating checkpoint, not the current ledger timestamp.
///
/// # Collateral health
/// `health_factor_bps` is `u32::MAX` when `utilized_amount == 0`. A value
/// below `10_000` signals the line is eligible for liquidation via
/// `default_credit_line`.
///
/// # Delinquency
/// `is_delinquent` is always `false` when `repayment_schedule` is `None` or
/// when `utilized_amount == 0` or status is `Closed`.
pub fn get_credit_line_snapshot(env: Env, borrower: Address) -> Option<CreditLineSnapshot> {
    // A single storage read; returns None immediately for unknown borrowers.
    let line = crate::storage::get_credit_line(&env, &borrower)?;

    let collateral_balance = crate::storage::get_collateral_balance(&env, &borrower);

    let health_factor_bps = compute_health_factor_bps(&env, &borrower, line.utilized_amount);

    let repayment_schedule: Option<RepaymentSchedule> = env
        .storage()
        .persistent()
        .get(&DataKey::RepaymentSchedule(borrower.clone()));

    let is_delinquent =
        check_is_delinquent(&env, &line.status, line.utilized_amount, &repayment_schedule);

    Some(CreditLineSnapshot {
        line,
        collateral_balance,
        health_factor_bps,
        repayment_schedule,
        is_delinquent,
    })
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Compute the collateral health factor in basis points for a borrower.
///
/// Returns `u32::MAX` when `utilized_amount <= 0` (no debt).
/// Formula: `collateral * 100_000_000 / (utilized * min_ratio_bps)`.
fn compute_health_factor_bps(env: &Env, borrower: &Address, utilized_amount: i128) -> u32 {
    if utilized_amount <= 0 {
        return u32::MAX;
    }

    let collateral = crate::storage::get_collateral_balance(env, borrower);
    let min_ratio_bps = crate::storage::get_min_collateral_ratio_bps(env).unwrap_or(15_000);

    let collateral_u128 = collateral.max(0) as u128;
    let utilized_u128 = utilized_amount.max(0) as u128;
    let min_ratio_u128 = min_ratio_bps as u128;

    let numerator = collateral_u128
        .checked_mul(100_000_000)
        .unwrap_or(u128::MAX);
    let denominator = utilized_u128
        .checked_mul(min_ratio_u128)
        .unwrap_or(u128::MAX);

    u32::try_from(numerator / denominator).unwrap_or(u32::MAX)
}

/// Determine whether the borrower is past an installment due date.
///
/// Returns `false` when:
/// - The line is `Closed` or `utilized_amount <= 0`.
/// - No repayment schedule is configured.
/// - The current timestamp is within the grace window.
fn check_is_delinquent(
    env: &Env,
    status: &CreditStatus,
    utilized_amount: i128,
    schedule: &Option<RepaymentSchedule>,
) -> bool {
    if *status == CreditStatus::Closed || utilized_amount <= 0 {
        return false;
    }
    let Some(sched) = schedule else {
        return false;
    };

    let grace_cfg: Option<GracePeriodConfig> = env
        .storage()
        .instance()
        .get(&crate::storage::grace_period_key(env));
    let grace_seconds = grace_cfg
        .map(|cfg| cfg.grace_period_seconds)
        .unwrap_or(0);
    let delinquent_after = sched.next_due_ts.saturating_add(grace_seconds);

    env.ledger().timestamp() > delinquent_after
}
