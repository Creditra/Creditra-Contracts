// SPDX-License-Identifier: MIT

//! Integration tests for the global protocol exposure cap (`max_total_exposure`).
//!
//! The cap enforces: `total_utilized + draw_amount <= max_total_exposure`.
//! It is checked on every `draw_credit` call and bypassed by `repay_credit` and
//! `forgive_debt` (those reduce exposure, never increase it).
//!
//! Covered scenarios:
//! - Happy path: draw succeeds when under cap
//! - Draw exactly at cap succeeds (boundary)
//! - Draw that would exceed cap reverts with `ExposureCapExceeded` (#31)
//! - Cap is admin-configurable; non-admin is rejected
//! - Setting cap = 0 removes it (draws unrestricted again)
//! - Negative cap value reverts with `InvalidAmount`
//! - Accumulator consistency: repay/forgive reduce exposure, re-enabling draws
//! - Multi-borrower: cap applies across all lines collectively
//! - Cap below current total blocks draws but repay still works
//! - get_max_total_exposure returns None before set, Some after
//!
//! ## Idle interest accrual (#1364)
//!
//! The cap compares a *stored* accumulator (`TotalUtilized`) against the draw.
//! Interest is capitalised lazily, so a line that has been idle for a long time
//! contributes only its principal to that accumulator. The scenarios below pin
//! the resulting staleness rather than a fix: a draw taken after a year of idle
//! time is evaluated against principal-only exposure, and because
//! `draw_credit` persists the line's fresh accrual *after* the cap check, the
//! persisted total can end up above the configured cap. Materialising the same
//! accrual beforehand makes the identical draw revert. Changing the contract's
//! ordering is out of scope here — these tests exist to make the behaviour
//! observable and quantified.

use creditra_credit::types::ContractError;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{Address, Env};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn setup(env: &Env) -> (CreditClient<'_>, Address, Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token = token_id.address();
    client.set_liquidity_token(&token);

    // Mint reserve tokens into the contract (liquidity source = contract address by default).
    StellarAssetClient::new(env, &token).mint(&contract_id, &1_000_000_i128);

    // Collateral must cover the 150 % minimum ratio at the largest draw below
    // (9_000), otherwise `draw_credit` reverts with #35 before the cap is read.
    let collateral = 15_000_i128;
    StellarAssetClient::new(env, &token).mint(&borrower, &collateral);

    client.open_credit_line(&borrower, &10_000_i128, &300_u32, &50_u32);
    client.deposit_collateral(&borrower, &collateral);

    (client, admin, borrower, contract_id)
}

fn setup_multi(
    env: &Env,
    borrower_count: usize,
) -> (CreditClient<'_>, Address, std::vec::Vec<Address>, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    StellarAssetClient::new(env, &token).mint(&contract_id, &1_000_000_i128);

    let mut borrowers = std::vec::Vec::new();
    for _ in 0..borrower_count {
        let b = Address::generate(env);
        // 1_500 collateral covers the full 1_000 limit at the 150 % minimum ratio.
        StellarAssetClient::new(env, &token).mint(&b, &1_500_i128);
        client.open_credit_line(&b, &1_000_i128, &300_u32, &50_u32);
        client.deposit_collateral(&b, &1_500_i128);
        borrowers.push(b);
    }

    (client, admin, borrowers, contract_id)
}

/// One Julian year in ledger seconds, matching `math_utils::SECONDS_PER_YEAR`.
///
/// At this duration a `300` bps line accrues exactly 3 % of principal
/// (`principal * 300 * ONE_YEAR / (10_000 * ONE_YEAR) = principal * 3 / 100`),
/// which keeps the expected interest figures in the #1364 scenarios exact
/// rather than rounded.
const ONE_YEAR: u64 = 31_557_600;

