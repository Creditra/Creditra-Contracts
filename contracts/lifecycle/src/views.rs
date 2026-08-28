// SPDX-License-Identifier: MIT

//! Read-only lifecycle capabilities view (v7).
//!
//! Mirrors the transition guards enforced by [`crate::lifecycle`] so
//! off-chain clients and on-chain integrators can check which lifecycle
//! transitions are currently permitted for a borrower's credit line without
//! simulating the full entrypoint (state lookup + auth + status checks).
//!
//! # What
//!
//! [`capabilities`] returns a [`crate::types::LifecycleCapabilities`] bitmap
//! covering every state-changing lifecycle entrypoint:
//! `suspend_credit_line`, `self_suspend_credit_line`, `close_credit_line`
//! (both the unconditional admin path and the zero-utilization borrower
//! path), `default_credit_line`, and `reinstate_credit_line`.
//!
//! # How
//!
//! Each field is derived purely from the credit line's current
//! [`crate::types::CreditStatus`], its `utilized_amount`, and the protocol
//! pause flag — a pure storage read with no token CPIs, no auth checks, and
//! no mutation. When no credit line exists for `borrower`, every field is
//! `false`.
//!
//! # Why
//!
//! Every lifecycle transition in [`crate::lifecycle`] starts with
//! `assert_not_paused` followed by a status check; duplicating that logic
//! here (read-only, no panics) lets callers pre-flight a transition — e.g. a
//! keeper deciding whether `default_credit_line` is currently callable —
//! without spending gas on a reverting simulation.
//!
//! See [`docs/PROTOCOL_SPEC.md`](../../../docs/PROTOCOL_SPEC.md) and
//! [`docs/state-machine.md`](../../../docs/state-machine.md) for the
//! authoritative transition table each field mirrors.

use crate::storage::{get_credit_line, is_paused};
use crate::types::{CreditStatus, LifecycleCapabilities};
use soroban_sdk::{Address, Env};

/// Return the lifecycle-transition capabilities bitmap for `borrower`.
///
/// Read-only, no-auth view. See [`LifecycleCapabilities`] for field
/// semantics.
///
/// [`LifecycleCapabilities`]: crate::types::LifecycleCapabilities
pub fn capabilities(env: Env, borrower: Address) -> LifecycleCapabilities {
    let credit_line = get_credit_line(&env, &borrower);
    let paused = is_paused(&env);

    let (
        can_suspend,
        can_self_suspend,
        can_close_admin,
        can_close_borrower,
        can_default,
        can_reinstate,
    ) = match credit_line {
        None => (false, false, false, false, false, false),
        Some(_) if paused => (false, false, false, false, false, false),
        Some(line) => {
            let status = line.status;

            // suspend_credit_line / self_suspend_credit_line: Active only.
            let can_suspend = status == CreditStatus::Active;
            let can_self_suspend = can_suspend;

            // close_credit_line (admin): any non-Closed status.
            let can_close_admin = status != CreditStatus::Closed;
            // close_credit_line (borrower): admin precondition + zero utilization.
            let can_close_borrower = can_close_admin && line.utilized_amount == 0;

            // default_credit_line: Active, Restricted, or Suspended.
            let can_default = matches!(
                status,
                CreditStatus::Active | CreditStatus::Restricted | CreditStatus::Suspended
            );

            // reinstate_credit_line: Defaulted only.
            let can_reinstate = status == CreditStatus::Defaulted;

            (
                can_suspend,
                can_self_suspend,
                can_close_admin,
                can_close_borrower,
                can_default,
                can_reinstate,
            )
        }
    };

    LifecycleCapabilities {
        can_suspend,
        can_self_suspend,
        can_close_admin,
        can_close_borrower,
        can_default,
        can_reinstate,
    }
}
