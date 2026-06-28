// SPDX-License-Identifier: MIT

//! Property test: `TotalUtilized` always matches the sum of borrower utilization.
//!
//! This test generates arbitrary sequences of draw/repay intents across several
//! borrowers, materializes each intent into a valid contract call, and asserts
//! after every successful step that:
//!
//! `get_total_utilized() == Σ get_credit_line(borrower).utilized_amount`
//!
//! A small in-test model also tracks the expected per-borrower utilization so
//! the on-chain aggregate is checked against both the live credit lines and the
//! independently maintained running sum.

use creditra_credit::{Credit, CreditClient};
use proptest::collection::vec as proptest_vec;
use proptest::prelude::*;
use proptest::test_runner::{Config as ProptestConfig, TestCaseError, TestCaseResult};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Env};

const BORROWER_COUNT: usize = 3;
const MAX_STEPS: usize = 48;
const MAX_REQUEST_AMOUNT: i128 = 12_000;
const INITIAL_TOKEN_BALANCE: i128 = 2_000_000;
const CREDIT_LIMITS: [i128; BORROWER_COUNT] = [20_000, 35_000, 50_000];
const COLLATERAL_AMOUNTS: [i128; BORROWER_COUNT] = [30_000, 52_500, 75_000];
const INITIAL_TIMESTAMP: u64 = 1_000;

#[derive(Clone, Debug)]
struct RawStep {
    borrower_index: usize,
    wants_draw: bool,
    requested_amount: i128,
}

#[derive(Clone, Copy, Debug)]
enum AppliedAction {
    Draw,
    Repay,
}

struct TestCtx {
    env: Env,
    contract_id: Address,
    borrowers: std::vec::Vec<Address>,
    credit_limits: [i128; BORROWER_COUNT],
}

impl TestCtx {
    fn client(&self) -> CreditClient<'_> {
        CreditClient::new(&self.env, &self.contract_id)
    }
}

fn setup() -> TestCtx {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    env.ledger().set_timestamp(INITIAL_TIMESTAMP);

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);
    client.set_liquidity_source(&contract_id);

    let asset = StellarAssetClient::new(&env, &token_address);
    asset.mint(&contract_id, &INITIAL_TOKEN_BALANCE);

    let token = TokenClient::new(&env, &token_address);
    let mut borrowers = std::vec::Vec::with_capacity(BORROWER_COUNT);

    for index in 0..BORROWER_COUNT {
        let borrower = Address::generate(&env);
        asset.mint(&borrower, &INITIAL_TOKEN_BALANCE);
        token.approve(&borrower, &contract_id, &INITIAL_TOKEN_BALANCE, &u32::MAX);

        client.deposit_collateral(&borrower, &COLLATERAL_AMOUNTS[index]);
        client.open_credit_line(&borrower, &CREDIT_LIMITS[index], &500_u32, &50_u32);

        borrowers.push(borrower);
    }

    TestCtx {
        env,
        contract_id,
        borrowers,
        credit_limits: CREDIT_LIMITS,
    }
}

fn raw_steps_strategy() -> impl Strategy<Value = std::vec::Vec<RawStep>> {
    proptest_vec(
        (0usize..BORROWER_COUNT, any::<bool>(), 1_i128..=MAX_REQUEST_AMOUNT),
        1..=MAX_STEPS,
    )
    .prop_map(|steps| {
        steps.into_iter()
            .map(|(borrower_index, wants_draw, requested_amount)| RawStep {
                borrower_index,
                wants_draw,
                requested_amount,
            })
            .collect()
    })
}

fn sum_modeled(modeled: &[i128]) -> Result<i128, TestCaseError> {
    modeled.iter().try_fold(0_i128, |acc, amount| {
        acc.checked_add(*amount)
            .ok_or_else(|| TestCaseError::fail("modeled total overflow"))
    })
}

fn assert_total_utilized_invariant(ctx: &TestCtx, modeled: &[i128]) -> TestCaseResult {
    let client = ctx.client();
    let expected_total = sum_modeled(modeled)?;

    let mut recomputed_total = 0_i128;
    for (index, borrower) in ctx.borrowers.iter().enumerate() {
        let line = client
            .get_credit_line(borrower)
            .expect("credit line must exist for every borrower in the harness");

        prop_assert_eq!(
            line.utilized_amount,
            modeled[index],
            "per-borrower utilized mismatch for borrower index {index}",
        );

        recomputed_total = recomputed_total
            .checked_add(line.utilized_amount)
            .ok_or_else(|| TestCaseError::fail("recomputed on-chain total overflow"))?;
    }

    let stored_total = client.get_total_utilized();

    prop_assert_eq!(
        stored_total,
        expected_total,
        "stored TotalUtilized diverged from modeled outstanding debt",
    );
    prop_assert_eq!(
        stored_total,
        recomputed_total,
        "stored TotalUtilized diverged from live per-borrower utilization sum",
    );

    Ok(())
}

fn apply_valid_step(ctx: &TestCtx, modeled: &mut [i128], step: &RawStep) -> TestCaseResult {
    let borrower_index = step.borrower_index;
    let borrower = &ctx.borrowers[borrower_index];
    let credit_limit = ctx.credit_limits[borrower_index];
    let utilized_before = modeled[borrower_index];
    let remaining_before = credit_limit
        .checked_sub(utilized_before)
        .ok_or_else(|| TestCaseError::fail("remaining credit underflow"))?;

    let (action, amount) = if step.wants_draw {
        if remaining_before > 0 {
            (AppliedAction::Draw, step.requested_amount.min(remaining_before))
        } else {
            (
                AppliedAction::Repay,
                step.requested_amount.min(utilized_before.max(1)),
            )
        }
    } else if utilized_before > 0 {
        (
            AppliedAction::Repay,
            step.requested_amount.min(utilized_before),
        )
    } else {
        (
            AppliedAction::Draw,
            step.requested_amount.min(remaining_before.max(1)),
        )
    };

    prop_assert!(amount > 0, "materialized action amount must stay positive");

    let client = ctx.client();
    match action {
        AppliedAction::Draw => {
            client.draw_credit(borrower, &amount);
            modeled[borrower_index] = utilized_before
                .checked_add(amount)
                .ok_or_else(|| TestCaseError::fail("modeled draw overflow"))?;
        }
        AppliedAction::Repay => {
            client.repay_credit(borrower, &amount);
            modeled[borrower_index] = utilized_before
                .checked_sub(amount)
                .ok_or_else(|| TestCaseError::fail("modeled repay underflow"))?;
        }
    }

    assert_total_utilized_invariant(ctx, modeled)
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 128,
        .. ProptestConfig::default()
    })]

    #[test]
    fn total_utilized_matches_sum_of_individual_utilization(steps in raw_steps_strategy()) {
        let ctx = setup();
        let mut modeled = vec![0_i128; BORROWER_COUNT];

        assert_total_utilized_invariant(&ctx, &modeled)?;

        for step in &steps {
            apply_valid_step(&ctx, &mut modeled, step)?;
        }

        assert_total_utilized_invariant(&ctx, &modeled)?;
    }
}
