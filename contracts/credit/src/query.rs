use crate::types::CreditLineData;
use soroban_sdk::{Address, Env};

pub fn get_credit_line(env: Env, borrower: Address) -> Option<CreditLineData> {
    env.storage().persistent().get(&borrower)
}
