#![no_std]
#![allow(clippy::unused_unit)]

mod events;
mod types;

use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, token, Address, Env, Symbol,
};

use events::{
    publish_credit_line_event, publish_drawn_event, publish_repayment_event,
    publish_risk_parameters_updated, CreditLineEvent, DrawnEvent, RepaymentEvent,
    RiskParametersUpdatedEvent,
};
use types::{CreditLineData, CreditStatus};

const MAX_INTEREST_RATE_BPS: u32 = 10_000;
const MAX_RISK_SCORE: u32 = 100;

fn reentrancy_key(env: &Env) -> Symbol { Symbol::new(env, "reentrancy") }
fn admin_key(env: &Env) -> Symbol { Symbol::new(env, "admin") }

fn require_admin(env: &Env) -> Address {
    env.storage().instance().get(&admin_key(env)).expect("admin not set")
}

fn require_admin_auth(env: &Env) -> Address {
    let admin = require_admin(env);
    admin.require_auth();
    admin
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    LiquidityToken,
    LiquiditySource,
}

fn set_reentrancy_guard(env: &Env) {
    let key = reentrancy_key(env);
    let current: bool = env.storage().instance().get(&key).unwrap_or(false);
    if current { panic!("reentrancy guard"); }
    env.storage().instance().set(&key, &true);
}

fn clear_reentrancy_guard(env: &Env) {
    env.storage().instance().set(&reentrancy_key(env), &false);
}

#[contract]
pub struct Credit;

#[contractimpl]
impl Credit {
    pub fn init(env: Env, admin: Address) {
        env.storage().instance().set(&admin_key(&env), &admin);
        env.storage().instance().set(&DataKey::LiquiditySource, &env.current_contract_address());
    }

    pub fn set_liquidity_token(env: Env, token_address: Address) {
        require_admin_auth(&env);
        env.storage().instance().set(&DataKey::LiquidityToken, &token_address);
    }

    pub fn set_liquidity_source(env: Env, reserve_address: Address) {
        require_admin_auth(&env);
        env.storage().instance().set(&DataKey::LiquiditySource, &reserve_address);
    }

    pub fn open_credit_line(env: Env, borrower: Address, credit_limit: i128, interest_rate_bps: u32, risk_score: u32) {
        assert!(credit_limit > 0, "credit_limit must be greater than zero");
        assert!(interest_rate_bps <= MAX_INTEREST_RATE_BPS, "interest_rate_bps cannot exceed 10000 (100%)");
        assert!(risk_score <= MAX_RISK_SCORE, "risk_score must be between 0 and 100");

        if let Some(existing) = env.storage().persistent().get::<Address, CreditLineData>(&borrower) {
            assert!(existing.status != CreditStatus::Active, "borrower already has an active credit line");
        }

        let credit_line = CreditLineData {
            borrower: borrower.clone(),
            credit_limit,
            utilized_amount: 0,
            interest_rate_bps,
            risk_score,
            status: CreditStatus::Active,
            last_rate_update_ts: env.ledger().timestamp(),
        };

        env.storage().persistent().set(&borrower, &credit_line);
        publish_credit_line_event(&env, (symbol_short!("credit"), symbol_short!("opened")), CreditLineEvent {
            event_type: symbol_short!("opened"), borrower, status: CreditStatus::Active,
            credit_limit, interest_rate_bps, risk_score,
        });
    }

    pub fn draw_credit(env: Env, borrower: Address, amount: i128) {
        set_reentrancy_guard(&env);
        borrower.require_auth();
        if amount <= 0 { panic!("amount must be positive"); }

        let mut line: CreditLineData = env.storage().persistent().get(&borrower).expect("Credit line not found");
        if line.status != CreditStatus::Active { panic!("Credit line not active"); }

        let updated = line.utilized_amount.checked_add(amount).expect("overflow");
        if updated > line.credit_limit { panic!("exceeds credit limit"); }

        let token_addr: Option<Address> = env.storage().instance().get(&DataKey::LiquidityToken);
        let source: Address = env.storage().instance().get(&DataKey::LiquiditySource).unwrap_or(env.current_contract_address());

        if let Some(addr) = token_addr {
            let client = token::Client::new(&env, &addr);
            if client.balance(&source) < amount { panic!("Insufficient liquidity reserve for requested draw amount"); }
            client.transfer(&source, &borrower, &amount);
        }

        line.utilized_amount = updated;
        env.storage().persistent().set(&borrower, &line);
        publish_drawn_event(&env, DrawnEvent { borrower, amount, new_utilized_amount: updated, timestamp: env.ledger().timestamp() });
        clear_reentrancy_guard(&env);
    }

