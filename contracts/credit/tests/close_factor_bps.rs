use creditra_credit::types::CreditStatus;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{token, Address, Env, Symbol, TryFromVal};
use std::panic::{catch_unwind, AssertUnwindSafe};

fn setup_defaulted_line(utilized_amount: i128) -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());

    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &1_000_000_i128);
    token::StellarAssetClient::new(&env, &token_address).mint(&borrower, &1_000_000_i128);
    token::Client::new(&env, &token_address).approve(
        &borrower,
        &contract_id,
        &1_000_000_i128,
        &1_000_000_u32,
    );

    client.open_credit_line(&borrower, &10_000, &300_u32, &60_u32);

    if utilized_amount > 0 {
        // Deposit ample collateral (3x utilized) to satisfy min ratio of 150%
        client.deposit_collateral(&borrower, &(utilized_amount.saturating_mul(3).max(1)));
        client.draw_credit(&borrower, &utilized_amount);
    }

    client.default_credit_line(&borrower);

    (env, contract_id, borrower)
}

fn has_event_topic(env: &Env, event_kind: &str) -> bool {
    let namespace = Symbol::new(env, "credit");
    let kind = Symbol::new(env, event_kind);

    for (_contract, topics, _data) in env.events().all().iter() {
        let t0: Symbol = Symbol::try_from_val(env, &topics.get(0).unwrap()).unwrap();
        let t1: Symbol = Symbol::try_from_val(env, &topics.get(1).unwrap()).unwrap();
        if t0 == namespace && t1 == kind {
            return true;
        }
    }

    false
}

#[test]
fn default_close_factor_is_10k() {
    let (env, contract_id, _borrower) = setup_defaulted_line(500);
    let client = CreditClient::new(&env, &contract_id);

    assert_eq!(client.get_close_factor_bps(), 10_000);
}

#[test]
fn set_close_factor_bps_stores_value() {
    let (env, contract_id, _borrower) = setup_defaulted_line(1_000);
    let client = CreditClient::new(&env, &contract_id);

    client.set_close_factor_bps(&5_000_u32);

    assert_eq!(client.get_close_factor_bps(), 5_000);
}

#[test]
fn set_close_factor_bps_updates_value() {
    let (env, contract_id, _borrower) = setup_defaulted_line(0);
    let client = CreditClient::new(&env, &contract_id);

    client.set_close_factor_bps(&2_500_u32);
    assert_eq!(client.get_close_factor_bps(), 2_500);

    client.set_close_factor_bps(&7_500_u32);
    assert_eq!(client.get_close_factor_bps(), 7_500);
}

#[test]
fn settle_within_capped_close_factor_succeeds() {
    let (env, contract_id, borrower) = setup_defaulted_line(1_000);
    let client = CreditClient::new(&env, &contract_id);

    client.set_close_factor_bps(&5_000_u32);

    client.settle_default_liquidation(
        &borrower,
        &300_i128,
        &Symbol::new(&env, "s1"),
        &5_000_u32,
        &None,
    );

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Defaulted);
    assert_eq!(line.utilized_amount, 700);
}

#[test]
fn settle_exceeding_capped_close_factor_fails() {
    let (env, contract_id, borrower) = setup_defaulted_line(1_000);
    let client = CreditClient::new(&env, &contract_id);

    client.set_close_factor_bps(&5_000_u32);

    let result = catch_unwind(AssertUnwindSafe(|| {
        client.settle_default_liquidation(
            &borrower,
            &300_i128,
            &Symbol::new(&env, "s1"),
            &6_000_u32,
            &None,
        );
    }));
    assert!(
        result.is_err(),
        "settlement with over-cap close_factor should panic"
    );

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Defaulted);
    assert_eq!(line.utilized_amount, 1_000);
}

#[test]
fn full_close_factor_default_10k_allows_full_settlement() {
    let (env, contract_id, borrower) = setup_defaulted_line(450);
    let client = CreditClient::new(&env, &contract_id);

    assert_eq!(client.get_close_factor_bps(), 10_000);

    client.settle_default_liquidation(
        &borrower,
        &450_i128,
        &Symbol::new(&env, "s_full"),
        &10_000_u32,
        &None,
    );

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Closed);
    assert_eq!(line.utilized_amount, 0);
}

#[test]
fn capped_at_50_percent_enforced_across_settlements() {
    let (env, contract_id, borrower) = setup_defaulted_line(1_000);
    let client = CreditClient::new(&env, &contract_id);

    client.set_close_factor_bps(&5_000_u32);

    // First settlement at exactly 50% — allowed
    client.settle_default_liquidation(
        &borrower,
        &500_i128,
        &Symbol::new(&env, "s1"),
        &5_000_u32,
        &None,
    );

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 500);

    // Second settlement also at 50% of remaining — allowed
    client.settle_default_liquidation(
        &borrower,
        &250_i128,
        &Symbol::new(&env, "s2"),
        &5_000_u32,
        &None,
    );

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 250);
}

#[test]
fn set_close_factor_bps_to_1_allows_minimal_settlement() {
    let (env, contract_id, borrower) = setup_defaulted_line(10_000);
    let client = CreditClient::new(&env, &contract_id);

    client.set_close_factor_bps(&1_u32);

    // 1 bps of 10000 = 1, so recovering 1 is within target
    client.settle_default_liquidation(
        &borrower,
        &1_i128,
        &Symbol::new(&env, "s_tiny"),
        &1_u32,
        &None,
    );

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 9_999);
}

#[test]
fn set_close_factor_bps_to_0_panics() {
    let (env, contract_id, _borrower) = setup_defaulted_line(0);
    let client = CreditClient::new(&env, &contract_id);

    let result = catch_unwind(AssertUnwindSafe(|| {
        client.set_close_factor_bps(&0_u32);
    }));
    assert!(result.is_err(), "close_factor_bps of 0 should panic");
}

#[test]
fn set_close_factor_bps_above_10k_panics() {
    let (env, contract_id, _borrower) = setup_defaulted_line(0);
    let client = CreditClient::new(&env, &contract_id);

    let result = catch_unwind(AssertUnwindSafe(|| {
        client.set_close_factor_bps(&10_001_u32);
    }));
    assert!(result.is_err(), "close_factor_bps > 10000 should panic");
}

#[test]
fn capped_settlement_still_emits_liquidation_event() {
    let (env, contract_id, borrower) = setup_defaulted_line(1_000);
    let client = CreditClient::new(&env, &contract_id);

    client.set_close_factor_bps(&3_000_u32);

    client.settle_default_liquidation(
        &borrower,
        &200_i128,
        &Symbol::new(&env, "s_evt"),
        &3_000_u32,
        &None,
    );

    assert!(has_event_topic(&env, "liq_setl"));
}
