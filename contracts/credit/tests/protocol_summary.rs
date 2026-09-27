// SPDX-License-Identifier: MIT

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{token, Address, Env};

fn setup<'a>(env: &'a Env) -> (Address, Address, Address, CreditClient<'a>) {
    env.mock_all_auths_allowing_non_root_auth();

    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);
    client.set_liquidity_source(&contract_id);

    (contract_id, token_address, borrower, client)
}

#[test]
fn protocol_summary_empty_state_returns_zeroes() {
    let env = Env::default();
    let (_contract_id, _token_address, _borrower, client) = setup(&env);

    let summary = client.get_protocol_summary();

    assert_eq!(summary.count, 0);
    assert_eq!(summary.total_utilized, 0);
    assert_eq!(summary.total_collateral, 0);
    assert_eq!(summary.treasury_balance, 0);
    assert_eq!(summary.bounty_balance, 0);
}

#[test]
fn protocol_summary_returns_aggregate_totals() {
    let env = Env::default();
    let (contract_id, token_address, borrower, client) = setup(&env);

    let asset = token::StellarAssetClient::new(&env, &token_address);
    asset.mint(&borrower, &5_000);
    asset.mint(&contract_id, &2_000);

    client.open_credit_line(&borrower, &2_000, &1_000_u32, &50_u32);
    client.deposit_collateral(&borrower, &3_000);
    client.draw_credit(&borrower, &1_000);

    client.set_protocol_fee_bps(&1_000_u32);
    env.ledger()
        .with_mut(|ledger| ledger.timestamp = 31_557_600);
    asset.mint(&borrower, &1_100);
    token::Client::new(&env, &token_address).approve(
        &borrower,
        &contract_id,
        &1_100,
        &6_000_000_u32,
    );
    client.repay_credit(&borrower, &1_100);

    let summary = client.get_protocol_summary();

    assert_eq!(summary.count, 1);
    assert_eq!(summary.total_utilized, 0);
    assert_eq!(summary.total_collateral, 3_000);
    assert_eq!(summary.treasury_balance, 10);
    assert_eq!(summary.bounty_balance, 0);
}

/// Proof-of-reserve invariant: the contract's actual token balance must always
/// cover `treasury_balance + bounty_balance + total_collateral` (see the
/// rustdoc on `get_proof_of_reserve` in `views.rs`).
///
/// `advance_repayment_schedule_after_repay` (lifecycle.rs) credits
/// `treasury_balance` with the flat late fee for every overdue installment,
/// but never collects a matching token transfer for it. The credit is
/// phantom: it stays invisible as long as idle reserve liquidity pads the
/// contract's balance, and is only exposed once that liquidity is redrawn.
///
/// This test drives that exact sequence with a real Stellar asset contract
/// (never a mock) and checks the invariant after every step. It holds
/// through the draw and the overdue repay, then breaks — by exactly the
/// flat late fee amount — once the idle reserve is redrawn. Tracked by
/// https://github.com/Creditra/Creditra-Contracts/issues/1342.
#[test]
#[should_panic(expected = "proof-of-reserve violated")]
fn proof_of_reserve_broken_by_phantom_late_fee_credit() {
    let env = Env::default();
    let (contract_id, token_address, borrower, client) = setup(&env);

    let asset = token::StellarAssetClient::new(&env, &token_address);
    let token_client = token::Client::new(&env, &token_address);

    let assert_reserve_holds = |label: &str| {
        let summary = client.get_protocol_summary();
        let contract_balance = token_client.balance(&contract_id);
        let backed = summary.treasury_balance + summary.bounty_balance + summary.total_collateral;
        assert!(
            contract_balance >= backed,
            "proof-of-reserve violated after {}: balance {} < treasury {} + bounty {} + collateral {}",
            label,
            contract_balance,
            summary.treasury_balance,
            summary.bounty_balance,
            summary.total_collateral,
        );
    };

    // Step 1 (per issue #1342): open a credit line for the borrower.
    client.open_credit_line(&borrower, &2_000_000, &1_000_u32, &50_u32);
    assert_reserve_holds("open_credit_line");

    // Step 2: post collateral before any draw — draw_credit enforces a
    // default 150% minimum collateral ratio.
    asset.mint(&borrower, &1_000_000);
    client.deposit_collateral(&borrower, &1_000_000);
    assert_reserve_holds("deposit_collateral");

    // Step 3: mint separate "lending reserve" liquidity directly to the
    // contract (not collateral), then draw against it.
    asset.mint(&contract_id, &500_000);
    client.draw_credit(&borrower, &500_000);
    assert_reserve_holds("draw_credit");

    // Step 4: configure an installment schedule, a protocol fee, and a flat
    // late fee per missed installment.
    client.set_repayment_schedule(&borrower, &500_000, &100_u64, &200_u64);
    client.set_protocol_fee_bps(&500_u32);
    client.set_late_fee_flat(&50_i128);
    assert_reserve_holds("set_repayment_schedule");

    // Step 5: advance past the due date and repay with a generous margin
    // over principal + accrued interest. This triggers the phantom late-fee
    // credit for the one overdue installment.
    env.ledger().with_mut(|ledger| ledger.timestamp = 300);
    asset.mint(&borrower, &600_000);
    token_client.approve(&borrower, &contract_id, &600_000, &6_000_000_u32);
    client.repay_credit(&borrower, &600_000);
    assert_reserve_holds("repay_credit");

    // Step 6: draw out the idle reserve liquidity that was padding the
    // contract's balance and masking the shortfall — mirroring a second loan
    // drawn against the same pool.
    let summary = client.get_protocol_summary();
    let contract_balance = token_client.balance(&contract_id);
    let idle_reserve = contract_balance - summary.total_collateral;
    client.draw_credit(&borrower, &idle_reserve);

    // The phantom late-fee credit is now exposed: the contract no longer
    // holds enough tokens to cover treasury + bounty + collateral.
    assert_reserve_holds("draw_credit (idle reserve drained)");
}
