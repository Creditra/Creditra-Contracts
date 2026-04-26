// SPDX-License-Identifier: MIT

use creditra_credit::types::CreditStatus;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{testutils::Address as _, Address, Env};

fn setup(env: &Env) -> (CreditClient, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    (client, admin)
}

/// No credit line opened → must return None.
#[test]
fn get_credit_line_returns_none_for_unknown_borrower() {
    let env = Env::default();
    let (client, _) = setup(&env);
    let borrower = Address::generate(&env);
    assert!(client.get_credit_line(&borrower).is_none());
}

/// After opening a credit line → must return Some with correct fields.
#[test]
fn get_credit_line_returns_some_after_open() {
    let env = Env::default();
    let (client, _) = setup(&env);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &5_000_i128, &300_u32, &50_u32);

    let line = client.get_credit_line(&borrower).expect("expected Some");
    assert_eq!(line.borrower, borrower);
    assert_eq!(line.credit_limit, 5_000);
    assert_eq!(line.utilized_amount, 0);
    assert_eq!(line.interest_rate_bps, 300);
    assert_eq!(line.risk_score, 50);
    assert_eq!(line.last_rate_update_ts, 0);
    assert_eq!(line.accrued_interest, 0);
}

/// After closing a credit line → still returns Some (record is preserved, status is Closed).
#[test]
fn get_credit_line_returns_some_after_close() {
    let env = Env::default();
    let (client, admin) = setup(&env);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &1_000_i128, &200_u32, &30_u32);
    client.close_credit_line(&borrower, &admin);

    let line = client.get_credit_line(&borrower).expect("expected Some after close");
    assert_eq!(
        line.status,
        CreditStatus::Closed
    );
}
