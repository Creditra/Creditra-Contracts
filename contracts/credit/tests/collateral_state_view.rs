// SPDX-License-Identifier: MIT
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

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&token);

    let token_admin = StellarAssetClient::new(env, &token);
    token_admin.mint(&borrower, &100_000_i128);
    token_admin.mint(&token, &100_000_i128);

    (client, admin, borrower, token)
}

/// No credit line, no deposit: balance = 0, health = u32::MAX.
#[test]
fn get_collateral_state_no_line_no_deposit() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env);

    let state = client.get_collateral_state(&borrower);

    assert_eq!(state.borrower, borrower);
    assert_eq!(state.balance, 0);
    assert_eq!(state.min_ratio_bps, 15_000);
    assert_eq!(state.health_factor_bps, u32::MAX);
}

/// After deposit with no debt, health = u32::MAX.
#[test]
fn get_collateral_state_after_deposit_no_debt() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env);

    client.deposit_collateral(&borrower, &5_000);
    let state = client.get_collateral_state(&borrower);

    assert_eq!(state.balance, 5_000);
    assert_eq!(state.health_factor_bps, u32::MAX);
}

/// With active debt: health_factor_bps = balance * 10_000 / utilized_amount.
#[test]
fn get_collateral_state_with_active_debt() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env);

    client.open_credit_line(&borrower, &10_000, &0, &0);
    // 1_500 satisfies the default 150% (15_000 bps) ratio for a 1_000 draw.
    client.deposit_collateral(&borrower, &1_500);
    client.draw_credit(&borrower, &1_000);

    let state = client.get_collateral_state(&borrower);

    assert_eq!(state.balance, 1_500);
    // health = 1_500 * 10_000 / 1_000 = 15_000
    assert_eq!(state.health_factor_bps, 15_000_u32);
}

/// After full repayment, health returns to u32::MAX.
#[test]
fn get_collateral_state_after_full_repay() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env);

    client.open_credit_line(&borrower, &10_000, &0, &0);
    client.deposit_collateral(&borrower, &1_500);
    client.draw_credit(&borrower, &1_000);
    client.repay_credit(&borrower, &1_000);

    let state = client.get_collateral_state(&borrower);

    assert_eq!(state.balance, 1_500);
    assert_eq!(state.health_factor_bps, u32::MAX);
}

/// collateral_token field matches the configured token address.
#[test]
fn get_collateral_state_token_field() {
    let env = Env::default();
    let (client, _, borrower, token) = setup(&env);

    let state = client.get_collateral_state(&borrower);
    assert_eq!(state.collateral_token, Some(token));
}

// ── Issue #1338 — health factor characterization ─────────────────────────────
//
// Two factors exist and they answer different questions:
//
// * `get_health_factor` is the min-ratio-aware factor keepers use. It divides
//   by `utilized_amount * min_ratio_bps`, so it reads exactly `10_000` when the
//   collateral covers the configured floor, `0` when a floored position has no
//   collateral, and `u32::MAX` when there is either no debt or no floor.
// * `get_collateral_state().health_factor_bps` is the raw collateral-to-debt
//   ratio (`balance * 10_000 / utilized_amount`) reported next to
//   `min_ratio_bps`. It stays meaningful when the floor is disabled, which is
//   why the floor is reported separately instead of being folded into it.
//
// Each test asserts both, so the two views cannot silently drift into
// disagreeing about a case one of them does not model.

/// Open a line and draw `amount` while the ratio floor is disabled, leaving the
/// borrower with debt and no collateral. Leaves the floor at `0`.
fn draw_without_collateral(client: &CreditClient, borrower: &Address, amount: i128) {
    client.open_credit_line(borrower, &10_000_i128, &0_u32, &0_u32);
    client.set_min_collateral_ratio_bps(&0_u32);
    client.draw_credit(borrower, &amount);
}

