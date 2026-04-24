use crate::auth::{require_admin, require_admin_auth};
use crate::events::{publish_credit_line_event, CreditLineEvent};
use crate::types::{CreditLineData, CreditStatus};
use soroban_sdk::{symbol_short, Address, Env};

/// Suspend a credit line temporarily.
///
/// Called by admin to freeze a borrower's credit line without closing it.
/// The credit line can be reactivated or closed after suspension.
///
/// # Parameters
/// - `borrower`: The borrower's address.
///
/// # Panics
/// - If no credit line exists for the given borrower.
///
/// # Events
/// Emits a `("credit", "suspend")` [`CreditLineEvent`].
pub fn suspend_credit_line(env: Env, borrower: Address) {
    require_admin_auth(&env);
    let mut credit_line: CreditLineData = env
        .storage()
        .persistent()
        .get(&borrower)
        .expect("Credit line not found");

    // Apply interest accrual before any mutation
    credit_line = crate::accrual::apply_accrual(&env, credit_line);

    if credit_line.status != CreditStatus::Active {
        panic!("Only active credit lines can be suspended");
    }

    credit_line.status = CreditStatus::Suspended;
    env.storage().persistent().set(&borrower, &credit_line);

    publish_credit_line_event(
        &env,
        (symbol_short!("credit"), symbol_short!("suspend")),
        CreditLineEvent {
            event_type: symbol_short!("suspend"),
            borrower: borrower.clone(),
            status: CreditStatus::Suspended,
            credit_limit: credit_line.credit_limit,
            interest_rate_bps: credit_line.interest_rate_bps,
            risk_score: credit_line.risk_score,
        },
    );
}

/// Close a credit line. Callable by admin (force-close) or by borrower when utilization is zero.
/// Allowed from Active, Suspended, or Defaulted. Idempotent if already Closed.
///
/// # Arguments
/// * `closer` - Address that must have authorized this call. Must be either the contract admin
///   (can close regardless of utilization) or the borrower (can close only when
///   `utilized_amount` is zero).
///
/// # Errors
/// * Panics if credit line does not exist, or if `closer` is not admin/borrower, or if
///   borrower closes while `utilized_amount != 0`.
///
/// Emits a CreditLineClosed event.
pub fn close_credit_line(env: Env, borrower: Address, closer: Address) {
    closer.require_auth();

    let admin: Address = require_admin(&env);

    let mut credit_line: CreditLineData = env
        .storage()
        .persistent()
        .get(&borrower)
        .expect("Credit line not found");

    // Apply interest accrual before any mutation
    credit_line = crate::accrual::apply_accrual(&env, credit_line);

    if credit_line.status == CreditStatus::Closed {
        return;
    }

    let allowed = closer == admin || (closer == borrower && credit_line.utilized_amount == 0);

    if !allowed {
        if closer == borrower {
            panic!("cannot close: utilized amount not zero");
        }
        panic!("unauthorized");
    }

    credit_line.status = CreditStatus::Closed;
    env.storage().persistent().set(&borrower, &credit_line);

    publish_credit_line_event(
        &env,
        (symbol_short!("credit"), symbol_short!("closed")),
        CreditLineEvent {
            event_type: symbol_short!("closed"),
            borrower: borrower.clone(),
            status: CreditStatus::Closed,
            credit_limit: credit_line.credit_limit,
            interest_rate_bps: credit_line.interest_rate_bps,
            risk_score: credit_line.risk_score,
        },
    );
}

/// Mark a credit line as defaulted (admin only).
///
/// Call when the line is past due or when an oracle/off-chain signal indicates default.
/// Transition: Active or Suspended → Defaulted.
/// After this, draw_credit is disabled and repay_credit remains allowed.
/// Emits a CreditLineDefaulted event.
pub fn default_credit_line(env: Env, borrower: Address) {
    require_admin_auth(&env);
    let mut credit_line: CreditLineData = env
        .storage()
        .persistent()
        .get(&borrower)
        .expect("Credit line not found");

    // Apply interest accrual before any mutation
    credit_line = crate::accrual::apply_accrual(&env, credit_line);

    credit_line.status = CreditStatus::Defaulted;
    env.storage().persistent().set(&borrower, &credit_line);

    publish_credit_line_event(
        &env,
        (symbol_short!("credit"), symbol_short!("default")),
        CreditLineEvent {
            event_type: symbol_short!("default"),
            borrower: borrower.clone(),
            status: CreditStatus::Defaulted,
            credit_limit: credit_line.credit_limit,
            interest_rate_bps: credit_line.interest_rate_bps,
            risk_score: credit_line.risk_score,
        },
    );
}

/// Reinstate a defaulted credit line to Active (admin only).
///
/// Allowed only when status is Defaulted. Transition: Defaulted → Active.
pub fn reinstate_credit_line(env: Env, borrower: Address) {
    require_admin_auth(&env);

    let mut credit_line: CreditLineData = env
        .storage()
        .persistent()
        .get(&borrower)
        .expect("Credit line not found");

    // Apply interest accrual before any mutation
    credit_line = crate::accrual::apply_accrual(&env, credit_line);

    if credit_line.status != CreditStatus::Defaulted {
        panic!("credit line is not defaulted");
    }

    credit_line.status = CreditStatus::Active;
    env.storage().persistent().set(&borrower, &credit_line);

    publish_credit_line_event(
        &env,
        (symbol_short!("credit"), symbol_short!("reinstate")),
        CreditLineEvent {
            event_type: symbol_short!("reinstate"),
            borrower: borrower.clone(),
            status: CreditStatus::Active,
            credit_limit: credit_line.credit_limit,
            interest_rate_bps: credit_line.interest_rate_bps,
            risk_score: credit_line.risk_score,
        },
    );
}
