// SPDX-License-Identifier: MIT

//! Per-entrypoint CPU/memory gas snapshots for lifecycle entrypoints.
//!
//! Run with:
//! ```bash
//! cargo test -p creditra-lifecycle --test gas_snap
//! ```
//!
//! To accept updated baselines:
//! ```bash
//! cargo test -p creditra-lifecycle --test gas_snap -- --accept
//! ```

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{
    testutils::{budget::Budget, Address as _, Ledger},
    Address, Env,
};

#[derive(Debug)]
struct LifecycleGasSample {
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
    let sample = LifecycleGasSample {
        entrypoint,
        cpu_instructions: cpu,
        memory_bytes: mem,
    };
    insta::assert_debug_snapshot!(entrypoint, sample);
}

fn setup() -> (Env, CreditClient<'static>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(admin.clone());
    client.set_liquidity_token(&token_id.address());
    client.set_liquidity_source(&admin);

    (env, client, admin, borrower)
}

fn setup_with_credit() -> (Env, CreditClient<'static>, Address, Address) {
    let (env, client, admin, borrower) = setup();
    client.open_credit_line(&borrower, &1_000_000_i128, &500_u32, &100_u32);
    (env, client, admin, borrower)
}

#[test]
fn gas_open_credit_line() {
    let (env, client, _admin, borrower) = setup();
    snap("open_credit_line", &env, || {
        client.open_credit_line(&borrower, &1_000_000_i128, &500_u32, &100_u32);
    });
}

#[test]
fn gas_close_credit_line() {
    let (env, client, admin, borrower) = setup_with_credit();
    snap("close_credit_line", &env, || {
        client.close_credit_line(&borrower, &admin);
    });
}

#[test]
fn gas_suspend_credit_line() {
    let (env, client, _admin, borrower) = setup_with_credit();
    snap("suspend_credit_line", &env, || {
        client.suspend_credit_line(&borrower);
    });
}

#[test]
fn gas_self_suspend_credit_line() {
    let (env, client, _admin, borrower) = setup_with_credit();
    snap("self_suspend_credit_line", &env, || {
        client.self_suspend_credit_line(&borrower);
    });
}

#[test]
fn gas_default_credit_line() {
    let (env, client, _admin, borrower) = setup_with_credit();
    snap("default_credit_line", &env, || {
        client.default_credit_line(&borrower);
    });
}

#[test]
fn gas_reinstate_credit_line() {
    let (env, client, _admin, borrower) = setup_with_credit();
    client.default_credit_line(&borrower);
    snap("reinstate_credit_line", &env, || {
        client.reinstate_credit_line(&borrower);
    });
}

#[test]
fn lifecycle_gas_summary() {
    let (env, client, admin, borrower) = setup();
    let mut samples = std::collections::BTreeMap::new();

    macro_rules! measure_one {
        ($name:expr, $body:block) => {
            let (cpu, mem) = measure(&env, || $body);
            samples.insert($name, (cpu, mem));
        };
    }

    measure_one!("open_credit_line", {
        client.open_credit_line(&borrower, &1_000_000_i128, &500_u32, &100_u32);
    });
    measure_one!("suspend_credit_line", {
        client.suspend_credit_line(&borrower);
    });
    measure_one!("default_credit_line", {
        client.default_credit_line(&borrower);
    });
    measure_one!("reinstate_credit_line", {
        client.reinstate_credit_line(&borrower);
    });
    measure_one!("self_suspend_credit_line", {
        client.self_suspend_credit_line(&borrower);
    });
    measure_one!("close_credit_line", {
        client.close_credit_line(&borrower, &admin);
    });

    insta::assert_debug_snapshot!("lifecycle_gas_summary", samples);
}
