// SPDX-License-Identifier: MIT

//! Token transfer failure and rollback semantics tests for the Credit contract.
//!
//! # Coverage
//! - draw_credit / repay_credit: insufficient reserve or allowance rolls back state
//! - Reentrancy guard lifecycle: failed mid-transfer CPI must not leave the guard set
//! - Fail-then-succeed sequencing for draw and repay (no permanent lock / `Reentrancy`)
//! - Soroban atomicity prevents inconsistent utilization on transfer failures

mod failing_token {
    use creditra_credit::types::ContractError;
    use soroban_sdk::{
        contract, contractimpl, symbol_short, testutils::Address as _, Address, Env,
    };

    type BalanceKey = (soroban_sdk::Symbol, Address);
    type AllowanceKey = (soroban_sdk::Symbol, Address, Address);

    /// In-memory token used to fail `transfer` / `transfer_from` mid CPI without SAC auth quirks.
    #[contract]
    pub struct FailingTokenContract;

    #[contractimpl]
    impl FailingTokenContract {
        pub fn init(env: Env, admin: Address) {
            env.storage()
                .instance()
                .set(&symbol_short!("admin"), &admin);
            env.storage()
                .instance()
                .set(&symbol_short!("fail_tx"), &false);
            env.storage()
                .instance()
                .set(&symbol_short!("fail_txf"), &false);
        }

        pub fn set_fail_transfer(env: Env, fail: bool) {
            let admin: Address = env
                .storage()
                .instance()
                .get(&symbol_short!("admin"))
                .unwrap();
            admin.require_auth();
            env.storage()
                .instance()
                .set(&symbol_short!("fail_tx"), &fail);
        }

        pub fn set_fail_transfer_from(env: Env, fail: bool) {
            let admin: Address = env
                .storage()
                .instance()
                .get(&symbol_short!("admin"))
                .unwrap();
            admin.require_auth();
            env.storage()
                .instance()
                .set(&symbol_short!("fail_txf"), &fail);
        }

        fn balance_key(id: &Address) -> BalanceKey {
            (symbol_short!("bal"), id.clone())
        }

        fn allowance_key(from: &Address, spender: &Address) -> AllowanceKey {
            (symbol_short!("all"), from.clone(), spender.clone())
        }

        fn read_balance(env: &Env, id: &Address) -> i128 {
            env.storage()
                .persistent()
                .get(&Self::balance_key(id))
                .unwrap_or(0)
        }

        fn write_balance(env: &Env, id: &Address, amount: i128) {
            let key = Self::balance_key(id);
            if amount == 0 {
                env.storage().persistent().remove(&key);
            } else {
                env.storage().persistent().set(&key, &amount);
            }
        }

        pub fn mint(env: Env, to: Address, amount: i128) {
            let balance = Self::read_balance(&env, &to);
            Self::write_balance(&env, &to, balance.saturating_add(amount));
        }

        pub fn balance(env: Env, id: Address) -> i128 {
            Self::read_balance(&env, &id)
        }

        pub fn allowance(env: Env, from: Address, spender: Address) -> i128 {
            env.storage()
                .persistent()
                .get(&Self::allowance_key(&from, &spender))
                .unwrap_or(0)
        }

        pub fn approve(
            env: Env,
            from: Address,
            spender: Address,
            amount: i128,
            _expiration_ledger: u32,
        ) {
            from.require_auth();
            env.storage()
                .persistent()
                .set(&Self::allowance_key(&from, &spender), &amount);
        }

        pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
            let fail: bool = env
                .storage()
                .instance()
                .get(&symbol_short!("fail_tx"))
                .unwrap_or(false);
            if fail {
                env.panic_with_error(ContractError::InvalidAmount);
            }
            let from_balance = Self::read_balance(&env, &from);
            if from_balance < amount {
                env.panic_with_error(ContractError::InvalidAmount);
            }
            Self::write_balance(&env, &from, from_balance - amount);
            let to_balance = Self::read_balance(&env, &to);
            Self::write_balance(&env, &to, to_balance.saturating_add(amount));
        }