/// Ratio floor disabled (`min_ratio_bps == 0`) → `u32::MAX`, never `0`.
///
/// The denominator `utilized * min_ratio` is zero here. Treating that as "no
/// collateral backing" would mark every unsecured position liquidatable the
/// moment an admin dials the floor off.
#[test]
fn health_factor_is_max_when_ratio_check_is_disabled() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env);

    draw_without_collateral(&client, &borrower, 1_000);

    assert_eq!(client.get_min_collateral_ratio_bps(), Some(0));
    assert_eq!(
        client.get_health_factor(&borrower),
        u32::MAX,
        "a disabled floor must not make an unsecured position look liquidatable"
    );

    // The raw ratio view still reports 0 collateral against 1_000 of debt.
    let state = client.get_collateral_state(&borrower);
    assert_eq!(state.min_ratio_bps, 0);
    assert_eq!(state.balance, 0);
    assert_eq!(state.health_factor_bps, 0);
}

/// Debt with zero collateral and a live floor → `0` (liquidatable).
#[test]
fn health_factor_is_zero_with_debt_and_no_collateral() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env);

    draw_without_collateral(&client, &borrower, 1_000);
    // Re-enable the default floor: the position is now genuinely unsecured.
    client.set_min_collateral_ratio_bps(&15_000_u32);

    assert_eq!(client.get_health_factor(&borrower), 0);

    let state = client.get_collateral_state(&borrower);
    assert_eq!(state.balance, 0);
    assert_eq!(state.min_ratio_bps, 15_000);
    assert_eq!(state.health_factor_bps, 0);
}

/// Exactly at the floor → exactly `10_000`; a stricter floor drops below it.
#[test]
fn health_factor_is_exactly_10_000_at_the_ratio_floor() {
    let env = Env::default();
    let (client, _, borrower, _) = setup(&env);

    client.open_credit_line(&borrower, &10_000_i128, &0_u32, &0_u32);
    // 1_500 collateral backs 1_000 of debt at the default 15_000 bps floor:
    // health = 1_500 * 100_000_000 / (1_000 * 15_000) = 10_000 exactly.
    client.deposit_collateral(&borrower, &1_500_i128);
    client.draw_credit(&borrower, &1_000_i128);

    assert_eq!(client.get_health_factor(&borrower), 10_000);

    // The raw view reports how much collateral backs the debt.
    let state = client.get_collateral_state(&borrower);
    assert_eq!(state.balance, 1_500);
    assert_eq!(state.min_ratio_bps, 15_000);
    assert_eq!(state.health_factor_bps, 15_000);

    // One step stricter on the floor puts the same position below 10_000.
    client.set_min_collateral_ratio_bps(&16_000_u32);
    assert_eq!(client.get_health_factor(&borrower), 9_375);
}

/// Collateral far above the u32 range → clamps to `u32::MAX`, never wraps.
#[test]
fn health_factor_clamps_to_u32_max_for_large_collateral() {
    let env = Env::default();
    let (client, _, borrower, token) = setup(&env);

    draw_without_collateral(&client, &borrower, 1);
    client.set_min_collateral_ratio_bps(&15_000_u32);

    // 1e10 collateral against one unit of debt is ~6.7e13 bps — past u32::MAX.
    let large: i128 = 10_000_000_000;
    StellarAssetClient::new(&env, &token).mint(&borrower, &large);
    client.deposit_collateral(&borrower, &large);

    assert_eq!(client.get_health_factor(&borrower), u32::MAX);

    let state = client.get_collateral_state(&borrower);
    assert_eq!(state.balance, large);
    assert_eq!(state.health_factor_bps, u32::MAX);
}

/// Collateral near `i128::MAX` saturates the intermediate multiply → `u32::MAX`.
///
/// `balance * 100_000_000` overflows `u128` here, so the numerator saturates
/// before the division. A wrapping multiply would instead report a tiny (or
/// zero) factor for the most over-collateralized position possible.
#[test]
fn health_factor_clamps_for_collateral_near_i128_max() {
    let env = Env::default();
    let (client, _, borrower, token) = setup(&env);

    draw_without_collateral(&client, &borrower, 1);
    client.set_min_collateral_ratio_bps(&15_000_u32);

    let huge: i128 = i128::MAX / 2;
    StellarAssetClient::new(&env, &token).mint(&borrower, &huge);
    client.deposit_collateral(&borrower, &huge);

    assert_eq!(client.get_health_factor(&borrower), u32::MAX);

    let state = client.get_collateral_state(&borrower);
    assert_eq!(state.balance, huge);
    assert_eq!(state.health_factor_bps, u32::MAX);
}
