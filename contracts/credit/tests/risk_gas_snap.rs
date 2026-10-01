// SPDX-License-Identifier: MIT

//! Per-entrypoint CPU/memory gas snapshots for every risk-related entrypoint.
//!
//! These snapshots establish a regression baseline so that future changes to
//! the risk module (rate formula, guardrails, accrual path) are flagged when
//! they shift CPU or memory consumption beyond the configured tolerance.
//!
//! Run with:
//! ```bash
//! cargo test -p creditra-credit --test risk_gas_snap
//! ```
//!
//! To accept updated baselines after an intentional change:
//! ```bash
//! cargo test -p creditra-credit --test risk_gas_snap -- --accept
//! ```

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{
    testutils::{budget::Budget, Address as _, Ledger},
    Address, Env,
};

// ── Snapshot type ────────────────────────────────────────────────────────────

/// Single entrypoint gas snapshot, serialised by `insta` for regression tracking.
#[derive(Debug)]
struct RiskGasSample {
    entrypoint: &'static str,
    cpu_instructions: u64,
    memory_bytes: u64,
}

fn budget(env: &Env) -> Budget {
    env.cost_estimate().budget()
}

fn measure(env: &Env, f: impl FnOnce()) -> (u64, u64) {
    budget(env).reset_unlimited();
    f();
    let cpu = budget(env).cpu_instruction_cost();
    let mem = budget(env).memory_bytes_cost();
    (cpu, mem)
}

fn snap(entrypoint: &'static str, env: &Env, f: impl FnOnce()) {
    let (cpu, mem) = measure(env, f);
    let sample = RiskGasSample {
        entrypoint,
        cpu_instructions: cpu,
        memory_bytes: mem,
    };
    insta::assert_debug_snapshot!(entrypoint, sample);
}

// ── Harness ──────────────────────────────────────────────────────────────────

fn setup() -> (Env, CreditClient<'static>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let credit_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &credit_id);
    client.init(&admin);
    (env, client, admin, borrower)
}

fn setup_with_credit() -> (Env, CreditClient<'static>, Address, Address) {
    let (env, client, admin, borrower) = setup();
    client.open_credit_line(&borrower, &1_000_000_i128, &500_u32, &100_u32);
    (env, client, admin, borrower)
}

fn setup_with_token() -> (Env, CreditClient<'static>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    let token_id = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token = soroban_sdk::token::StellarAssetClient::new(&env, &token_id);
    let token_client = soroban_sdk::token::Client::new(&env, &token_id);

    token.mint(&admin, &1_000_000_000_i128);
    token.mint(&borrower, &500_000_000_i128);

    let credit_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &credit_id);

    token_client.approve(&borrower, &credit_id, &500_000_000_i128, &2000_u32);
    token_client.approve(&admin, &credit_id, &1_000_000_000_i128, &2000_u32);

    client.init(&admin);
    client.set_liquidity_token(&token_id);
    client.set_liquidity_source(&admin);
    client.open_credit_line(&borrower, &1_000_000_i128, &500_u32, &100_u32);
    (env, client, admin, borrower)
}

// ═════════════════════════════════════════════════════════════════════════════
//  1. update_risk_parameters — the core risk entrypoint
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn gas_update_risk_parameters_basic() {
    let (env, client, _admin, borrower) = setup_with_credit();
    snap("update_risk_parameters_basic", &env, || {
        client.update_risk_parameters(&borrower, &900_000_i128, &400_u32, &50_u32);
    });
}

#[test]
fn gas_update_risk_parameters_with_formula() {
    let (env, client, _admin, borrower) = setup_with_credit();
    client.set_rate_formula_config(&200_u32, &10_u32, &100_u32, &2_000_u32);
    snap("update_risk_parameters_with_formula", &env, || {
        client.update_risk_parameters(&borrower, &800_000_i128, &0_u32, &75_u32);
    });
}