    pub fn repay_credit(env: Env, borrower: Address, amount: i128) {
        set_reentrancy_guard(&env);
        borrower.require_auth();
        let mut line: CreditLineData = env.storage().persistent().get(&borrower).expect("Credit line not found");
        if line.status == CreditStatus::Closed { panic!("credit line is closed"); }
        if amount <= 0 { panic!("amount must be positive"); }

        let new_util = line.utilized_amount.saturating_sub(amount).max(0);
        line.utilized_amount = new_util;
        env.storage().persistent().set(&borrower, &line);
        publish_repayment_event(&env, RepaymentEvent { borrower, amount, new_utilized_amount: new_util, timestamp: env.ledger().timestamp() });
        clear_reentrancy_guard(&env);
    }

    pub fn update_risk_parameters(env: Env, borrower: Address, credit_limit: i128, interest_rate_bps: u32, risk_score: u32) {
        require_admin_auth(&env);
        let mut line: CreditLineData = env.storage().persistent().get(&borrower).expect("Credit line not found");
        if credit_limit < 0 || credit_limit < line.utilized_amount { panic!("invalid credit_limit"); }
        line.credit_limit = credit_limit;
        line.interest_rate_bps = interest_rate_bps;
        line.risk_score = risk_score;
        env.storage().persistent().set(&borrower, &line);
        publish_risk_parameters_updated(&env, RiskParametersUpdatedEvent { borrower, credit_limit, interest_rate_bps, risk_score });
    }

    pub fn suspend_credit_line(env: Env, borrower: Address) {
        require_admin_auth(&env);
        let mut line: CreditLineData = env.storage().persistent().get(&borrower).expect("Credit line not found");
        line.status = CreditStatus::Suspended;
        env.storage().persistent().set(&borrower, &line);
    }

    pub fn close_credit_line(env: Env, borrower: Address, closer: Address) {
        closer.require_auth();
        let mut line: CreditLineData = env.storage().persistent().get(&borrower).expect("Credit line not found");
        if line.status == CreditStatus::Closed { return; }
        if closer != require_admin(&env) && (closer != borrower || line.utilized_amount != 0) { panic!("unauthorized"); }
        line.status = CreditStatus::Closed;
        env.storage().persistent().set(&borrower, &line);
    }

    pub fn default_credit_line(env: Env, borrower: Address) {
        require_admin_auth(&env);
        let mut line: CreditLineData = env.storage().persistent().get(&borrower).expect("Credit line not found");
        line.status = CreditStatus::Defaulted;
        env.storage().persistent().set(&borrower, &line);
    }

    pub fn reinstate_credit_line(env: Env, borrower: Address) {
        require_admin_auth(&env);
        let mut line: CreditLineData = env.storage().persistent().get(&borrower).expect("Credit line not found");
        if line.status != CreditStatus::Defaulted { panic!("credit line is not defaulted"); }
        line.status = CreditStatus::Active;
        env.storage().persistent().set(&borrower, &line);
    }

    pub fn get_credit_line(env: Env, borrower: Address) -> Option<CreditLineData> {
        env.storage().persistent().get(&borrower)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[test]
    fn test_init_and_open_credit_line() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);
        client.open_credit_line(&borrower, &1000, &300, &70);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.credit_limit, 1000);
        assert_eq!(line.status, CreditStatus::Active);
    }

    #[test]
    fn test_draw_credit_updates_utilized() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);
        client.open_credit_line(&borrower, &1000, &300, &70);
        client.draw_credit(&borrower, &200);
        assert_eq!(client.get_credit_line(&borrower).unwrap().utilized_amount, 200);
    }
}
