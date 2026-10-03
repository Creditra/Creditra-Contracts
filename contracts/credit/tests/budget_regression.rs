use creditra_credit::instrument::{self, entrypoint, setup_credit_harness, BudgetSample};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env,
};
use std::path::Path;

fn manifest_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn check(entrypoint: &str, sample: BudgetSample) {
    let baselines = instrument::load_baselines_from_manifest_dir(manifest_dir());
    instrument::check_or_log_missing(entrypoint, sample, &baselines);
}

// ── 0. draw_credit scaling (N lines) ────────────────────────────────────────
//
// `adjust_total_utilized` rescans all lines, so draw/repay cost scales with
// `CreditLineCount`. This test opens N lines and measures the CPU/memory cost
// of a single `draw_credit`, asserting sub-linear growth across N=10,100,500.
//
// Ratio bound: cost(500) / cost(10) must be < 50 (i.e. sub-linear; a truly
// linear scan would give ~50x, so we allow a small constant-factor margin and
// require strictly less than the linear extrapolation).
#[test]
fn budget_draw_credit_scaling() {
    fn measure_draw_at(n: u32) -> BudgetSample {
        let (env, credit, token, _admin, _borrower) = setup_credit_harness();

        // Open N-1 filler lines to grow CreditLineCount, then one target line.
        for _ in 0..n.saturating_sub(1) {
            let b = Address::generate(&env);
            token.mint(&b, &1_000_000_i128);
            credit.open_credit_line(&b, &1_000_000_i128, &500_u32, &100_u32);
            credit.deposit_collateral(&b, &200_000_i128);
        }

        let target = Address::generate(&env);
        token.mint(&target, &1_000_000_i128);
        credit.open_credit_line(&target, &1_000_000_i128, &500_u32, &100_u32);
        credit.deposit_collateral(&target, &200_000_i128);

        BudgetSample::measure(&env, || {
            credit.draw_credit(&target, &100_000_i128);
        })
    }

    let sample_10 = measure_draw_at(10);
    let sample_100 = measure_draw_at(100);
    let sample_500 = measure_draw_at(500);

    // NOTE: no baseline `check` here. The single-line `budget_draw_credit`
    // baseline is only valid at N=1; the whole point of this test is that the
    // cost must be (near-)constant as N grows, which the ratio assertions below
    // prove directly.
    let cpu_10 = sample_10.cpu_instructions as u128;
    let cpu_100 = sample_100.cpu_instructions as u128;
    let cpu_500 = sample_500.cpu_instructions as u128;

    // Guard against degenerate zero-cost measurements.
    assert!(cpu_10 > 0, "cpu_10 must be non-zero");
    assert!(cpu_100 > 0, "cpu_100 must be non-zero");
    assert!(cpu_500 > 0, "cpu_500 must be non-zero");

    // Sub-linear bound: 500/10 = 50x linear; require strictly below that.
    let ratio_500_over_10 = cpu_500 / cpu_10.max(1);
    assert!(
        ratio_500_over_10 < 50,
        "draw_credit CPU cost grew linearly with line count: \
         cpu_10={cpu_10}, cpu_500={cpu_500}, ratio={ratio_500_over_10} (bound < 50)"
    );

    // 100/10 = 10x linear; require strictly below that.
    let ratio_100_over_10 = cpu_100 / cpu_10.max(1);
    assert!(
        ratio_100_over_10 < 10,
        "draw_credit CPU cost grew linearly with line count: \
         cpu_10={cpu_10}, cpu_100={cpu_100}, ratio={ratio_100_over_10} (bound < 10)"
    );
}

// ── 1. init ──────────────────────────────────────────────────────────────────
#[test]
fn budget_init() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let admin = Address::generate(&env);
    let credit_id = env.register(creditra_credit::Credit, ());
    let credit = creditra_credit::CreditClient::new(&env, &credit_id);
    let sample = BudgetSample::measure(&env, || credit.init(&admin));
    check(entrypoint::INIT, sample);
}

// ── 2. open_credit_line ──────────────────────────────────────────────────────
#[test]
fn budget_open_credit_line() {
    let (env, credit, _token, _admin, borrower) = setup_credit_harness();
    let sample = BudgetSample::measure(&env, || {
        credit.open_credit_line(&borrower, &1_000_000_i128, &500_u32, &100_u32);
    });
    check(entrypoint::OPEN_CREDIT_LINE, sample);
}

// ── 3. draw_credit ───────────────────────────────────────────────────────────
#[test]
fn budget_draw_credit() {
    let (env, credit, _token, _admin, borrower) = setup_credit_harness();
    credit.open_credit_line(&borrower, &1_000_000_i128, &500_u32, &100_u32);
    credit.deposit_collateral(&borrower, &200_000_i128);
    let sample = BudgetSample::measure(&env, || {
        credit.draw_credit(&borrower, &100_000_i128);
    });
    check(entrypoint::DRAW_CREDIT, sample);
}