#[test]
fn gas_update_risk_parameters_with_rate_change_limits() {
    let (env, client, _admin, borrower) = setup_with_credit();
    client.set_rate_change_limits(&200_u32, &3600_u64);
    snap(
        "update_risk_parameters_with_rate_change_limits",
        &env,
        || {
            client.update_risk_parameters(&borrower, &800_000_i128, &600_u32, &50_u32);
        },
    );
}

#[test]
fn gas_update_risk_parameters_with_borrower_floor() {
    let (env, client, _admin, borrower) = setup_with_credit();
    client.set_borrower_rate_floor(&borrower, &Some(350_u32));
    snap("update_risk_parameters_with_borrower_floor", &env, || {
        client.update_risk_parameters(&borrower, &800_000_i128, &200_u32, &50_u32);
    });
}

#[test]
fn gas_update_risk_parameters_with_borrower_ceiling() {
    let (env, client, _admin, borrower) = setup_with_credit();
    client.set_borrower_rate_ceiling(&borrower, &Some(600_u32));
    snap("update_risk_parameters_with_borrower_ceiling", &env, || {
        client.update_risk_parameters(&borrower, &800_000_i128, &800_u32, &50_u32);
    });
}

#[test]
fn gas_update_risk_parameters_triggers_restricted() {
    let (env, client, _admin, borrower) = setup_with_token();
    client.deposit_collateral(&borrower, &200_000_i128);
    client.draw_credit(&borrower, &100_000_i128);
    snap("update_risk_parameters_triggers_restricted", &env, || {
        client.update_risk_parameters(&borrower, &50_000_i128, &500_u32, &50_u32);
    });
}

#[test]
fn gas_update_risk_parameters_cures_restricted() {
    let (env, client, _admin, borrower) = setup_with_token();
    client.deposit_collateral(&borrower, &200_000_i128);
    client.draw_credit(&borrower, &100_000_i128);
    client.update_risk_parameters(&borrower, &50_000_i128, &500_u32, &50_u32);
    snap("update_risk_parameters_cures_restricted", &env, || {
        client.update_risk_parameters(&borrower, &200_000_i128, &500_u32, &50_u32);
    });
}

#[test]
fn gas_update_risk_parameters_score_zero() {
    let (env, client, _admin, borrower) = setup_with_credit();
    snap("update_risk_parameters_score_zero", &env, || {
        client.update_risk_parameters(&borrower, &1_000_000_i128, &100_u32, &0_u32);
    });
}

#[test]
fn gas_update_risk_parameters_score_max() {
    let (env, client, _admin, borrower) = setup_with_credit();
    snap("update_risk_parameters_score_max", &env, || {
        client.update_risk_parameters(&borrower, &1_000_000_i128, &10_000_u32, &100_u32);
    });
}

// ═════════════════════════════════════════════════════════════════════════════
//  2. set_rate_change_limits
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn gas_set_rate_change_limits() {
    let (env, client, _admin, _borrower) = setup();
    snap("set_rate_change_limits", &env, || {
        client.set_rate_change_limits(&500_u32, &86_400_u64);
    });
}

// ═════════════════════════════════════════════════════════════════════════════
//  3. get_rate_change_limits
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn gas_get_rate_change_limits_none() {
    let (env, client, _admin, _borrower) = setup();
    snap("get_rate_change_limits_none", &env, || {
        client.get_rate_change_limits();
    });
}

#[test]
fn gas_get_rate_change_limits_some() {
    let (env, client, _admin, _borrower) = setup();
    client.set_rate_change_limits(&500_u32, &86_400_u64);
    snap("get_rate_change_limits_some", &env, || {
        client.get_rate_change_limits();
    });
}

// ═════════════════════════════════════════════════════════════════════════════
//  4. set_penalty_surcharge_bps
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn gas_set_penalty_surcharge_bps() {
    let (env, client, _admin, _borrower) = setup();
    snap("set_penalty_surcharge_bps", &env, || {
        client.set_penalty_surcharge_bps(&500_u32);
    });
}

