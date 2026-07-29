// SPDX-License-Identifier: MIT

//! Read-only capabilities view for collateral operations (v7).
//!
//! This module exposes [`collateral_capabilities`] — a pure read that computes
//! which collateral operations are currently permitted for a given borrower by
//! querying the credit contract's public entrypoints via [`CreditClient`].
//!
//! # Why a separate module
//!
//! The capabilities bitmap is a convenient utility for off-chain dashboards,
//! integration tests, and on-chain composability layers that want to check
//! collateral pre-conditions before constructing a transaction. Rather than
//! replicating the logic in every caller, this helper centralizes it in the
//! collateral crate.
//!
//! # No authentication required
//!
//! [`collateral_capabilities`] only calls read-only entrypoints on the credit
//! contract. It does not mutate state and requires no auth on the caller side.
//!
//! # What is evaluated
//!
//! The function evaluates the same structural pre-flight checks that
//! `deposit_collateral`, `withdraw_collateral`, and
//! `partial_release_collateral` perform, **except** amount-dependent checks
//! (health-factor post-withdrawal, exposure caps). A `true` value means the
//! structural preconditions pass; the actual entrypoint may still revert if
//! the supplied amount violates numeric limits.
//!
//! # See also
//! - `contracts/credit/src/views.rs::borrow_capabilities` — borrow-side bitmap.
//! - `contracts/credit/src/types.rs::CollateralCapabilities` — returned struct.

use creditra_credit::types::{CollateralCapabilities, CreditStatus};
use creditra_credit::CreditClient;
use soroban_sdk::{Address, Env};

/// Return the collateral-operation capabilities bitmap for `borrower`.
///
/// Queries the Creditra credit contract at `contract_id` on `env` for all
/// relevant state, then derives the capability flags from that state.
///
/// # Parameters
///
/// - `env` — the Soroban environment.
/// - `contract_id` — the address of the deployed `Credit` contract.
/// - `borrower` — the borrower address to query.
///
/// # Returns
///
/// A [`CollateralCapabilities`] struct with six fields:
///
/// | Field                  | Meaning                                               |
/// |------------------------|-------------------------------------------------------|
/// | `can_deposit`          | `deposit_collateral` structural preconditions pass    |
/// | `can_withdraw`         | `can_deposit` AND `collateral_balance > 0`            |
/// | `can_partial_release`  | `can_withdraw` AND positive utilization               |
/// | `collateral_required`  | `min_ratio_bps > 0` AND positive utilization          |
/// | `collateral_balance`   | Current accounting collateral balance (i128 units)    |
/// | `min_ratio_bps`        | Configured minimum collateral ratio bps (`0` = off)   |
///
/// # Examples
///
/// ```ignore
/// let caps = collateral_capabilities(env, contract_id, borrower);
/// if caps.can_deposit {
///     // safe to construct a deposit_collateral transaction
/// }
/// if caps.collateral_required && !caps.can_deposit {
///     // borrower is blocked or line is closed — warn the user
/// }
/// ```
///
/// # Security
///
/// Pure read-only. No state mutations. No `require_auth`. TTL may be bumped
/// on the borrower's persistent storage entry as a side-effect of reading
/// the credit line.
pub fn collateral_capabilities(
    env: Env,
    contract_id: Address,
    borrower: Address,
) -> CollateralCapabilities {
    let client = CreditClient::new(&env, &contract_id);

    // ── Fetch all relevant state via read-only query entrypoints ────────────

    // Per-borrower credit line (status, utilized_amount).
    let line_opt = client.get_credit_line(&borrower);

    // Per-borrower collateral accounting balance.
    let collateral_balance = client.get_collateral(&borrower);

    // Protocol-wide minimum collateral ratio (0 = disabled / not set).
    let min_ratio_bps = client.get_min_collateral_ratio_bps().unwrap_or(0);

    // Protocol pause flag.
    let paused = client.is_protocol_paused();

    // Borrower-level block flag.
    let blocked = client.is_borrower_blocked(&borrower);

    // ── Derive structural capability flags ──────────────────────────────────

    // Line exists and is NOT permanently Closed.
    let line_open = line_opt
        .as_ref()
        .map(|l| l.status != CreditStatus::Closed)
        .unwrap_or(false);

    // Shared base condition: protocol is live, line exists/open, borrower unblocked.
    let base_ok = !paused && line_open && !blocked;

    // `can_deposit` — all base preconditions pass; amount checks are deferred.
    let can_deposit = base_ok;

    // `can_withdraw` — additionally requires a positive collateral balance.
    let can_withdraw = base_ok && collateral_balance > 0;

    // Outstanding principal drawn by the borrower.
    let utilized = line_opt.as_ref().map(|l| l.utilized_amount).unwrap_or(0);

    // `can_partial_release` — partial release only makes sense when the
    // borrower has both collateral AND outstanding debt to partially release
    // against.
    let can_partial_release = can_withdraw && utilized > 0;

    // `collateral_required` — the protocol enforces a minimum ratio AND the
    // borrower has active debt (so the constraint is binding for this line).
    let collateral_required = min_ratio_bps > 0 && utilized > 0;

    CollateralCapabilities {
        can_deposit,
        can_withdraw,
        can_partial_release,
        collateral_required,
        collateral_balance,
        min_ratio_bps,
    }
}