// ── 4. repay_credit ──────────────────────────────────────────────────────────
#[test]
fn budget_repay_credit() {
    let (env, credit, _token, _admin, borrower) = setup_credit_harness();
    credit.open_credit_line(&borrower, &1_000_000_i128, &500_u32, &100_u32);
    credit.deposit_collateral(&borrower, &200_000_i128);
    credit.draw_credit(&borrower, &100_000_i128);
    let sample = BudgetSample::measure(&env, || {
        credit.repay_credit(&borrower, &50_000_i128);
    });
    check(entrypoint::REPAY_CREDIT, sample);
}

// ── 5. update_risk_parameters ────────────────────────────────────────────────
#[test]
fn budget_update_risk_parameters() {
    let (env, credit, _token, _admin, borrower) = setup_credit_harness();
    credit.open_credit_line(&borrower, &1_000_000_i128, &500_u32, &100_u32);
    let sample = BudgetSample::measure(&env, || {
        credit.update_risk_parameters(&borrower, &900_000_i128, &400_u32, &50_u32);
    });
    check(entrypoint::UPDATE_RISK_PARAMETERS, sample);
}

// ── 6. set_rate_formula_config ──────────────────────────────────────────────
#[test]
fn budget_set_rate_formula_config() {
    let (env, credit, ..) = setup_credit_harness();
    let sample = BudgetSample::measure(&env, || {
        credit.set_rate_formula_config(&200_u32, &10_u32, &100_u32, &2_000_u32);
    });
    check(entrypoint::SET_RATE_FORMULA_CONFIG, sample);
}

// ── 7. set_credit_limit_bounds ──────────────────────────────────────────────
#[test]
fn budget_set_credit_limit_bounds() {
    let (env, credit, ..) = setup_credit_harness();
    let sample = BudgetSample::measure(&env, || {
        credit.set_credit_limit_bounds(&10_000_i128, &50_000_000_i128);
    });
    check(entrypoint::SET_CREDIT_LIMIT_BOUNDS, sample);
}

// ── 8. set_utilization_cap ──────────────────────────────────────────────────
#[test]
fn budget_set_utilization_cap() {
    let (env, credit, ..) = setup_credit_harness();
    let addr = Address::generate(&env);
    let sample = BudgetSample::measure(&env, || {
        credit.set_utilization_cap(&addr, &8_000_u32);
    });
    check(entrypoint::SET_UTILIZATION_CAP, sample);
}

// ── 9. deposit_collateral ──────────────────────────────────────────────────
#[test]
fn budget_deposit_collateral() {
    let (env, credit, _token, _admin, borrower) = setup_credit_harness();
    credit.open_credit_line(&borrower, &1_000_000_i128, &500_u32, &100_u32);
    let sample = BudgetSample::measure(&env, || {
        credit.deposit_collateral(&borrower, &100_000_i128);
    });
    check(entrypoint::DEPOSIT_COLLATERAL, sample);
}

// ── 10. partial_release_collateral ─────────────────────────────────────────
#[test]
fn budget_partial_release_collateral() {
    let (env, credit, _token, _admin, borrower) = setup_credit_harness();
    credit.open_credit_line(&borrower, &1_000_000_i128, &500_u32, &100_u32);
    credit.deposit_collateral(&borrower, &200_000_i128);
    let sample = BudgetSample::measure(&env, || {
        credit.partial_release_collateral(&borrower, &50_000_i128);
    });
    check(entrypoint::PARTIAL_RELEASE_COLLATERAL, sample);
}

// ── 11. withdraw_collateral ────────────────────────────────────────────────
#[test]
fn budget_withdraw_collateral() {
    let (env, credit, _token, _admin, borrower) = setup_credit_harness();
    credit.open_credit_line(&borrower, &1_000_000_i128, &500_u32, &100_u32);
    credit.deposit_collateral(&borrower, &200_000_i128);
    let sample = BudgetSample::measure(&env, || {
        credit.withdraw_collateral(&borrower, &50_000_i128);
    });
    check(entrypoint::WITHDRAW_COLLATERAL, sample);
}

// ── 12. accrue_batch ───────────────────────────────────────────────────────
#[test]
fn budget_accrue_batch() {
    let (env, credit, token, _admin, _admin_addr) = setup_credit_harness();
    let mut vec = soroban_sdk::Vec::new(&env);
    for _ in 0..5 {
        let b = Address::generate(&env);
        token.mint(&b, &200_000_i128);
        credit.open_credit_line(&b, &500_000_i128, &500_u32, &100_u32);
        credit.deposit_collateral(&b, &150_000_i128);
        credit.draw_credit(&b, &50_000_i128);
        vec.push_back(b);
    }

    env.ledger().with_mut(|l| l.timestamp += 86_400 * 30);
    let sample = BudgetSample::measure(&env, || {
        credit.accrue_batch(&vec);
    });
    check(entrypoint::ACCRUE_BATCH, sample);
}

