#![cfg(test)]

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, Address, Env};

fn setup(env: &Env) -> (CreditClient, Address, Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let borrower = Address::generate(env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    client.set_min_collateral_ratio_bps(&15000);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&contract_id);

    let token_admin = StellarAssetClient::new(env, &token);
    token_admin.mint(&borrower, &100_000_i128); // borrower funds
    token_admin.mint(&contract_id, &100_000_i128); // reserve funds

    (client, admin, borrower, token)
}

#[test]
fn test_deposit_and_withdraw_collateral() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env);

    client.deposit_collateral(&borrower, &5000);
    assert_eq!(client.get_collateral(&borrower), 5000);

    // Borrower doesn't have an active credit line, can withdraw all
    client.withdraw_collateral(&borrower, &5000);
    assert_eq!(client.get_collateral(&borrower), 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #35)")] // CollateralRatioBelowMinimum
fn test_withdraw_breaches_min_ratio() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env);

    client.open_credit_line(&borrower, &10000, &0, &0);
    client.deposit_collateral(&borrower, &2000); // Deposited 2000

    client.draw_credit(&borrower, &1000); // Drew 1000. Required collateral = 1000 * 1.5 = 1500

    client.withdraw_collateral(&borrower, &1000); // Attempt to withdraw 1000, leaving 1000. 1000 < 1500 => PANIC
}

#[test]
#[should_panic(expected = "Error(Contract, #35)")] // CollateralRatioBelowMinimum
fn test_draw_credit_breaches_min_ratio() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env);

    client.open_credit_line(&borrower, &10000, &0, &0);
    client.deposit_collateral(&borrower, &1000); // Deposited 1000

    // Attempt to draw 1000. Required collateral = 1000 * 1.5 = 1500. Have 1000. 1000 < 1500 => PANIC
    client.draw_credit(&borrower, &1000);
}

#[test]
fn test_draw_credit_succeeds_with_sufficient_collateral() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env);

    client.open_credit_line(&borrower, &10000, &0, &0);
    client.deposit_collateral(&borrower, &1500); // Deposited 1500

    // Attempt to draw 1000. Required collateral = 1500. Have 1500. OK
    client.draw_credit(&borrower, &1000);
    assert_eq!(client.get_collateral(&borrower), 1500);
}

#[test]
fn test_withdraw_with_open_credit_line_zero_utilized() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env);

    client.open_credit_line(&borrower, &10000, &0, &0);
    client.deposit_collateral(&borrower, &5000);
    assert_eq!(client.get_collateral(&borrower), 5000);

    // Credit line exists but utilized_amount = 0 → no ratio check → can withdraw all.
    client.withdraw_collateral(&borrower, &5000);
    assert_eq!(client.get_collateral(&borrower), 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #22)")] // MissingLiquidityToken
fn test_deposit_without_collateral_token_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    // No liquidity token set → deposit should fail with MissingLiquidityToken.
    client.deposit_collateral(&borrower, &1000);
}

#[test]
#[should_panic(expected = "Error(Contract, #22)")] // MissingLiquidityToken
fn test_withdraw_without_collateral_token_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    // Set collateral balance directly so amount <= cur_balance check passes before MissingLiquidityToken check
    env.as_contract(&contract_id, || {
        creditra_credit::storage::set_collateral_balance(&env, &borrower, 1000);
    });
    // No liquidity token set → withdraw should fail with MissingLiquidityToken.
    client.withdraw_collateral(&borrower, &1000);
}

#[test]
#[should_panic(expected = "Error(Contract, #39)")] // InsufficientCollateralBalance
fn test_withdraw_collateral_insufficient_balance() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env);

    client.deposit_collateral(&borrower, &1000);
    assert_eq!(client.get_collateral(&borrower), 1000);

    // Attempt to withdraw 1001, which is more than the deposited 1000.
    // Should panic with InsufficientCollateralBalance (39).
    client.withdraw_collateral(&borrower, &1001);
}

#[test]
#[should_panic(expected = "Error(Contract, #39)")]
fn test_withdraw_zero_collateral_balance_reverts_with_error_39() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env);

    // Borrower has 0 collateral balance; attempting to withdraw 100 fails with InsufficientCollateralBalance (#39)
    client.withdraw_collateral(&borrower, &100);
}

#[test]
fn test_withdraw_exact_collateral_balance_succeeds() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env);

    client.deposit_collateral(&borrower, &1000);
    assert_eq!(client.get_collateral(&borrower), 1000);

    // Exact balance withdrawal leaves 0 balance
    client.withdraw_collateral(&borrower, &1000);
    assert_eq!(client.get_collateral(&borrower), 0);
}


// ── Risk-weighted collateral valuation (Issue #1223) ─────────────────────────