// ═════════════════════════════════════════════════════════════════════════════
//  5. get_penalty_surcharge_bps
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn gas_get_penalty_surcharge_bps_default() {
    let (env, client, _admin, _borrower) = setup();
    snap("get_penalty_surcharge_bps_default", &env, || {
        client.get_penalty_surcharge_bps();
    });
}

#[test]
fn gas_get_penalty_surcharge_bps_set() {
    let (env, client, _admin, _borrower) = setup();
    client.set_penalty_surcharge_bps(&500_u32);
    snap("get_penalty_surcharge_bps_set", &env, || {
        client.get_penalty_surcharge_bps();
    });
}

// ═════════════════════════════════════════════════════════════════════════════
//  6. set_borrower_rate_floor
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn gas_set_borrower_rate_floor_some() {
    let (env, client, _admin, borrower) = setup();
    snap("set_borrower_rate_floor_some", &env, || {
        client.set_borrower_rate_floor(&borrower, &Some(300_u32));
    });
}

#[test]
fn gas_set_borrower_rate_floor_clear() {
    let (env, client, _admin, borrower) = setup();
    client.set_borrower_rate_floor(&borrower, &Some(300_u32));
    snap("set_borrower_rate_floor_clear", &env, || {
        client.set_borrower_rate_floor(&borrower, &None);
    });
}

// ═════════════════════════════════════════════════════════════════════════════
//  7. get_borrower_rate_floor
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn gas_get_borrower_rate_floor_none() {
    let (env, client, _admin, borrower) = setup();
    snap("get_borrower_rate_floor_none", &env, || {
        client.get_borrower_rate_floor(&borrower);
    });
}

#[test]
fn gas_get_borrower_rate_floor_some() {
    let (env, client, _admin, borrower) = setup();
    client.set_borrower_rate_floor(&borrower, &Some(300_u32));
    snap("get_borrower_rate_floor_some", &env, || {
        client.get_borrower_rate_floor(&borrower);
    });
}

// ═════════════════════════════════════════════════════════════════════════════
//  8. set_borrower_rate_ceiling
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn gas_set_borrower_rate_ceiling_some() {
    let (env, client, _admin, borrower) = setup();
    snap("set_borrower_rate_ceiling_some", &env, || {
        client.set_borrower_rate_ceiling(&borrower, &Some(800_u32));
    });
}

#[test]
fn gas_set_borrower_rate_ceiling_clear() {
    let (env, client, _admin, borrower) = setup();
    client.set_borrower_rate_ceiling(&borrower, &Some(800_u32));
    snap("set_borrower_rate_ceiling_clear", &env, || {
        client.set_borrower_rate_ceiling(&borrower, &None);
    });
}

// ═════════════════════════════════════════════════════════════════════════════
//  9. get_borrower_rate_ceiling
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn gas_get_borrower_rate_ceiling_none() {
    let (env, client, _admin, borrower) = setup();
    snap("get_borrower_rate_ceiling_none", &env, || {
        client.get_borrower_rate_ceiling(&borrower);
    });
}

#[test]
fn gas_get_borrower_rate_ceiling_some() {
    let (env, client, _admin, borrower) = setup();
    client.set_borrower_rate_ceiling(&borrower, &Some(800_u32));
    snap("get_borrower_rate_ceiling_some", &env, || {
        client.get_borrower_rate_ceiling(&borrower);
    });
}

// ═════════════════════════════════════════════════════════════════════════════
//  10. set_rate_formula_config
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn gas_set_rate_formula_config() {
    let (env, client, _admin, _borrower) = setup();
    snap("set_rate_formula_config", &env, || {
        client.set_rate_formula_config(&200_u32, &50_u32, &100_u32, &5_000_u32);
    });
}

// ═════════════════════════════════════════════════════════════════════════════
//  11. get_rate_formula_config
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn gas_get_rate_formula_config_none() {
    let (env, client, _admin, _borrower) = setup();
    snap("get_rate_formula_config_none", &env, || {
        client.get_rate_formula_config();
    });
}

