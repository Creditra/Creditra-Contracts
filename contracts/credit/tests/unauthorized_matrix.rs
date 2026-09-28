// SPDX-License-Identifier: MIT

//! Unauthorized caller matrix: negative tests for every admin-only and
//! role-gated entrypoint.
//!
//! Each test verifies that calling a privileged function without the
//! correct signer reverts. Setup uses targeted `mock_auths` so only
//! the intended addresses are authorized for setup operations; the
//! function under test receives no valid authorization.
//!
//! # Auth matrix (summary)
//!
//! | Function                    | Required auth          |
//! |-----------------------------|------------------------|
//! | `init`                      | none (one-shot)        |
//! | `propose_admin`             | admin                  |
//! | `accept_admin`              | proposed_admin         |
//! | `open_credit_line`          | admin                  |
//! | `set_liquidity_token`       | admin                  |
//! | `set_liquidity_source`      | admin                  |
//! | `set_max_draw_amount`       | admin                  |
//! | `set_max_repay_amount`      | admin                  |
//! | `set_draw_min_interval`     | admin                  |
//! | `set_utilization_cap`       | admin                  |
//! | `set_rate_change_limits`    | admin                  |
//! | `set_rate_formula_config`   | admin                  |
//! | `clear_rate_formula_config` | admin                  |
//! | `set_grace_period_config`   | admin                  |
//! | `set_protocol_paused`       | admin                  |
//! | `freeze_draws`              | admin                  |
//! | `unfreeze_draws`            | admin                  |
//! | `suspend_credit_line`       | admin                  |
//! | `default_credit_line`       | admin                  |
//! | `reinstate_credit_line`     | admin                  |
//! | `forgive_debt`              | admin                  |
//! | `settle_default_liquidation`| admin                  |
//! | `close_credit_line`         | closer.require_auth()  |
//! | `block_borrower`            | admin (derived)        |
//! | `unblock_borrower`          | admin (derived)        |
//! | `bulk_block_borrowers`      | admin (derived)        |
//! | `freeze_borrower_until`     | admin (derived)        |
//! | `unfreeze_borrower`         | admin (derived)        |
//! | `set_treasury`              | admin (derived)        |
//! | `set_bounty`                | admin (derived)        |
//! | `withdraw_treasury`         | admin (derived)        |
//! | `withdraw_bounty`           | admin (derived)        |
//! | `propose_treasury_withdrawal` | admin (derived)      |
//! | `execute_treasury_withdrawal` | admin (derived)      |
//! | `draw_credit`               | borrower               |
//! | `repay_credit`              | borrower               |
//! | `self_suspend_credit_line`  | borrower               |
//! | `get_*` / `is_*` / `enumerate_*` | none (read-only) |

use creditra_credit::types::CreditStatus;
use creditra_credit::{Credit, CreditClient, FreezeReason};
use soroban_sdk::testutils::{Address as _, Ledger as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{Address, Env, IntoVal, Symbol};

/// Registers the contract, initializes it, and opens one credit line.
///
/// The admin's authorization for `open_credit_line` is mocked explicitly rather
/// than with `mock_all_auths`. `mock_all_auths` would authorize every address,
/// which makes a `#[should_panic]` assertion pass for the wrong reason.
/// `mock_auths` entries are consumed by the call they are registered for, so
/// the function under test in each test below runs with no valid authorization.
fn setup(env: &Env) -> (CreditClient<'_>, Address, Address, Address) {
    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    client
        .mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "open_credit_line",
                args: soroban_sdk::vec![
                    env,
                    borrower.into_val(env),
                    1_000_i128.into_val(env),
                    300_u32.into_val(env),
                    50_u32.into_val(env),
                ],
                sub_invokes: &[],
            },
        }])
        .open_credit_line(&borrower, &1_000_i128, &300_u32, &50_u32);
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
fn set_max_repay_amount_unauthorized() {
    let env = Env::default();
    let (client, _, _, _) = setup(&env);
    client.set_max_repay_amount(&500_i128);
}

#[test]
#[should_panic]
fn set_draw_min_interval_unauthorized() {
    let env = Env::default();
    let (client, _, _, _) = setup(&env);
    client.set_draw_min_interval(&3600_u64);
}

