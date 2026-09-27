// SPDX-License-Identifier: MIT
//! Borrow module: draw-time status gating.
//!
//! # Status semantics
//!
//! - [`CreditStatus::Active`]: full borrowing capability.
//! - [`CreditStatus::Restricted`]: cure state. Repayments are allowed and
//!   draws still flow through the numeric limit check, so they cannot create
//!   new net borrowing while the line remains over its reduced limit.
//! - [`CreditStatus::Suspended`]: draws blocked, repayments allowed (admin-initiated).
//! - [`CreditStatus::SelfSuspended`]: draws blocked, repayments allowed (borrower-initiated).
//!   Distinct from `Suspended` for auditability and authorization — an admin
//!   suspension cannot be cleared by the borrower, while a self-suspension
//!   can be cleared by either the borrower (self-unsuspend) or the admin.
//! - [`CreditStatus::Defaulted`]: draws blocked, repayments allowed.
//! - [`CreditStatus::Closed`]: draws blocked, repayments blocked.
//!
//! See [`docs/state-machine.md`](../../../docs/state-machine.md) for the
//! authoritative transition diagram.

use crate::types::{ContractError, CreditStatus};

/// Map a credit-line status to the draw-time error, if any.
///
/// Restricted is intentionally allowed to reach the numeric limit check in
/// `draw_credit`; that keeps the status distinct from terminal states while
/// still preventing fresh borrowing until the line is cured.
/// Both `Suspended` (admin) and `SelfSuspended` (borrower) block draws with
/// the same `CreditLineSuspended` error to preserve the external error API —
/// callers distinguish the origin via the stored `CreditStatus` value and the
/// suspension event topic, not via a new error code.
pub(crate) fn draw_status_error(status: CreditStatus) -> Option<ContractError> {
    match status {
        CreditStatus::Active | CreditStatus::Restricted => None,
        CreditStatus::Suspended | CreditStatus::SelfSuspended => {
            Some(ContractError::CreditLineSuspended)
        }
        CreditStatus::Defaulted => Some(ContractError::CreditLineDefaulted),
        CreditStatus::Closed => Some(ContractError::CreditLineClosed),
    }
}