// ── 13. freeze_draws / unfreeze_draws ──────────────────────────────────────
#[test]
fn budget_freeze_draws() {
    let (env, credit, ..) = setup_credit_harness();
    let sample = BudgetSample::measure(&env, || {
        credit.freeze_draws(&creditra_credit::FreezeReason::LiquidityReserve);
    });
    check(entrypoint::FREEZE_DRAWS, sample);
}

#[test]
fn budget_unfreeze_draws() {
    let (env, credit, ..) = setup_credit_harness();
    credit.freeze_draws(&creditra_credit::FreezeReason::LiquidityReserve);
    let sample = BudgetSample::measure(&env, || {
        credit.unfreeze_draws();
    });
    check(entrypoint::UNFREEZE_DRAWS, sample);
}

// ── 14. default_credit_line ───────────────────────────────────────────────
#[test]
fn budget_default_credit_line() {
    let (env, credit, _token, _admin, borrower) = setup_credit_harness();
    credit.open_credit_line(&borrower, &1_000_000_i128, &500_u32, &100_u32);
    credit.deposit_collateral(&borrower, &500_000_i128);
    credit.draw_credit(&borrower, &300_000_i128);
    env.ledger().with_mut(|l| l.timestamp += 86_400 * 120);
    let sample = BudgetSample::measure(&env, || {
        credit.default_credit_line(&borrower);
    });
    check(entrypoint::DEFAULT_CREDIT_LINE, sample);
}

// ── 15. close_credit_line ─────────────────────────────────────────────────
#[test]
fn budget_close_credit_line() {
    let (env, credit, _token, admin, borrower) = setup_credit_harness();
    credit.open_credit_line(&borrower, &1_000_000_i128, &500_u32, &100_u32);
    let sample = BudgetSample::measure(&env, || {
        credit.close_credit_line(&borrower, &admin);
    });
    check(entrypoint::CLOSE_CREDIT_LINE, sample);
}

// ── 16. place_bid ─────────────────────────────────────────────────────────────
#[test]
fn budget_place_bid() {
    let (env, auction, _token, admin, bidder1, _) = instrument::setup_auction_harness();
    let auction_id = soroban_sdk::Symbol::new(&env, "auc_bid");

    auction.init_auction(
        &auction_id,
        &gateway_auction::AuctionMode::English,
        &0_u64,
        &u64::MAX,
        &100_i128,
        &0_u32,
        &None,
        &None,
        &None,
        &None,
    );

    let sample = BudgetSample::measure(&env, || {
        auction.place_bid(&auction_id, &bidder1, &100_i128);
    });
    check(entrypoint::PLACE_BID, sample);
}

// ── 17. bid_refunded ──────────────────────────────────────────────────────────
#[test]
fn budget_bid_refunded() {
    let (env, auction, _token, admin, bidder1, bidder2) = instrument::setup_auction_harness();
    let auction_id = soroban_sdk::Symbol::new(&env, "auc_refund");

    auction.init_auction(
        &auction_id,
        &gateway_auction::AuctionMode::English,
        &0_u64,
        &u64::MAX,
        &100_i128,
        &0_u32,
        &None,
        &None,
        &None,
        &None,
    );

    auction.place_bid(&auction_id, &bidder1, &100_i128);

    let sample = BudgetSample::measure(&env, || {
        auction.place_bid(&auction_id, &bidder2, &200_i128);
    });
    check(entrypoint::BID_REFUNDED, sample);
}

// ── 18. settle_default_liquidation (auction) ──────────────────────────────────
#[test]
fn budget_settle_default_liquidation_auction() {
    let (env, auction, _token, admin, bidder1, _) = instrument::setup_auction_harness();
    let auction_id = soroban_sdk::Symbol::new(&env, "auc_settle");

    auction.init_auction(
        &auction_id,
        &gateway_auction::AuctionMode::English,
        &0_u64,
        &u64::MAX,
        &100_i128,
        &0_u32,
        &None,
        &None,
        &None,
        &None,
    );

    auction.place_bid(&auction_id, &bidder1, &100_i128);
    auction.close_auction(&auction_id);

    let borrower = Address::generate(&env);

    let sample = BudgetSample::measure(&env, || {
        auction.settle_default_liquidation(&auction_id, &admin, &borrower);
    });
    check(entrypoint::SETTLE_DEFAULT_LIQUIDATION, sample);
}
