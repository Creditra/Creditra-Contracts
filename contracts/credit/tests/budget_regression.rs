use soroban_sdk::{
    testutils::{budget::Budget, Address as _, Ledger},
    token, Address, Env,
};
use std::{collections::HashMap, path::Path};

const SNAPSHOT_PATH: &str = "test_snapshots/budget.json";
const BUDGET_TOLERANCE_PCT: f64 = 5.0;

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct Baseline {
    entrypoint: String,
    cpu_instructions: u64,
    memory_bytes: u64,
    #[serde(default)]
    tolerance_pct: Option<f64>,
}

fn load_baselines() -> HashMap<String, Baseline> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(SNAPSHOT_PATH);
    if !path.exists() {
        return HashMap::new();
    }
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let list: Vec<Baseline> =
        serde_json::from_str(&raw).unwrap_or_else(|e| panic!("bad JSON in snapshot: {e}"));
    list.into_iter()
        .map(|b| (b.entrypoint.clone(), b))
        .collect()
}

fn assert_within_tolerance(
    entrypoint: &str,
    observed_cpu: u64,
    observed_mem: u64,
    baseline: &Baseline,
) {
    let tol = baseline.tolerance_pct.unwrap_or(BUDGET_TOLERANCE_PCT) / 100.0;
    let check = |label: &str, observed: u64, pinned: u64| {
        let delta_pct = (observed as f64 - pinned as f64).abs() / (pinned as f64) * 100.0;
        assert!(
            delta_pct <= tol * 100.0,
            "budget regression [{entrypoint}] {label}:\n  observed  = {observed}\n  baseline  = {pinned}\n  delta_pct = {delta_pct:.2} %  (tolerance ±{:.1} %)",
            tol * 100.0
        );
    };
    check("cpu_instructions", observed_cpu, baseline.cpu_instructions);
    check("memory_bytes", observed_mem, baseline.memory_bytes);
}

fn setup() -> (
    Env,
    creditra_credit::CreditClient<'static>,
    token::StellarAssetClient<'static>,
    Address,
    Address,
) {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();
    env.mock_all_auths_allowing_non_root_auth();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    let token_id = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token = token::StellarAssetClient::new(&env, &token_id);
    let token_client = token::Client::new(&env, &token_id);

    token.mint(&admin, &1_000_000_000_i128);
    token.mint(&borrower, &500_000_000_i128);

    let credit_id = env.register(creditra_credit::Credit, ());
    let credit = creditra_credit::CreditClient::new(&env, &credit_id);

    token_client.approve(&borrower, &credit_id, &500_000_000_i128, &2000_u32);
    token_client.approve(&admin, &credit_id, &1_000_000_000_i128, &2000_u32);

    credit.init(&admin);
    credit.set_liquidity_token(&token_id);
    credit.set_liquidity_source(&admin);

    (env, credit, token, admin, borrower)
}

fn budget(env: &Env) -> Budget {
    env.cost_estimate().budget()
}

macro_rules! budget_test {
    ($name:ident, $ep:expr, $setup:expr, $call:expr) => {
        #[test]
        fn $name() {
            let baselines = load_baselines();
            let (env, credit, _token, admin, borrower) = $setup;
            budget(&env).reset_unlimited();
            $call;
            let cpu = budget(&env).cpu_instruction_cost();
            let mem = budget(&env).memory_bytes_cost();
            if let Some(baseline) = baselines.get($ep) {
                assert_within_tolerance($ep, cpu, mem, baseline);
            } else {
                println!(
                    "[budget_regression] no baseline for '{ep}'; observed cpu={cpu} mem={mem}",
                    ep = $ep
                );
            }
        }
    };
}

// ── 1. init ──────────────────────────────────────────────────────────────────
#[test]
fn budget_init() {
    let baselines = load_baselines();
    let env = Env::default();
    budget(&env).reset_unlimited();
    env.mock_all_auths_allowing_non_root_auth();
    let admin = Address::generate(&env);
    let credit_id = env.register(creditra_credit::Credit, ());
    let credit = creditra_credit::CreditClient::new(&env, &credit_id);
    budget(&env).reset_unlimited();
    credit.init(&admin);
    let cpu = budget(&env).cpu_instruction_cost();
    let mem = budget(&env).memory_bytes_cost();
    if let Some(b) = baselines.get("init") {
        assert_within_tolerance("init", cpu, mem, b);
    } else {
        println!("[budget_regression] no baseline for 'init'; observed cpu={cpu} mem={mem}");
    }
}