/// An unset weight means full value.
#[test]
fn test_unset_risk_weight_keeps_full_value() {
    let env = Env::default();
    let (client, _, borrower, _token) = setup(&env);

    client.open_credit_line(&borrower, &10_000, &0, &0);
    client.deposit_collateral(&borrower, &3_000);
    client.draw_credit(&borrower, &1_000);

    // 3_000 * 100_000_000 / (1_000 * 15_000) = 20_000
    assert_eq!(client.get_health_factor(&borrower), 20_000);
}

/// A 5_000 bps weight halves the asset's contribution to the draw ratio.
#[test]
fn test_risk_weight_halves_draw_capacity() {
    let env = Env::default();
    let (client, _, borrower, token) = setup(&env);

    client.set_collateral_risk_weight(&token, &5_000);
    client.open_credit_line(&borrower, &10_000, &0, &0);
    client.deposit_collateral(&borrower, &3_000);
    client.draw_credit(&borrower, &1_000);

    // 1_500 weighted / 1_000 debt @ 150 % -> exactly on the floor.
    assert_eq!(client.get_health_factor(&borrower), 10_000);
}

/// The same weight must constrain withdrawals, not just draws.
#[test]
#[should_panic(expected = "Error(Contract, #35)")] // CollateralRatioBelowMinimum
fn test_risk_weight_constrains_withdrawals() {
    let env = Env::default();
    let (client, _, borrower, token) = setup(&env);

    client.set_collateral_risk_weight(&token, &5_000);
    client.open_credit_line(&borrower, &10_000, &0, &0);
    client.deposit_collateral(&borrower, &3_000);
    client.draw_credit(&borrower, &1_000);

    // Removing 1_000 leaves 1_000 weighted < 1_500 required.
    client.withdraw_collateral(&borrower, &1_000);
}

/// A weight change takes effect on the next draw without any migration.
#[test]
#[should_panic(expected = "Error(Contract, #35)")] // CollateralRatioBelowMinimum
fn test_risk_weight_change_applies_to_next_draw() {
    let env = Env::default();
    let (client, _, borrower, token) = setup(&env);

    client.open_credit_line(&borrower, &10_000, &0, &0);
    client.deposit_collateral(&borrower, &3_000);
    client.draw_credit(&borrower, &1_000);

    client.set_collateral_risk_weight(&token, &5_000);
    // updated_utilized = 1_001 -> required 1_501 > 1_500 weighted.
    client.draw_credit(&borrower, &1);
}

/// Floor rounding must never manufacture borrowing power.
#[test]
#[should_panic(expected = "Error(Contract, #35)")] // CollateralRatioBelowMinimum
fn test_floor_rounding_never_over_credits() {
    let env = Env::default();
    let (client, _, borrower, token) = setup(&env);

    // 3_333 bps on 2 units floors to 0; ceiling rounding would credit 1 and
    // let this draw through.
    client.set_collateral_risk_weight(&token, &3_333);
    client.open_credit_line(&borrower, &10_000, &0, &0);
    client.deposit_collateral(&borrower, &2);
    client.draw_credit(&borrower, &1);
}

// ─── Issue #1345: error-code ordering at the withdrawal entrypoints ─────────
//
// These tests pin the *precedence* of error codes emitted by the
// withdrawal entrypoints when the collateral token is not configured.
// Error ordering is part of the ABI — frontends branch on these codes —
// so a future refactor that reorders the checks must fail these tests.
//
// Observed ordering (see `contracts/credit/src/collateral.rs`):
//
//   withdraw_collateral / partial_release_collateral
//     1. InvalidAmount              (#7? — amount <= 0)
//     2. require_auth               (no code)
//     3. InsufficientCollateralBalance (#39) — amount > balance
//     4. CollateralRatioBelowMinimum  (#35) — when utilized_amount > 0
//     5. MissingLiquidityToken        (#22) — no token configured
//
// Therefore, when no token is configured:
//   - zero debt + amount <= balance  →  #22
//   - amount > balance                →  #39 (before #22)
//   - debt present + ratio breached  →  #35 (before #22)

// ── Helpers ────────────────────────────────────────────────────────────────

