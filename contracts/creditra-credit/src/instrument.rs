// SPDX-License-Identifier: MIT
//! Per-entrypoint CPU-time instrumentation for regression baselines.
//!
//! Host-only utilities used by `tests/instrument.rs`, `tests/cpu_regression.rs`,
//! and the `examples/cpu_baseline` generator. This module is not compiled into
//! contract WASM (`target_arch = "wasm32"`) and is gated behind the
//! `instrument` Cargo feature.
//!
//! # What
//!
//! Unlike Soroban, CosmWasm does not expose a metered CPU-instruction budget
//! to host-side test code. This module instead samples **wall-clock CPU
//! time** around a single entrypoint invocation — averaged over several
//! repetitions via [`CpuSample::measure_avg`] to dampen scheduler/allocator
//! noise — and compares the result against a pinned baseline with a
//! tolerance band.
//!
//! # How
//!
//! Call [`CpuSample::measure`] (single sample) or [`CpuSample::measure_avg`]
//! (averaged over `iterations` repetitions — preferred for stable baselines)
//! with a closure that invokes exactly one entrypoint after any required
//! setup. Compare the sample against a loaded [`CpuBaseline`] via
//! [`assert_within_tolerance`], or use [`check_or_log_missing`] to log rather
//! than fail when no baseline has been committed yet.
//!
//! # Why
//!
//! Every state-changing entrypoint in this contract enforces authorization
//! via an `info.sender` ownership check (the CosmWasm analogue of Soroban's
//! `require_auth`) before mutating storage — see [`crate::contract`].
//! Centralising CPU-time measurement here gives reviewers a single place to
//! extend when new entrypoints are added, and gives regression tests a way
//! to catch an accidentally-introduced algorithmic blow-up (e.g. an O(n^2)
//! loop) before it reaches production.
//!
//! Wall-clock sampling is inherently noisier than Soroban's deterministic
//! instruction budget, so baselines here default to a wide ±40% tolerance
//! and [`check_or_log_missing`] never fails a build for a baseline that
//! hasn't been generated on the current machine — see
//! `examples/cpu_baseline.rs` to (re)generate one intentionally.

#![cfg(not(target_arch = "wasm32"))]

use std::{collections::HashMap, path::Path, time::Instant};

/// Relative path (from the `creditra-credit` crate root) to the pinned snapshot.
pub const SNAPSHOT_REL_PATH: &str = "test_snapshots/cpu_baseline.json";

/// Default ± tolerance applied when a baseline omits `tolerance_pct`.
///
/// Wider than a deterministic-instruction-budget gate (e.g. Soroban's 5%)
/// because wall-clock timing varies with host hardware and system load.
pub const DEFAULT_TOLERANCE_PCT: f64 = 40.0;

/// Default repetition count for [`CpuSample::measure_avg`].
pub const DEFAULT_ITERATIONS: u32 = 50;

/// Canonical string identifiers for every instrumented (state-changing) entrypoint.
pub mod entrypoint {
    pub const INSTANTIATE: &str = "instantiate";
    pub const CREATE_CREDIT_LINE: &str = "create_credit_line";
    pub const CREATE_DRAW: &str = "create_draw";
    pub const REPAY_DRAW: &str = "repay_draw";
    pub const ADD_AUDIT_MEMO: &str = "add_audit_memo";
    pub const UPDATE_PROTOCOL_VERSION: &str = "update_protocol_version";
    pub const SET_ORACLE_QUORUM_CONFIG: &str = "set_oracle_quorum_config";
    pub const SUBMIT_ORACLE_PRICES: &str = "submit_oracle_prices";
    pub const SET_LATE_FEE_CONFIG: &str = "set_late_fee_config";

    /// Every entrypoint tracked by the CPU-regression matrix, in stable order.
    pub const ALL: &[&str] = &[
        INSTANTIATE,
        CREATE_CREDIT_LINE,
        CREATE_DRAW,
        REPAY_DRAW,
        ADD_AUDIT_MEMO,
        UPDATE_PROTOCOL_VERSION,
        SET_ORACLE_QUORUM_CONFIG,
        SUBMIT_ORACLE_PRICES,
        SET_LATE_FEE_CONFIG,
    ];
}

/// Pinned CPU-time budget for one entrypoint, serialised in `cpu_baseline.json`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CpuBaseline {
    pub entrypoint: String,
    pub cpu_nanos: u64,
    #[serde(default)]
    pub tolerance_pct: Option<f64>,
}

