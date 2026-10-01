// SPDX-License-Identifier: MIT

use creditra_credit::types::ContractError;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Ledger, MockAuth, MockAuthInvoke};
use soroban_sdk::{token, Address, Env, IntoVal};

fn setup<'a>(
    env: &'a Env,
) -> (
    Address,
    Address,
    Address,
    Address,
    Address,
    Address,
    CreditClient<'a>,
) {
    env.mock_all_auths_allowing_non_root_auth();

    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let treasury = Address::generate(env);
    let bounty = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token_address = token_id.address();

    client.set_liquidity_token(&token_address);
    client.set_liquidity_source(&contract_id);
    client.set_treasury(&admin, &treasury);
    client.set_bounty(&admin, &bounty);

    (
        contract_id,
        token_address,
        admin,
        borrower,
        treasury,
        bounty,
        client,
    )
}

fn prepare_repay<'a>(
    env: &Env,
    contract_id: &Address,
    token_address: &Address,
    borrower: &Address,
    client: &CreditClient<'a>,
    draw_amount: i128,
    repay_amount: i128,
    fee_bps: u32,
) {
    // One year at 100% interest on 11_000 principal accrues 11_000 interest.
    // The maximum 10% protocol fee then skims exactly 110 from a 1_100
    // interest repayment, which makes the fee split assertions observable.
    client.open_credit_line(borrower, &draw_amount, &10_000_u32, &50_u32);

    let asset = token::StellarAssetClient::new(env, token_address);
    let collateral = draw_amount * 3;
    asset.mint(borrower, &collateral);
    client.deposit_collateral(borrower, &collateral);

    asset.mint(contract_id, &draw_amount);
    client.draw_credit(borrower, &draw_amount);
    client.set_protocol_fee_bps(&fee_bps);

    env.ledger()
        .with_mut(|ledger| ledger.timestamp = 31_557_600);

    asset.mint(borrower, &repay_amount);
    let expiration = 6_000_000_u32;
    token::Client::new(env, token_address).approve(
        borrower,
        contract_id,
        &repay_amount,
        &expiration,
    );
}

#[test]
fn fee_split_default_is_all_treasury() {
    let env = Env::default();
    let (contract_id, token_address, _admin, borrower, _treasury, _bounty, client) = setup(&env);
    prepare_repay(
        &env,
        &contract_id,
        &token_address,
        &borrower,
        &client,
        11_000,
        1_100,
        1_000,
    );

    assert_eq!(client.get_treasury_fee_share_bps(), None);

    client.repay_credit(&borrower, &1_100);

    let summary = client.get_protocol_summary();
    assert_eq!(summary.treasury_balance, 110);
    assert_eq!(summary.bounty_balance, 0);
}

#[test]
fn fee_split_even_ratio_splits_fee_between_pools() {
    let env = Env::default();
    let (contract_id, token_address, _admin, borrower, _treasury, _bounty, client) = setup(&env);
    prepare_repay(
        &env,
        &contract_id,
        &token_address,
        &borrower,
        &client,
        11_000,
        1_100,
        1_000,
    );

    client.set_treasury_fee_share_bps(&5_000_u32);
    client.repay_credit(&borrower, &1_100);

    let summary = client.get_protocol_summary();
    assert_eq!(summary.treasury_balance, 55);
    assert_eq!(summary.bounty_balance, 55);
}

#[test]
fn fee_split_remainder_goes_to_bounty_on_rounding() {
    let env = Env::default();
    let (contract_id, token_address, _admin, borrower, _treasury, _bounty, client) = setup(&env);
    prepare_repay(
        &env,
        &contract_id,
        &token_address,
        &borrower,
        &client,
        11_000,
        1_100,
        1_000,
    );

    client.set_treasury_fee_share_bps(&3_333_u32);
    client.repay_credit(&borrower, &1_100);

    let summary = client.get_protocol_summary();
    // Deterministic largest-remainder split of 110 at a 3333/6667 ratio:
    // treasury floor = 36, bounty floor = 73, leftover unit goes to the larger
    // fractional claim (treasury), so 37 / 73. Sum is always conserved (= 110).
    assert_eq!(summary.treasury_balance, 37);
    assert_eq!(summary.bounty_balance, 73);
    assert_eq!(summary.treasury_balance + summary.bounty_balance, 110);
}

#[test]
fn fee_split_all_bounty_when_share_is_zero() {
    let env = Env::default();
    let (contract_id, token_address, _admin, borrower, _treasury, _bounty, client) = setup(&env);
    prepare_repay(
        &env,
        &contract_id,
        &token_address,
        &borrower,
        &client,
        11_000,
        1_100,
        1_000,
    );

    client.set_treasury_fee_share_bps(&0_u32);
    client.repay_credit(&borrower, &1_100);

    let summary = client.get_protocol_summary();
    assert_eq!(summary.treasury_balance, 0);
    assert_eq!(summary.bounty_balance, 110);
}

