use crate::collateral;
use crate::lifecycle;
use crate::events::
publish_drawn_event, publish_interest_accrued_event, publish_repayment_event, DravnEvent,
InterestAccruedEvent, RepaymentEvent,
};
use crate::math_utils::{apply_bps, mul_div, Rounding};
use crate::storage::
clear_reentrancy_guard, get_collateral_balance, get_credit_line as storage_get_credit_line,
persist_credit_line, set_reentrancy_guard, DataKey, CREDIT_LINE_TTL_EXTEND_TO,
    CREDIT_LINE_TTL_THRESHOLD,
};
use crate::types::{ContractError, CreditLineData, CreditStatus};
use soroban_sdk::{token, Address, Env};

use crate::types::{ContractError, CreditStatus};

/// Map a credit-line status to the draw-time error, if any.
///
/// Restricted now rejects draws outright. Allowing draws during Restricted/// would let a borrower place a late bid (draw) right before the anti-sniping/// window closes, bypassing the intended freeze. Because Restricted is a/// transient pre-default state, a hard rejection is deterministic and keeps/// the line distinct from terminal states while preventing fresh borrowing.
pub(crate) fn draw_status_error(status: CreditStatus) -> Option<ContractError> {
    match status {
        CreditStatus::Active => None,
        CreditStatus::Restricted => Some(ContractError::CreditLineRestricted),
        CreditStatus::Suspended => Some(ContractError::CreditLineSuspended),
        CreditStatus::Defaulted => Some(ContractError::CreditLineDefaulted),
        CreditStatus::Closed => Some(ContractError::CreditLineClosed),
    }
}