/// Two-line setup used by the idle-accrual scenarios.
///
/// Both borrowers get `credit_limit` of credit, an interest rate of
/// `rate_bps` and `collateral_each` deposited, so at every utilisation the
/// scenarios reach the 150 % minimum collateral ratio still holds and the
/// exposure-cap decision is the only thing under test.
fn setup_two_lines(
    env: &Env,
    credit_limit: i128,
    rate_bps: u32,
    collateral_each: i128,
) -> (CreditClient<'_>, Address, Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let borrower_a = Address::generate(env);
    let borrower_b = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token = token_id.address();
    client.set_liquidity_token(&token);

    // Liquidity reserve held by the contract (default liquidity source).
    StellarAssetClient::new(env, &token).mint(&contract_id, &1_000_000_i128);

    for borrower in [&borrower_a, &borrower_b] {
        StellarAssetClient::new(env, &token).mint(borrower, &collateral_each);
        client.open_credit_line(borrower, &credit_limit, &rate_bps, &50_u32);
        client.deposit_collateral(borrower, &collateral_each);
    }

    (client, admin, borrower_a, borrower_b)
}

// ── Basic cap management ──────────────────────────────────────────────────────

#[test]
fn get_max_total_exposure_returns_none_before_set() {
    let env = Env::default();
    let (client, _admin, _borrower, _cid) = setup(&env);
    assert_eq!(client.get_max_total_exposure(), None);
}

#[test]
fn set_and_get_max_total_exposure_round_trips() {
    let env = Env::default();
    let (client, _admin, _borrower, _cid) = setup(&env);
    client.set_max_total_exposure(&5_000_i128);
    assert_eq!(client.get_max_total_exposure(), Some(5_000_i128));
}

#[test]
fn set_max_total_exposure_zero_removes_cap() {
    let env = Env::default();
    let (client, _admin, _borrower, _cid) = setup(&env);
    client.set_max_total_exposure(&5_000_i128);
    assert_eq!(client.get_max_total_exposure(), Some(5_000_i128));
    client.set_max_total_exposure(&0_i128);
    assert_eq!(client.get_max_total_exposure(), None);
}

#[test]
fn set_max_total_exposure_can_be_updated() {
    let env = Env::default();
    let (client, _admin, _borrower, _cid) = setup(&env);
    client.set_max_total_exposure(&3_000_i128);
    client.set_max_total_exposure(&7_500_i128);
    assert_eq!(client.get_max_total_exposure(), Some(7_500_i128));
}

// ── Authorization ─────────────────────────────────────────────────────────────

#[test]
#[should_panic]
fn set_max_total_exposure_requires_admin() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    // Drop all auths so the next call is unauthorized.
    let env2 = Env::default();
    let client2 = CreditClient::new(&env2, &contract_id);
    client2.set_max_total_exposure(&1_000_i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn set_max_total_exposure_rejects_negative_value() {
    let env = Env::default();
    let (client, _admin, _borrower, _cid) = setup(&env);
    client.set_max_total_exposure(&-1_i128);
}

// ── Draw enforcement ──────────────────────────────────────────────────────────

#[test]
fn draw_succeeds_when_under_cap() {
    let env = Env::default();
    let (client, _admin, borrower, _cid) = setup(&env);
    client.set_max_total_exposure(&5_000_i128);

    client.draw_credit(&borrower, &1_000_i128);

    assert_eq!(client.get_total_utilized(), 1_000);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().utilized_amount,
        1_000
    );
}

#[test]
fn draw_succeeds_at_exact_cap_boundary() {
    let env = Env::default();
    let (client, _admin, borrower, _cid) = setup(&env);
    client.set_max_total_exposure(&3_000_i128);

    // Draw exactly up to the cap — must not revert.
    client.draw_credit(&borrower, &3_000_i128);
    assert_eq!(client.get_total_utilized(), 3_000);
}

