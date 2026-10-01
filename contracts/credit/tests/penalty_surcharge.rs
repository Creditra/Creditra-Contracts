// SPDX-License-Identifier: MIT

//! Integration tests for the delinquent APR surcharge, including precedence
//! over the legacy surcharge and the flat-fee mode.

use creditra_credit::{
    penalties::{AprFeeConfig, FlatFeeConfig, LateFeeConfig},
    Credit, CreditClient,
};
use soroban_sdk::{
    testutils::{Address as _, Events as _, Ledger as _},
    token, Address, Env, Symbol, TryFromVal, Vec,
};

const T0: u64 = 1_000_000;
const YEAR: u64 = 31_557_600;
const DRAW: i128 = 100_000;

struct Ctx {
    env: Env,
    contract_id: Address,
    token_id: Address,
    borrower: Address,
}

impl Ctx {
    fn client(&self) -> CreditClient<'_> {
        CreditClient::new(&self.env, &self.contract_id)
    }

    fn accrue(&self) {
        self.client()
            .accrue_batch(&Vec::from_array(&self.env, [self.borrower.clone()]));
    }

    fn interest(&self) -> i128 {
        self.client()
            .get_credit_line(&self.borrower)
            .unwrap()
            .accrued_interest
    }

    fn fund_repay(&self, amount: i128) {
        token::Client::new(&self.env, &self.token_id).approve(
            &self.borrower,
            &self.contract_id,
            &amount,
            &self.env.ledger().sequence().saturating_add(1_000),
        );
    }
}

fn setup(rate_bps: u32) -> Ctx {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    env.ledger().set_timestamp(T0);
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let token_id = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    client.set_liquidity_token(&token_id);
    let sac = token::StellarAssetClient::new(&env, &token_id);
    sac.mint(&contract_id, &1_000_000);
    sac.mint(&borrower, &1_000_000);
    client.open_credit_line(&borrower, &500_000, &rate_bps, &50);
    client.deposit_collateral(&borrower, &150_000);
    client.draw_credit(&borrower, &DRAW);
    client.set_repayment_schedule(&borrower, &1_000, &(YEAR * 2), &(T0 + 1));
    Ctx {
        env,
        contract_id,
        token_id,
        borrower,
    }
}

#[test]
fn test_set_and_get_penalty_surcharge_bps() {
    let ctx = setup(500);
    for bps in [500, 1_000, 0] {
        ctx.client().set_penalty_surcharge_bps(&bps);
        assert_eq!(ctx.client().get_penalty_surcharge_bps(), bps);
    }
}

#[test]
fn test_penalty_surcharge_default_is_zero() {
    let ctx = setup(500);
    assert_eq!(ctx.client().get_penalty_surcharge_bps(), 0);
}

#[test]
fn test_penalty_surcharge_exceeds_max_rate() {
    let ctx = setup(500);
    assert!(ctx.client().try_set_penalty_surcharge_bps(&10_001).is_err());
    assert_eq!(ctx.client().get_penalty_surcharge_bps(), 0);
    assert!(ctx
        .client()
        .try_set_late_fee_config(&Some(LateFeeConfig::AprBased(AprFeeConfig {
            surcharge_bps: 10_001
        },)))
        .is_err());
    assert_eq!(ctx.client().get_late_fee_config(), None);
}

#[test]
fn test_penalty_surcharge_applied_to_delinquent_line() {
    let ctx = setup(500);
    ctx.client().set_penalty_surcharge_bps(&200);
    ctx.env.ledger().set_timestamp(T0 + YEAR);
    ctx.accrue();
    assert_eq!(ctx.interest(), 7_000); // 100_000 at 700 bps for 1 year
}

#[test]
fn test_penalty_surcharge_not_applied_to_non_delinquent_line() {
    let ctx = setup(500);
    ctx.client().set_penalty_surcharge_bps(&200);
    // Replace the due date with a future date: no penalty is due.
    ctx.client()
        .set_repayment_schedule(&ctx.borrower, &1_000, &YEAR, &(T0 + 2 * YEAR));
    ctx.env.ledger().set_timestamp(T0 + YEAR);
    ctx.accrue();
    assert_eq!(ctx.interest(), 5_000);
    assert_eq!(
        ctx.client()
            .get_credit_line(&ctx.borrower)
            .unwrap()
            .interest_rate_bps,
        500
    );
}

#[test]
fn test_penalty_rate_entered_event_emitted() {
    let ctx = setup(500);
    ctx.client().set_penalty_surcharge_bps(&200);
    ctx.env.ledger().set_timestamp(T0 + YEAR);
    ctx.accrue();
    let events = ctx.env.events().all();
    assert!(events.iter().any(|event| {
        event.1.len() >= 2
            && Symbol::try_from_val(&ctx.env, &event.1.get(0).unwrap()).ok()
                == Some(Symbol::new(&ctx.env, "credit"))
            && Symbol::try_from_val(&ctx.env, &event.1.get(1).unwrap()).ok()
                == Some(Symbol::new(&ctx.env, "pen_enter"))
    }));
}

