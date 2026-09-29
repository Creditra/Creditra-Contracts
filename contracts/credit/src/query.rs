use crate::storage::grace_period_key;
use crate::types::{
    CreditLineData, CreditStatus, GracePeriodConfig, ProtocolSummary, RepaymentSchedule,
};
use soroban_sdk::{Address, Env};

/// Return the credit line for `borrower`, or `None` if no line exists.
///
/// # Authentication
/// No authentication required. This is a pure read — it does not mutate
/// any storage and carries no trust boundary. Any caller (indexer, client,
/// or another contract) may invoke it freely.
///
/// # Stability
/// The returned [`CreditLineData`] struct is stable for integrators.
/// All fields — including `last_rate_update_ts`, `accrued_interest`, and
/// `last_accrual_ts` — are serialized in the order declared in `types.rs`.
/// New fields will only be appended; existing field positions will not change.
///
/// # Note on accrual
/// Interest accrual is lazy: `accrued_interest` and `utilized_amount` reflect
/// the last mutating call (draw, repay, suspend, etc.). Pending interest since
/// the last checkpoint is **not** applied by this query.
/// Return the credit line for `borrower`, or `None` if no line exists.
///
/// # Authentication
/// No authentication required. This is a pure read — it does not mutate
/// any storage and carries no trust boundary. Any caller (indexer, client,
/// or another contract) may invoke it freely.
///
/// # Stability
/// The returned [`CreditLineData`] struct is stable for integrators.
/// All fields — including `last_rate_update_ts`, `accrued_interest`, and
/// `last_accrual_ts` — are serialized in the order declared in `types.rs`.
/// New fields will only be appended; existing field positions will not change.
///
/// # Note on accrual
/// Interest accrual is lazy: `accrued_interest` and `utilized_amount` reflect
/// the last mutating call (draw, repay, suspend, etc.). Pending interest since
/// the last checkpoint is **not** applied by this query.
pub fn get_credit_line(env: Env, borrower: Address) -> Option<CreditLineData> {
    crate::storage::get_credit_line(&env, &borrower)
}

/// Return protocol-level dashboard aggregates in one read-only call.
///
/// # Authentication
/// No authentication required. This is a pure read — it reads only aggregate
/// storage slots and does not touch per-borrower records, so it does not
/// bump persistent-entry TTL.
pub fn get_protocol_summary(env: Env) -> ProtocolSummary {
    ProtocolSummary {
        count: crate::storage::get_credit_line_count(&env),
        total_utilized: crate::storage::get_total_utilized(&env),
        total_collateral: crate::storage::get_total_collateral(&env),
        treasury_balance: crate::storage::get_treasury_balance(&env),
        bounty_balance: crate::storage::get_bounty_balance(&env),
    }
}

/// Return the configured installment repayment schedule for `borrower`, if any.
///
/// # Authentication
/// No authentication required. This is a pure read — it delegates to
/// [`crate::storage::get_repayment_schedule`], which bumps the schedule
/// entry's TTL on read so an active borrower's schedule stays live.
pub fn get_repayment_schedule(env: Env, borrower: Address) -> Option<RepaymentSchedule> {
    env.storage()
        .persistent()
        .get(&crate::storage::DataKey::RepaymentSchedule(borrower))
}