impl CpuBaseline {
    pub fn new(entrypoint: &'static str, cpu_nanos: u64) -> Self {
        Self {
            entrypoint: entrypoint.to_string(),
            cpu_nanos,
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

/// Observed CPU-time cost for a single entrypoint invocation, in nanoseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuSample {
    pub cpu_nanos: u64,
}

impl CpuSample {
    /// Run `f` once and return the elapsed wall-clock time.
    ///
    /// Prefer [`Self::measure_avg`] for regression baselines — a single
    /// sample is vulnerable to scheduler jitter.
    pub fn measure(f: impl FnOnce()) -> Self {
        let start = Instant::now();
        f();
        Self {
            cpu_nanos: u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX),
        }
    }

    /// Run `f` `iterations` times (clamped to at least 1) and return the
    /// mean elapsed time per call.
    ///
    /// The closure is `FnMut` so callers may vary per-repetition inputs
    /// (e.g. an incrementing id) to avoid measuring a degenerate cached path.
    pub fn measure_avg(iterations: u32, mut f: impl FnMut()) -> Self {
        let iterations = iterations.max(1);
        let start = Instant::now();
        for _ in 0..iterations {
            f();
        }
        let total_nanos = u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX);
        Self {
            cpu_nanos: total_nanos / u64::from(iterations),
        }
    }
}

/// Assert `sample` is within the baseline tolerance.
///
/// # Panics
///
/// Panics with a detailed message when the relative deviation between
/// `sample` and `baseline` exceeds [`CpuBaseline::effective_tolerance_pct`].
pub fn assert_within_tolerance(entrypoint: &str, sample: CpuSample, baseline: &CpuBaseline) {
    let tol_pct = baseline.effective_tolerance_pct();
    let pinned = baseline.cpu_nanos as f64;
    let delta_pct = if pinned == 0.0 {
        0.0
    } else {
        (sample.cpu_nanos as f64 - pinned).abs() / pinned * 100.0
    };
    assert!(
        delta_pct <= tol_pct,
        "cpu regression [{entrypoint}]:\n  observed  = {} ns\n  baseline  = {} ns\n  delta_pct = {delta_pct:.2} %  (tolerance ±{tol_pct:.1} %)",
        sample.cpu_nanos,
        baseline.cpu_nanos,
    );
}

/// Load baselines keyed by entrypoint name from `manifest_dir`/`SNAPSHOT_REL_PATH`.
///
/// Returns an empty map (rather than erroring) when no snapshot file exists
/// yet, since a freshly-cloned checkout may not have one committed.
pub fn load_baselines_from_manifest_dir(manifest_dir: &Path) -> HashMap<String, CpuBaseline> {
    let path = manifest_dir.join(SNAPSHOT_REL_PATH);
    if !path.exists() {
        return HashMap::new();
    }
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let list: Vec<CpuBaseline> =
        serde_json::from_str(&raw).unwrap_or_else(|e| panic!("bad JSON in snapshot: {e}"));
    list.into_iter()
        .map(|b| (b.entrypoint.clone(), b))
        .collect()
}