#[test]
#[should_panic(expected = "Error(Contract, #31)")]
fn draw_reverts_when_exceeding_cap_by_one() {
    let env = Env::default();
    let (client, _admin, borrower, _cid) = setup(&env);
    client.set_max_total_exposure(&500_i128);

    client.draw_credit(&borrower, &501_i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #31)")]
fn draw_reverts_when_second_draw_would_exceed_cap() {
    let env = Env::default();
    let (client, _admin, borrower, _cid) = setup(&env);
    client.set_max_total_exposure(&600_i128);

    client.draw_credit(&borrower, &400_i128);
    // total_utilized = 400; cap = 600; next draw of 201 → projected = 601 > 600
    client.draw_credit(&borrower, &201_i128);
}

#[test]
fn draw_without_cap_is_unrestricted() {
    let env = Env::default();
    let (client, _admin, borrower, _cid) = setup(&env);
    // No cap set — large draw within line limit succeeds.
    client.draw_credit(&borrower, &9_000_i128);
    assert_eq!(client.get_total_utilized(), 9_000);
}

#[test]
fn removing_cap_re_enables_large_draws() {
    let env = Env::default();
    let (client, _admin, borrower, _cid) = setup(&env);
    client.set_max_total_exposure(&200_i128);

    client.draw_credit(&borrower, &200_i128);
    // Would fail with cap in place; remove it first.
    client.set_max_total_exposure(&0_i128);
    client.draw_credit(&borrower, &500_i128);

    assert_eq!(client.get_total_utilized(), 700);
}

#[test]
fn global_cap_blocks_draws_above_limit_for_a_single_borrower() {
    let env = Env::default();
    let (client, _admin, borrower, _cid) = setup(&env);

    client.set_max_total_exposure(&3_000_i128);

    client.draw_credit(&borrower, &3_000_i128);
    assert_eq!(client.get_total_utilized(), 3_000);

    let result = client.try_draw_credit(&borrower, &1_i128);
    assert!(result.is_err());
}

#[test]
fn global_cap_can_be_cleared() {
    let env = Env::default();
    let (client, _admin, borrower, _cid) = setup(&env);

    client.set_max_total_exposure(&2_000_i128);
    client.draw_credit(&borrower, &2_000_i128);

    client.set_max_total_exposure(&0_i128);
    client.draw_credit(&borrower, &1_000_i128);

    assert_eq!(client.get_total_utilized(), 3_000);
}

// ── Accumulator consistency after repay/forgive ───────────────────────────────

#[test]
fn repay_reduces_total_utilized_and_re_enables_draws() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    StellarAssetClient::new(&env, &token).mint(&contract_id, &10_000_i128);
    // Collateral for the 1_000 draw below (150 % of utilization).
    StellarAssetClient::new(&env, &token).mint(&borrower, &1_500_i128);
    client.open_credit_line(&borrower, &5_000_i128, &300_u32, &50_u32);
    client.deposit_collateral(&borrower, &1_500_i128);

    client.set_max_total_exposure(&1_000_i128);
    client.draw_credit(&borrower, &1_000_i128);
    assert_eq!(client.get_total_utilized(), 1_000);

    // Repay 400 — total drops to 600, cap is 1_000 so next draw of 400 should work.
    StellarAssetClient::new(&env, &token).mint(&borrower, &400_i128);
    soroban_sdk::token::Client::new(&env, &token).approve(
        &borrower,
        &contract_id,
        &400_i128,
        &9_999_u32,
    );
    client.repay_credit(&borrower, &400_i128);
    assert_eq!(client.get_total_utilized(), 600);

    client.draw_credit(&borrower, &400_i128);
    assert_eq!(client.get_total_utilized(), 1_000);
}

#[test]
fn forgive_debt_reduces_total_utilized_and_re_enables_draws() {
    let env = Env::default();
    let (client, _admin, borrower, _cid) = setup(&env);
    client.set_max_total_exposure(&1_000_i128);

    client.draw_credit(&borrower, &1_000_i128);
    assert_eq!(client.get_total_utilized(), 1_000);

    // Forgive 500 — total drops to 500, next draw of 500 should succeed.
    client.forgive_debt(&borrower, &500_i128);
    assert_eq!(client.get_total_utilized(), 500);

    client.draw_credit(&borrower, &500_i128);
    assert_eq!(client.get_total_utilized(), 1_000);
}

// ── Multi-borrower cap enforcement ───────────────────────────────────────────

#[test]
fn cap_applies_across_multiple_borrowers() {
    let env = Env::default();
    let (client, _admin, borrowers, _cid) = setup_multi(&env, 3);

    // Each borrower has a 1_000 limit; set protocol cap at 2_000.
    client.set_max_total_exposure(&2_000_i128);

    let b0 = borrowers[0].clone();
    let b1 = borrowers[1].clone();
    let b2 = borrowers[2].clone();

    client.draw_credit(&b0, &800_i128); // total = 800
    client.draw_credit(&b1, &800_i128); // total = 1_600
    client.draw_credit(&b2, &400_i128); // total = 2_000, exactly at cap

    assert_eq!(client.get_total_utilized(), 2_000);
}

#[test]
#[should_panic(expected = "Error(Contract, #31)")]
fn cap_blocks_third_borrower_that_would_exceed_aggregate() {
    let env = Env::default();
    let (client, _admin, borrowers, _cid) = setup_multi(&env, 3);

    client.set_max_total_exposure(&2_000_i128);

    let b0 = borrowers[0].clone();
    let b1 = borrowers[1].clone();
    let b2 = borrowers[2].clone();

    client.draw_credit(&b0, &800_i128);
    client.draw_credit(&b1, &800_i128);
    // total = 1_600; cap = 2_000; draw 401 → projected 2_001 > 2_000
    client.draw_credit(&b2, &401_i128);
}

#[test]
fn cap_includes_interest_accrued_on_idle_credit_lines() {
    let env = Env::default();
    env.mock_all_auths();
    // Ensure the initial draws establish a non-zero accrual checkpoint.
    env.ledger().set_timestamp(1_000);
    let (client, _admin, borrowers, _cid) = setup_multi(&env, 2);
    let borrower_a = borrowers[0].clone();
    let borrower_b = borrowers[1].clone();

    client.draw_credit(&borrower_a, &900_i128);
    client.draw_credit(&borrower_b, &900_i128);
    assert_eq!(client.get_total_utilized(), 1_800);

    // 900 at 3% for one Julian year accrues 27 on each line.  A 10-unit
    // draw would look like 1,810 against the stale accumulator, but its
    // actual post-draw exposure is 1,800 + 27 + 27 + 10 = 1,864.
    env.ledger().set_timestamp(1_000 + 31_557_600);
    client.set_max_total_exposure(&1_863_i128);

    let result = client.try_draw_credit(&borrower_a, &10_i128);
    assert!(result.is_err(), "idle-line interest must count toward the cap");

    // Removing the cap is still an explicit opt-out, even after the same
    // accrued-interest scenario.
    client.set_max_total_exposure(&0_i128);
    client.draw_credit(&borrower_a, &10_i128);
    assert_eq!(client.get_credit_line(&borrower_a).unwrap().utilized_amount, 937);
    assert_eq!(client.get_credit_line(&borrower_b).unwrap().utilized_amount, 900);
}

#[test]
fn cap_below_current_total_blocks_new_draws_but_not_repayments() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    StellarAssetClient::new(&env, &token).mint(&contract_id, &10_000_i128);
    // Collateral for the 2_000 draw below (150 % of utilization).
    StellarAssetClient::new(&env, &token).mint(&borrower, &3_000_i128);
    client.open_credit_line(&borrower, &5_000_i128, &300_u32, &50_u32);
    client.deposit_collateral(&borrower, &3_000_i128);

    // Draw 2_000 without a cap, then retroactively set cap below current total.
    client.draw_credit(&borrower, &2_000_i128);
    assert_eq!(client.get_total_utilized(), 2_000);

    client.set_max_total_exposure(&1_500_i128); // cap < current total

    // Any new draw must revert even for amount = 1.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.draw_credit(&borrower, &1_i128);
    }));
    assert!(result.is_err(), "draw should revert when projected > cap");

    // Repayment must still succeed regardless of cap.
    StellarAssetClient::new(&env, &token).mint(&borrower, &500_i128);
    soroban_sdk::token::Client::new(&env, &token).approve(
        &borrower,
        &contract_id,
        &500_i128,
        &9_999_u32,
    );
    client.repay_credit(&borrower, &500_i128);
    assert_eq!(client.get_total_utilized(), 1_500);
}

