// SPDX-License-Identifier: MIT

use creditra_credit::types::GraceWaiverMode;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Events as _, Ledger};
use soroban_sdk::{token, Address, Env};

fn setup_env() -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();
    CreditClient::new(&env, &contract_id).init(&admin);
    CreditClient::new(&env, &contract_id).set_liquidity_token(&token_address);

    (env, admin, contract_id, token_address)
}

fn setup_borrower_with_draw(
    env: &Env,
    contract_id: &Address,
    token_address: &Address,
    borrower: &Address,
    draw_amount: i128,
) {
    let client = CreditClient::new(env, contract_id);
    client.open_credit_line(borrower, &10_000, &300_u32, &50_u32);
    // `draw_credit` enforces the default minimum collateral ratio of 150 %,
    // so the borrower must post at least `ceil(draw_amount * 1.5)` collateral
    // before drawing. Deposit 2× for headroom (exact for any even amount).
    let collateral = draw_amount * 2;
    token::StellarAssetClient::new(env, token_address)
        .mint(borrower, &(collateral + draw_amount * 2));
    client.deposit_collateral(borrower, &collateral);
    // The contract is its own liquidity source, so it must hold the tokens
    // that back the draw.
    token::StellarAssetClient::new(env, token_address).mint(contract_id, &(draw_amount * 2));
    client.draw_credit(borrower, &draw_amount);
}

#[test]
fn qualifying_repayment_advances_next_due_timestamp() {
    let (env, admin, contract_id, token_address) = setup_env();
    let _ = admin;
    let borrower = Address::generate(&env);
    let client = CreditClient::new(&env, &contract_id);
    env.ledger().with_mut(|li| li.timestamp = 1_000);
    setup_borrower_with_draw(&env, &contract_id, &token_address, &borrower, 500);

    token::StellarAssetClient::new(&env, &token_address).mint(&borrower, &100);
    // SAC approve caps `live_until` at a bounded ledger range; `u32::MAX` is
    // rejected by the environment host, so use a finite expiry window.
    token::Client::new(&env, &token_address).approve(
        &borrower,
        &contract_id,
        &100,
        &(env.ledger().sequence() + 1_000),
    );

    client.set_repayment_schedule(&borrower, &100, &86_400, &2_000);
    client.repay_credit(&borrower, &100);

    let schedule = client.get_repayment_schedule(&borrower).unwrap();
    assert_eq!(schedule.next_due_ts, 88_400);
    assert!(!client.is_delinquent(&borrower));
}

#[test]
fn repayment_within_grace_is_not_delinquent() {
    let (env, admin, contract_id, token_address) = setup_env();
    let _ = admin;
    let borrower = Address::generate(&env);
    let client = CreditClient::new(&env, &contract_id);
    env.ledger().with_mut(|li| li.timestamp = 10_000);
    setup_borrower_with_draw(&env, &contract_id, &token_address, &borrower, 500);

    client.set_grace_period_config(&60, &GraceWaiverMode::FullWaiver, &0);
    client.set_repayment_schedule(&borrower, &100, &86_400, &9_970);

    assert!(!client.is_delinquent(&borrower));
}

#[test]
fn delinquency_triggers_after_the_grace_boundary() {
    let (env, admin, contract_id, token_address) = setup_env();
    let _ = admin;
    let borrower = Address::generate(&env);
    let client = CreditClient::new(&env, &contract_id);
    env.ledger().with_mut(|li| li.timestamp = 10_000);
    setup_borrower_with_draw(&env, &contract_id, &token_address, &borrower, 500);

    client.set_grace_period_config(&60, &GraceWaiverMode::FullWaiver, &0);
    // due = 9_940, grace = 60 → boundary = 10_000. The assertions below pin
    // the exclusive boundary: `now == next_due_ts + grace` is the last
    // non-delinquent second and delinquency starts one second later.
    client.set_repayment_schedule(&borrower, &100, &86_400, &9_940);

    // `now == next_due_ts + grace_seconds` → not delinquent.
    assert!(!client.is_delinquent(&borrower));

    // One second later (`now == next_due_ts + grace_seconds + 1`) → delinquent.
    env.ledger().with_mut(|li| li.timestamp = 10_001);
    assert!(client.is_delinquent(&borrower));
}

#[test]
fn delinquency_boundary_without_grace_config_starts_strictly_after_due() {
    let (env, admin, contract_id, token_address) = setup_env();
    let _ = admin;
    let borrower = Address::generate(&env);
    let client = CreditClient::new(&env, &contract_id);
    env.ledger().with_mut(|li| li.timestamp = 20_000);
    setup_borrower_with_draw(&env, &contract_id, &token_address, &borrower, 500);

    // No `set_grace_period_config` call → grace_seconds defaults to 0, so the
    // boundary is exactly `next_due_ts`.
    client.set_repayment_schedule(&borrower, &100, &86_400, &20_000);

    // `now == next_due_ts + 0` → not delinquent (boundary is exclusive).
    assert!(!client.is_delinquent(&borrower));

    // `now == next_due_ts + 1` → delinquent.
    env.ledger().with_mut(|li| li.timestamp = 20_001);
    assert!(client.is_delinquent(&borrower));
}

