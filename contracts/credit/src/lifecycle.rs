use crate::auth::{require_admin, require_admin_auth};
use crate::events::{publish_credit_line_event, CreditLineEvent};
use crate::storage::assert_not_paused;
use crate::types::{CreditLineData, CreditStatus};
use soroban_sdk::{symbol_short, Address, Env, Symbol};

fn liquidation_settlement_key(borrower: &Address, settlement_id: &Symbol) -> (Symbol, Address, Symbol) {
    (
        symbol_short!("liq_seen"),
        borrower.clone(),
        settlement_id.clone(),
    )
}

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
/// - If the protocol is paused.
///
/// # Events
/// Emits a `("credit", "suspend")` [`CreditLineEvent`].
pub fn suspend_credit_line(env: Env, borrower: Address) {
    assert_not_paused(&env);
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
///   borrower closes while `utilized_amount != 0`, or if the protocol is paused.
///
/// Emits a CreditLineClosed event.
pub fn close_credit_line(env: Env, borrower: Address, closer: Address) {
    assert_not_paused(&env);
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
/// Transitions the credit line to [`CreditStatus::Defaulted`].
///
/// # Valid source statuses
/// - [`CreditStatus::Active`] → Defaulted
/// - [`CreditStatus::Suspended`] → Defaulted
///
/// Closed lines cannot be defaulted (they are permanently closed).
/// Already-Defaulted lines are idempotent (no-op, no event emitted).
///
/// # Effects
/// - `draw_credit` is disabled for the borrower after this call.
/// - `repay_credit` remains allowed so the borrower can reduce their debt.
///
/// # Errors
/// - Panics if the credit line does not exist.
/// - Panics if the caller is not the contract admin.
/// - Panics if the credit line is `Closed`.
///
/// # Events
/// Emits `("credit", "default")` with a [`CreditLineEvent`] payload.
pub fn default_credit_line(env: Env, borrower: Address) {
    assert_not_paused(&env);
    require_admin_auth(&env);
    let mut credit_line: CreditLineData = env
        .storage()
        .persistent()
        .get(&borrower)
        .expect("Credit line not found");

    // Apply interest accrual before any mutation
    credit_line = crate::accrual::apply_accrual(&env, credit_line);

    if credit_line.status == CreditStatus::Closed {
        panic!("cannot default a closed credit line");
    }

    if credit_line.status == CreditStatus::Defaulted {
        // Idempotent: already defaulted, nothing to do.
        return;
    }

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

    publish_default_liquidation_requested_event(
        &env,
        DefaultLiquidationRequestedEvent {
            borrower,
            utilized_amount: credit_line.utilized_amount,
            timestamp: env.ledger().timestamp(),
        },
    );
}

/// Apply auction liquidation proceeds to a defaulted credit line (admin only).
///
/// This hook is accounting-only and intentionally performs no token transfer.
/// Off-chain orchestration is responsible for ensuring auction proceeds are settled
/// into protocol custody before this function is called.
pub fn settle_default_liquidation(
    env: Env,
    borrower: Address,
    recovered_amount: i128,
    settlement_id: Symbol,
) {
    require_admin_auth(&env);

    if recovered_amount <= 0 {
        panic!("recovered amount must be positive");
    }

    let settlement_key = liquidation_settlement_key(&borrower, &settlement_id);
    if env.storage().persistent().has(&settlement_key) {
        panic!("liquidation settlement already applied");
    }

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

    if recovered_amount > credit_line.utilized_amount {
        panic!("recovered amount exceeds utilized amount");
    }

    credit_line.utilized_amount = credit_line
        .utilized_amount
        .checked_sub(recovered_amount)
        .expect("overflow while applying liquidation settlement");

    if credit_line.utilized_amount == 0 {
        credit_line.status = CreditStatus::Closed;
    }

    env.storage().persistent().set(&borrower, &credit_line);
    env.storage().persistent().set(&settlement_key, &true);

    if credit_line.status == CreditStatus::Closed {
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

    publish_default_liquidation_settled_event(
        &env,
        DefaultLiquidationSettledEvent {
            borrower,
            settlement_id,
            recovered_amount,
            remaining_utilized_amount: credit_line.utilized_amount,
            status: credit_line.status,
            timestamp: env.ledger().timestamp(),
        },
    );
}

/// Reinstate a defaulted credit line to Active (admin only).
///
/// Allowed only when status is Defaulted. Transition: Defaulted → Active.
///
/// # Panics
/// - If the protocol is paused.
pub fn reinstate_credit_line(env: Env, borrower: Address) {
    assert_not_paused(&env);
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