// ── Total utilized invariant with cap ────────────────────────────────────────

#[test]
fn total_utilized_matches_sum_of_credit_lines_with_cap_active() {
    let env = Env::default();
    let (client, _admin, borrowers, _cid) = setup_multi(&env, 4);

    client.set_max_total_exposure(&3_000_i128);

    // Draw varying amounts from each borrower.
    let amounts = [300_i128, 500_i128, 700_i128, 400_i128];
    for (i, &amt) in amounts.iter().enumerate() {
        let b = borrowers[i].clone();
        client.draw_credit(&b, &amt);
    }

    let total_from_accumulator = client.get_total_utilized();

    // Verify by summing individual credit lines.
    let mut total_from_lines = 0_i128;
    for b in borrowers.iter() {
        total_from_lines += client.get_credit_line(b).unwrap().utilized_amount;
    }

    assert_eq!(total_from_accumulator, total_from_lines);
    assert_eq!(total_from_accumulator, 1_900);
}

#[test]
fn total_utilized_stays_consistent_after_mixed_draw_repay_forgive() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    StellarAssetClient::new(&env, &token).mint(&contract_id, &10_000_i128);
    // Collateral for the 3_000 of draws below (150 % of utilization).
    StellarAssetClient::new(&env, &token).mint(&borrower, &4_500_i128);
    client.open_credit_line(&borrower, &5_000_i128, &300_u32, &50_u32);
    client.deposit_collateral(&borrower, &4_500_i128);

    client.set_max_total_exposure(&4_000_i128);

    client.draw_credit(&borrower, &2_000_i128);
    assert_eq!(client.get_total_utilized(), 2_000);

    client.draw_credit(&borrower, &1_000_i128);
    assert_eq!(client.get_total_utilized(), 3_000);

    StellarAssetClient::new(&env, &token).mint(&borrower, &500_i128);
    soroban_sdk::token::Client::new(&env, &token).approve(
        &borrower,
        &contract_id,
        &500_i128,
        &9_999_u32,
    );
    client.repay_credit(&borrower, &500_i128);
    assert_eq!(client.get_total_utilized(), 2_500);

    client.forgive_debt(&borrower, &300_i128);
    assert_eq!(client.get_total_utilized(), 2_200);

    // Accumulator must equal the stored line's utilized_amount.
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(client.get_total_utilized(), line.utilized_amount);
}