// ── 2. open_credit_line ──────────────────────────────────────────────────────
#[test]
fn budget_open_credit_line() {
    let baselines = load_baselines();
    let (env, credit, _token, _admin, borrower) = setup();
    budget(&env).reset_unlimited();
    credit.open_credit_line(&borrower, &1_000_000_i128, &500_u32, &100_u32);
    let cpu = budget(&env).cpu_instruction_cost();
    let mem = budget(&env).memory_bytes_cost();
    if let Some(b) = baselines.get("open_credit_line") {
        assert_within_tolerance("open_credit_line", cpu, mem, b);
    } else {
        println!(
            "[budget_regression] no baseline for 'open_credit_line'; observed cpu={cpu} mem={mem}"
        );
    }
}

// ── 3. draw_credit ───────────────────────────────────────────────────────────
#[test]
fn budget_draw_credit() {
    let baselines = load_baselines();
    let (env, credit, token, admin, borrower) = setup();
    credit.open_credit_line(&borrower, &1_000_000_i128, &500_u32, &100_u32);
    credit.deposit_collateral(&borrower, &200_000_i128);
    budget(&env).reset_unlimited();
    credit.draw_credit(&borrower, &100_000_i128);
    let cpu = budget(&env).cpu_instruction_cost();
    let mem = budget(&env).memory_bytes_cost();
    if let Some(b) = baselines.get("draw_credit") {
        assert_within_tolerance("draw_credit", cpu, mem, b);
    } else {
        println!("[budget_regression] no baseline for 'draw_credit'; observed cpu={cpu} mem={mem}");
    }
}

// ── 4. repay_credit ──────────────────────────────────────────────────────────
#[test]
fn budget_repay_credit() {
    let baselines = load_baselines();
    let (env, credit, token, admin, borrower) = setup();
    credit.open_credit_line(&borrower, &1_000_000_i128, &500_u32, &100_u32);
    credit.deposit_collateral(&borrower, &200_000_i128);
    credit.draw_credit(&borrower, &100_000_i128);
    budget(&env).reset_unlimited();
    credit.repay_credit(&borrower, &50_000_i128);
    let cpu = budget(&env).cpu_instruction_cost();
    let mem = budget(&env).memory_bytes_cost();
    if let Some(b) = baselines.get("repay_credit") {
        assert_within_tolerance("repay_credit", cpu, mem, b);
    } else {
        println!(
            "[budget_regression] no baseline for 'repay_credit'; observed cpu={cpu} mem={mem}"
        );
    }
}

// ── 5. update_risk_parameters ────────────────────────────────────────────────
#[test]
fn budget_update_risk_parameters() {
    let baselines = load_baselines();
    let (env, credit, _token, _admin, borrower) = setup();
    credit.open_credit_line(&borrower, &1_000_000_i128, &500_u32, &100_u32);
    budget(&env).reset_unlimited();
    credit.update_risk_parameters(&borrower, &900_000_i128, &400_u32, &50_u32);
    let cpu = budget(&env).cpu_instruction_cost();
    let mem = budget(&env).memory_bytes_cost();
    if let Some(b) = baselines.get("update_risk_parameters") {
        assert_within_tolerance("update_risk_parameters", cpu, mem, b);
    } else {
        println!("[budget_regression] no baseline for 'update_risk_parameters'; observed cpu={cpu} mem={mem}");
    }
}

// ── 6. set_rate_formula_config ──────────────────────────────────────────────
#[test]
fn budget_set_rate_formula_config() {
    let baselines = load_baselines();
    let (env, credit, _token, _admin, _borrower) = setup();
    budget(&env).reset_unlimited();
    credit.set_rate_formula_config(&200_u32, &10_u32, &100_u32, &2_000_u32);
    let cpu = budget(&env).cpu_instruction_cost();
    let mem = budget(&env).memory_bytes_cost();
    if let Some(b) = baselines.get("set_rate_formula_config") {
        assert_within_tolerance("set_rate_formula_config", cpu, mem, b);
    } else {
        println!("[budget_regression] no baseline for 'set_rate_formula_config'; observed cpu={cpu} mem={mem}");
    }
}

// ── 7. set_credit_limit_bounds ──────────────────────────────────────────────
#[test]
fn budget_set_credit_limit_bounds() {
    let baselines = load_baselines();
    let (env, credit, _token, _admin, _borrower) = setup();
    budget(&env).reset_unlimited();
    credit.set_credit_limit_bounds(&10_000_i128, &50_000_000_i128);
    let cpu = budget(&env).cpu_instruction_cost();
    let mem = budget(&env).memory_bytes_cost();
    if let Some(b) = baselines.get("set_credit_limit_bounds") {
        assert_within_tolerance("set_credit_limit_bounds", cpu, mem, b);
    } else {
        println!("[budget_regression] no baseline for 'set_credit_limit_bounds'; observed cpu={cpu} mem={mem}");
    }
}

