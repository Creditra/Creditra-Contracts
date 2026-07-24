// SPDX-License-Identifier: MIT

//! # Integration test: `mul_div` snapshot fuzzing
//!
//! Two modes, selected by whether the string `"regenerate"` appears in
//! `CARGO_TEST_ARGS` (passed via `-- regenerate` on the CLI):
//!
//! ## Verify mode (default, CI)
//!
//! ```bash
//! cargo test -p creditra-credit --test snap_safe_mul_div
//! ```
//!
//! Loads `contracts/credit/test_snapshots/safe_mul_div.json`, re-runs
//! `mul_div` for every entry, and fails immediately on any mismatch.
//!
//! ## Regenerate mode
//!
//! ```bash
//! cargo test -p creditra-credit --test snap_safe_mul_div \
//!     -- --nocapture regenerate
//! ```
//!
//! Rewrites the snapshot file with freshly computed values.

use std::fs;
use std::path::PathBuf;
use std::panic::{catch_unwind, AssertUnwindSafe};

use creditra_credit::math_utils::{mul_div, Rounding};
use serde::{Deserialize, Serialize};

// ─── Snapshot path ────────────────────────────────────────────────────────────

/// Resolves the snapshot path relative to the workspace root.
fn snapshot_path() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("test_snapshots")
        .join("safe_mul_div.json")
}

// ─── Snapshot schema ──────────────────────────────────────────────────────────

/// One row in the pinned snapshot JSON array.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct SnapshotEntry {
    /// Input a (u128 as decimal string).
    a: String,
    /// Input numerator (u128 as decimal string).
    numerator: String,
    /// Input denominator (u128 as decimal string).
    denominator: String,
    /// Rounding mode.
    rounding: String,
    /// Expected result or panic message.
    expected: String,
}

// ─── Deterministic input generation ──────────────────────────────────────────

struct Lcg {
    state: u64,
}

impl Lcg {
    const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.state
    }

    fn next_u128_varwidth(&mut self) -> u128 {
        let raw = ((self.next_u64() as u128) << 64) | (self.next_u64() as u128);
        let bits = (self.next_u64() % 129) as u32; // 0..=128
        if bits == 0 {
            0
        } else if bits >= 128 {
            raw
        } else {
            raw & ((1u128 << bits) - 1)
        }
    }
}

/// Generate the deterministic input corpus of boundary cases and random triples.
fn generate_inputs() -> Vec<(u128, u128, u128, Rounding)> {
    let mut inputs = Vec::new();

    // 1. Explicit boundary cases
    let boundary_values = [
        0,
        1,
        2,
        10,
        100,
        1000,
        u64::MAX as u128,
        u64::MAX as u128 - 1,
        u64::MAX as u128 + 1,
        u128::MAX / 2,
        u128::MAX - 1,
        u128::MAX,
    ];

    // Combine boundary values to create a rich set of deterministic edge cases
    for &a in &boundary_values {
        for &num in &boundary_values {
            for &denom in &boundary_values {
                for &rounding in &[Rounding::Floor, Rounding::Ceil] {
                    inputs.push((a, num, denom, rounding));
                }
            }
        }
    }

    // 2. Generate pseudo-random varwidth values to ensure wide distribution
    let mut lcg = Lcg::new(0x5AFE_5AFE_1234_5678_u64);
    while inputs.len() < 4096 {
        let a = lcg.next_u128_varwidth();
        let num = lcg.next_u128_varwidth();
        let denom = lcg.next_u128_varwidth();
        for &rounding in &[Rounding::Floor, Rounding::Ceil] {
            inputs.push((a, num, denom, rounding));
        }
    }

    inputs.truncate(4096);
    inputs
}

// ─── Core helper to run mul_div safely and capture output/panic ───────────────

fn run_mul_div(a: u128, numerator: u128, denominator: u128, rounding: Rounding) -> String {
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let res = catch_unwind(AssertUnwindSafe(|| {
        mul_div(a, numerator, denominator, rounding)
    }));
    std::panic::set_hook(prev_hook);

    match res {
        Ok(val) => val.to_string(),
        Err(err) => {
            if let Some(s) = err.downcast_ref::<&str>() {
                format!("PANIC: {}", s)
            } else if let Some(s) = err.downcast_ref::<String>() {
                format!("PANIC: {}", s)
            } else {
                "PANIC: unknown".to_string()
            }
        }
    }
}

// ─── Snapshot generation ──────────────────────────────────────────────────────

fn build_snapshot() -> Vec<SnapshotEntry> {
    generate_inputs()
        .into_iter()
        .map(|(a, num, denom, rounding)| {
            let expected = run_mul_div(a, num, denom, rounding);
            let rounding_str = match rounding {
                Rounding::Floor => "Floor".to_string(),
                Rounding::Ceil => "Ceil".to_string(),
            };
            SnapshotEntry {
                a: a.to_string(),
                numerator: num.to_string(),
                denominator: denom.to_string(),
                rounding: rounding_str,
                expected,
            }
        })
        .collect()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[test]
fn verify_safe_mul_div_snapshot() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "regenerate") {
        regenerate_safe_mul_div_snapshot();
        return;
    }

    let path = snapshot_path();
    let raw = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "snapshot not found at '{}': {e}\n\
             Run: cargo test -p creditra-credit --test snap_safe_mul_div \
             -- --nocapture regenerate",
            path.display()
        )
    });

    let entries: Vec<SnapshotEntry> =
        serde_json::from_str(&raw).expect("safe_mul_div.json is malformed");

    assert_eq!(
        entries.len(),
        4096,
        "snapshot must contain exactly 4096 entries, found {}",
        entries.len()
    );

    for (i, entry) in entries.iter().enumerate() {
        let a: u128 = entry.a.parse().unwrap();
        let num: u128 = entry.numerator.parse().unwrap();
        let denom: u128 = entry.denominator.parse().unwrap();
        let rounding = match entry.rounding.as_str() {
            "Floor" => Rounding::Floor,
            "Ceil" => Rounding::Ceil,
            other => panic!("entry {i}: invalid rounding mode '{other}'"),
        };

        let live = run_mul_div(a, num, denom, rounding);
        assert_eq!(
            live, entry.expected,
            "SNAPSHOT MISMATCH at entry {i} (a={a}, numerator={num}, denominator={denom}, rounding={:?}): live={}, pinned={}",
            rounding, live, entry.expected
        );
    }

    println!("✓ All {} snapshot entries verified against mul_div", entries.len());
}

#[test]
fn regenerate_safe_mul_div_snapshot() {
    let entries = build_snapshot();
    let path = snapshot_path();

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("could not create test_snapshots dir");
    }

    let json = serde_json::to_string_pretty(&entries).expect("failed to serialise snapshot");
    fs::write(&path, json)
        .unwrap_or_else(|e| panic!("failed to write snapshot to '{}': {e}", path.display()));

    println!("✓ Wrote {} entries to '{}'", entries.len(), path.display());
}
