// SPDX-License-Identifier: MIT
//! Per-entrypoint CPU/memory instrumentation for budget regression baselines.
//!
//! Host-only utilities used by `tests/budget_regression.rs`, the
//! `examples/budget_baseline` generator, and CI gas-regression workflows.
//! This module is not compiled into contract WASM (`target_arch = "wasm32"`).
//!
//! # What
//!
//! Provides a canonical registry of instrumented entrypoints, helpers to
//! sample Soroban [`Budget`] costs around a single invocation, and
//! tolerance-checked comparison against pinned baselines in
//! `test_snapshots/budget.json`.
//!
//! # How
//!
//! Call [`BudgetSample::measure`] with a closure that invokes exactly one
//! entrypoint after any required setup. Compare the sample against a loaded
//! [`BudgetBaseline`] via [`assert_within_tolerance`].
//!
//! # Why
//!
//! Centralising measurement avoids drift between the baseline generator and
//! the regression tests, and gives operators a single module to extend when
//! adding new entrypoints to the gas-regression matrix.

#![cfg(not(target_arch = "wasm32"))]

use soroban_sdk::{
    testutils::{budget::Budget, Address as _},
    token, Address, Env,
};

use std::{collections::HashMap, path::Path};

/// Relative path (from the `creditra-credit` crate root) to the pinned snapshot.
pub const SNAPSHOT_REL_PATH: &str = "test_snapshots/budget.json";

/// Default ± tolerance applied when a baseline omits `tolerance_pct`.
pub const DEFAULT_TOLERANCE_PCT: f64 = 5.0;

/// Higher tolerance for batch entrypoints whose cost scales with input size.
pub const BATCH_TOLERANCE_PCT: f64 = 10.0;

/// Canonical string identifiers for every instrumented entrypoint.
pub mod entrypoint {
    pub const INIT: &str = "init";
    pub const OPEN_CREDIT_LINE: &str = "open_credit_line";
    pub const DRAW_CREDIT: &str = "draw_credit";
    pub const REPAY_CREDIT: &str = "repay_credit";
    pub const UPDATE_RISK_PARAMETERS: &str = "update_risk_parameters";
    pub const SET_RATE_FORMULA_CONFIG: &str = "set_rate_formula_config";
    pub const SET_CREDIT_LIMIT_BOUNDS: &str = "set_credit_limit_bounds";
    pub const SET_UTILIZATION_CAP: &str = "set_utilization_cap";
    pub const DEPOSIT_COLLATERAL: &str = "deposit_collateral";
    pub const WITHDRAW_COLLATERAL: &str = "withdraw_collateral";
    pub const ACCRUE_BATCH: &str = "accrue_batch";
    pub const FREEZE_DRAWS: &str = "freeze_draws";
    pub const UNFREEZE_DRAWS: &str = "unfreeze_draws";
    pub const DEFAULT_CREDIT_LINE: &str = "default_credit_line";
    pub const CLOSE_CREDIT_LINE: &str = "close_credit_line";

    /// Every entrypoint tracked by the gas-regression matrix, in stable order.
    pub const ALL: &[&str] = &[
        INIT,
        OPEN_CREDIT_LINE,
        DRAW_CREDIT,
        REPAY_CREDIT,
        UPDATE_RISK_PARAMETERS,
        SET_RATE_FORMULA_CONFIG,
        SET_CREDIT_LIMIT_BOUNDS,
        SET_UTILIZATION_CAP,
        DEPOSIT_COLLATERAL,
        WITHDRAW_COLLATERAL,
        ACCRUE_BATCH,
        FREEZE_DRAWS,
        UNFREEZE_DRAWS,
        DEFAULT_CREDIT_LINE,
        CLOSE_CREDIT_LINE,
    ];
}

/// Pinned CPU/memory budget for one entrypoint, serialised in `budget.json`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BudgetBaseline {
    pub entrypoint: String,
    pub cpu_instructions: u64,
    pub memory_bytes: u64,
    #[serde(default)]
    pub tolerance_pct: Option<f64>,
}

impl BudgetBaseline {
    pub fn new(entrypoint: &'static str, cpu_instructions: u64, memory_bytes: u64) -> Self {
        Self {
            entrypoint: entrypoint.to_string(),
            cpu_instructions,
            memory_bytes,
            tolerance_pct: Some(DEFAULT_TOLERANCE_PCT),
        }
    }

    pub fn with_tolerance_pct(mut self, tolerance_pct: f64) -> Self {
        self.tolerance_pct = Some(tolerance_pct);
        self
    }