// ── 8. set_utilization_cap ──────────────────────────────────────────────────
#[test]
fn budget_set_utilization_cap() {
    let baselines = load_baselines();
    let (env, credit, _token, _admin, _borrower) = setup();
    let addr = Address::generate(&env);
    budget(&env).reset_unlimited();
    credit.set_utilization_cap(&addr, &8_000_u32);
    let cpu = budget(&env).cpu_instruction_cost();
    let mem = budget(&env).memory_bytes_cost();
    if let Some(b) = baselines.get("set_utilization_cap") {
        assert_within_tolerance("set_utilization_cap", cpu, mem, b);
    } else {
        println!("[budget_regression] no baseline for 'set_utilization_cap'; observed cpu={cpu} mem={mem}");
    }
}

// ── 9. deposit_collateral ──────────────────────────────────────────────────
#[test]
fn budget_deposit_collateral() {
    let baselines = load_baselines();
    let (env, credit, token, _admin, borrower) = setup();
    credit.open_credit_line(&borrower, &1_000_000_i128, &500_u32, &100_u32);
    budget(&env).reset_unlimited();
    credit.deposit_collateral(&borrower, &100_000_i128);
    let cpu = budget(&env).cpu_instruction_cost();
    let mem = budget(&env).memory_bytes_cost();
    if let Some(b) = baselines.get("deposit_collateral") {
        assert_within_tolerance("deposit_collateral", cpu, mem, b);
    } else {
        println!("[budget_regression] no baseline for 'deposit_collateral'; observed cpu={cpu} mem={mem}");
    }
}

// ── 10. withdraw_collateral ────────────────────────────────────────────────
#[test]
fn budget_withdraw_collateral() {
    let baselines = load_baselines();
    let (env, credit, token, _admin, borrower) = setup();
    credit.open_credit_line(&borrower, &1_000_000_i128, &500_u32, &100_u32);
    credit.deposit_collateral(&borrower, &200_000_i128);
    budget(&env).reset_unlimited();
    credit.withdraw_collateral(&borrower, &50_000_i128);
    let cpu = budget(&env).cpu_instruction_cost();
    let mem = budget(&env).memory_bytes_cost();
    if let Some(b) = baselines.get("withdraw_collateral") {
        assert_within_tolerance("withdraw_collateral", cpu, mem, b);
    } else {
        println!("[budget_regression] no baseline for 'withdraw_collateral'; observed cpu={cpu} mem={mem}");
    }
}

// ── 11. accrue_batch ───────────────────────────────────────────────────────
#[test]
fn budget_accrue_batch() {
    let baselines = load_baselines();
    let (env, credit, token, admin, _admin_addr) = setup();
    let token_client = token::Client::new(
        &env,
        &env.register_stellar_asset_contract_v2(admin.clone())
            .address(),
    );

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
    budget(&env).reset_unlimited();
    credit.accrue_batch(&vec);
    let cpu = budget(&env).cpu_instruction_cost();
    let mem = budget(&env).memory_bytes_cost();
    if let Some(b) = baselines.get("accrue_batch") {
        assert_within_tolerance("accrue_batch", cpu, mem, b);
    } else {
        println!(
            "[budget_regression] no baseline for 'accrue_batch'; observed cpu={cpu} mem={mem}"
        );
    }
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
    let baselines = load_baselines();
    let (env, credit, token, admin, borrower) = setup();
    credit.open_credit_line(&borrower, &1_000_000_i128, &500_u32, &100_u32);
    credit.deposit_collateral(&borrower, &500_000_i128);
    credit.draw_credit(&borrower, &300_000_i128);
    env.ledger().with_mut(|l| l.timestamp += 86_400 * 120);
    budget(&env).reset_unlimited();
    credit.default_credit_line(&borrower);
    let cpu = budget(&env).cpu_instruction_cost();
    let mem = budget(&env).memory_bytes_cost();
    if let Some(b) = baselines.get("default_credit_line") {
        assert_within_tolerance("default_credit_line", cpu, mem, b);
    } else {
        println!("[budget_regression] no baseline for 'default_credit_line'; observed cpu={cpu} mem={mem}");
    }
}

// ── 14. close_credit_line ─────────────────────────────────────────────────
#[test]
fn budget_close_credit_line() {
    let baselines = load_baselines();
    let (env, credit, _token, admin, borrower) = setup();
    credit.open_credit_line(&borrower, &1_000_000_i128, &500_u32, &100_u32);
    budget(&env).reset_unlimited();
    credit.close_credit_line(&borrower, &admin);
    let cpu = budget(&env).cpu_instruction_cost();
    let mem = budget(&env).memory_bytes_cost();
    if let Some(b) = baselines.get("close_credit_line") {
        assert_within_tolerance("close_credit_line", cpu, mem, b);
    } else {
        println!(
            "[budget_regression] no baseline for 'close_credit_line'; observed cpu={cpu} mem={mem}"
        );
    }
}