/// Return the collateral-aware health factor for a borrower, expressed in basis
/// points (bps).
///
/// # Formula
///
/// ```text
/// health_bps = collateral_value * 10_000 / (utilized_amount * min_ratio_bps / 10_000)
/// ```
///
/// This simplifies to:
///
/// ```text
/// health_bps = collateral_value * 100_000_000 / (utilized_amount * min_ratio_bps)
/// ```
///
/// # What this metric includes
///
/// - **Single-token collateral balance**: Reads the single on-chain collateral balance
///   deposited for `borrower` via [`crate::storage::get_collateral_balance`].
/// - **Recorded debt utilization**: Reads `utilized_amount` currently stored in the
///   borrower's [`CreditLineData`].
/// - **Configured minimum collateral ratio**: Reads global `MinCollateralRatioBps`
///   (defaults to `15_000` bps / 150% if unset).
///
/// # Excluded inputs & blind spots (WARNING for off-chain keepers)
///
/// Keepers and integrators must **not** rely on `get_health_factor` as a complete,
/// autonomous liquidation signal. The metric has significant blind spots:
///
/// 1. **No multi-token collateral support**: Only the single configured collateral
///    token balance is evaluated. Any multi-asset collateral deposits, external tokens,
///    or cross-asset portfolios are excluded.
/// 2. **No risk weights or haircuts**: Collateral units are weighted 1:1 without
///    haircuts, risk tiers, or asset-specific collateral factors.
/// 3. **No oracle prices or exchange rates**: The calculation assumes a 1:1 nominal
///    parity between the collateral token and the borrowed liquidity token. It does
///    not consult oracle price feeds, ignoring price volatility, exchange-rate swings,
///    and stablecoin depeg events.
/// 4. **Ignores delinquency and missed installments**: The health factor measures
///    collateralization only; it does **not** evaluate whether the borrower is delinquent
///    on repayment installments. Keepers must inspect [`is_delinquent`] separately.
/// 5. **Excludes uncheckpointed pending interest**: Stored `utilized_amount` reflects
///    debt as of the last mutating transaction; accrued interest elapsed since
///    `last_accrual_ts` is not applied by this read-only query.
///
/// # Default eligibility & on-chain enforcement
///
/// **Important**: `default_credit_line` does **NOT** consult or enforce `get_health_factor`.
/// On-chain default is an **admin-discretionary** lifecycle transition (`require_admin_auth`).
/// The contract validates only that the protocol is not paused, the credit line is in an
/// eligible status (`Active`, `Suspended`, `SelfSuspended`, or `Restricted`), and any active
/// liquidation grace period has expired. The contract will **not** revert a default call
/// simply because `get_health_factor >= 10_000`, nor does it automatically trigger default
/// when `get_health_factor < 10_000`.
///
/// # Recommended keeper evaluation logic
///
/// Off-chain keepers should combine both financial health and payment behavior before
/// initiating or proposing a default:
///
/// 1. **Check delinquency**: Call [`is_delinquent`]. If `true` and the grace period
///    has elapsed, the borrower has missed a scheduled installment and may be defaulted
///    regardless of collateral backing.
/// 2. **Check health factor with off-chain pricing**: Evaluate `get_health_factor` alongside
///    real-time oracle prices and effective risk weights.
/// 3. **Check liquidation grace**: Verify whether the borrower is within an active
///    per-borrower liquidation grace window (`get_per_borrower_liquidation_grace`).
///
/// # Example thresholds
///
/// | Health Factor (bps) | Collateral Status | Recommended Keeper Action |
/// |---|---|---|
/// | `u32::MAX` | No outstanding debt (`utilized <= 0`) | Safe — no liquidation possible |
/// | `≥ 12_000` (≥ 120%) | Well-collateralized buffer | Healthy — no action required |
/// | `10_000..11_999` (100%–120%) | Nearing minimum required ratio | Caution — monitor closely for price volatility |
/// | `< 10_000` (< 100%) | Under-collateralized advisory threshold | Liquidation candidate — trigger off-chain review / default |
/// | `< 8_000` (< 80%) | Critically under-collateralized | Urgent liquidation candidate — immediate action recommended |
///
/// *(Note: A borrower with `is_delinquent == true` past grace is default-eligible even if `health_factor >= 10_000`.)*
///
/// # Interpretation
///
/// - Returns `u32::MAX` when `utilized_amount == 0` (no debt → infinitely
///   healthy).
/// - A value below `10_000` indicates nominal under-collateralization relative to
///   the minimum collateral ratio.
/// - A value of `10_000` means the collateral exactly covers the minimum
///   required amount.
/// - A value above `10_000` means the position is over-collateralized relative
///   to the minimum ratio.
///
/// # Read-only guarantee
///
/// This function reads the borrower's credit line (which may bump Persistent
/// entry TTL if below the threshold), the collateral balance, and the global
/// `MinCollateralRatioBps` config.  It performs **no** storage writes.
///
/// # Default minimum collateral ratio
///
/// When `MinCollateralRatioBps` is not configured, the function falls back to
/// `15000` (150 %), matching the draw-time enforcement in `draw_credit`.
///
/// # Edge cases
///
/// - Borrower has no credit line or a `Closed` line: still computes the ratio
///   from the on-chain collateral balance and the stored `utilized_amount`.
///   Returning `u32::MAX` for zero utilization covers the "healthy" case even
///   for a closed line.
/// - `utilized_amount` is negative (should never happen): returns `u32::MAX`
///   via the zero-utilised short-circuit since the storage invariant enforces
///   `utilized_amount >= 0`.
///
/// # Authentication
/// No authentication required. This is a pure read — it computes the health
/// ratio from on-chain data without modifying any storage.
pub fn get_health_factor(env: Env, borrower: Address) -> u32 {
    // Load the borrower's credit line.  If none exists, treat as zero
    // utilization → infinitely healthy.
    let utilized = match get_credit_line(env.clone(), borrower.clone()) {
        Some(line) => line.utilized_amount,
        None => return u32::MAX,
    };

    // No outstanding debt — the position cannot be liquidated.
    if utilized <= 0 {
        return u32::MAX;
    }

    // Fetch collateral balance.  Defaults to 0 if no collateral has been
    // deposited.
    let collateral = crate::storage::get_collateral_balance(&env, &borrower);

    // Fetch the global minimum collateral ratio.  When unset the draw-time
    // default of 15_000 bps (150 %) applies.
    let min_ratio_bps = crate::storage::get_min_collateral_ratio_bps(&env).unwrap_or(15_000);

    // Convert to u128 for overflow-safe multiplication.
    let collateral_u128 = collateral.max(0) as u128;
    let utilized_u128 = utilized.max(0) as u128;
    let min_ratio_u128 = min_ratio_bps as u128;

    // health_bps = collateral * 100_000_000 / (utilized * min_ratio)
    //
    // The intermediate numerator is `collateral * 10_000` scaled up by another
    // `10_000` to preserve precision before the final division:
    //
    //   collateral * 10_000                 collateral * 100_000_000
    //   ───────────────────────    =    ─────────────────────────────
    //   utilized * min_ratio / 10_000        utilized * min_ratio
    let numerator = collateral_u128
        .checked_mul(100_000_000)
        .unwrap_or(u128::MAX);

    let denominator = utilized_u128
        .checked_mul(min_ratio_u128)
        .unwrap_or(u128::MAX);

    // If the denominator is 0 (due to min_ratio_bps = 0), the position is infinitely healthy.
    // We also guard against division-by-zero.
    let health_bps = if denominator == 0 {
        u128::from(u32::MAX)
    } else {
        numerator / denominator
    };

    // Clamp to u32 range.  Values beyond u32::MAX are theoretically possible
    // with extreme collateral-to-debt ratios but serve the same keeper
    // decision ("definitely not liquidatable") as u32::MAX itself.
    u32::try_from(health_bps).unwrap_or(u32::MAX)
}

