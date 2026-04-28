// SPDX-License-Identifier: MIT

//! Unauthorized caller matrix: negative tests for every admin-only and
//! role-gated entrypoint.
//!
//! Each test verifies that calling a privileged function without the
//! correct signer reverts. Setup uses targeted `mock_auths` so only
//! the intended addresses are authorized for setup operations; the
//! function under test receives no valid authorization.

use creditra_credit::types::CreditStatus;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{Address, Env, IntoVal, Symbol};

fn setup(env: &Env) -> (CreditClient<'_>, Address, Address, Address) {
    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &50_u32);
    (client, contract_id, admin, borrower)
}

fn admin_default(
    env: &Env,
    client: &CreditClient,
    admin: &Address,
    contract_id: &Address,
    borrower: &Address,
) {
    client
        .mock_auths(&[MockAuth {
            address: admin,
            invoke: &MockAuthInvoke {
                contract: contract_id,
                fn_name: "default_credit_line",
                args: (borrower,).into_val(env),
                sub_invokes: &[],
            },
        }])
        .default_credit_line(borrower);
}

// ── Liquidity setters ────────────────────────────────────────────────────────

#[test]
#[should_panic]
fn set_liquidity_token_unauthorized() {
    let env = Env::default();
    let (client, _, _, _) = setup(&env);
    let token = Address::generate(&env);
    client.set_liquidity_token(&token);
}

#[test]
#[should_panic]
fn set_liquidity_source_unauthorized() {
    let env = Env::default();
    let (client, _, _, _) = setup(&env);
    let source = Address::generate(&env);
    client.set_liquidity_source(&source);
}

#[test]
#[should_panic]
fn set_max_draw_amount_unauthorized() {
    let env = Env::default();
    let (client, _, _, _) = setup(&env);
    client.set_max_draw_amount(&500_i128);
}

#[test]
#[should_panic]
fn freeze_draws_unauthorized() {
    let env = Env::default();
    let (client, _, _, _) = setup(&env);
    client.freeze_draws();
}

#[test]
#[should_panic]
fn unfreeze_draws_unauthorized() {
    let env = Env::default();
    let (client, _, _, _) = setup(&env);
    client.unfreeze_draws();
}

// ── Admin rotation ──────────────────────────────────────────────────────────

#[test]
#[should_panic]
fn propose_admin_unauthorized() {
    let env = Env::default();
    let (client, _, _, _) = setup(&env);
    let candidate = Address::generate(&env);
    client.propose_admin(&candidate, &0_u64);
}

// ── Lifecycle admin functions ───────────────────────────────────────────────

#[test]
#[should_panic]
fn suspend_credit_line_unauthorized() {
    let env = Env::default();
    let (client, _, _, borrower) = setup(&env);
    client.suspend_credit_line(&borrower);
}

#[test]
#[should_panic]
fn default_credit_line_unauthorized() {
    let env = Env::default();
    let (client, _, _, borrower) = setup(&env);
    client.default_credit_line(&borrower);
}

#[test]
#[should_panic]
fn reinstate_credit_line_unauthorized() {
    let env = Env::default();
    let (client, contract_id, admin, borrower) = setup(&env);
    admin_default(&env, &client, &admin, &contract_id, &borrower);
    client.reinstate_credit_line(&borrower, &CreditStatus::Active);
}

#[test]
#[should_panic]
fn settle_default_liquidation_unauthorized() {
    let env = Env::default();
    let (client, contract_id, admin, borrower) = setup(&env);
    admin_default(&env, &client, &admin, &contract_id, &borrower);
    let settlement_id = Symbol::new(&env, "settle_1");
    client.settle_default_liquidation(&borrower, &100_i128, &settlement_id);
}

#[test]
#[should_panic]
fn close_credit_line_stranger_unauthorized() {
    let env = Env::default();
    let (client, _, _, borrower) = setup(&env);
    let stranger = Address::generate(&env);
    client.close_credit_line(&borrower, &stranger);
}

// ── Risk updates ────────────────────────────────────────────────────────────

#[test]
#[should_panic]
fn update_risk_parameters_unauthorized() {
    let env = Env::default();
    let (client, _, _, borrower) = setup(&env);
    client.update_risk_parameters(&borrower, &2_000_i128, &400_u32, &60_u32);
}

#[test]
#[should_panic]
fn set_rate_change_limits_unauthorized() {
    let env = Env::default();
    let (client, _, _, _) = setup(&env);
    client.set_rate_change_limits(&500_u32, &3600_u64);
}