#[test]
#[should_panic]
fn freeze_draws_unauthorized() {
    let env = Env::default();
    let (client, _, _, _) = setup(&env);
    client.freeze_draws(&FreezeReason::LiquidityReserve);
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

/// accept_admin requires the proposed_admin to sign; a stranger cannot accept.
#[test]
#[should_panic]
fn accept_admin_wrong_signer() {
    let env = Env::default();
    let (client, contract_id, admin, _) = setup(&env);
    let candidate = Address::generate(&env);

    // Propose legitimately.
    client
        .mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "propose_admin",
                args: (&candidate, 0_u64).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .propose_admin(&candidate, &0_u64);

    // A stranger tries to accept — must revert.
    let stranger = Address::generate(&env);
    client
        .mock_auths(&[MockAuth {
            address: &stranger,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "accept_admin",
                args: soroban_sdk::vec![&env],
                sub_invokes: &[],
            },
        }])
        .accept_admin();
}

// ── Credit line management ───────────────────────────────────────────────────

#[test]
#[should_panic]
fn open_credit_line_unauthorized() {
    let env = Env::default();
    let (client, _, _, _) = setup(&env);
    let new_borrower = Address::generate(&env);
    client.open_credit_line(&new_borrower, &500_i128, &300_u32, &50_u32);
}

#[test]
#[should_panic]
fn set_utilization_cap_unauthorized() {
    let env = Env::default();
    let (client, _, _, borrower) = setup(&env);
    client.set_utilization_cap(&borrower, &5000_u32);
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
fn forgive_debt_unauthorized() {
    let env = Env::default();
    let (client, _, _, borrower) = setup(&env);
    client.forgive_debt(&borrower, &100_i128);
}

#[test]
#[should_panic]
fn settle_default_liquidation_unauthorized() {
    let env = Env::default();
    let (client, contract_id, admin, borrower) = setup(&env);
    admin_default(&env, &client, &admin, &contract_id, &borrower);
    let settlement_id = Symbol::new(&env, "settle_1");
    client.settle_default_liquidation(&borrower, &100_i128, &settlement_id, &10_000_u32, &None);
}

#[test]
#[should_panic]
fn close_credit_line_stranger_unauthorized() {
    let env = Env::default();
    let (client, _, _, borrower) = setup(&env);
    let stranger = Address::generate(&env);
    client.close_credit_line(&borrower, &stranger);
}

// ── Borrower blocklist ───────────────────────────────────────────────────────
//
// The blocklist entrypoints no longer take an `admin` argument (Issue #1281):
// the admin is derived from the `Symbol("admin")` instance slot. The negative
// case is therefore "the only authorized signer is not the admin", so each test
// mocks a single non-admin auth and then issues the call.

#[test]
#[should_panic]
fn block_borrower_unauthorized() {
    let env = Env::default();
    let (client, contract_id, _, borrower) = setup(&env);
    let non_admin = Address::generate(&env);
    client
        .mock_auths(&[MockAuth {
            address: &non_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "block_borrower",
                args: soroban_sdk::vec![&env, borrower.into_val(&env)],
                sub_invokes: &[],
            },
        }])
        .block_borrower(&borrower);
}

#[test]
#[should_panic]
fn unblock_borrower_unauthorized() {
    let env = Env::default();
    let (client, contract_id, _, borrower) = setup(&env);
    let non_admin = Address::generate(&env);
    client
        .mock_auths(&[MockAuth {
            address: &non_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "unblock_borrower",
                args: soroban_sdk::vec![&env, borrower.into_val(&env)],
                sub_invokes: &[],
            },
        }])
        .unblock_borrower(&borrower);
}

#[test]
#[should_panic]
fn bulk_block_borrowers_unauthorized() {
    let env = Env::default();
    let (client, contract_id, _, borrower) = setup(&env);
    let non_admin = Address::generate(&env);
    let list = soroban_sdk::vec![&env, borrower];
    client
        .mock_auths(&[MockAuth {
            address: &non_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "bulk_block_borrowers",
                args: soroban_sdk::vec![&env, (&list).into_val(&env)],
                sub_invokes: &[],
            },
        }])
        .bulk_block_borrowers(&list);
}

// ── Borrower freeze (Issue #1281) ───────────────────────────────────────────

#[test]
#[should_panic]
fn freeze_borrower_until_unauthorized() {
    let env = Env::default();
    let (client, contract_id, _, borrower) = setup(&env);
    let non_admin = Address::generate(&env);
    let now = 1_700_000_000u64;
    env.ledger().set_timestamp(now);
    client
        .mock_auths(&[MockAuth {
            address: &non_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "freeze_borrower_until",
                args: soroban_sdk::vec![
                    &env,
                    borrower.into_val(&env),
                    (now + 3600).into_val(&env),
                ],
                sub_invokes: &[],
            },
        }])
        .freeze_borrower_until(&borrower, &(now + 3600));
}

