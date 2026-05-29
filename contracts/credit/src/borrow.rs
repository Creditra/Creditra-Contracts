// SPDX-License-Identifier: MIT
//! Borrow module: draw and repay helpers.
//!
//! Restricted lines are repayment-capable cure states. Draw requests still
//! flow through the normal utilization check, so they cannot create new net
//! borrowing while the line remains over its reduced limit.

use crate::types::{ContractError, CreditStatus};

/// Map a credit-line status to the draw-time error, if any.
///
/// Restricted is intentionally allowed to reach the numeric limit check in
/// `draw_credit`; that keeps the status distinct from terminal states while
/// still preventing fresh borrowing until the line is cured.
pub(crate) fn draw_status_error(status: CreditStatus) -> Option<ContractError> {
    match status {
        CreditStatus::Active | CreditStatus::Restricted => None,
        CreditStatus::Suspended => Some(ContractError::CreditLineSuspended),
        CreditStatus::Defaulted => Some(ContractError::CreditLineDefaulted),
        CreditStatus::Closed => Some(ContractError::CreditLineClosed),
    }
}
