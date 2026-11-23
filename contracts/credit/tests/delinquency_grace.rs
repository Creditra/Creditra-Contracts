// SPDX-License-Identifier: MIT

//! Tests for the installment-delinquency grace window (#1246).
//!
//! `is_delinquent` reads a dedicated grace value that is independent of the
//! suspension grace in `GracePeriodConfig`. Until the dedicated key is
//! written, delinquency falls back to the suspension grace so existing
//! deployments keep their boundary.

use creditra_credit::types::GraceWaiverMode;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Env, Vec,
};

const T0: u64 = 1_000_000;
const YEAR: u64 = 31_557_600;
const DUE: u64 = T0 + 1;
const DRAW: i128 = 100_000;

struct Ctx {
    env: Env,
    contract_id: Address,
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

    fn set_suspension_grace(&self, seconds: u64) {
        self.client()
            .set_grace_period_config(&seconds, &GraceWaiverMode::FullWaiver, &0);
    }

    fn assert_boundary(&self, grace_seconds: u64) {
        self.env.ledger().set_timestamp(DUE + grace_seconds);
        assert!(
            !self.client().is_delinquent(&self.borrower),
            "not delinquent at next_due_ts + {grace_seconds}"
        );
        self.env.ledger().set_timestamp(DUE + grace_seconds + 1);
        assert!(
            self.client().is_delinquent(&self.borrower),
            "delinquent one second past next_due_ts + {grace_seconds}"
        );
    }
}

fn setup() -> Ctx {
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
    client.open_credit_line(&borrower, &500_000, &500, &50);
    client.deposit_collateral(&borrower, &150_000);
    client.draw_credit(&borrower, &DRAW);
    client.set_repayment_schedule(&borrower, &1_000, &(YEAR * 2), &DUE);
    Ctx {
        env,
        contract_id,
        borrower,
    }
}

#[test]
fn no_config_means_no_grace() {
    let ctx = setup();
    assert_eq!(ctx.client().get_delinquency_grace_seconds(), 0);
    ctx.assert_boundary(0);
}

#[test]
fn unset_key_falls_back_to_suspension_grace() {
    let ctx = setup();
    ctx.set_suspension_grace(120);
    assert_eq!(ctx.client().get_delinquency_grace_seconds(), 120);
    ctx.assert_boundary(120);
}

#[test]
fn explicit_grace_overrides_suspension_grace() {
    let ctx = setup();
    ctx.set_suspension_grace(YEAR);
    ctx.client().set_delinquency_grace_seconds(&60);
    assert_eq!(ctx.client().get_delinquency_grace_seconds(), 60);
    ctx.assert_boundary(60);
}

#[test]
fn suspension_grace_changes_do_not_move_delinquency_boundary() {
    let ctx = setup();
    ctx.client().set_delinquency_grace_seconds(&60);
    for suspension_grace in [0, 3_600, YEAR * 10] {
        ctx.set_suspension_grace(suspension_grace);
        assert_eq!(ctx.client().get_delinquency_grace_seconds(), 60);
        ctx.assert_boundary(60);
    }
}

#[test]
fn explicit_zero_disables_fallback() {
    let ctx = setup();
    ctx.set_suspension_grace(1_000);
    ctx.client().set_delinquency_grace_seconds(&0);
    assert_eq!(ctx.client().get_delinquency_grace_seconds(), 0);
    ctx.assert_boundary(0);
}

#[test]
fn grace_saturates_near_u64_max() {
    let ctx = setup();
    ctx.client()
        .set_repayment_schedule(&ctx.borrower, &1_000, &YEAR, &(u64::MAX - 1));
    ctx.client().set_delinquency_grace_seconds(&10);
    ctx.env.ledger().set_timestamp(u64::MAX);
    assert!(!ctx.client().is_delinquent(&ctx.borrower));
}

#[test]
fn penalty_surcharge_follows_delinquency_grace() {
    let ctx = setup();
    ctx.client().set_penalty_surcharge_bps(&200);
    // A long suspension grace must not suppress the penalty once the
    // delinquency grace has been pinned to zero.
    ctx.set_suspension_grace(YEAR * 30);
    ctx.client().set_delinquency_grace_seconds(&0);
    ctx.env.ledger().set_timestamp(T0 + YEAR);
    ctx.accrue();
    assert_eq!(ctx.interest(), 7_000); // 100_000 at 500 + 200 bps for one year
}

#[test]
fn delinquency_grace_defers_penalty_surcharge() {
    let ctx = setup();
    ctx.client().set_penalty_surcharge_bps(&200);
    ctx.set_suspension_grace(0);
    ctx.client().set_delinquency_grace_seconds(&(YEAR * 2));
    ctx.env.ledger().set_timestamp(T0 + YEAR);
    ctx.accrue();
    assert_eq!(ctx.interest(), 5_000); // base rate only: still inside the grace window
}
