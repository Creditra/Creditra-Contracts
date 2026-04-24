// SPDX-License-Identifier: MIT
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(coverage_nightly, coverage(off))]
#![allow(dead_code)]

use crate::events::{publish_drawn_event, DrawnEvent};
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
