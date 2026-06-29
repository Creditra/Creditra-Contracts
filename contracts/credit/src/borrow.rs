use crate::collateral;
use crate::events::{
    publish_drawn_event, publish_interest_accrued_event, publish_repayment_event, DrawnEvent,
    InterestAccruedEvent, RepaymentEvent,
};
use crate::math_utils::{mul_div, Rounding};
use crate::storage::{
    clear_reentrancy_guard, get_collateral_balance, persist_credit_line, set_reentrancy_guard,
    DataKey, CREDIT_LINE_TTL_EXTEND_TO, CREDIT_LINE_TTL_THRESHOLD,
};
use crate::types::{ContractError, CreditLineData, CreditStatus};
use soroban_sdk::{token, Address, Env};

pub fn draw_credit(env: Env, borrower: Address, amount: i128) {
    set_reentrancy_guard(&env);
    borrower.require_auth();

    if amount <= 0 {
        clear_reentrancy_guard(&env);
        panic!("amount must be positive");
    }

    let token_address: Option<Address> = env.storage().instance().get(&DataKey::LiquidityToken);
    let reserve_address: Address = env
        .storage()
        .instance()
        .get(&DataKey::LiquiditySource)
        .unwrap_or_else(|| env.current_contract_address());

    let mut credit_line: CreditLineData =
        env.storage()
            .persistent()
            .get(&borrower)
            .unwrap_or_else(|| {
                clear_reentrancy_guard(&env);
                env.panic_with_error(ContractError::CreditLineNotFound)
            });

    if credit_line.borrower != borrower {
        clear_reentrancy_guard(&env);
        panic!("Borrower mismatch for credit line");
    }

    if credit_line.status == CreditStatus::Closed {
        clear_reentrancy_guard(&env);
        env.panic_with_error(ContractError::CreditLineClosed);
    }

    if credit_line.status == CreditStatus::Suspended {
        clear_reentrancy_guard(&env);
        panic!("credit line is suspended");
    }

    if credit_line.status == CreditStatus::Defaulted {
        clear_reentrancy_guard(&env);
        panic!("credit line is defaulted");
    }

    if credit_line.status != CreditStatus::Active {
        clear_reentrancy_guard(&env);
        env.panic_with_error(ContractError::InvalidAmount);
    }

    let updated_utilized = credit_line
        .utilized_amount
        .checked_add(amount)
        .unwrap_or_else(|| {
            clear_reentrancy_guard(&env);
            panic!("overflow");
        });

    if updated_utilized > credit_line.credit_limit {
        clear_reentrancy_guard(&env);
        panic!("exceeds credit limit");
    }

    if let Some(token_address) = token_address {
        let token_client = token::Client::new(&env, &token_address);
        let reserve_balance = token_client.balance(&reserve_address);
        if reserve_balance < amount {
            clear_reentrancy_guard(&env);
            panic!("Insufficient liquidity reserve for requested draw amount");
        }

        token_client.transfer(&reserve_address, &borrower, &amount);
    }

    credit_line.utilized_amount = updated_utilized;
    env.storage().persistent().set(&borrower, &credit_line);
    // Bump TTL: every draw is an interaction that resets the expiry window.
    env.storage().persistent().extend_ttl(
        &borrower,
        CREDIT_LINE_TTL_THRESHOLD,
        CREDIT_LINE_TTL_EXTEND_TO,
    );
    let _timestamp = env.ledger().timestamp();
    publish_drawn_event(
        &env,
        DrawnEvent {
            borrower,
            amount,
            new_utilized_amount: updated_utilized,
        },
    );
    clear_reentrancy_guard(&env);
}