#[test]
fn delinquency_boundary_saturates_near_u64_max_and_never_wraps() {
    let (env, admin, contract_id, token_address) = setup_env();
    let _ = admin;
    let borrower = Address::generate(&env);
    let client = CreditClient::new(&env, &contract_id);
    env.ledger().with_mut(|li| li.timestamp = 1_000);
    setup_borrower_with_draw(&env, &contract_id, &token_address, &borrower, 500);

    client.set_grace_period_config(&10, &GraceWaiverMode::FullWaiver, &0);
    // next_due_ts = u64::MAX - 1 → `saturating_add(10)` clamps the boundary
    // to u64::MAX. A wrapping add would instead produce 8, spuriously marking
    // the borrower delinquent at `now = u64::MAX - 1`; saturation prevents it.
    client.set_repayment_schedule(&borrower, &100, &86_400, &(u64::MAX - 1));

    // `now` = u64::MAX - 1 ≤ saturated boundary u64::MAX → not delinquent.
    env.ledger().with_mut(|li| li.timestamp = u64::MAX - 1);
    assert!(!client.is_delinquent(&borrower));

    // Even at the maximum representable timestamp the comparison never wraps:
    // `u64::MAX > u64::MAX` is false → still not delinquent.
    env.ledger().with_mut(|li| li.timestamp = u64::MAX);
    assert!(!client.is_delinquent(&borrower));
}

#[test]
fn closed_line_is_never_delinquent_even_far_past_due() {
    let (env, admin, contract_id, token_address) = setup_env();
    let borrower = Address::generate(&env);
    let client = CreditClient::new(&env, &contract_id);
    env.ledger().with_mut(|li| li.timestamp = 10_000);
    setup_borrower_with_draw(&env, &contract_id, &token_address, &borrower, 500);

    // Due date already in the past with no grace window: delinquent while open.
    client.set_repayment_schedule(&borrower, &100, &86_400, &9_000);
    assert!(client.is_delinquent(&borrower));

    // Admin force-close the line (the borrower still has outstanding
    // utilization, so a borrower self-close would be rejected).
    client.close_credit_line(&borrower, &admin);

    // Closed lines short-circuit to `false` no matter how far the clock moves
    // past the (now cleared) schedule's due date.
    env.ledger().with_mut(|li| li.timestamp = 10_000_000);
    assert!(!client.is_delinquent(&borrower));
    assert!(client.get_repayment_schedule(&borrower).is_none());
}

#[test]
fn rescheduling_overdue_borrower_resets_due_date_and_clears_delinquency() {
    let (env, _admin, contract_id, token_address) = setup_env();
    let borrower = Address::generate(&env);
    let client = CreditClient::new(&env, &contract_id);
    env.ledger().with_mut(|li| li.timestamp = 10_000);
    setup_borrower_with_draw(&env, &contract_id, &token_address, &borrower, 500);

    client.set_repayment_schedule(&borrower, &100, &86_400, &9_940);
    assert!(client.is_delinquent(&borrower));

    let event_count_before_reschedule = env.events().all().len();
    client.set_repayment_schedule(&borrower, &200, &172_800, &20_000);

    let schedule = client.get_repayment_schedule(&borrower).unwrap();
    assert_eq!(schedule.amount_per_period, 200);
    assert_eq!(schedule.period_seconds, 172_800);
    assert_eq!(schedule.next_due_ts, 20_000);
    assert!(!client.is_delinquent(&borrower));
    assert_eq!(env.events().all().len(), event_count_before_reschedule);
}

#[test]
fn closed_line_can_be_rescheduled_but_is_not_delinquent() {
    let (env, admin, contract_id, token_address) = setup_env();
    let borrower = Address::generate(&env);
    let client = CreditClient::new(&env, &contract_id);
    setup_borrower_with_draw(&env, &contract_id, &token_address, &borrower, 500);

    client.set_repayment_schedule(&borrower, &100, &86_400, &1_000);
    client.close_credit_line(&borrower, &admin);
    assert!(client.get_repayment_schedule(&borrower).is_none());

    client.set_repayment_schedule(&borrower, &200, &172_800, &20_000);

    let schedule = client.get_repayment_schedule(&borrower).unwrap();
    assert_eq!(schedule.amount_per_period, 200);
    assert_eq!(schedule.period_seconds, 172_800);
    assert_eq!(schedule.next_due_ts, 20_000);
    assert!(!client.is_delinquent(&borrower));
}