#[test]
fn withdraw_bounty_transfers_only_bounty_share_and_second_call_is_noop() {
    let env = Env::default();
    let (contract_id, token_address, admin, borrower, treasury, bounty, client) = setup(&env);
    prepare_repay(
        &env,
        &contract_id,
        &token_address,
        &borrower,
        &client,
        11_000,
        1_100,
        1_000,
    );

    client.set_treasury_fee_share_bps(&7_000_u32);
    client.repay_credit(&borrower, &1_100);

    let token_client = token::Client::new(&env, &token_address);
    let contract_before = token_client.balance(&contract_id);
    let treasury_before = token_client.balance(&treasury);
    assert_eq!(token_client.balance(&bounty), 0);
    let before = client.get_protocol_summary();
    assert_eq!(before.treasury_balance, 77);
    assert_eq!(before.bounty_balance, 33);

    client.withdraw_bounty(&admin);

    assert_eq!(token_client.balance(&bounty), 33);
    assert_eq!(token_client.balance(&contract_id), contract_before - 33);
    assert_eq!(token_client.balance(&treasury), treasury_before);
    let after = client.get_protocol_summary();
    assert_eq!(after.treasury_balance, 77);
    assert_eq!(after.bounty_balance, 0);

    client.withdraw_bounty(&admin);

    assert_eq!(token_client.balance(&bounty), 33);
    assert_eq!(token_client.balance(&contract_id), contract_before - 33);
    assert_eq!(token_client.balance(&treasury), treasury_before);
    let after_second = client.get_protocol_summary();
    assert_eq!(after_second.treasury_balance, 77);
    assert_eq!(after_second.bounty_balance, 0);
}

#[test]
fn withdraw_bounty_with_configured_address_and_empty_pool_is_noop() {
    let env = Env::default();
    let (contract_id, token_address, admin, _borrower, treasury, bounty, client) = setup(&env);
    let token_client = token::Client::new(&env, &token_address);
    let contract_before = token_client.balance(&contract_id);
    let treasury_before = token_client.balance(&treasury);
    let bounty_before = token_client.balance(&bounty);

    client.withdraw_bounty(&admin);

    assert_eq!(token_client.balance(&contract_id), contract_before);
    assert_eq!(token_client.balance(&treasury), treasury_before);
    assert_eq!(token_client.balance(&bounty), bounty_before);
    let summary = client.get_protocol_summary();
    assert_eq!(summary.treasury_balance, 0);
    assert_eq!(summary.bounty_balance, 0);
}

#[test]
fn withdraw_bounty_rejects_non_admin_without_transfers_or_state_change() {
    let env = Env::default();
    let (contract_id, token_address, admin, borrower, treasury, bounty, client) = setup(&env);
    prepare_repay(
        &env,
        &contract_id,
        &token_address,
        &borrower,
        &client,
        11_000,
        1_100,
        1_000,
    );
    client.set_treasury_fee_share_bps(&7_000_u32);
    client.repay_credit(&borrower, &1_100);

    let token_client = token::Client::new(&env, &token_address);
    let contract_before = token_client.balance(&contract_id);
    let treasury_before = token_client.balance(&treasury);
    let bounty_before = token_client.balance(&bounty);
    let summary_before = client.get_protocol_summary();
    assert_eq!(summary_before.treasury_balance, 77);
    assert_eq!(summary_before.bounty_balance, 33);

    let non_admin = Address::generate(&env);
    let result = client
        .mock_auths(&[MockAuth {
            address: &non_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "withdraw_bounty",
                args: (non_admin.clone(),).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_withdraw_bounty(&non_admin);
    assert!(result.is_err());

    assert_eq!(token_client.balance(&contract_id), contract_before);
    assert_eq!(token_client.balance(&treasury), treasury_before);
    assert_eq!(token_client.balance(&bounty), bounty_before);
    let summary_after = client.get_protocol_summary();
    assert_eq!(
        summary_after.treasury_balance,
        summary_before.treasury_balance
    );
    assert_eq!(summary_after.bounty_balance, summary_before.bounty_balance);

    // Even a valid admin signature cannot authorize a different admin argument.
    let mismatched_admin = client
        .mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "withdraw_bounty",
                args: (non_admin.clone(),).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_withdraw_bounty(&non_admin);
    assert_eq!(
        mismatched_admin.err().unwrap().unwrap(),
        ContractError::NotAdmin.into()
    );
    assert_eq!(token_client.balance(&contract_id), contract_before);
    assert_eq!(token_client.balance(&bounty), bounty_before);

    // Positive control: the same scoped auth mechanism permits the real admin.
    client
        .mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "withdraw_bounty",
                args: (admin.clone(),).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .withdraw_bounty(&admin);
    assert_eq!(token_client.balance(&bounty), bounty_before + 33);
    assert_eq!(client.get_protocol_summary().bounty_balance, 0);
}

#[test]
fn withdraw_bounty_without_address_reverts() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let result = client.try_withdraw_bounty(&admin);
    assert!(result.is_err());
    assert_eq!(
        result.err().unwrap().unwrap(),
        ContractError::BountyNotSet.into()
    );
}

#[test]
fn set_treasury_fee_share_bps_rejects_above_max() {
    let env = Env::default();
    let (_contract_id, _token_address, _admin, _borrower, _treasury, _bounty, client) = setup(&env);

    let result = client.try_set_treasury_fee_share_bps(&10_001_u32);
    assert!(result.is_err());
    assert_eq!(
        result.err().unwrap().unwrap(),
        ContractError::Overflow.into()
    );
}

#[test]
fn get_bounty_returns_configured_address() {
    let env = Env::default();
    let (_contract_id, _token_address, _admin, _borrower, _treasury, bounty, client) = setup(&env);
    assert_eq!(client.get_bounty(), Some(bounty));
}
