// SPDX-License-Identifier: MIT

use crate::events::{publish_drawn_event, publish_repayment_event, DrawnEvent, RepaymentEvent};
use crate::storage::{clear_reentrancy_guard, set_reentrancy_guard, DataKey};
use crate::types::{CreditLineData, CreditStatus};
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
        .unwrap_or(env.current_contract_address());

    let mut credit_line: CreditLineData = env
        .storage()
        .persistent()
        .get(&borrower)
        .expect("Credit line not found");

    if credit_line.status == CreditStatus::Closed {
        clear_reentrancy_guard(&env);
        panic!("credit line is closed");
    }

    let updated_utilized = credit_line
        .utilized_amount
        .checked_add(amount)
        .expect("overflow");

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
    credit_line.last_draw_ts = env.ledger().timestamp();
    env.storage().persistent().set(&borrower, &credit_line);
    let timestamp = env.ledger().timestamp();
    publish_drawn_event(
        &env,
        DrawnEvent {
            borrower,
            amount,
            new_utilized_amount: updated_utilized,
            timestamp,
        },
    );
    clear_reentrancy_guard(&env);
}

/// Repay credit (borrower).
/// Allowed when status is Active, Suspended, or Defaulted. Reverts if credit line does not exist,
/// is Closed, or borrower has not authorized. Reduces utilized_amount by amount (capped at 0).
pub fn repay_credit(env: Env, borrower: Address, amount: i128) {
    set_reentrancy_guard(&env);
    borrower.require_auth();
    let mut credit_line: CreditLineData = env
        .storage()
        .persistent()
        .get(&borrower)
        .expect("Credit line not found");

    if credit_line.status == CreditStatus::Closed {
        clear_reentrancy_guard(&env);
        panic!("credit line is closed");
    }
    if amount <= 0 {
        clear_reentrancy_guard(&env);
        panic!("amount must be positive");
    }
    
    let effective_repay = amount.min(credit_line.utilized_amount);
    let new_utilized = credit_line.utilized_amount.saturating_sub(effective_repay).max(0);

    credit_line.utilized_amount = new_utilized;
    env.storage().persistent().set(&borrower, &credit_line);

    let timestamp = env.ledger().timestamp();
    publish_repayment_event(
        &env,
        RepaymentEvent {
            borrower: borrower.clone(),
            amount: effective_repay,
            interest_repaid: 0,
            principal_repaid: effective_repay,
            new_utilized_amount: new_utilized,
            new_accrued_interest: credit_line.accrued_interest,
            timestamp,
        },
    );
    clear_reentrancy_guard(&env);
}