/// Finalize a repayment: update credit line state, persist, and emit events.
///
/// This is the single source of truth for post-transfer repay bookkeeping.
/// Both [`repay_credit`] and [`repay_and_release_collateral`] call this
/// helper to avoid duplicating financial logic.
///
/// # Preconditions
/// - `effective_repay` has already been transferred (borrower → reserve).
/// - `interest_repaid` is the interest component of `effective_repay`.
/// - `previous_utilized` is `credit_line.utilized_amount` **before** accrual.
/// - `previous_status` is `credit_line.status` **before** any mutation.
///
/// # Effects
/// - `credit_line.accrued_interest -= interest_repaid`
/// - `credit_line.utilized_amount -= effective_repay`
/// - Persists the credit line via [`persist_credit_line`]
/// - Emits [`InterestAccruedEvent`] and [`RepaymentEvent`]
pub(crate) fn repay_credit_internal(
    env: &Env,
    borrower: &Address,
    credit_line: &mut CreditLineData,
    effective_repay: i128,
    interest_repaid: i128,
    previous_utilized: i128,
    previous_status: CreditStatus,
) {
    credit_line.accrued_interest = credit_line
        .accrued_interest
        .checked_sub(interest_repaid)
        .unwrap_or(0);

    let new_utilized = credit_line
        .utilized_amount
        .saturating_sub(effective_repay)
        .max(0);
    credit_line.utilized_amount = new_utilized;

    persist_credit_line(
        env,
        borrower,
        credit_line,
        previous_utilized,
        Some(previous_status),
    );

    publish_interest_accrued_event(
        env,
        InterestAccruedEvent {
            borrower: borrower.clone(),
            accrued_amount: 0,
            new_utilized_amount: new_utilized,
        },
    );
    publish_repayment_event(
        env,
        RepaymentEvent {
            borrower: borrower.clone(),
            amount: effective_repay,
            new_utilized_amount: new_utilized,
        },
    );
}

pub fn repay_credit(env: Env, borrower: Address, amount: i128) {
    set_reentrancy_guard(&env);
    borrower.require_auth();

    if amount <= 0 {
        clear_reentrancy_guard(&env);
        env.panic_with_error(ContractError::InvalidAmount);
    }

    let mut credit_line: CreditLineData =
        env.storage()
            .persistent()
            .get(&borrower)
            .unwrap_or_else(|| {
                clear_reentrancy_guard(&env);
                env.panic_with_error(ContractError::CreditLineNotFound)
            });

    if credit_line.status == CreditStatus::Closed {
        clear_reentrancy_guard(&env);
        env.panic_with_error(ContractError::CreditLineClosed);
    }

    let effective_repay = if amount > credit_line.utilized_amount {
        credit_line.utilized_amount
    } else {
        amount
    };

    let interest_repaid = effective_repay.min(credit_line.accrued_interest);

    if effective_repay > 0 {
        let token_address: Option<Address> = env.storage().instance().get(&DataKey::LiquidityToken);

        if let Some(token_address) = token_address {
            let reserve_address: Address = env
                .storage()
                .instance()
                .get(&DataKey::LiquiditySource)
                .unwrap_or_else(|| env.current_contract_address());

            let token_client = token::Client::new(&env, &token_address);
            let contract_address = env.current_contract_address();

            let allowance = token_client.allowance(&borrower, &contract_address);
            if allowance < effective_repay {
                clear_reentrancy_guard(&env);
                panic!("Insufficient allowance");
            }

            let balance = token_client.balance(&borrower);
            if balance < effective_repay {
                clear_reentrancy_guard(&env);
                panic!("Insufficient balance");
            }

            token_client.transfer_from(
                &contract_address,
                &borrower,
                &reserve_address,
                &effective_repay,
            );
        }
    }

    let previous_utilized = credit_line.utilized_amount;
    let previous_status = credit_line.status;

    repay_credit_internal(
        &env,
        &borrower,
        &mut credit_line,
        effective_repay,
        interest_repaid,
        previous_utilized,
        previous_status,
    );

    clear_reentrancy_guard(&env);
}

