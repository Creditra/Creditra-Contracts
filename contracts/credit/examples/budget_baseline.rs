use creditra_credit::instrument::{
    self, entrypoint, setup_credit_harness, BudgetBaseline, BudgetSample, BATCH_TOLERANCE_PCT,
    DEFAULT_TOLERANCE_PCT,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token, Address, Env,
};
use std::path::Path;

fn push(
    results: &mut Vec<BudgetBaseline>,
    entrypoint: &'static str,
    sample: BudgetSample,
    tolerance_pct: f64,
) {
    eprintln!(
        "{entrypoint}  cpu={}  mem={}",
        sample.cpu_instructions, sample.memory_bytes
    );
    results.push(
        BudgetBaseline::new(entrypoint, sample.cpu_instructions, sample.memory_bytes)
            .with_tolerance_pct(tolerance_pct),
    );
}

fn main() {
    let mut results: Vec<BudgetBaseline> = Vec::new();

    // ── init ─────────────────────────────────────────────────────────────────
    {
        let env = Env::default();
        env.mock_all_auths_allowing_non_root_auth();
        let admin = Address::generate(&env);
        let credit_id = env.register(creditra_credit::Credit, ());
        let credit = creditra_credit::CreditClient::new(&env, &credit_id);
        let sample = BudgetSample::measure(&env, || credit.init(&admin));
        push(
            &mut results,
            entrypoint::INIT,
            sample,
            DEFAULT_TOLERANCE_PCT,
        );
    }

    // ── open_credit_line ─────────────────────────────────────────────────────
    {
        let (env, credit, _tok, _adm, borrower) = setup_credit_harness();
        let sample = BudgetSample::measure(&env, || {
            credit.open_credit_line(&borrower, &1_000_000_i128, &500_u32, &100_u32);
        });
        push(
            &mut results,
            entrypoint::OPEN_CREDIT_LINE,
            sample,
            DEFAULT_TOLERANCE_PCT,
        );
    }

    // ── draw_credit ──────────────────────────────────────────────────────────
    {
        let (env, credit, _token, _admin, borrower) = setup_credit_harness();
        credit.open_credit_line(&borrower, &1_000_000_i128, &500_u32, &100_u32);
        credit.deposit_collateral(&borrower, &200_000_i128);
        let sample = BudgetSample::measure(&env, || {
            credit.draw_credit(&borrower, &100_000_i128);
        });
        push(
            &mut results,
            entrypoint::DRAW_CREDIT,
            sample,
            DEFAULT_TOLERANCE_PCT,
        );
    }

    // ── repay_credit ─────────────────────────────────────────────────────────
    {
        let (env, credit, _token, _admin, borrower) = setup_credit_harness();
        credit.open_credit_line(&borrower, &1_000_000_i128, &500_u32, &100_u32);
        credit.deposit_collateral(&borrower, &200_000_i128);
        credit.draw_credit(&borrower, &100_000_i128);
        let sample = BudgetSample::measure(&env, || {
            credit.repay_credit(&borrower, &50_000_i128);
        });
        push(
            &mut results,
            entrypoint::REPAY_CREDIT,
            sample,
            DEFAULT_TOLERANCE_PCT,
        );
    }

    // ── update_risk_parameters ───────────────────────────────────────────────
    {
        let (env, credit, _tok, _adm, borrower) = setup_credit_harness();
        credit.open_credit_line(&borrower, &1_000_000_i128, &500_u32, &100_u32);
        let sample = BudgetSample::measure(&env, || {
            credit.update_risk_parameters(&borrower, &900_000_i128, &400_u32, &50_u32);
        });
        push(
            &mut results,
            entrypoint::UPDATE_RISK_PARAMETERS,
            sample,
            DEFAULT_TOLERANCE_PCT,
        );
    }

    // ── set_rate_formula_config ──────────────────────────────────────────────
    {
        let (env, credit, ..) = setup_credit_harness();
        let sample = BudgetSample::measure(&env, || {
            credit.set_rate_formula_config(&200_u32, &10_u32, &100_u32, &2_000_u32);
        });
        push(
            &mut results,
            entrypoint::SET_RATE_FORMULA_CONFIG,
            sample,
            DEFAULT_TOLERANCE_PCT,
        );
    }

    // ── set_credit_limit_bounds ──────────────────────────────────────────────
    {
        let (env, credit, ..) = setup_credit_harness();
        let sample = BudgetSample::measure(&env, || {
            credit.set_credit_limit_bounds(&10_000_i128, &50_000_000_i128);
        });
        push(
            &mut results,
            entrypoint::SET_CREDIT_LIMIT_BOUNDS,
            sample,
            DEFAULT_TOLERANCE_PCT,
        );
    }

    // ── set_utilization_cap ──────────────────────────────────────────────────
    {
        let (env, credit, ..) = setup_credit_harness();
        let addr = Address::generate(&env);
        let sample = BudgetSample::measure(&env, || {
            credit.set_utilization_cap(&addr, &8_000_u32);
        });
        push(
            &mut results,
            entrypoint::SET_UTILIZATION_CAP,
            sample,
            DEFAULT_TOLERANCE_PCT,
        );
    }

    // ── deposit_collateral ───────────────────────────────────────────────────
    {
        let (env, credit, _token, _adm, borrower) = setup_credit_harness();
        credit.open_credit_line(&borrower, &1_000_000_i128, &500_u32, &100_u32);
        let sample = BudgetSample::measure(&env, || {
            credit.deposit_collateral(&borrower, &100_000_i128);
        });
        push(
            &mut results,
            entrypoint::DEPOSIT_COLLATERAL,
            sample,
            DEFAULT_TOLERANCE_PCT,
        );
    }

    // ── withdraw_collateral ──────────────────────────────────────────────────
    {
        let (env, credit, _token, _adm, borrower) = setup_credit_harness();
        credit.open_credit_line(&borrower, &1_000_000_i128, &500_u32, &100_u32);
        credit.deposit_collateral(&borrower, &100_000_i128);
        let sample = BudgetSample::measure(&env, || {
            credit.withdraw_collateral(&borrower, &50_000_i128);
        });
        push(
            &mut results,
            entrypoint::WITHDRAW_COLLATERAL,
            sample,
            DEFAULT_TOLERANCE_PCT,
        );
    }

    // ── accrue_batch (5-borrower batch) ───────────────────────────────────────
    {
        let (env, credit, token, _admin, _admin_addr) = setup_credit_harness();
        let mut accrue_vec = soroban_sdk::Vec::new(&env);
        for _ in 0..5 {
            let b = Address::generate(&env);
            token.mint(&b, &200_000_i128);
            credit.open_credit_line(&b, &500_000_i128, &500_u32, &100_u32);
            credit.deposit_collateral(&b, &150_000_i128);
            credit.draw_credit(&b, &50_000_i128);
            accrue_vec.push_back(b);
        }
        env.ledger().with_mut(|l| l.timestamp += 86_400 * 30);
        let sample = BudgetSample::measure(&env, || {
            credit.accrue_batch(&accrue_vec);
        });
        push(
            &mut results,
            entrypoint::ACCRUE_BATCH,
            sample,
            BATCH_TOLERANCE_PCT,
        );
    }

    // ── freeze_draws ──────────────────────────────────────────────────────
    {
        let (env, credit, ..) = setup();
        let (cpu, mem) = measure(&env, || {
            credit.freeze_draws(&creditra_credit::FreezeReason::LiquidityReserve);
        });
        push(
            &mut results,
            entrypoint::FREEZE_DRAWS,
            sample,
            DEFAULT_TOLERANCE_PCT,
        );
    }

    // ── unfreeze_draws ────────────────────────────────────────────────────
    {
        let (env, credit, ..) = setup();
        credit.freeze_draws(&creditra_credit::FreezeReason::LiquidityReserve);
        let (cpu, mem) = measure(&env, || {
            credit.unfreeze_draws();
        });
        push(
            &mut results,
            entrypoint::UNFREEZE_DRAWS,
            sample,
            DEFAULT_TOLERANCE_PCT,
        );
    }

    // ── default_credit_line ───────────────────────────────────────────────────
    {
        let (env, credit, _token, _admin, borrower) = setup_credit_harness();
        credit.open_credit_line(&borrower, &1_000_000_i128, &500_u32, &100_u32);
        credit.deposit_collateral(&borrower, &500_000_i128);
        credit.draw_credit(&borrower, &300_000_i128);
        env.ledger().with_mut(|l| l.timestamp += 86_400 * 120);
        let sample = BudgetSample::measure(&env, || {
            credit.default_credit_line(&borrower);
        });
        push(
            &mut results,
            entrypoint::DEFAULT_CREDIT_LINE,
            sample,
            DEFAULT_TOLERANCE_PCT,
        );
    }

    // ── close_credit_line ─────────────────────────────────────────────────────
    {
        let (env, credit, _tok, admin, borrower) = setup_credit_harness();
        credit.open_credit_line(&borrower, &1_000_000_i128, &500_u32, &100_u32);
        let sample = BudgetSample::measure(&env, || {
            credit.close_credit_line(&borrower, &admin);
        });
        push(
            &mut results,
            entrypoint::CLOSE_CREDIT_LINE,
            sample,
            DEFAULT_TOLERANCE_PCT,
        );
    }

    assert_eq!(results.len(), entrypoint::ALL.len());

    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let out_path = instrument::write_baselines_to_manifest_dir(manifest_dir, &results);

    eprintln!(
        "\n✓  Wrote {} baselines to {}",
        results.len(),
        out_path.display()
    );
}