/// Contract with no collateral/liquidity token configured.
fn setup_no_token(env: &Env) -> (CreditClient, Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let borrower = Address::generate(env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    (client, admin, borrower)
}

/// Seed a credit line with a non-zero `utilized_amount` directly in storage.
/// Needed because `draw_credit` requires a token and would short-circuit with
/// #22 before the credit line exists in state.
fn seed_debt(env: &Env, contract_id: &Address, borrower: &Address, utilized: i128) {
    client_open_line_minimal(env, contract_id, borrower);
    env.as_contract(contract_id, || {
        let key = creditra_credit::storage::DataKey::CreditLine(borrower.clone());
        let mut line: creditra_credit::types::CreditLineData = env
            .storage()
            .persistent()
            .get(&key)
            .expect("credit line must exist after open");
        line.utilized_amount = utilized;
        env.storage().persistent().set(&key, &line);
    });
}

/// Open a credit line with the minimum viable parameters.
fn client_open_line_minimal(env: &Env, contract_id: &Address, borrower: &Address) {
    let client = CreditClient::new(env, contract_id);
    client.open_credit_line(borrower, &10_000_i128, &0_u32, &0_u32);
}

// ── withdraw_collateral ────────────────────────────────────────────────────

#[test]
#[should_panic(expected = "Error(Contract, #22)")] // MissingLiquidityToken
fn withdraw_without_token_zero_debt_reverts_22() {
    let env = Env::default();
    let (client, _admin, borrower) = setup_no_token(&env);

    let contract_id = client.address.clone();
    client_open_line_minimal(&env, &contract_id, &borrower);

    // Seed a balance so `amount > balance` (=> #39) does not fire.
    env.as_contract(&contract_id, || {
        creditra_credit::storage::set_collateral_balance(&env, &borrower, 1_000);
    });

    // No token configured. Zero debt. Amount within balance.
    // Expected precedence: #22 (no token) because the ratio branch is
    // skipped when utilized_amount == 0.
    client.withdraw_collateral(&borrower, &500);
}

#[test]
#[should_panic(expected = "Error(Contract, #39)")] // InsufficientCollateralBalance
fn withdraw_insufficient_balance_reverts_39_before_22() {
    let env = Env::default();
    let (client, _admin, borrower) = setup_no_token(&env);

    let contract_id = client.address.clone();
    client_open_line_minimal(&env, &contract_id, &borrower);

    env.as_contract(&contract_id, || {
        creditra_credit::storage::set_collateral_balance(&env, &borrower, 100);
    });

    // Amount > balance triggers #39 before the missing-token check (#22).
    client.withdraw_collateral(&borrower, &500);
}

#[test]
#[should_panic(expected = "Error(Contract, #35)")] // CollateralRatioBelowMinimum
fn withdraw_without_token_with_debt_reverts_35_before_22() {
    let env = Env::default();
    let (client, _admin, borrower) = setup_no_token(&env);

    let contract_id = client.address.clone();
    // Default min ratio is 15000 bps (150 %). Set it explicitly so the
    // test is independent of the default.
    client.set_min_collateral_ratio_bps(&15000_u32);

    // Seed debt + balance such that any withdrawal breaches the ratio.
    seed_debt(&env, &contract_id, &borrower, 1_000);
    env.as_contract(&contract_id, || {
        creditra_credit::storage::set_collateral_balance(&env, &borrower, 1_500);
    });

    // 1_500 collateral against 1_000 debt at 150 % floor.
    // Withdrawing 1 unit leaves 1_499 < 1_500 → ratio breach fires #35
    // *before* the missing-token check (#22) is reached.
    client.withdraw_collateral(&borrower, &1);
}

// ── partial_release_collateral ─────────────────────────────────────────────

#[test]
#[should_panic(expected = "Error(Contract, #22)")] // MissingLiquidityToken
fn partial_release_without_token_zero_debt_reverts_22() {
    let env = Env::default();
    let (client, _admin, borrower) = setup_no_token(&env);

    let contract_id = client.address.clone();
    client_open_line_minimal(&env, &contract_id, &borrower);

    env.as_contract(&contract_id, || {
        creditra_credit::storage::set_collateral_balance(&env, &borrower, 1_000);
    });

    client.partial_release_collateral(&borrower, &500);
}

#[test]
#[should_panic(expected = "Error(Contract, #39)")] // InsufficientCollateralBalance
fn partial_release_insufficient_balance_reverts_39_before_22() {
    let env = Env::default();
    let (client, _admin, borrower) = setup_no_token(&env);

    let contract_id = client.address.clone();
    client_open_line_minimal(&env, &contract_id, &borrower);

    env.as_contract(&contract_id, || {
        creditra_credit::storage::set_collateral_balance(&env, &borrower, 100);
    });

    client.partial_release_collateral(&borrower, &500);
}

#[test]
#[should_panic(expected = "Error(Contract, #35)")] // CollateralRatioBelowMinimum
fn partial_release_without_token_with_debt_reverts_35_before_22() {
    let env = Env::default();
    let (client, _admin, borrower) = setup_no_token(&env);

    let contract_id = client.address.clone();
    client.set_min_collateral_ratio_bps(&15000_u32);

    seed_debt(&env, &contract_id, &borrower, 1_000);
    env.as_contract(&contract_id, || {
        creditra_credit::storage::set_collateral_balance(&env, &borrower, 1_500);
    });

    client.partial_release_collateral(&borrower, &1);
}
