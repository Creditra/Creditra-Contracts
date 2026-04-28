// SPDX-License-Identifier: MIT

use creditra_credit::types::CreditStatus;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{Address, Env, IntoVal};

fn setup_active_line() -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &50_u32);

    (env, admin, borrower, contract_id)
}

#[test]
fn self_suspend_requires_only_borrower_auth() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &50_u32);

    client
        .mock_auths(&[MockAuth {
            address: &borrower,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "self_suspend_credit_line",
                args: (&borrower,).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .self_suspend_credit_line(&borrower);

    let auths = env.auths();
    assert_eq!(
        auths.len(),
        1,
        "self-suspend should require exactly one auth"
    );
    assert_eq!(auths[0].0, borrower, "borrower auth must be required");

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Suspended);
}

#[test]
fn self_suspend_blocks_draws_but_allows_repayments() {
    let (env, _admin, borrower, contract_id) = setup_active_line();
    let client = CreditClient::new(&env, &contract_id);

    client.draw_credit(&borrower, &600_i128);
    client.self_suspend_credit_line(&borrower);

    let draw_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.draw_credit(&borrower, &100_i128);
    }));
    assert!(draw_result.is_err(), "draws must fail while self-suspended");

    client.repay_credit(&borrower, &200_i128);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Suspended);
    assert_eq!(line.utilized_amount, 400);
}

#[test]
fn self_suspended_line_cannot_be_reopened_without_admin_auth() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &50_u32);

    client
        .mock_auths(&[MockAuth {
            address: &borrower,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "self_suspend_credit_line",
                args: (&borrower,).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .self_suspend_credit_line(&borrower);

    let unauthorized_reopen = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.open_credit_line(&borrower, &2_000_i128, &400_u32, &60_u32);
    }));
    assert!(
        unauthorized_reopen.is_err(),
        "re-opening a self-suspended line must require admin approval"
    );

    client
        .mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "open_credit_line",
                args: (&borrower, 2_000_i128, 400_u32, 60_u32).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .open_credit_line(&borrower, &2_000_i128, &400_u32, &60_u32);

    let reopened = client.get_credit_line(&borrower).unwrap();
    assert_eq!(reopened.status, CreditStatus::Active);
    assert_eq!(reopened.credit_limit, 2_000);
    assert_eq!(reopened.interest_rate_bps, 400);
    assert_eq!(reopened.risk_score, 60);
}