        pub fn transfer_from(env: Env, spender: Address, from: Address, to: Address, amount: i128) {
            let fail: bool = env
                .storage()
                .instance()
                .get(&symbol_short!("fail_txf"))
                .unwrap_or(false);
            if fail {
                env.panic_with_error(ContractError::InvalidAmount);
            }
            let allowance_key = Self::allowance_key(&from, &spender);
            let allowed: i128 = env.storage().persistent().get(&allowance_key).unwrap_or(0);
            if allowed < amount {
                env.panic_with_error(ContractError::InvalidAmount);
            }
            env.storage()
                .persistent()
                .set(&allowance_key, &(allowed - amount));
            Self::transfer(env, from, to, amount);
        }
    }

    /// Test helper for deploying and configuring [`FailingTokenContract`].
    pub struct FailingToken {
        pub address: Address,
        env: Env,
    }

    impl FailingToken {
        pub fn deploy(env: &Env) -> Self {
            let admin = Address::generate(env);
            let contract_id = env.register(FailingTokenContract, ());
            FailingTokenContractClient::new(env, &contract_id).init(&admin);
            Self {
                address: contract_id,
                env: env.clone(),
            }
        }

        pub fn set_fail_transfer(&self, fail: bool) {
            FailingTokenContractClient::new(&self.env, &self.address).set_fail_transfer(&fail);
        }

        pub fn set_fail_transfer_from(&self, fail: bool) {
            FailingTokenContractClient::new(&self.env, &self.address).set_fail_transfer_from(&fail);
        }

        pub fn address(&self) -> Address {
            self.address.clone()
        }

        pub fn mint(&self, to: &Address, amount: i128) {
            FailingTokenContractClient::new(&self.env, &self.address).mint(to, &amount);
        }

        pub fn approve(&self, from: &Address, spender: &Address, amount: i128, expiry: u32) {
            FailingTokenContractClient::new(&self.env, &self.address)
                .approve(from, spender, &amount, &expiry);
        }

        pub fn transfer(&self, from: &Address, to: &Address, amount: i128) {
            FailingTokenContractClient::new(&self.env, &self.address).transfer(from, to, &amount);
        }
    }
}

use creditra_credit::{Credit, CreditClient};
use failing_token::FailingToken;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, Address, Env, Symbol};

// ── helpers ──────────────────────────────────────────────────────────────────

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    (env, admin, contract_id)
}

fn setup_with_token() -> (Env, Address, Address, Address) {
    let (env, admin, contract_id) = setup();
    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();
    let client = CreditClient::new(&env, &contract_id);
    client.set_liquidity_token(&token_address);
    (env, admin, contract_id, token_address)
}

fn setup_with_failing_token() -> (Env, Address, Address, FailingToken) {
    let (env, admin, contract_id) = setup();
    let failing = FailingToken::deploy(&env);
    let client = CreditClient::new(&env, &contract_id);
    client.set_liquidity_token(&failing.address());
    (env, admin, contract_id, failing)
}

fn reentrancy_guard_active(env: &Env, credit_contract: &Address) -> bool {
    let key = Symbol::new(env, "reentrancy");
    env.as_contract(credit_contract, || {
        env.storage()
            .instance()
            .get::<_, bool>(&key)
            .unwrap_or(false)
    })
}

fn approve_expiry(env: &Env) -> u32 {
    env.ledger().timestamp().saturating_add(10_000) as u32
}

fn assert_guard_cleared(env: &Env, credit_contract: &Address, context: &str) {
    assert!(
        !reentrancy_guard_active(env, credit_contract),
        "{context}: reentrancy guard must be cleared"
    );
}

// ── draw_credit failure rollback ─────────────────────────────────────────────

#[test]
fn draw_credit_insufficient_reserve_rolls_back() {
    let (env, _admin, contract_id, token_address) = setup_with_token();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &400);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.draw_credit(&borrower, &500);
    }));

    assert!(
        result.is_err(),
        "draw_credit should fail on insufficient reserve"
    );

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 0);
    assert_eq!(line.status, creditra_credit::types::CreditStatus::Active);
}

#[test]
fn repay_credit_insufficient_allowance_rolls_back() {
    let (env, _admin, contract_id, token_address) = setup_with_token();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &1_000);
    client.draw_credit(&borrower, &500);

    token::StellarAssetClient::new(&env, &token_address).mint(&borrower, &500);
    token::Client::new(&env, &token_address).approve(
        &borrower,
        &contract_id,
        &200,
        &approve_expiry(&env),
    );

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.repay_credit(&borrower, &500);
    }));

    assert!(
        result.is_err(),
        "repay_credit should fail on insufficient allowance"
    );

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 500);
    assert_eq!(line.accrued_interest, 0);
}

