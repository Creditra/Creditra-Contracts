//! Black-box validation of the `instrument` module's public API surface:
//! the entrypoint registry and the baseline load/write/tolerance helpers.
//!
//! Requires the `instrument` Cargo feature (see `Cargo.toml`).

use creditra_credit::instrument::{
    entrypoint, load_baselines_from_manifest_dir, write_baselines_to_manifest_dir, CpuBaseline,
};

#[test]
fn every_state_changing_entrypoint_is_registered() {
    // One entry per state-changing `ExecuteMsg` variant, plus `instantiate`.
    let expected = [
        entrypoint::INSTANTIATE,
        entrypoint::CREATE_CREDIT_LINE,
        entrypoint::CREATE_DRAW,
        entrypoint::REPAY_DRAW,
        entrypoint::ADD_AUDIT_MEMO,
        entrypoint::UPDATE_PROTOCOL_VERSION,
        entrypoint::SET_ORACLE_QUORUM_CONFIG,
        entrypoint::SUBMIT_ORACLE_PRICES,
        entrypoint::SET_LATE_FEE_CONFIG,
    ];

    assert_eq!(entrypoint::ALL.len(), expected.len());
    for name in expected {
        assert!(
            entrypoint::ALL.contains(&name),
            "entrypoint registry is missing '{name}'"
        );
    }
}

#[test]
fn baseline_roundtrips_through_disk_for_every_entrypoint() {
    let dir = std::env::temp_dir().join(format!(
        "creditra_instrument_integration_{}",
        std::process::id()
    ));

    let baselines: Vec<CpuBaseline> = entrypoint::ALL
        .iter()
        .enumerate()
        .map(|(i, name)| CpuBaseline::new(name, 1_000 + i as u64))
        .collect();

    write_baselines_to_manifest_dir(&dir, &baselines);
    let loaded = load_baselines_from_manifest_dir(&dir);

    assert_eq!(loaded.len(), entrypoint::ALL.len());
    for (i, name) in entrypoint::ALL.iter().enumerate() {
        let entry = loaded
            .get(*name)
            .unwrap_or_else(|| panic!("missing baseline for '{name}' after roundtrip"));
        assert_eq!(entry.cpu_nanos, 1_000 + i as u64);
    }

    std::fs::remove_dir_all(&dir).ok();
}