// ── Idle interest accrual vs the cap (#1364) ─────────────────────────────────
//
// Shared state for the three scenarios below:
//   * cap            = 20_000
//   * borrower A     = 10_000 drawn at 300 bps
//   * borrower B     =  9_000 drawn at 300 bps
//   * stored total   = 19_000 (principal only — interest is lazy)
// One Julian year later each line owes 3 % of its principal: 300 for A, 270
// for B. Nothing has been persisted, so the accumulator the cap reads is still
// 19_000 and the real economic exposure is 19_570.

const CAP: i128 = 20_000;
const A_DRAWN: i128 = 10_000;
const B_DRAWN: i128 = 9_000;
const A_IDLE_INTEREST: i128 = 300;
const B_IDLE_INTEREST: i128 = 270;

/// Drive the shared near-cap state and idle for one year.
///
/// Returns `(client, borrower_a, borrower_b)` with no accrual materialised.
fn near_cap_after_one_idle_year(
    env: &Env,
) -> (CreditClient<'_>, Address, Address) {
    let (client, _admin, a, b) = setup_two_lines(env, 20_000, 300, 20_000);
    client.set_max_total_exposure(&CAP);
    client.draw_credit(&a, &A_DRAWN);
    client.draw_credit(&b, &B_DRAWN);

    env.ledger().with_mut(|li| li.timestamp += ONE_YEAR);

    (client, a, b)
}