#[test]
#[should_panic]
fn set_rate_formula_config_unauthorized() {
    let env = Env::default();
    let (client, _, _, _) = setup(&env);
    client.set_rate_formula_config(&100_u32, &10_u32, &50_u32, &5000_u32);
}

#[test]
#[should_panic]
fn clear_rate_formula_config_unauthorized() {
    let env = Env::default();
    let (client, _, _, _) = setup(&env);
    client.clear_rate_formula_config();
}

// ── Grace period config ─────────────────────────────────────────────────────

#[test]
#[should_panic]
fn set_grace_period_config_unauthorized() {
    let env = Env::default();
    let (client, _, _, _) = setup(&env);
    client.set_grace_period_config(
        &86400_u64,
        &creditra_credit::types::GraceWaiverMode::FullWaiver,
        &0_u32,
    );
}

// ── Protocol pause ──────────────────────────────────────────────────────────

#[test]
#[should_panic]
fn set_protocol_paused_unauthorized() {
    let env = Env::default();
    let (client, _, _, _) = setup(&env);
    client.set_protocol_paused(&true);
}

// ── Borrower role-gated functions: wrong signer ─────────────────────────────

#[test]
#[should_panic]
fn draw_credit_wrong_signer() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    client.set_liquidity_token(&token_id.address());
    soroban_sdk::token::StellarAssetClient::new(&env, &token_id.address())
        .mint(&contract_id, &5_000_i128);
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &50_u32);

    let impersonator = Address::generate(&env);
    client
        .mock_auths(&[MockAuth {
            address: &impersonator,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "draw_credit",
                args: (&borrower, 100_i128).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .draw_credit(&borrower, &100_i128);
}

#[test]
#[should_panic]
fn repay_credit_wrong_signer() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);
    soroban_sdk::token::StellarAssetClient::new(&env, &token_address)
        .mint(&contract_id, &5_000_i128);
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &50_u32);
    client.draw_credit(&borrower, &200_i128);

    let impersonator = Address::generate(&env);
    client
        .mock_auths(&[MockAuth {
            address: &impersonator,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "repay_credit",
                args: (&borrower, 100_i128).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .repay_credit(&borrower, &100_i128);
}

#[test]
#[should_panic]
fn self_suspend_wrong_signer() {
    let env = Env::default();
    let (client, contract_id, _, borrower) = setup(&env);

    let impersonator = Address::generate(&env);
    client
        .mock_auths(&[MockAuth {
            address: &impersonator,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "self_suspend_credit_line",
                args: (&borrower,).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .self_suspend_credit_line(&borrower);
}

// ── Admin functions called by non-admin using mock_auths ────────────────────

#[test]
#[should_panic]
fn suspend_credit_line_non_admin_mock_auth() {
    let env = Env::default();
    let (client, contract_id, _, borrower) = setup(&env);

    let non_admin = Address::generate(&env);
    client
        .mock_auths(&[MockAuth {
            address: &non_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "suspend_credit_line",
                args: (&borrower,).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .suspend_credit_line(&borrower);
}

#[test]
#[should_panic]
fn default_credit_line_non_admin_mock_auth() {
    let env = Env::default();
    let (client, contract_id, _, borrower) = setup(&env);

    let non_admin = Address::generate(&env);
    client
        .mock_auths(&[MockAuth {
            address: &non_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "default_credit_line",
                args: (&borrower,).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .default_credit_line(&borrower);
}

#[test]
#[should_panic]
fn freeze_draws_non_admin_mock_auth() {
    let env = Env::default();
    let (client, contract_id, _, _) = setup(&env);

    let non_admin = Address::generate(&env);
    client
        .mock_auths(&[MockAuth {
            address: &non_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "freeze_draws",
                args: ().into_val(&env),
                sub_invokes: &[],
            },
        }])
        .freeze_draws();
}

#[test]
#[should_panic]
fn update_risk_parameters_non_admin_mock_auth() {
    let env = Env::default();
    let (client, contract_id, _, borrower) = setup(&env);

    let non_admin = Address::generate(&env);
    client
        .mock_auths(&[MockAuth {
            address: &non_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "update_risk_parameters",
                args: (&borrower, 2_000_i128, 400_u32, 60_u32).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .update_risk_parameters(&borrower, &2_000_i128, &400_u32, &60_u32);
}

#[test]
#[should_panic]
fn set_protocol_paused_non_admin_mock_auth() {
    let env = Env::default();
    let (client, contract_id, _, _) = setup(&env);

    let non_admin = Address::generate(&env);
    client
        .mock_auths(&[MockAuth {
            address: &non_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_protocol_paused",
                args: (true,).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .set_protocol_paused(&true);
}
