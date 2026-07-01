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

// ── 10. withdraw_collateral ────────────────────────────────────────────────
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

// ── 11. accrue_batch ───────────────────────────────────────────────────────
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

// ── 12. freeze_draws / unfreeze_draws ──────────────────────────────────────
#[test]
fn budget_freeze_draws() {
    let baselines = load_baselines();
    let (env, credit, _token, _admin, _borrower) = setup();
    budget(&env).reset_unlimited();
    credit.freeze_draws(&creditra_credit::FreezeReason::LiquidityReserve);
    let cpu = budget(&env).cpu_instruction_cost();
    let mem = budget(&env).memory_bytes_cost();
    if let Some(b) = baselines.get("freeze_draws") {
        assert_within_tolerance("freeze_draws", cpu, mem, b);
    } else {
        println!(
            "[budget_regression] no baseline for 'freeze_draws'; observed cpu={cpu} mem={mem}"
        );
    }
}

#[test]
fn budget_unfreeze_draws() {
    let baselines = load_baselines();
    let (env, credit, _token, _admin, _borrower) = setup();
    credit.freeze_draws(&creditra_credit::FreezeReason::LiquidityReserve);
    budget(&env).reset_unlimited();
    credit.unfreeze_draws();
    let cpu = budget(&env).cpu_instruction_cost();
    let mem = budget(&env).memory_bytes_cost();
    if let Some(b) = baselines.get("unfreeze_draws") {
        assert_within_tolerance("unfreeze_draws", cpu, mem, b);
    } else {
        println!(
            "[budget_regression] no baseline for 'unfreeze_draws'; observed cpu={cpu} mem={mem}"
        );
    }
}

// ── 13. default_credit_line ───────────────────────────────────────────────
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

// ── 14. close_credit_line ─────────────────────────────────────────────────
#[test]
fn budget_close_credit_line() {
    let (env, credit, _token, admin, borrower) = setup_credit_harness();
    credit.open_credit_line(&borrower, &1_000_000_i128, &500_u32, &100_u32);
    let sample = BudgetSample::measure(&env, || {
        credit.close_credit_line(&borrower, &admin);
    });
    check(entrypoint::CLOSE_CREDIT_LINE, sample);
}