#[test]
fn idle_accrual_is_invisible_to_the_cap_check() {
    let env = Env::default();
    let (client, a, b) = near_cap_after_one_idle_year(&env);

    // Nothing has materialised the interest yet: both the accumulator and the
    // lazily-read line still report principal only.
    assert_eq!(client.get_total_utilized(), A_DRAWN + B_DRAWN);
    assert_eq!(client.get_credit_line(&a).unwrap().utilized_amount, A_DRAWN);
    assert_eq!(client.get_credit_line(&b).unwrap().utilized_amount, B_DRAWN);

    // A draws 900 more. The cap check projects 19_000 + 900 = 19_900 <= 20_000,
    // so it passes — it never sees the 570 of accrued interest already owed.
    client.draw_credit(&a, &900);

    // Persisting the draw materialises A's year of interest (300) *after* the cap
    // check, so the stored total overshoots the cap by exactly 200.
    assert_eq!(
        client.get_total_utilized(),
        A_DRAWN + B_DRAWN + A_IDLE_INTEREST + 900
    );
    let cap = client.get_max_total_exposure().unwrap();
    assert_eq!(cap, CAP);
    assert_eq!(
        client.get_total_utilized() - cap,
        200,
        "accrued interest (300) minus the 100 of headroom is realised above the cap"
    );

    // B's 270 is still untouched: materialising it takes the total to 20_470,
    // 470 above the cap, while A stays put because its checkpoint is current.
    client.accrue_batch(&soroban_sdk::vec![&env, a.clone(), b.clone()]);
    assert_eq!(
        client.get_total_utilized(),
        A_DRAWN + B_DRAWN + A_IDLE_INTEREST + B_IDLE_INTEREST + 900
    );

    // With the real exposure now visible, any further draw is refused.
    assert!(client.try_draw_credit(&a, &1_i128).is_err());
}

#[test]
#[should_panic(expected = "Error(Contract, #31)")]
fn materialising_accrual_first_changes_the_decision() {
    let env = Env::default();
    let (client, a, b) = near_cap_after_one_idle_year(&env);

    // Materialise both lines before drawing — the only difference from
    // `idle_accrual_is_invisible_to_the_cap_check`.
    client.accrue_batch(&soroban_sdk::vec![&env, a.clone(), b.clone()]);
    assert_eq!(
        client.get_total_utilized(),
        A_DRAWN + A_IDLE_INTEREST + B_DRAWN + B_IDLE_INTEREST
    );

    // The identical 900 draw now projects 19_570 + 900 = 20_470 > 20_000.
    client.draw_credit(&a, &900);
}

#[test]
fn removing_cap_after_idle_accrual_still_re_enables_draws() {
    let env = Env::default();
    let (client, a, b) = near_cap_after_one_idle_year(&env);

    // Same year of idle interest, materialised: total is 19_570 against a
    // 20_000 cap, so the 900 draw would still fit.
    client.accrue_batch(&soroban_sdk::vec![&env, a.clone(), b.clone()]);

    // Remove the cap entirely — even after a year of accrual this must clear it.
    client.set_max_total_exposure(&0_i128);
    assert_eq!(client.get_max_total_exposure(), None);

    client.draw_credit(&a, &900);
    assert_eq!(
        client.get_total_utilized(),
        A_DRAWN + A_IDLE_INTEREST + B_DRAWN + B_IDLE_INTEREST + 900
    );

    // With the cap gone the accumulator is allowed to exceed the old limit.
    assert!(client.get_total_utilized() > CAP);
}

// ── Error discriminant stability ──────────────────────────────────────────────

#[test]
fn exposure_cap_error_discriminant_is_31() {
    // The ContractError discriminants are frozen per the stability guarantee.
    // This test pins the numeric value so a rename or reorder is caught immediately.
    // 30 is `TreasuryNotSet`; the cap error has always been 31.
    assert_eq!(ContractError::ExposureCapExceeded as u32, 31);
    assert_eq!(ContractError::TreasuryNotSet as u32, 30);
}