#[test]
fn repay_credit_insufficient_balance_rolls_back() {
    // FailingToken enforces balances in-contract; SAC + mock_all_auths would not fail here.
    let (env, _admin, contract_id, failing_token) = setup_with_failing_token();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);
    failing_token.mint(&contract_id, 1_000);
    client.draw_credit(&borrower, &500);

    failing_token.approve(&borrower, &contract_id, 500, approve_expiry(&env));
    // Draw credits the borrower; drain tokens so repay fails on balance, not allowance.
    let sink = Address::generate(&env);
    failing_token.transfer(&borrower, &sink, 500);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.repay_credit(&borrower, &500);
    }));

    assert!(
        result.is_err(),
        "repay_credit should fail on insufficient balance"
    );

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 500);
}

// ── reentrancy guard: pre-transfer validation failures ───────────────────────

#[test]
fn reentrancy_guard_cleared_on_draw_failure() {
    let (env, _admin, contract_id, token_address) = setup_with_token();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &400);

    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.draw_credit(&borrower, &500);
    }));

    assert_guard_cleared(&env, &contract_id, "after insufficient-reserve draw");

    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &100);
    client.draw_credit(&borrower, &500);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 500);
}

#[test]
fn reentrancy_guard_cleared_on_repay_failure() {
    let (env, _admin, contract_id, token_address) = setup_with_token();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &1_000);
    client.draw_credit(&borrower, &500);

    token::Client::new(&env, &token_address).approve(
        &borrower,
        &contract_id,
        &200,
        &approve_expiry(&env),
    );

    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.repay_credit(&borrower, &500);
    }));

    assert_guard_cleared(&env, &contract_id, "after insufficient-allowance repay");

    token::Client::new(&env, &token_address).approve(
        &borrower,
        &contract_id,
        &500,
        &approve_expiry(&env),
    );
    token::StellarAssetClient::new(&env, &token_address).mint(&borrower, &500);
    client.repay_credit(&borrower, &500);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 0);
}

// ── reentrancy guard: mid-transfer CPI failure (FailingToken) ────────────────

/// Failed `transfer` during draw must roll back utilization and clear the guard so a
/// subsequent draw succeeds (no `ContractError::Reentrancy` lock).
#[test]
fn rollback_draw_fail_then_draw_succeeds_guard_cleared() {
    let (env, _admin, contract_id, failing_token) = setup_with_failing_token();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);
    failing_token.mint(&contract_id, 1_000);

    failing_token.set_fail_transfer(true);
    let fail = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.draw_credit(&borrower, &400);
    }));
    assert!(
        fail.is_err(),
        "draw must fail when token transfer is configured to fail"
    );

    assert_guard_cleared(&env, &contract_id, "after mid-transfer draw failure");
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().utilized_amount,
        0
    );

    failing_token.set_fail_transfer(false);
    client.draw_credit(&borrower, &400);
    assert_guard_cleared(&env, &contract_id, "after successful draw");
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().utilized_amount,
        400
    );

    client.draw_credit(&borrower, &100);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().utilized_amount,
        500
    );
}

/// Failed `transfer_from` during repay must roll back and allow a subsequent repay.
#[test]
fn rollback_repay_fail_then_repay_succeeds_guard_cleared() {
    let (env, _admin, contract_id, failing_token) = setup_with_failing_token();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);
    failing_token.mint(&contract_id, 1_000);
    client.draw_credit(&borrower, &500);

    failing_token.mint(&borrower, 500);
    failing_token.approve(&borrower, &contract_id, 500, approve_expiry(&env));

    failing_token.set_fail_transfer_from(true);
    let fail = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.repay_credit(&borrower, &300);
    }));
    assert!(
        fail.is_err(),
        "repay must fail when token transfer_from is configured to fail"
    );

    assert_guard_cleared(&env, &contract_id, "after mid-transfer repay failure");
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().utilized_amount,
        500
    );

    failing_token.set_fail_transfer_from(false);
    client.repay_credit(&borrower, &300);
    assert_guard_cleared(&env, &contract_id, "after successful repay");
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().utilized_amount,
        200
    );

    client.repay_credit(&borrower, &200);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().utilized_amount,
        0
    );
}