    pub fn effective_tolerance_pct(&self) -> f64 {
        self.tolerance_pct.unwrap_or(DEFAULT_TOLERANCE_PCT)
    }
}

/// Observed CPU/memory cost for a single entrypoint invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetSample {
    pub cpu_instructions: u64,
    pub memory_bytes: u64,
}

impl BudgetSample {
    /// Reset the env budget, run `f` once, then return the consumed resources.
    pub fn measure(env: &Env, f: impl FnOnce()) -> Self {
        budget(env).reset_unlimited();
        f();
        Self {
            cpu_instructions: budget(env).cpu_instruction_cost(),
            memory_bytes: budget(env).memory_bytes_cost(),
        }
    }
}

/// Return the Soroban cost-estimate budget handle for `env`.
pub fn budget(env: &Env) -> Budget {
    env.cost_estimate().budget()
}

/// Assert `sample` is within the baseline tolerance (CPU and memory).
pub fn assert_within_tolerance(entrypoint: &str, sample: BudgetSample, baseline: &BudgetBaseline) {
    let tol = baseline.effective_tolerance_pct() / 100.0;
    let check = |label: &str, observed: u64, pinned: u64| {
        let delta_pct = (observed as f64 - pinned as f64).abs() / (pinned as f64) * 100.0;
        assert!(
            delta_pct <= tol * 100.0,
            "budget regression [{entrypoint}] {label}:\n  observed  = {observed}\n  baseline  = {pinned}\n  delta_pct = {delta_pct:.2} %  (tolerance ±{:.1} %)",
            tol * 100.0
        );
    };
    check(
        "cpu_instructions",
        sample.cpu_instructions,
        baseline.cpu_instructions,
    );
    check("memory_bytes", sample.memory_bytes, baseline.memory_bytes);
}

/// Load baselines keyed by entrypoint name from `manifest_dir`/`SNAPSHOT_REL_PATH`.
pub fn load_baselines_from_manifest_dir(manifest_dir: &Path) -> HashMap<String, BudgetBaseline> {
    let path = manifest_dir.join(SNAPSHOT_REL_PATH);
    if !path.exists() {
        return HashMap::new();
    }
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let list: Vec<BudgetBaseline> =
        serde_json::from_str(&raw).unwrap_or_else(|e| panic!("bad JSON in snapshot: {e}"));
    list.into_iter()
        .map(|b| (b.entrypoint.clone(), b))
        .collect()
}

/// Write `baselines` as pretty JSON to `manifest_dir`/`SNAPSHOT_REL_PATH`.
pub fn write_baselines_to_manifest_dir(
    manifest_dir: &Path,
    baselines: &[BudgetBaseline],
) -> std::path::PathBuf {
    let path = manifest_dir.join(SNAPSHOT_REL_PATH);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|e| panic!("cannot create {}: {e}", parent.display()));
    }
    let json = serde_json::to_string_pretty(baselines).expect("serialization failed");
    std::fs::write(&path, format!("{json}\n"))
        .unwrap_or_else(|e| panic!("cannot write {}: {e}", path.display()));
    path
}

/// Compare `sample` against an optional baseline; log when no baseline exists.
pub fn check_or_log_missing(
    entrypoint: &str,
    sample: BudgetSample,
    baselines: &HashMap<String, BudgetBaseline>,
) {
    if let Some(baseline) = baselines.get(entrypoint) {
        assert_within_tolerance(entrypoint, sample, baseline);
    } else {
        eprintln!(
            "[budget_regression] no baseline for '{entrypoint}'; observed cpu={} mem={}",
            sample.cpu_instructions, sample.memory_bytes
        );
    }
}

/// Shared test harness: admin + borrower + SAC + deployed credit contract.
pub fn setup_credit_harness() -> (
    Env,
    crate::CreditClient<'static>,
    token::StellarAssetClient<'static>,
    Address,
    Address,
) {
    let env = Env::default();
    budget(&env).reset_unlimited();
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

    let credit_id = env.register(crate::Credit, ());
    let credit = crate::CreditClient::new(&env, &credit_id);

    token_client.approve(&borrower, &credit_id, &500_000_000_i128, &2000_u32);
    token_client.approve(&admin, &credit_id, &1_000_000_000_i128, &2000_u32);

    credit.init(&admin);
    credit.set_liquidity_token(&token_id);
    credit.set_liquidity_source(&admin);

    (env, credit, token, admin, borrower)
}
