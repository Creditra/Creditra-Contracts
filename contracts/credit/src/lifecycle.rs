use crate::auth::{require_admin, require_admin_auth};
use crate::events::{publish_credit_line_event, CreditLineEvent};
use crate::storage::DataKey;
use crate::types::{ContractError, CreditLineData, CreditStatus};
use soroban_sdk::{symbol_short, Address, Env};

pub fn open_credit_line(
    env: Env,
    borrower: Address,
    credit_limit: i128,
    interest_rate_bps: u32,
    risk_score: u32,
) {
    assert!(credit_limit > 0, "credit_limit must be greater than zero");
    assert!(
        interest_rate_bps <= 10_000,
        "interest_rate_bps cannot exceed 10000 (100%)"
    );
    assert!(risk_score <= 100, "risk_score must be between 0 and 100");

    // Prevent overwriting an existing Active credit line
    if let Some(existing) = env
        .storage()
        .persistent()
        .get::<Address, CreditLineData>(&borrower)
    {
        assert!(
            existing.status != CreditStatus::Active,
            "borrower already has an active credit line"
        );
    }

    // Generate a new line_id
    let mut counter: u32 = env.storage().instance().get(&DataKey::LineIdCounter).unwrap_or(0);
    counter += 1;
    env.storage().instance().set(&DataKey::LineIdCounter, &counter);

    let credit_line = CreditLineData {
        line_id: counter,
        borrower: borrower.clone(),
        credit_limit,
        utilized_amount: 0,
        interest_rate_bps,
        risk_score,
        status: CreditStatus::Active,
        last_rate_update_ts: 0,
        accrued_interest: 0,
        last_accrual_ts: 0,
    };

    env.storage().persistent().set(&borrower, &credit_line);
    env.storage().persistent().set(&DataKey::LineIdMap(counter), &borrower);

    publish_credit_line_event(
        &env,
        (symbol_short!("credit"), symbol_short!("opened")),
        CreditLineEvent {
            event_type: symbol_short!("opened"),
            line_id: counter,
            borrower: borrower.clone(),
            status: CreditStatus::Active,
            credit_limit,
            interest_rate_bps,
            risk_score,
        },
    );
}

/// Suspend a credit line temporarily.
///
/// Called by admin to freeze a borrower's credit line without closing it.
/// Transition: Active → Suspended.
/// While suspended, draws are disabled but repayments remain allowed.
///
/// # Parameters
/// - `line_id`: The unique identifier for the credit line.
///
/// # Errors
/// - `ContractError::NotAdmin`: If caller is not the contract administrator.
/// - `ContractError::CreditLineNotFound`: If no credit line exists for the given ID.
/// - `ContractError::InvalidStatus`: If the current status is not `Active`.
///
/// # Events
/// Emits a `("credit", "suspend")` [`CreditLineEvent`].
pub fn suspend_credit_line(env: Env, line_id: u32) {
    require_admin_auth(&env);

    let borrower: Address = env
        .storage()
        .persistent()
        .get(&DataKey::LineIdMap(line_id))
        .unwrap_or_else(|| {
            env.panic_with_error(ContractError::CreditLineNotFound);
        });

    let mut credit_line: CreditLineData = env
        .storage()
        .persistent()
        .get(&borrower)
        .unwrap_or_else(|| {
            env.panic_with_error(ContractError::CreditLineNotFound);
        });

    if credit_line.status != CreditStatus::Active {
        env.panic_with_error(ContractError::InvalidStatus);
    }

    credit_line.status = CreditStatus::Suspended;
    env.storage().persistent().set(&borrower, &credit_line);

    publish_credit_line_event(
        &env,
        (symbol_short!("credit"), symbol_short!("suspend")),
        CreditLineEvent {
            event_type: symbol_short!("suspend"),
            line_id,
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
            line_id: credit_line.line_id,
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

    credit_line.status = CreditStatus::Defaulted;
    env.storage().persistent().set(&borrower, &credit_line);

    publish_credit_line_event(
        &env,
        (symbol_short!("credit"), symbol_short!("default")),
        CreditLineEvent {
            event_type: symbol_short!("default"),
            line_id: credit_line.line_id,
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
            line_id: credit_line.line_id,
            borrower: borrower.clone(),
            status: CreditStatus::Active,
            credit_limit: credit_line.credit_limit,
            interest_rate_bps: credit_line.interest_rate_bps,
            risk_score: credit_line.risk_score,
        },
    );
}
