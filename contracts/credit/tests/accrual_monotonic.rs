// SPDX-License-Identifier: MIT
//! Property tests ensuring interest accrual is monotonically non-decreasing
//! as ledger time advances for active, suspended, and delinquent lines.

use proptest::prelude::*;
use soroban_sdk::{token, Address, Env};

use creditra_credit::{types::GraceWaiverMode, Credit, CreditClient};

fn deploy_credit_contract(env: &Env) -> (CreditClient<'_>, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    token::StellarAssetClient::new(env, &token).mint(&contract_id, &1_000_000_000_i128);

    (client, borrower)
}

fn setup_active_line(env: &Env) -> (CreditClient<'_>, Address) {
    let (client, borrower) = deploy_credit_contract(env);
    env.ledger().set_timestamp(1);
    client.open_credit_line(&borrower, &10_000_i128, &500_u32, &70_u32);
    client.draw_credit(&borrower, &1_000_i128);
    (client, borrower)
}

fn setup_suspended_line(env: &Env) -> (CreditClient<'_>, Address) {
    let (client, borrower) = deploy_credit_contract(env);
    env.ledger().set_timestamp(1);
    client.open_credit_line(&borrower, &10_000_i128, &500_u32, &70_u32);
    client.draw_credit(&borrower, &1_000_i128);
    client.suspend_credit_line(&borrower);
    client.set_grace_period_config(&31_536_000_u64, &GraceWaiverMode::FullWaiver, &0_u32);
    (client, borrower)
}

fn setup_delinquent_line(env: &Env) -> (CreditClient<'_>, Address) {
    let (client, borrower) = deploy_credit_contract(env);
    env.ledger().set_timestamp(1);
    client.open_credit_line(&borrower, &10_000_i128, &500_u32, &70_u32);
    client.draw_credit(&borrower, &1_000_i128);
    client.set_penalty_surcharge_bps(&500_u32);
    client.set_repayment_schedule(&borrower, &100_i128, &1_u64, &1_u64);
    (client, borrower)
}

fn accrue_via_update_risk(client: &CreditClient<'_>, borrower: &Address) {
    client.update_risk_parameters(&borrower, &10_000_i128, &500_u32, &70_u32);
}

proptest! {
    /// Default proptest settings generate 256 cases, satisfying the
    /// requirement for randomized timelines per status.
    #[test]
    fn prop_accrual_monotonic_active(
        t1 in 2u64..=31_536_000_u64,
        delta in 1u64..=31_536_000_u64,
    ) {
        let t2 = t1.saturating_add(delta);
        let env = Env::default();
        let (client, borrower) = setup_active_line(&env);

        env.ledger().set_timestamp(t1);
        accrue_via_update_risk(&client, &borrower);
        let before = client.get_credit_line(&borrower).unwrap();

        env.ledger().set_timestamp(t2);
        accrue_via_update_risk(&client, &borrower);
        let after = client.get_credit_line(&borrower).unwrap();

        prop_assert!(
            after.accrued_interest >= before.accrued_interest,
            "active accrual must be monotonic: t1={} t2={} before={} after={}",
            t1,
            t2,
            before.accrued_interest,
            after.accrued_interest
        );
    }

    #[test]
    fn prop_accrual_monotonic_suspended(
        t1 in 2u64..=47_304_001_u64,
        delta in 1u64..=31_536_000_u64,
    ) {
        let t2 = t1.saturating_add(delta);
        let env = Env::default();
        let (client, borrower) = setup_suspended_line(&env);

        env.ledger().set_timestamp(t1);
        accrue_via_update_risk(&client, &borrower);
        let before = client.get_credit_line(&borrower).unwrap();

        env.ledger().set_timestamp(t2);
        accrue_via_update_risk(&client, &borrower);
        let after = client.get_credit_line(&borrower).unwrap();

        prop_assert!(
            after.accrued_interest >= before.accrued_interest,
            "suspended accrual must be monotonic: t1={} t2={} before={} after={}",
            t1,
            t2,
            before.accrued_interest,
            after.accrued_interest
        );
    }

    #[test]
    fn prop_accrual_monotonic_delinquent(
        t1 in 2u64..=31_536_000_u64,
        delta in 1u64..=31_536_000_u64,
    ) {
        let t2 = t1.saturating_add(delta);
        let env = Env::default();
        let (client, borrower) = setup_delinquent_line(&env);

        env.ledger().set_timestamp(t1);
        accrue_via_update_risk(&client, &borrower);
        let before = client.get_credit_line(&borrower).unwrap();

        env.ledger().set_timestamp(t2);
        accrue_via_update_risk(&client, &borrower);
        let after = client.get_credit_line(&borrower).unwrap();

        prop_assert!(
            after.accrued_interest >= before.accrued_interest,
            "delinquent accrual must be monotonic: t1={} t2={} before={} after={}",
            t1,
            t2,
            before.accrued_interest,
            after.accrued_interest
        );
    }
}

#[test]
fn accrual_monotonicity_suspended_crosses_grace_boundary() {
    let env = Env::default();
    let (client, borrower) = setup_suspended_line(&env);

    // Inside grace window, interest is waived. After grace_end, interest resumes.
    let grace_end = 1 + 31_536_000_u64;
    env.ledger().set_timestamp(15_768_000_u64); // inside grace window
    accrue_via_update_risk(&client, &borrower);
    let before = client.get_credit_line(&borrower).unwrap();

    env.ledger().set_timestamp(grace_end + 1);
    accrue_via_update_risk(&client, &borrower);
    let after = client.get_credit_line(&borrower).unwrap();

    assert_eq!(before.accrued_interest, 0);
    assert!(after.accrued_interest >= before.accrued_interest);
}
