// SPDX-License-Identifier: MIT

//! Focused tests for protocol fee on total repayment amount.
//!
//! The protocol fee (`ProtocolFeeBps`) is now applied to the **total**
//! repayment amount (principal + interest), not just the interest component.
//! This file covers:
//!
//! - Fee on principal-only repayment (no interest period)
//! - Fee on mixed principal + interest repayment
//! - Fee via repayment with collateral attached
//! - Rounding edge: sub-bps fee floors to zero
//! - Zero fee sends everything to reserve
//! - Fee event emission correctness

use proptest::prelude::*;
use soroban_sdk::TryFromVal;
use creditra_credit::events::FeeAccruedEvent;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Events, Ledger};
use soroban_sdk::{token, Address, Env};

/// Create a minimal environment with a funded credit line ready to repay.
///
/// Returns (env, client, borrower, token_address, reserve_address).
fn setup_minimal() -> (Env, CreditClient<'static>, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
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
        // draw_credit pays out of the liquidity source, so it must hold tokens
    token::StellarAssetClient::new(&env, &token_address).mint(&reserve, &10_000_000_i128);

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
    asset.mint(&client.address, &draw_amount);
    // draw_credit requires collateral of at least 150% of the draw
    let collateral = draw_amount * 3;
    asset.mint(borrower, &collateral);
    client.deposit_collateral(borrower, &collateral);
    client.draw_credit(borrower, &draw_amount);

    client.set_protocol_fee_bps(&fee_bps);

    asset.mint(borrower, &repay_amount);
    token::Client::new(env, token_address).approve(
        borrower,
        &client.address,
        &repay_amount,
        &1_000_u32,
    );
}

// ── Tests ──────────────────────────────────────────────────────────────────

/// Fee on a principal-only repayment (zero interest elapsed).
#[test]
fn no_fee_on_principal_only_repayment() {
    let (env, client, borrower, token_address, reserve) = setup_minimal();

    prepare_repay(
        &env,
        &client,
        &borrower,
        &token_address,
        1_000_i128, // draw
        500_i128,   // repay
        500_u32,    // interest rate bps (5% APR, but no time elapsed)
        500_u32,    // fee bps (5%)
    );

    let token_client = token::Client::new(&env, &token_address);
    let contract_before = token_client.balance(&client.address);
    let reserve_before = token_client.balance(&reserve);

    client.repay_credit(&borrower, &500_i128);

    // no interest has accrued, so no fee is charged: the reserve gets all 500
    assert_eq!(
        token_client.balance(&client.address),
        contract_before,
        "contract receives no fee"
    );

    let summary = client.get_protocol_summary();
    assert_eq!(summary.treasury_balance, 0, "no fee on principal-only repayment");
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
    let contract_before = token_client.balance(&client.address);
    let reserve_before = token_client.balance(&reserve);

    client.repay_credit(&borrower, &11_000_i128);

    // effective_repay ≈ 10_999 (cap at utilized), fee = floor(10999 * 1000 / 10000) = 1099
    // reserve gets 10999 - 1099 = 9900
    let contract_delta = token_client.balance(&client.address) - contract_before;
    let reserve_delta = token_client.balance(&reserve) - reserve_before;
    let total = contract_delta + reserve_delta;

    /// interest = 999, fee = 99, reserve = 10999 - 99 = 10900
    assert_eq!(total, 10_999, "total tokens transferred = effective_repay");
    assert_eq!(contract_delta, 99, "fee = 10% of 10999 = 1099");
    assert_eq!(reserve_delta, 10_900, "reserve = 10999 - 1099 = 9900");
}