#[test]
#[should_panic]
fn unfreeze_borrower_unauthorized() {
    let env = Env::default();
    let (client, contract_id, _, borrower) = setup(&env);
    let non_admin = Address::generate(&env);
    client
        .mock_auths(&[MockAuth {
            address: &non_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "unfreeze_borrower",
                args: soroban_sdk::vec![&env, borrower.into_val(&env)],
                sub_invokes: &[],
            },
        }])
        .unfreeze_borrower(&borrower);
}

// ── Treasury and bounty (Issue #1281) ───────────────────────────────────────
//
// These fund setters previously took an `admin: Address` argument that was
// required in addition to the stored admin's authorization. The argument is
// gone, so the negative case is a sole non-admin signer.

#[test]
#[should_panic]
fn set_treasury_unauthorized() {
    let env = Env::default();
    let (client, contract_id, _, _) = setup(&env);
    let non_admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    client
        .mock_auths(&[MockAuth {
            address: &non_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_treasury",
                args: soroban_sdk::vec![&env, treasury.into_val(&env)],
                sub_invokes: &[],
            },
        }])
        .set_treasury(&treasury);
}

#[test]
#[should_panic]
fn set_bounty_unauthorized() {
    let env = Env::default();
    let (client, contract_id, _, _) = setup(&env);
    let non_admin = Address::generate(&env);
    let bounty = Address::generate(&env);
    client
        .mock_auths(&[MockAuth {
            address: &non_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_bounty",
                args: soroban_sdk::vec![&env, bounty.into_val(&env)],
                sub_invokes: &[],
            },
        }])
        .set_bounty(&bounty);
}

#[test]
#[should_panic]
fn withdraw_treasury_unauthorized() {
    let env = Env::default();
    let (client, contract_id, _, _) = setup(&env);
    let non_admin = Address::generate(&env);
    client
        .mock_auths(&[MockAuth {
            address: &non_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "withdraw_treasury",
                args: soroban_sdk::vec![&env],
                sub_invokes: &[],
            },
        }])
        .withdraw_treasury();
}

#[test]
#[should_panic]
fn withdraw_bounty_unauthorized() {
    let env = Env::default();
    let (client, contract_id, _, _) = setup(&env);
    let non_admin = Address::generate(&env);
    client
        .mock_auths(&[MockAuth {
            address: &non_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "withdraw_bounty",
                args: soroban_sdk::vec![&env],
                sub_invokes: &[],
            },
        }])
        .withdraw_bounty();
}

#[test]
#[should_panic]
fn propose_treasury_withdrawal_unauthorized() {
    let env = Env::default();
    let (client, contract_id, _, _) = setup(&env);
    let non_admin = Address::generate(&env);
    client
        .mock_auths(&[MockAuth {
            address: &non_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "propose_treasury_withdrawal",
                args: soroban_sdk::vec![&env],
                sub_invokes: &[],
            },
        }])
        .propose_treasury_withdrawal();
}

#[test]
#[should_panic]
fn execute_treasury_withdrawal_unauthorized() {
    let env = Env::default();
    let (client, contract_id, _, _) = setup(&env);
    let non_admin = Address::generate(&env);
    client
        .mock_auths(&[MockAuth {
            address: &non_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "execute_treasury_withdrawal",
                args: soroban_sdk::vec![&env],
                sub_invokes: &[],
            },
        }])
        .execute_treasury_withdrawal();
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

#[test]
#[should_panic]
fn set_protocol_paused_with_reason_unauthorized() {
    let env = Env::default();
    let (client, _, _, _) = setup(&env);
    let reason = soroban_sdk::Symbol::new(&env, "test");
    client.set_protocol_paused_with_reason(&true, &reason);
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
                args: soroban_sdk::vec![&env, borrower.into_val(&env)],
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
                args: soroban_sdk::vec![&env, borrower.into_val(&env)],
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
                args: soroban_sdk::vec![&env, borrower.into_val(&env)],
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
                args: soroban_sdk::vec![&env],
                sub_invokes: &[],
            },
        }])
        .freeze_draws(&FreezeReason::LiquidityReserve);
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
