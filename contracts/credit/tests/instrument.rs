use creditra_credit::instrument::{
    self, assert_within_tolerance, entrypoint, load_baselines_from_manifest_dir,
    write_baselines_to_manifest_dir, BudgetBaseline, BudgetSample, BATCH_TOLERANCE_PCT,
    DEFAULT_TOLERANCE_PCT,
};

#[test]
fn entrypoint_registry_is_unique_and_complete() {
    let mut seen = std::collections::HashSet::new();
    for name in entrypoint::ALL {
        assert!(seen.insert(*name), "duplicate entrypoint id: {name}");
    }
    assert_eq!(entrypoint::ALL.len(), 15);
}

#[test]
fn assert_within_tolerance_accepts_exact_match() {
    let baseline = BudgetBaseline::new(entrypoint::INIT, 100, 200);
    let sample = BudgetSample {
        cpu_instructions: 100,
        memory_bytes: 200,
    };
    assert_within_tolerance(entrypoint::INIT, sample, &baseline);
}

#[test]
#[should_panic(expected = "budget regression")]
fn assert_within_tolerance_rejects_cpu_drift() {
    let baseline = BudgetBaseline::new(entrypoint::INIT, 100, 200);
    let sample = BudgetSample {
        cpu_instructions: 200,
        memory_bytes: 200,
    };
    assert_within_tolerance(entrypoint::INIT, sample, &baseline);
}

#[test]
fn effective_tolerance_pct_defaults_to_five() {
    let mut baseline = BudgetBaseline::new(entrypoint::INIT, 1, 1);
    baseline.tolerance_pct = None;
    assert!((baseline.effective_tolerance_pct() - DEFAULT_TOLERANCE_PCT).abs() < f64::EPSILON);
}

#[test]
fn baseline_roundtrip_json() {
    let dir = std::env::temp_dir().join("creditra_instrument_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let rows = vec![
        BudgetBaseline::new(entrypoint::INIT, 42, 84),
        BudgetBaseline::new(entrypoint::ACCRUE_BATCH, 900, 1800)
            .with_tolerance_pct(BATCH_TOLERANCE_PCT),
    ];
    write_baselines_to_manifest_dir(&dir, &rows);
    let loaded = load_baselines_from_manifest_dir(&dir);
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[entrypoint::INIT].cpu_instructions, 42);
    assert!(
        (loaded[entrypoint::ACCRUE_BATCH].effective_tolerance_pct() - BATCH_TOLERANCE_PCT).abs()
            < f64::EPSILON
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn setup_credit_harness_deploys_contract() {
    let (_env, credit, _token, _admin, borrower) = instrument::setup_credit_harness();
    credit.open_credit_line(&borrower, &1_000_000_i128, &500_u32, &100_u32);
}
