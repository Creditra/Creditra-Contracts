use crate::types::CreditLineData;
use crate::Credit;
use soroban_sdk::{contractimpl, Address, Env};

#[allow(dead_code)]
pub fn get_credit_line(env: Env, borrower: Address) -> Option<CreditLineData> {
        env.storage().persistent().get(&borrower)
    }
