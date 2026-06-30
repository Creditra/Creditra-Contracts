// SPDX-License-Identifier: MIT

//! Focused tests for protocol fee on total repayment amount.
//!
//! The protocol fee (`ProtocolFeeBps`) is now applied to the **total**
//! repayment amount (principal + interest), not just the interest component.
//! This file covers:
//!
//! - Fee on principal-only repayment (no interest period)
//! - Fee on mixed principal + interest repayment
//! - Fee via `repay_and_release_collateral` path
//! - Rounding edge: sub-bps fee floors to zero
//! - Zero fee sends everything to reserve
//! - Fee event emission correctness

use creditra_credit::events::FeeAccruedEvent;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Events, Ledger};
use soroban_sdk::{token, Address, Env};

/// Create a minimal environment with a funded credit line ready to repay.
///
/// Returns (env, client, borrower, token_address, reserve_address).
fn setup_minimal() -> (Env, CreditClient<'static>, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = 1_000);

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let reserve = Address::generate(&env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();

    client.set_liquidity_token(&token_address);
    client.set_liquidity_source(&reserve);

    (env, client, borrower, token_address, reserve)
}

/// Open a line, mint reserve, draw, and approve `repay_amount` for the borrower.
fn prepare_repay(
    env: &Env,
    client: &CreditClient,
    borrower: &Address,
    token_address: &Address,
    draw_amount: i128,
    repay_amount: i128,
    interest_rate_bps: u32,
    fee_bps: u32,
) {
    client.open_credit_line(borrower, &draw_amount, &interest_rate_bps, &50_u32);

    let asset = token::StellarAssetClient::new(env, token_address);
    asset.mint(&client.contract_id, &draw_amount);
    client.draw_credit(borrower, &draw_amount);

    // Widen bounds if fee_bps exceeds the default 1000 cap.
    if fee_bps > 1000 {
        client.set_protocol_fee_bounds(&0_u32, &fee_bps);
    }
    client.set_protocol_fee_bps(&fee_bps);

    asset.mint(borrower, &repay_amount);
    token::Client::new(env, token_address).approve(borrower, &client.contract_id, &repay_amount, &u32::MAX);
}

// ── Tests ──────────────────────────────────────────────────────────────────

/// Fee on a principal-only repayment (zero interest elapsed).
#[test]
fn fee_on_principal_only_repayment() {
    let (env, client, borrower, token_address, reserve) = setup_minimal();

    prepare_repay(
        &env,
        &client,
        &borrower,
        &token_address,
        1_000_i128,  // draw
        500_i128,    // repay
        500_u32,     // interest rate bps (5% APR, but no time elapsed)
        500_u32,     // fee bps (5%)
    );

    let token_client = token::Client::new(&env, &token_address);
    let contract_before = token_client.balance(&client.contract_id);
    let reserve_before = token_client.balance(&reserve);

    client.repay_credit(&borrower, &500_i128);

    // fee = floor(500 * 500 / 10000) = 25
    // reserve gets 500 - 25 = 475
    assert_eq!(
        token_client.balance(&client.contract_id),
        contract_before + 25,
        "contract should hold the skimmed fee"
    );
    assert_eq!(
        token_client.balance(&reserve),
        reserve_before + 475,
        "reserve gets repayment minus fee"
    );

    let summary = client.get_protocol_summary();
    assert_eq!(summary.treasury_balance, 25, "treasury balance matches fee");
}

/// Fee on a mixed principal + interest repayment.
#[test]
fn fee_on_mixed_principal_and_interest() {
    let (env, client, borrower, token_address, reserve) = setup_minimal();

    prepare_repay(
        &env,
        &client,
        &borrower,
        &token_address,
        10_000_i128, // draw
        11_000_i128, // repay
        1_000_u32,   // interest rate bps (10%)
        1_000_u32,   // fee bps (10%)
    );

    // Advance one year so interest accrues.
    env.ledger().with_mut(|l| l.timestamp = 31_536_000);

    let token_client = token::Client::new(&env, &token_address);
    let contract_before = token_client.balance(&client.contract_id);
    let reserve_before = token_client.balance(&reserve);

    client.repay_credit(&borrower, &11_000_i128);

    // effective_repay ≈ 10_999 (cap at utilized), fee = floor(10999 * 1000 / 10000) = 1099
    // reserve gets 10999 - 1099 = 9900
    let contract_delta = token_client.balance(&client.contract_id) - contract_before;
    let reserve_delta = token_client.balance(&reserve) - reserve_before;
    let total = contract_delta + reserve_delta;

    assert_eq!(total, 10_999, "total tokens transferred = effective_repay");
    assert_eq!(
        contract_delta, 1_099,
        "fee = 10% of 10999 = 1099"
    );
    assert_eq!(
        reserve_delta, 9_900,
        "reserve = 10999 - 1099 = 9900"
    );
}

/// Fee via `repay_and_release_collateral` path.
#[test]
fn fee_with_repay_and_release_collateral() {
    let (env, client, borrower, token_address, reserve) = setup_minimal();

    let draw = 5_000_i128;
    let collateral = 10_000_i128;

    client.open_credit_line(&borrower, &draw, &500_u32, &50_u32);
    let asset = token::StellarAssetClient::new(&env, &token_address);

    // Fund borrower with collateral + repayment buffer.
    asset.mint(&borrower, &(collateral + draw + 1_000));
    asset.mint(&client.contract_id, &draw);

    client.deposit_collateral(&borrower, &collateral);
    client.draw_credit(&borrower, &draw);

    // Set fee.
    client.set_protocol_fee_bps(&300_u32); // 3% fee

    let repay = 1_000_i128;
    token::Client::new(&env, &token_address)
        .approve(&borrower, &client.contract_id, &repay, &u32::MAX);

    let token_client = token::Client::new(&env, &token_address);
    let contract_before = token_client.balance(&client.contract_id);
    let reserve_before = token_client.balance(&reserve);

    client.repay_and_release_collateral(&borrower, &repay);

    // fee = floor(1000 * 300 / 10000) = 30
    // reserve gets 1000 - 30 = 970
    assert_eq!(
        token_client.balance(&client.contract_id),
        contract_before + 30,
        "fee skimmed in repay_and_release_collateral"
    );
    assert_eq!(
        token_client.balance(&reserve),
        reserve_before + 970,
        "reserve gets remainder"
    );

    let summary = client.get_protocol_summary();
    assert_eq!(summary.treasury_balance, 30);
}

/// Fee event is emitted with correct amounts.
#[test]
fn fee_event_emitted_on_repayment() {
    let (env, client, borrower, token_address, _reserve) = setup_minimal();

    prepare_repay(
        &env,
        &client,
        &borrower,
        &token_address,
        1_000_i128,
        500_i128,
        500_u32,
        500_u32,
    );

    client.repay_credit(&borrower, &500_i128);

    // Locate the FeeAccruedEvent in the event log.
    let events = env.events().all();
    let fee_event = events
        .iter()
        .find(|e| {
            let topic_str = format!("{:?}", e.0);
            topic_str.contains("fee_accrd")
        })
        .expect("FeeAccruedEvent must be emitted");

    let (_topics, data) = fee_event;
    let fee_data: FeeAccruedEvent = data.clone().try_into().expect("valid FeeAccruedEvent");

    assert_eq!(fee_data.borrower, borrower);
    assert_eq!(fee_data.fee_amount, 25, "total fee = 25");
    assert_eq!(fee_data.treasury_amount, 25, "default 100% to treasury");
    assert_eq!(fee_data.bounty_amount, 0);
    assert!(
        fee_data.new_treasury_balance >= fee_data.treasury_amount,
        "new_treasury_balance includes this fee"
    );
}

/// Rounding: sub-basis-point fee floors to zero.
#[test]
fn fee_rounds_to_zero_when_below_one_unit() {
    let (env, client, borrower, token_address, reserve) = setup_minimal();

    prepare_repay(
        &env,
        &client,
        &borrower,
        &token_address,
        10_000_i128,
        5_000_i128,
        500_u32,
        1_u32, // 0.01% — sub-bps on 5000
    );

    let token_client = token::Client::new(&env, &token_address);
    let reserve_before = token_client.balance(&reserve);

    client.repay_credit(&borrower, &5_000_i128);

    // fee = floor(5000 * 1 / 10000) = 0
    // All 5000 goes to reserve.
    assert_eq!(
        token_client.balance(&reserve),
        reserve_before + 5_000,
        "entire repayment goes to reserve when fee rounds to zero"
    );
}

/// Zero fee sends everything to reserve.
#[test]
fn zero_fee_sends_all_to_reserve() {
    let (env, client, borrower, token_address, reserve) = setup_minimal();

    prepare_repay(
        &env,
        &client,
        &borrower,
        &token_address,
        1_000_i128,
        500_i128,
        500_u32,
        0_u32,
    );

    let token_client = token::Client::new(&env, &token_address);
    let contract_before = token_client.balance(&client.contract_id);
    let reserve_before = token_client.balance(&reserve);

    client.repay_credit(&borrower, &500_i128);

    assert_eq!(
        token_client.balance(&client.contract_id),
        contract_before,
        "no fee when fee_bps = 0"
    );
    assert_eq!(
        token_client.balance(&reserve),
        reserve_before + 500,
        "all to reserve when fee_bps = 0"
    );
}
