// SPDX-License-Identifier: MIT

use creditra_credit::math_utils::{mul_div, Rounding};
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, Address, Env};

fn setup_with_credit_line(env: &Env, credit_limit: i128) -> (CreditClient<'_>, Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);
    token::StellarAssetClient::new(env, &token_address).mint(&contract_id, &(credit_limit * 10));

    client.open_credit_line(&borrower, &credit_limit, &300_u32, &50_u32);
    (client, borrower, contract_id)
}

fn effective_ceiling(credit_limit: i128, cap_bps: u32) -> i128 {
    let cap_amount = i128::try_from(mul_div(
        credit_limit as u128,
        cap_bps as u128,
        10_000,
        Rounding::Floor,
    ))
    .unwrap();
    credit_limit.min(cap_amount)
}

#[test]
fn cap_10000_is_no_op_effective_ceiling_equals_credit_limit() {
    let env = Env::default();
    let credit_limit = 1_000_i128;
    let (client, borrower, _) = setup_with_credit_line(&env, credit_limit);

    client.set_utilization_cap(&borrower, &10_000_u32);

    let ceiling = effective_ceiling(credit_limit, 10_000);
    assert_eq!(ceiling, credit_limit);

    client.draw_credit(&borrower, &ceiling);
    assert_eq!(client.get_credit_line(&borrower).unwrap().utilized_amount, ceiling);
}

#[test]
fn draws_are_capped_at_min_credit_limit_and_cap_amount() {
    let env = Env::default();
    let credit_limit = 1_000_i128;
    let (client, borrower, _) = setup_with_credit_line(&env, credit_limit);

    client.set_utilization_cap(&borrower, &6_000_u32);

    let ceiling = effective_ceiling(credit_limit, 6_000);
    assert_eq!(ceiling, 600);

    client.draw_credit(&borrower, &ceiling);
    assert_eq!(client.get_credit_line(&borrower).unwrap().utilized_amount, 600);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.draw_credit(&borrower, &1_i128);
    }));
    assert!(result.is_err());
    let panic_msg = format!("{:?}", result.unwrap_err());
    assert!(
        panic_msg.contains("exceeds utilization cap"),
        "unexpected panic: {panic_msg}"
    );
}

#[test]
fn cap_below_current_utilization_blocks_new_draws_until_cap_removed() {
    let env = Env::default();
    let credit_limit = 1_000_i128;
    let (client, borrower, _) = setup_with_credit_line(&env, credit_limit);

    client.draw_credit(&borrower, &700_i128);
    client.set_utilization_cap(&borrower, &6_000_u32);
    assert_eq!(effective_ceiling(credit_limit, 6_000), 600);

    let blocked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.draw_credit(&borrower, &1_i128);
    }));
    assert!(blocked.is_err());
    let panic_msg = format!("{:?}", blocked.unwrap_err());
    assert!(panic_msg.contains("exceeds utilization cap"));

    client.set_utilization_cap(&borrower, &0_u32);
    assert!(client.get_utilization_cap(&borrower).is_none());

    client.draw_credit(&borrower, &300_i128);
    assert_eq!(client.get_credit_line(&borrower).unwrap().utilized_amount, 1_000_i128);
}

#[test]
fn cap_composes_with_credit_limit_updates_documented_in_docs() {
    let env = Env::default();
    let (client, borrower, _) = setup_with_credit_line(&env, 1_000_i128);

    client.set_utilization_cap(&borrower, &8_000_u32);
    client.draw_credit(&borrower, &800_i128);

    client.update_risk_parameters(&borrower, &2_000_i128, &300_u32, &50_u32);
    let raised_limit_ceiling = effective_ceiling(2_000, 8_000);
    assert_eq!(raised_limit_ceiling, 1_600);

    client.draw_credit(&borrower, &800_i128);
    assert_eq!(client.get_credit_line(&borrower).unwrap().utilized_amount, 1_600_i128);

    let over_new_cap = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.draw_credit(&borrower, &1_i128);
    }));
    assert!(over_new_cap.is_err());
    let panic_msg = format!("{:?}", over_new_cap.unwrap_err());
    assert!(panic_msg.contains("exceeds utilization cap"));

    client.update_risk_parameters(&borrower, &1_500_i128, &300_u32, &50_u32);
    let lowered_limit_ceiling = effective_ceiling(1_500, 8_000);
    assert_eq!(lowered_limit_ceiling, 1_200);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.credit_limit, 1_500_i128);
    assert!(line.utilized_amount > lowered_limit_ceiling);

    let blocked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.draw_credit(&borrower, &1_i128);
    }));
    assert!(blocked.is_err());
    let panic_msg = format!("{:?}", blocked.unwrap_err());
    assert!(
        panic_msg.contains("Error(Contract, #22)") || panic_msg.contains("Error(Contract, #6)"),
        "unexpected panic: {panic_msg}"
    );
}

#[test]
fn effective_ceiling_math_matches_overflow_safe_mul_div() {
    let huge_limit = i128::MAX;
    let cap_bps = 10_000_u32;

    let ceiling = effective_ceiling(huge_limit, cap_bps);
    let via_mul_div = i128::try_from(mul_div(
        huge_limit as u128,
        cap_bps as u128,
        10_000,
        Rounding::Floor,
    ))
    .unwrap();

    assert_eq!(ceiling, huge_limit.min(via_mul_div));
    assert_eq!(ceiling, huge_limit);
}
