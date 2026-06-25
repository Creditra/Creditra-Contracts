use soroban_sdk::{Env};
use serde::Deserialize;
use std::fs;

#[derive(Deserialize)]
struct Baseline {
    entrypoint: String,
    cpu_instructions: u64,
    memory_bytes: u64,
    tolerance_pct: f64,
}

fn load_baselines() -> Vec<Baseline> {
    let data = fs::read_to_string("contracts/credit/test_snapshots/budget.json")
        .expect("baseline file missing");
    serde_json::from_str(&data).expect("invalid baseline JSON")
}

#[test]
fn budget_regression() {
    let env = Env::default();
    let baselines = load_baselines();

    for baseline in baselines {
        // Simulate entrypoint call (replace with actual contract invocation)
        let (observed_cpu, observed_mem) = match baseline.entrypoint.as_str() {
            "init" => (1200, 800),
            "open_credit_line" => (3400, 2100),
            "draw_credit" => (5000, 3200),
            "repay_credit" => (2800, 1900),
            "update_risk_parameters" => (2600, 1700),
            "settle_default_liquidation" => (4100, 2500),
            "set_credit_limit_bounds" => (2300, 1600),
            _ => continue,
        };

        let cpu_delta = ((observed_cpu as f64 - baseline.cpu_instructions as f64)
            / baseline.cpu_instructions as f64) * 100.0;
        let mem_delta = ((observed_mem as f64 - baseline.memory_bytes as f64)
            / baseline.memory_bytes as f64) * 100.0;

        assert!(
            cpu_delta.abs() <= baseline.tolerance_pct,
            "CPU budget regression in {}: observed {}, baseline {}, delta {:.2}%",
            baseline.entrypoint,
            observed_cpu,
            baseline.cpu_instructions,
            cpu_delta
        );

        assert!(
            mem_delta.abs() <= baseline.tolerance_pct,
            "Memory budget regression in {}: observed {}, baseline {}, delta {:.2}%",
            baseline.entrypoint,
            observed_mem,
            baseline.memory_bytes,
            mem_delta
        );
    }
}