/// Atomic repay that also releases proportional collateral.
///
/// Repays `amount` of the borrower's outstanding debt and simultaneously
/// returns a proportional share of their deposited collateral. The release
/// formula is:
///
/// ```text
/// released = collateral_balance * effective_repay / utilized_before
/// ```
///
/// This preserves the collateral ratio exactly (verified by the linear
/// `required = utilized * ratio / 10_000` constraint).
///
/// # Full repay
/// When `effective_repay == utilized_before`, all collateral is released
/// (explicit branch avoids rounding residue).
///
/// # Overpayment
/// When `amount > utilized_amount`, `effective_repay` is capped at
/// `utilized_amount`. All collateral is released.
pub fn repay_and_release_collateral(env: Env, borrower: Address, amount: i128) {
    set_reentrancy_guard(&env);
    borrower.require_auth();

    if amount <= 0 {
        clear_reentrancy_guard(&env);
        env.panic_with_error(ContractError::InvalidAmount);
    }

    let mut credit_line: CreditLineData =
        env.storage()
            .persistent()
            .get(&borrower)
            .unwrap_or_else(|| {
                clear_reentrancy_guard(&env);
                env.panic_with_error(ContractError::CreditLineNotFound)
            });

    if credit_line.status == CreditStatus::Closed {
        clear_reentrancy_guard(&env);
        env.panic_with_error(ContractError::CreditLineClosed);
    }

    let previous_utilized = credit_line.utilized_amount;
    let previous_status = credit_line.status;

    let effective_repay = if amount > credit_line.utilized_amount {
        credit_line.utilized_amount
    } else {
        amount
    };

    let interest_repaid = effective_repay.min(credit_line.accrued_interest);

    // --- Token transfer (repayment) ---
    if effective_repay > 0 {
        let token_address: Option<Address> = env.storage().instance().get(&DataKey::LiquidityToken);

        if let Some(token_address) = token_address {
            let reserve_address: Address = env
                .storage()
                .instance()
                .get(&DataKey::LiquiditySource)
                .unwrap_or_else(|| env.current_contract_address());

            let token_client = token::Client::new(&env, &token_address);
            let contract_address = env.current_contract_address();

            let allowance = token_client.allowance(&borrower, &contract_address);
            if allowance < effective_repay {
                clear_reentrancy_guard(&env);
                panic!("Insufficient allowance");
            }

            let balance = token_client.balance(&borrower);
            if balance < effective_repay {
                clear_reentrancy_guard(&env);
                panic!("Insufficient balance");
            }

            token_client.transfer_from(
                &contract_address,
                &borrower,
                &reserve_address,
                &effective_repay,
            );
        }
    }

    // --- Calculate proportional collateral release ---
    // Must happen BEFORE state update (uses old utilized_amount).
    let collateral_balance = get_collateral_balance(&env, &borrower);
    if collateral_balance > 0 && effective_repay > 0 && previous_utilized > 0 {
        let released = if effective_repay >= previous_utilized {
            // Full repay: release all collateral (avoids rounding residue).
            collateral_balance
        } else {
            mul_div(
                collateral_balance as u128,
                effective_repay as u128,
                previous_utilized as u128,
                Rounding::Floor,
            ) as i128
        };

        if released > 0 {
            collateral::release_collateral(&env, &borrower, released);
        }
    }

    // --- Finalize repay (state update + persist + events) ---
    repay_credit_internal(
        &env,
        &borrower,
        &mut credit_line,
        effective_repay,
        interest_repaid,
        previous_utilized,
        previous_status,
    );

    clear_reentrancy_guard(&env);
}

/// Return a `ContractError` if `status` should block draws, otherwise `None`.
///
/// Called by `draw_credit` to centralize status → error mapping.
pub fn draw_status_error(status: CreditStatus) -> Option<ContractError> {
    match status {
        CreditStatus::Active => None,
        CreditStatus::Suspended => Some(ContractError::CreditLineSuspended),
        CreditStatus::Defaulted => Some(ContractError::CreditLineDefaulted),
        CreditStatus::Closed => Some(ContractError::CreditLineClosed),
        CreditStatus::Restricted => None,
    }
}
