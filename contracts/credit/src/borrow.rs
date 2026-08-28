use crate::collateral;
use crate::lifecycle;
use crate::events::{
    publish_drawn_event, publish_interest_accrued_event, publish_repayment_event, DrawnEvent,
    InterestAccruedEvent, RepaymentEvent,
};
use crate::math_utils::{apply_bps, mul_div, Rounding};
use crate::storage::{
    clear_reentrancy_guard, get_collateral_balance, get_credit_line as storage_get_credit_line,
    persist_credit_line, set_reentrancy_guard, DataKey, CREDIT_LINE_TTL_EXTEND_TO,
    CREDIT_LINE_TTL_THRESHOLD,
};
use crate::types::{ContractError, CreditLineData, CreditStatus};
use soroban_sdk::{token, Address, Env};

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
