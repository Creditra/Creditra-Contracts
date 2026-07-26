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

pub fn draw_status_error(status: CreditStatus) -> Option<ContractError> {
    match status {
        CreditStatus::Active | CreditStatus::Restricted => None,
        CreditStatus::Suspended => Some(ContractError::CreditLineSuspended),
        CreditStatus::Defaulted => Some(ContractError::CreditLineDefaulted),
        CreditStatus::Closed => Some(ContractError::CreditLineClosed),
    }
}

/// Draw funds from the borrower's credit line.
///
/// Transfers `amount` of liquidity tokens from the reserve to the borrower
/// and increases the credit line's utilized amount. This is the primary
/// borrowing operation that converts available credit into actual borrowed funds.
///
/// # What
/// - Validates the credit line exists and belongs to the borrower
/// - Checks the credit line status allows draws (Active or Restricted only)
/// - Ensures the draw amount is positive and within credit limit
/// - Verifies sufficient liquidity reserve exists
/// - Transfers tokens from reserve to borrower
/// - Updates utilized amount and bumps TTL
/// - Emits a DrawnEvent
///
/// # How
/// 1. Sets reentrancy guard to prevent re-entry attacks
/// 2. Requires borrower authentication
/// 3. Validates amount is positive
/// 4. Retrieves liquidity token and reserve addresses
/// 5. Loads the borrower's credit line
/// 6. Validates borrower matches credit line owner
/// 7. Checks credit line status allows draws
/// 8. Calculates new utilized amount with overflow protection
/// 9. Ensures new utilized amount doesn't exceed credit limit
/// 10. Verifies reserve has sufficient liquidity
/// 11. Transfers tokens from reserve to borrower
/// 12. Updates credit line state and persists
/// 13. Extends TTL to prevent expiry
/// 14. Emits DrawnEvent
/// 15. Clears reentrancy guard
///
/// # Why
/// Borrowers need to access their available credit to obtain liquidity for
/// their intended use cases. This function provides the mechanism to convert
/// approved credit limits into actual borrowed funds while maintaining all
/// safety checks and protocol invariants.
///
/// # Errors
/// - `ContractError::InvalidAmount` - if amount <= 0
/// - `ContractError::CreditLineNotFound` - if no credit line exists for borrower
/// - `ContractError::CreditLineSuspended` - if credit line is suspended
/// - `ContractError::CreditLineDefaulted` - if credit line is defaulted
/// - `ContractError::CreditLineClosed` - if credit line is closed
/// - Panics with "Borrower mismatch" - if borrower doesn't match credit line owner
/// - Panics with "overflow" - if utilized amount would overflow
/// - Panics with "exceeds credit limit" - if draw would exceed credit limit
/// - Panics with "Insufficient liquidity reserve" - if reserve lacks funds
pub fn draw_credit(env: Env, borrower: Address, amount: i128) {
    borrower.require_auth();
    set_reentrancy_guard(&env);

    if amount <= 0 {
        clear_reentrancy_guard(&env);
        env.panic_with_error(ContractError::InvalidAmount);
    }

    let token_address: Option<Address> = env.storage().instance().get(&DataKey::LiquidityToken);
    let reserve_address: Address = env
        .storage()
        .instance()
        .get(&DataKey::LiquiditySource)
        .unwrap_or_else(|| env.current_contract_address());

    let mut credit_line: CreditLineData =
        storage_get_credit_line(&env, &borrower).unwrap_or_else(|| {
            clear_reentrancy_guard(&env);
            env.panic_with_error(ContractError::CreditLineNotFound)
        });

    if credit_line.borrower != borrower {
        clear_reentrancy_guard(&env);
        env.panic_with_error(ContractError::BorrowerMismatch);
    }

    if let Some(status_error) = draw_status_error(credit_line.status) {
        clear_reentrancy_guard(&env);
        env.panic_with_error(status_error);
    }

    let updated_utilized = credit_line
        .utilized_amount
        .checked_add(amount)
        .unwrap_or_else(|| {
            clear_reentrancy_guard(&env);
            env.panic_with_error(ContractError::MathOverflow);
        });

    if updated_utilized > credit_line.credit_limit {
        clear_reentrancy_guard(&env);
        env.panic_with_error(ContractError::CreditLimitExceeded);
    }

    if let Some(token_address) = token_address {
        let token_client = token::Client::new(&env, &token_address);
        let reserve_balance = token_client.balance(&reserve_address);
        
        if reserve_balance < amount {
            clear_reentrancy_guard(&env);
            env.panic_with_error(ContractError::InsufficientLiquidity);
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
/// - `effective_repay` has already been transferred (borrower -> reserve).
/// - `interest_repaid` is the interest component of `effective_repay`.
/// - `previous_utilized` is `credit_line.utilized_amount` **before** accrual.
/// - `previous_status` is `credit_line.status` **before** any mutation.
///
/// # Effects
/// - `credit_line.accrued_interest -= interest_repaid`
/// - `credit_line.utilized_amount -= effective_repay`
/// - Persists the credit line via [`persist_credit_line`]
/// - Advances the installment schedule by principal installments only
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
    lifecycle::advance_repayment_schedule_after_repay(
        env,
        borrower,
        effective_repay,
        interest_repaid,
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

/// Repay funds to reduce the borrower's outstanding debt.
///
/// Transfers `amount` of liquidity tokens from the borrower to the reserve
/// and decreases the credit line's utilized amount. Repayment is applied
/// interest-first: any accrued interest is paid down before principal reduction.
///
/// # What
/// - Validates the credit line exists and is not closed
/// - Ensures the repayment amount is positive
/// - Caps effective repayment at outstanding utilized amount (no overpayment)
/// - Calculates interest portion of repayment (interest-first allocation)
/// - Verifies borrower has sufficient allowance and balance
/// - Transfers tokens from borrower to reserve
/// - Updates credit line state via repay_credit_internal
/// - Emits InterestAccruedEvent and RepaymentEvent
///
/// # How
/// 1. Sets reentrancy guard to prevent re-entry attacks
/// 2. Requires borrower authentication
/// 3. Validates amount is positive
/// 4. Loads the borrower's credit line
/// 5. Checks credit line is not closed
/// 6. Calculates effective repayment (capped at utilized amount)
/// 7. Calculates interest portion (min of effective repay and accrued interest)
/// 8. If effective repayment > 0:
///    a. Retrieves liquidity token and reserve addresses
///    b. Verifies borrower has sufficient allowance
///    c. Verifies borrower has sufficient balance
///    d. Transfers tokens from borrower to reserve
/// 9. Captures previous utilized amount and status
/// 10. Calls repay_credit_internal to update state and emit events
/// 11. Clears reentrancy guard
///
/// # Why
/// Borrowers must be able to repay their debt at any time, even if the protocol
/// is paused. This function is NOT pause-gated to ensure borrowers can always
/// deleverage. Interest-first allocation ensures protocol revenue is captured
/// before principal reduction.
///
/// # Errors
/// - `ContractError::InvalidAmount` - if amount <= 0
/// - `ContractError::CreditLineNotFound` - if no credit line exists for borrower
/// - `ContractError::CreditLineClosed` - if credit line is closed
/// - Panics with "Insufficient allowance" - if borrower hasn't approved spend
/// - Panics with "Insufficient balance" - if borrower lacks tokens
pub fn repay_credit(env: Env, borrower: Address, amount: i128) {
    borrower.require_auth();
    set_reentrancy_guard(&env);

    if amount <= 0 {
        clear_reentrancy_guard(&env);
        env.panic_with_error(ContractError::InvalidAmount);
    }

    let mut credit_line: CreditLineData =
        storage_get_credit_line(&env, &borrower).unwrap_or_else(|| {
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
                env.panic_with_error(ContractError::InsufficientRepaymentAllowance);
            }

            let balance = token_client.balance(&borrower);
            if balance < effective_repay {
                clear_reentrancy_guard(&env);
                env.panic_with_error(ContractError::InsufficientRepaymentBalance);
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

/// Atomically repays a specified amount of the borrower's outstanding debt 
/// and releases a proportional share of their deposited collateral.
///
/// The release formula preserves the collateral ratio exactly:
/// `released = collateral_balance * effective_repay / utilized_before`
///
/// # What
/// - Validates the credit line exists and is not closed
/// - Ensures the repayment amount is positive
/// - Caps effective repayment at outstanding utilized amount (no overpayment)
/// - Calculates interest portion of repayment (interest-first allocation)
/// - Verifies borrower has sufficient allowance and balance
/// - Calculates and deducts protocol fee if configured
/// - Transfers fee portion to contract treasury accumulator
/// - Transfers remaining amount to reserve
/// - Calculates proportional collateral release
/// - Releases collateral to borrower
/// - Updates credit line state via repay_credit_internal
/// - Emits InterestAccruedEvent and RepaymentEvent
///
/// # How
/// 1. Sets reentrancy guard to prevent re-entry attacks
/// 2. Requires borrower authentication
/// 3. Validates amount is positive
/// 4. Loads the borrower's credit line
/// 5. Checks credit line is not closed
/// 6. Captures previous utilized amount and status
/// 7. Calculates effective repayment (capped at utilized amount)
/// 8. Calculates interest portion (min of effective repay and accrued interest)
/// 9. If effective repayment > 0:
///    a. Retrieves liquidity token and reserve addresses
///    b. Verifies borrower has sufficient allowance
///    c. Verifies borrower has sufficient balance
///    d. Calculates protocol fee if configured
///    e. Transfers fee portion to contract (treasury accumulator)
///    f. Transfers remaining amount to reserve
/// 10. Calculates proportional collateral release:
///     - If full repay (effective_repay >= utilized_before): release all collateral
///     - Otherwise: release = collateral * effective_repay / utilized_before
/// 11. If release amount > 0: calls release_collateral
/// 12. Calls repay_credit_internal to update state and emit events
/// 13. Clears reentrancy guard
///
/// # Why
/// Borrowers need to deleverage while simultaneously reclaiming collateral
/// as their debt decreases. This atomic operation ensures the collateral ratio
/// remains exactly preserved during partial repayments, preventing rounding errors
/// that could otherwise force liquidation. The protocol fee on repayment provides
/// ongoing revenue to support operations.
///
/// # Full repay
/// When `effective_repay == utilized_before`, all collateral is released
/// (explicit branch avoids rounding residue).
///
/// # Overpayment
/// When `amount > utilized_amount`, `effective_repay` is capped at
/// `utilized_amount`. All collateral is released.
///
/// # Arguments
/// * `env` - The execution environment.
/// * `borrower` - The address of the borrower making the repayment.
/// * `amount` - The total amount being repaid.
pub fn repay_and_release_collateral(env: Env, borrower: Address, amount: i128) {
    borrower.require_auth();
    set_reentrancy_guard(&env);

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
                env.panic_with_error(ContractError::InsufficientAllowance);
            }

            let balance = token_client.balance(&borrower);
            if balance < effective_repay {
                clear_reentrancy_guard(&env);
                env.panic_with_error(ContractError::InsufficientBalance);
            }

            // Compute protocol fee on the total repayment amount.
            let fee_bps: u32 = crate::storage::get_protocol_fee_bps(&env).unwrap_or(0);
            let mut fee: i128 = 0;
            if fee_bps > 0 && effective_repay > 0 {
                fee = apply_bps(effective_repay as u128, fee_bps, Rounding::Floor) as i128;
            }

            // Transfer fee portion into contract (treasury accumulator), then
            // transfer remaining amount into the reserve.
            if fee > 0 {
                token_client.transfer_from(&contract_address, &borrower, &contract_address, &fee);
                crate::fees::accrue_protocol_fee(&env, &borrower, fee);
            }

            let reserve_amount = effective_repay.saturating_sub(fee);
            if reserve_amount > 0 {
                token_client.transfer_from(
                    &contract_address,
                    &borrower,
                    &reserve_address,
                    &reserve_amount,
                );
            }
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