/// Write `baselines` as pretty JSON to `manifest_dir`/`SNAPSHOT_REL_PATH`.
pub fn write_baselines_to_manifest_dir(
    manifest_dir: &Path,
    baselines: &[CpuBaseline],
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

/// Compare `sample` against an optional baseline; log (never panic) when no
/// baseline exists for `entrypoint` yet.
///
/// Use this in CI-facing regression tests instead of [`assert_within_tolerance`]
/// directly, so a freshly-added entrypoint or an unseeded checkout does not
/// fail the build — it only starts gating once a baseline is committed.
pub fn check_or_log_missing(
    entrypoint: &str,
    sample: CpuSample,
    baselines: &HashMap<String, CpuBaseline>,
) {
    if let Some(baseline) = baselines.get(entrypoint) {
        assert_within_tolerance(entrypoint, sample, baseline);
    } else {
        eprintln!(
            "[cpu_regression] no baseline for '{entrypoint}'; observed cpu_nanos={}",
            sample.cpu_nanos
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entrypoint_registry_has_no_duplicates() {
        let mut seen = std::collections::HashSet::new();
        for ep in entrypoint::ALL {
            assert!(seen.insert(*ep), "duplicate entrypoint id: {ep}");
        }
    }

    #[test]
    fn entrypoint_registry_count() {
        assert_eq!(entrypoint::ALL.len(), 9);
    }

    #[test]
    fn measure_runs_closure_exactly_once() {
        let mut calls = 0u32;
        CpuSample::measure(|| {
            calls += 1;
        });
        assert_eq!(calls, 1);
    }

    #[test]
    fn measure_avg_divides_by_iteration_count() {
        let mut calls = 0u32;
        CpuSample::measure_avg(10, || {
            calls += 1;
        });
        assert_eq!(calls, 10);
    }

    #[test]
    fn measure_avg_clamps_zero_iterations_to_one() {
        let mut calls = 0u32;
        CpuSample::measure_avg(0, || {
            calls += 1;
        });
        assert_eq!(calls, 1);
    }

    #[test]
    fn baseline_default_tolerance_applies_when_unset() {
        let baseline = CpuBaseline::new(entrypoint::INSTANTIATE, 1_000);
        assert_eq!(baseline.effective_tolerance_pct(), DEFAULT_TOLERANCE_PCT);
    }

    #[test]
    fn baseline_custom_tolerance_overrides_default() {
        let baseline = CpuBaseline::new(entrypoint::INSTANTIATE, 1_000).with_tolerance_pct(10.0);
        assert_eq!(baseline.effective_tolerance_pct(), 10.0);
    }

    #[test]
    fn within_tolerance_passes() {
        let baseline = CpuBaseline::new(entrypoint::CREATE_DRAW, 1_000_000);
        let sample = CpuSample {
            cpu_nanos: 1_050_000,
        }; // +5%, default tolerance 40%
        assert_within_tolerance(entrypoint::CREATE_DRAW, sample, &baseline);
    }

    #[test]
    #[should_panic(expected = "cpu regression")]
    fn outside_tolerance_panics() {
        let baseline = CpuBaseline::new(entrypoint::CREATE_DRAW, 1_000_000).with_tolerance_pct(5.0);
        let sample = CpuSample {
            cpu_nanos: 2_000_000,
        }; // +100%, tolerance 5%
        assert_within_tolerance(entrypoint::CREATE_DRAW, sample, &baseline);
    }

    #[test]
    fn zero_baseline_does_not_divide_by_zero() {
        let baseline = CpuBaseline::new(entrypoint::CREATE_DRAW, 0);
        let sample = CpuSample { cpu_nanos: 0 };
        assert_within_tolerance(entrypoint::CREATE_DRAW, sample, &baseline);
    }

    #[test]
    fn baseline_json_roundtrip() {
        let dir =
            std::env::temp_dir().join(format!("creditra_cpu_instrument_{}", std::process::id()));
        let baselines = vec![
            CpuBaseline::new(entrypoint::INSTANTIATE, 500),
            CpuBaseline::new(entrypoint::CREATE_DRAW, 750).with_tolerance_pct(20.0),
        ];
        write_baselines_to_manifest_dir(&dir, &baselines);
        let loaded = load_baselines_from_manifest_dir(&dir);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded.get(entrypoint::INSTANTIATE).unwrap().cpu_nanos, 500);
        assert_eq!(
            loaded.get(entrypoint::CREATE_DRAW).unwrap().tolerance_pct,
            Some(20.0)
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_baselines_returns_empty_map_when_missing() {
        let dir = std::env::temp_dir().join(format!(
            "creditra_cpu_instrument_missing_{}",
            std::process::id()
        ));
        let loaded = load_baselines_from_manifest_dir(&dir);
        assert!(loaded.is_empty());
    }

    #[test]
    fn check_or_log_missing_does_not_panic_without_baseline() {
        let baselines = HashMap::new();
        check_or_log_missing(
            entrypoint::REPAY_DRAW,
            CpuSample { cpu_nanos: 123 },
            &baselines,
        );
    }

    #[test]
    fn check_or_log_missing_asserts_when_baseline_present() {
        let mut baselines = HashMap::new();
        baselines.insert(
            entrypoint::REPAY_DRAW.to_string(),
            CpuBaseline::new(entrypoint::REPAY_DRAW, 1_000_000),
        );
        check_or_log_missing(
            entrypoint::REPAY_DRAW,
            CpuSample {
                cpu_nanos: 1_100_000,
            }, // +10%, within default 40% tolerance
            &baselines,
        );
    }
}