#[test]
fn test_penalty_surcharge_stops_after_catching_up() {
    let ctx = setup(500);
    ctx.client().set_penalty_surcharge_bps(&200);
    ctx.env.ledger().set_timestamp(T0 + YEAR);
    ctx.accrue();
    assert_eq!(ctx.interest(), 7_000);

    // Paying the interest plus one installment advances the due date two
    // years, putting the remaining 99,000 of principal back in good standing.
    ctx.fund_repay(8_000);
    ctx.client().repay_credit(&ctx.borrower, &8_000);
    assert!(!ctx.client().is_delinquent(&ctx.borrower));
    ctx.env.ledger().set_timestamp(T0 + 2 * YEAR);
    ctx.accrue();
    assert_eq!(ctx.interest(), 4_950); // 99,000 at base 500 bps
}

#[test]
fn test_penalty_surcharge_with_zero_surcharge_no_effect() {
    let ctx = setup(500);
    ctx.client().set_penalty_surcharge_bps(&200);
    ctx.client()
        .set_late_fee_config(&Some(LateFeeConfig::AprBased(AprFeeConfig {
            surcharge_bps: 0,
        })));
    ctx.env.ledger().set_timestamp(T0 + YEAR);
    ctx.accrue();
    assert_eq!(ctx.interest(), 5_000); // explicit zero overrides legacy 200
}

#[test]
fn test_penalty_surcharge_clamped_to_max_rate() {
    let ctx = setup(9_500);
    ctx.client()
        .set_late_fee_config(&Some(LateFeeConfig::AprBased(AprFeeConfig {
            surcharge_bps: 1_000,
        })));
    ctx.env.ledger().set_timestamp(T0 + YEAR);
    ctx.accrue();
    assert_eq!(ctx.interest(), 100_000); // capped at 10,000 bps, not 10,500
}

#[test]
fn structured_flat_does_not_add_legacy_apr_to_accrued_interest() {
    let ctx = setup(500);
    ctx.client().set_penalty_surcharge_bps(&200);
    ctx.client()
        .set_late_fee_config(&Some(LateFeeConfig::Flat(FlatFeeConfig { amount: 17 })));
    ctx.env.ledger().set_timestamp(T0 + YEAR);
    ctx.accrue();
    assert_eq!(ctx.interest(), 5_000);
}

#[test]
fn structured_flat_rejects_negative_amount_without_changing_config() {
    let ctx = setup(500);
    let valid = LateFeeConfig::Flat(FlatFeeConfig { amount: 7 });
    ctx.client().set_late_fee_config(&Some(valid));
    assert!(ctx
        .client()
        .try_set_late_fee_config(&Some(LateFeeConfig::Flat(FlatFeeConfig { amount: -1 })))
        .is_err());
    assert_eq!(ctx.client().get_late_fee_config(), Some(valid));
}

fn has_credit_event(ctx: &Ctx, kind: &str) -> bool {
    ctx.env.events().all().iter().any(|event| {
        event.1.len() >= 2
            && Symbol::try_from_val(&ctx.env, &event.1.get(0).unwrap()).ok()
                == Some(Symbol::new(&ctx.env, "credit"))
            && Symbol::try_from_val(&ctx.env, &event.1.get(1).unwrap()).ok()
                == Some(Symbol::new(&ctx.env, kind))
    })
}

#[test]
fn test_zero_legacy_surcharge_emits_no_penalty_event() {
    let ctx = setup(500);
    ctx.client().set_penalty_surcharge_bps(&0);
    ctx.env.ledger().set_timestamp(T0 + YEAR);
    ctx.accrue();
    assert_eq!(ctx.interest(), 5_000);
    assert!(
        !has_credit_event(&ctx, "pen_enter"),
        "a zero surcharge must never emit a penalty-rate-entered event"
    );
}

#[test]
fn test_penalty_rate_exited_event_emitted() {
    let ctx = setup(500);
    ctx.client().set_penalty_surcharge_bps(&200);
    ctx.env.ledger().set_timestamp(T0 + YEAR);
    ctx.accrue();

    // Catch up: interest plus one installment moves the due date out two years.
    ctx.fund_repay(8_000);
    ctx.client().repay_credit(&ctx.borrower, &8_000);
    ctx.env.ledger().set_timestamp(T0 + 2 * YEAR);
    ctx.accrue();
    assert!(
        has_credit_event(&ctx, "pen_exit"),
        "leaving delinquency must emit a penalty-rate-exited event"
    );
}