/// Return `true` when the borrower has missed an installment past the grace window.
///
/// # Authentication
/// No authentication required. This is a pure read — it examines the stored
/// repayment schedule and grace-period config without modifying any storage.
///
/// Returns `false` for the following short-circuit cases:
/// - The borrower has no credit line.
/// - The line is `Closed` or has zero outstanding principal.
/// - The line has no configured [`RepaymentSchedule`].
///
/// The grace window is determined by the global [`GracePeriodConfig`]. When no
/// config is set, `grace_seconds` defaults to `0`, so any timestamp strictly
/// greater than `next_due_ts` is treated as delinquent. The comparison uses
/// `saturating_add` to ensure timestamps near `u64::MAX` do not wrap.
pub fn is_delinquent(env: Env, borrower: Address) -> bool {
    let Some(line) = get_credit_line(env.clone(), borrower.clone()) else {
        return false;
    };

    if line.status == CreditStatus::Closed || line.utilized_amount <= 0 {
        return false;
    }

    let Some(schedule) = get_repayment_schedule(env.clone(), borrower.clone()) else {
        return false;
    };

    let grace_cfg: Option<GracePeriodConfig> =
        env.storage().instance().get(&grace_period_key(&env));
    let grace_seconds = grace_cfg.map(|cfg| cfg.grace_period_seconds).unwrap_or(0);
    let delinquent_after = schedule.next_due_ts.saturating_add(grace_seconds);

    env.ledger().timestamp() > delinquent_after
}