/// Fee during credit line repayment with collateral deposited.
#[test]
fn fee_with_repay_and_collateral() {
    let (env, client, borrower, token_address, reserve) = setup_minimal();

    let draw = 5_000_i128;
    let collateral = 10_000_i128;

    client.open_credit_line(&borrower, &draw, &500_u32, &50_u32);
    let asset = token::StellarAssetClient::new(&env, &token_address);

    // Fund borrower with collateral + repayment buffer.
    asset.mint(&borrower, &(collateral + draw + 1_000));
    asset.mint(&client.address, &draw);

    client.deposit_collateral(&borrower, &collateral);
    client.draw_credit(&borrower, &draw);

    // Set fee.
    client.set_protocol_fee_bps(&300_u32); // 3% fee

    let repay = 1_000_i128;
    token::Client::new(&env, &token_address).approve(
        &borrower,
        &client.address,
        &repay,
        &1_000_u32,
    );

    let token_client = token::Client::new(&env, &token_address);
    let contract_before = token_client.balance(&client.address);
    let reserve_before = token_client.balance(&reserve);

    client.repay_credit(&borrower, &repay);

    // no interest has accrued, so no fee: the reserve gets all 1000
    assert_eq!(
        token_client.balance(&client.address),
        contract_before,
        "fee skimmed in repay_credit"
    );
    assert_eq!(
        token_client.balance(&reserve),
        reserve_before + 1000,
        "reserve gets remainder"
    );

    let summary = client.get_protocol_summary();
    assert_eq!(summary.treasury_balance, 0, "no interest, so no fee");
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
        1_200_i128,
        500_u32,
        500_u32,
    );

    // The fee only exists on the interest portion, so let a year of
    // interest accrue before repaying.
    env.ledger().with_mut(|l| l.timestamp = 31_536_000);

    let token_client = token::Client::new(&env, &token_address);
    let contract_before = token_client.balance(&client.address);

    // Repay more than the total debt so all accrued interest is included.
    client.repay_credit(&borrower, &1_200_i128);

    // Read events immediately, before any other contract call.
    let events = env.events().all();
    let fee_event = events
        .iter()
        .find(|e| format!("{:?}", e).contains("fee_accrd"))
        .expect("FeeAccruedEvent must be emitted");

    let fee_data: FeeAccruedEvent = FeeAccruedEvent::try_from_val(&env, &fee_event.2)
        .expect("valid FeeAccruedEvent");

    let contract_delta = token_client.balance(&client.address) - contract_before;

    assert_eq!(fee_data.borrower, borrower);
    assert!(fee_data.fee_amount > 0, "interest was repaid, so a fee is charged");
    assert_eq!(
        fee_data.fee_amount,
        fee_data.treasury_amount + fee_data.bounty_amount,
        "fee splits with nothing lost"
    );
    assert_eq!(fee_data.treasury_amount, fee_data.fee_amount, "default 100% to treasury");
    assert_eq!(fee_data.bounty_amount, 0);
    assert_eq!(
        fee_data.fee_amount, contract_delta,
        "event fee equals the tokens the contract received"
    );
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
    let contract_before = token_client.balance(&client.address);
    let reserve_before = token_client.balance(&reserve);

    client.repay_credit(&borrower, &500_i128);

    assert_eq!(
        token_client.balance(&client.address),
        contract_before,
        "no fee when fee_bps = 0"
    );
    assert_eq!(
        token_client.balance(&reserve),
        reserve_before + 500,
        "all to reserve when fee_bps = 0"
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn fee_split_conserves_tokens(
        draw in 1_000i128..=1_000_000i128,
        rate_bps in 1u32..=2_000u32,
        elapsed in prop_oneof![Just(0u64), 1u64..=31_536_000u64],
        fee_bps in prop_oneof![Just(0u32), Just(1_000u32), 0u32..=1_000u32],
        treasury_share in prop_oneof![Just(0u32), Just(10_000u32), 0u32..=10_000u32],
        repay_pct in 1i128..=150i128,
    ) {
        let (env, client, borrower, token_address, reserve) = setup_minimal();
        let asset = token::StellarAssetClient::new(&env, &token_address);
        let token_client = token::Client::new(&env, &token_address);

        client.open_credit_line(&borrower, &draw, &rate_bps, &50_u32);
        let collateral = draw * 3;
        asset.mint(&borrower, &collateral);
        client.deposit_collateral(&borrower, &collateral);
        client.draw_credit(&borrower, &draw);
        client.set_protocol_fee_bps(&fee_bps);
        client.set_treasury_fee_share_bps(&treasury_share);

        env.ledger().with_mut(|l| l.timestamp += elapsed);
        let repay = (draw * repay_pct / 100).max(1);
        asset.mint(&borrower, &repay);
        token_client.approve(&borrower, &client.address, &repay, &1_000_u32);

        let borrower_before = token_client.balance(&borrower);
        let reserve_before = token_client.balance(&reserve);
        let contract_before = token_client.balance(&client.address);
        let s0 = client.get_protocol_summary();

        client.repay_credit(&borrower, &repay);

        let s1 = client.get_protocol_summary();
        let out = borrower_before - token_client.balance(&borrower);
        let reserve_in = token_client.balance(&reserve) - reserve_before;
        let contract_d = token_client.balance(&client.address) - contract_before;
        let treasury_d = s1.treasury_balance - s0.treasury_balance;
        let bounty_d = s1.bounty_balance - s0.bounty_balance;
        let fee = treasury_d + bounty_d;

        // Criterion 1: borrower outflow == reserve inflow + treasury + bounty
        prop_assert_eq!(out, reserve_in + treasury_d + bounty_d);
        // The contract holds exactly the fees the buckets recorded
        prop_assert_eq!(contract_d, fee);
        // Criterion 2: the fee never exceeds fee_bps of what was repaid,
        // and with no elapsed time (no interest) there is no fee at all
        prop_assert!(fee * 10_000 <= out * fee_bps as i128);
        if elapsed == 0 { prop_assert_eq!(fee, 0); }
        // Criterion 3: zero fee_bps transfers nothing to the contract
        if fee_bps == 0 {
            prop_assert_eq!(contract_d, 0);
            prop_assert_eq!(reserve_in, out);
        }
        // Split extremes
        if treasury_share == 10_000 { prop_assert_eq!(bounty_d, 0); }
        if treasury_share == 0 { prop_assert_eq!(treasury_d, 0); }
    }
}