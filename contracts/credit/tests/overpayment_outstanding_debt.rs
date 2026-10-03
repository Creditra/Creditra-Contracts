// SPDX-License-Identifier: MIT
//! Issue #1356: an overpayment must charge the borrower only the outstanding
//! debt (principal + accrued interest). The remainder stays in their wallet.

use creditra_credit::events::RepaymentEvent;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Events, Ledger};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Env, Symbol, TryFromVal, TryIntoVal};

const START_TS: u64 = 1_000;
const YEAR_SECS: u64 = 31_536_000;
const DRAWN: i128 = 1_000;
const REPAY_REQUESTED: i128 = 5_000;
const FEE_BPS: u32 = 500; // 5% of the interest component

struct Ctx<'a> {
    env: &'a Env,
    client: CreditClient<'a>,
    contract_id: Address,
    borrower: Address,
    reserve: Address,
    token: TokenClient<'a>,
}

fn setup<'a>(env: &'a Env, rate_bps: u32) -> Ctx<'a> {
    env.mock_all_auths();
    env.ledger().set_timestamp(START_TS);

    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let reserve = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    let token_addr = env
        .register_stellar_asset_contract_v2(Address::generate(env))
        .address();
    client.set_liquidity_token(&token_addr);
    client.set_liquidity_source(&reserve);
    client.set_protocol_fee_bps(&FEE_BPS);

    let sac = StellarAssetClient::new(env, &token_addr);
    sac.mint(&reserve, &100_000);
    sac.mint(&contract_id, &100_000);
    // After the draw the borrower wallet holds exactly REPAY_REQUESTED.
    sac.mint(&borrower, &(REPAY_REQUESTED - DRAWN));

    client.open_credit_line(&borrower, &10_000, &rate_bps, &50);
    client.draw_credit(&borrower, &DRAWN);

    let token = TokenClient::new(env, &token_addr);
    token.approve(&borrower, &contract_id, &REPAY_REQUESTED, &u32::MAX);

    Ctx {
        env,
        client,
        contract_id,
        borrower,
        reserve,
        token,
    }
}

fn last_repayment_event(env: &Env) -> RepaymentEvent {
    let ns = Symbol::new(env, "credit");
    let kind = Symbol::new(env, "repay");
    for (_c, topics, data) in env.events().all().iter().rev() {
        let t0 = Symbol::try_from_val(env, &topics.get(0).unwrap()).unwrap();
        let t1 = Symbol::try_from_val(env, &topics.get(1).unwrap()).unwrap();
        if t0 == ns && t1 == kind {
            return data.try_into_val(env).unwrap();
        }
    }
    panic!("no repayment event found");
}

/// Owes 1_000 + 1 year of interest at 50% APR (= 500) → debt 1_500.
/// Repays 5_000. Only 1_500 may leave the borrower's wallet.
#[test]
fn overpayment_with_interest_charges_only_outstanding_debt() {
    let env = Env::default();
    let c = setup(&env, 5_000);

    env.ledger().set_timestamp(START_TS + YEAR_SECS);

    let expected_debt: i128 = 1_500;
    let expected_interest: i128 = 500;
    let expected_fee: i128 = expected_interest * FEE_BPS as i128 / 10_000; // 25

    let borrower_before = c.token.balance(&c.borrower);
    let reserve_before = c.token.balance(&c.reserve);
    let contract_before = c.token.balance(&c.contract_id);
    assert_eq!(borrower_before, REPAY_REQUESTED);

    c.client.repay_credit(&c.borrower, &REPAY_REQUESTED);

    // Borrower pays exactly the outstanding debt; the rest stays in the wallet.
    assert_eq!(
        borrower_before - c.token.balance(&c.borrower),
        expected_debt
    );
    assert_eq!(
        c.token.balance(&c.borrower),
        REPAY_REQUESTED - expected_debt
    );

    // Reserve and fee buckets receive the correct amounts (sum == debt).
    assert_eq!(
        c.token.balance(&c.reserve) - reserve_before,
        expected_debt - expected_fee
    );
    assert_eq!(
        c.token.balance(&c.contract_id) - contract_before,
        expected_fee
    );

    // Line is fully cleared.
    let line = c.client.get_credit_line(&c.borrower).unwrap();
    assert_eq!(line.utilized_amount, 0);
    assert_eq!(line.accrued_interest, 0);

    // RepaymentEvent reports the effective amount, not the requested one.
    let ev = last_repayment_event(c.env);
    assert_eq!(ev.borrower, c.borrower);
    assert_eq!(ev.amount, expected_debt);
    assert_eq!(ev.new_utilized_amount, 0);
}

/// No time elapsed → no interest, no fee. Debt is exactly the 1_000 drawn.
#[test]
fn overpayment_without_interest_charges_only_principal() {
    let env = Env::default();
    let c = setup(&env, 5_000);

    let borrower_before = c.token.balance(&c.borrower);
    let reserve_before = c.token.balance(&c.reserve);
    let contract_before = c.token.balance(&c.contract_id);

    c.client.repay_credit(&c.borrower, &REPAY_REQUESTED);

    assert_eq!(borrower_before - c.token.balance(&c.borrower), DRAWN);
    assert_eq!(c.token.balance(&c.reserve) - reserve_before, DRAWN);
    assert_eq!(c.token.balance(&c.contract_id), contract_before);

    let line = c.client.get_credit_line(&c.borrower).unwrap();
    assert_eq!(line.utilized_amount, 0);
    assert_eq!(line.accrued_interest, 0);

    let ev = last_repayment_event(c.env);
    assert_eq!(ev.amount, DRAWN);
    assert_eq!(ev.new_utilized_amount, 0);
}