#[test]
fn gas_get_rate_formula_config_some() {
    let (env, client, _admin, _borrower) = setup();
    client.set_rate_formula_config(&200_u32, &50_u32, &100_u32, &5_000_u32);
    snap("get_rate_formula_config_some", &env, || {
        client.get_rate_formula_config();
    });
}

// ═════════════════════════════════════════════════════════════════════════════
//  12. clear_rate_formula_config
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn gas_clear_rate_formula_config() {
    let (env, client, _admin, _borrower) = setup();
    client.set_rate_formula_config(&200_u32, &50_u32, &100_u32, &5_000_u32);
    snap("clear_rate_formula_config", &env, || {
        client.clear_rate_formula_config();
    });
}

// ═════════════════════════════════════════════════════════════════════════════
//  13. get_health_factor
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn gas_get_health_factor() {
    let (env, client, _admin, borrower) = setup_with_token();
    client.deposit_collateral(&borrower, &200_000_i128);
    snap("get_health_factor", &env, || {
        client.get_health_factor(&borrower);
    });
}

// ═════════════════════════════════════════════════════════════════════════════
//  14. Combined risk update with all guardrails active
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn gas_update_risk_parameters_all_guardrails() {
    let (env, client, _admin, borrower) = setup_with_credit();
    client.set_rate_formula_config(&200_u32, &10_u32, &100_u32, &5_000_u32);
    client.set_rate_change_limits(&500_u32, &0_u64);
    client.set_borrower_rate_floor(&borrower, &Some(300_u32));
    client.set_borrower_rate_ceiling(&borrower, &Some(4_000_u32));
    client.set_penalty_surcharge_bps(&250_u32);
    snap("update_risk_parameters_all_guardrails", &env, || {
        client.update_risk_parameters(&borrower, &750_000_i128, &0_u32, &60_u32);
    });
}

// ═════════════════════════════════════════════════════════════════════════════
//  15. Aggregate snapshot of all risk entrypoints
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn risk_gas_summary() {
    let (env, client, _admin, borrower) = setup_with_credit();

    let mut samples = std::collections::BTreeMap::new();

    macro_rules! measure_one {
        ($name:expr, $body:block) => {
            let (cpu, mem) = measure(&env, || $body);
            samples.insert($name, (cpu, mem));
        };
    }

    measure_one!("update_risk_parameters", {
        client.update_risk_parameters(&borrower, &900_000_i128, &400_u32, &50_u32);
    });
    measure_one!("set_rate_change_limits", {
        client.set_rate_change_limits(&500_u32, &86_400_u64);
    });
    measure_one!("get_rate_change_limits", {
        client.get_rate_change_limits();
    });
    measure_one!("set_penalty_surcharge_bps", {
        client.set_penalty_surcharge_bps(&500_u32);
    });
    measure_one!("get_penalty_surcharge_bps", {
        client.get_penalty_surcharge_bps();
    });
    measure_one!("set_borrower_rate_floor", {
        client.set_borrower_rate_floor(&borrower, &Some(300_u32));
    });
    measure_one!("get_borrower_rate_floor", {
        client.get_borrower_rate_floor(&borrower);
    });
    measure_one!("set_borrower_rate_ceiling", {
        client.set_borrower_rate_ceiling(&borrower, &Some(800_u32));
    });
    measure_one!("get_borrower_rate_ceiling", {
        client.get_borrower_rate_ceiling(&borrower);
    });
    measure_one!("set_rate_formula_config", {
        client.set_rate_formula_config(&200_u32, &50_u32, &100_u32, &5_000_u32);
    });
    measure_one!("get_rate_formula_config", {
        client.get_rate_formula_config();
    });
    measure_one!("clear_rate_formula_config", {
        client.clear_rate_formula_config();
    });
    measure_one!("get_health_factor", {
        client.get_health_factor(&borrower);
    });

    insta::assert_debug_snapshot!("risk_gas_summary", samples);
}
