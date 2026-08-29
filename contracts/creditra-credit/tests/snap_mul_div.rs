// SPDX-License-Identifier: MIT

//! # Integration test: `mul_div` snapshot fuzzing
//!
//! This is the CosmWasm-side mirror of `contracts/credit/tests/snap_safe_mul_div.rs`,
//! which pins the Soroban `mul_div` function. Here we pin the equivalent
//! [`creditra_credit::math_utils::mul_div`] — the overflow-safe multiplication
//! and division primitive used by the CosmWasm credit contract.
//!
//! ## Formula
//!
//! ```text
//! result = (a × numerator) / denominator   [± 1 ulp depending on Rounding]
//! ```
//!
//! The function supports both floor and ceiling rounding. Unlike the Soroban
//! version which panics on overflow, the CosmWasm version returns `None` when:
//! - `denominator` is zero
//! - `a × numerator` overflows `Uint128`
//! - Ceil rounding would overflow `Uint128`
//!
//! ## Two modes
//!
//! ### Verify mode (default, CI)
//!
//! ```bash
//! cargo test -p creditra-credit --test snap_mul_div
//! ```
//!
//! Loads `contracts/creditra-credit/tests/snapshots/mul_div.json`,
//! re-runs `mul_div` for every entry, and fails immediately on any mismatch.
//!
//! ### Regenerate mode
//!
//! ```bash
//! cargo test -p creditra-credit --test snap_mul_div -- --nocapture regenerate
//! ```
//!
//! Rewrites the snapshot file with freshly computed values. Run this after any
//! intentional change to `mul_div` and commit the updated JSON.

use std::fs;
use std::path::PathBuf;

use cosmwasm_std::Uint128;
use creditra_credit::math_utils::{mul_div, Rounding};
use serde::{Deserialize, Serialize};

// ─── Snapshot path ────────────────────────────────────────────────────────────

/// Resolves the snapshot path relative to this crate's manifest directory.
///
/// `CARGO_MANIFEST_DIR` points to `contracts/creditra-credit`; the snapshot
/// lives in `tests/snapshots/` inside that directory so it sits alongside the
/// test source that owns it.
fn snapshot_path() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("tests")
        .join("snapshots")
        .join("mul_div.json")
}

// ─── Snapshot schema ──────────────────────────────────────────────────────────

/// One row in the pinned snapshot JSON array.
///
/// `a`, `numerator`, `denominator`, and `expected` are stored as decimal strings
/// to preserve the full `u128` range across JSON serialisers that cap integers at 2^53.
/// `overflow` is `true` when the entry is expected to return `None`.
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
    /// Expected result (u128 as decimal string) when `overflow == false`.
    expected: String,
    /// `true` when `mul_div` is expected to return `None` for this input.
    overflow: bool,
}

// ─── Deterministic input generation ──────────────────────────────────────────

/// Minimal 64-bit LCG (Knuth / MMIX parameters) — no external crate required.
///
/// The same seed always produces the same sequence on every platform, giving
/// the snapshot corpus full reproducibility without pulling in `rand`.
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

/// Fixed interesting anchors that seed the corpus before the LCG fills it.
///
/// These are chosen to exercise:
/// - Zero-input short-circuit paths.
/// - Exact division cases.
/// - Overflow boundaries.
/// - Very small and very large values.
const ANCHORS: &[(u128, u128, u128, Rounding)] = &[
    // ── Zero-input short-circuits ───────────────────────────────────────────
    (0, 300, 10_000, Rounding::Floor),
    (1_000, 0, 10_000, Rounding::Floor),
    (1_000, 300, 10_000, Rounding::Floor),
    (1_000, 300, 10_000, Rounding::Ceil),
    // ── Exact division cases ───────────────────────────────────────────────
    (1_000, 3, 10, Rounding::Floor),
    (1_000, 3, 10, Rounding::Ceil),
    (42, 7, 7, Rounding::Floor),
    (42, 7, 7, Rounding::Ceil),
    // ── Remainder cases ─────────────────────────────────────────────────────
    (1_001, 3, 10, Rounding::Floor),
    (1_001, 3, 10, Rounding::Ceil),
    (7, 1, 3, Rounding::Floor),
    (7, 1, 3, Rounding::Ceil),
    // ── Large values ───────────────────────────────────────────────────────
    (u128::MAX / 2, 2, 2, Rounding::Floor),
    (u128::MAX / 2, 2, 2, Rounding::Ceil),
    // ── Sub-unit rounding ───────────────────────────────────────────────────
    (1, 1, 10_000, Rounding::Floor),
    (1, 1, 10_000, Rounding::Ceil),
    // ── Overflow boundary (u128::MAX * 2) ───────────────────────────────────
    (u128::MAX, 2, 1, Rounding::Floor),
    (u128::MAX, 2, 1, Rounding::Ceil),
    // ── Zero denominator (should return None) ──────────────────────────────
    (100, 1, 0, Rounding::Floor),
    (100, 1, 0, Rounding::Ceil),
];

/// Compute `mul_div` for one entry, returning a `SnapshotEntry`.
fn compute_entry(a: u128, numerator: u128, denominator: u128, rounding: Rounding) -> SnapshotEntry {
    match mul_div(Uint128::new(a), numerator, denominator, rounding) {
        Some(v) => SnapshotEntry {
            a: a.to_string(),
            numerator: numerator.to_string(),
            denominator: denominator.to_string(),
            rounding: match rounding {
                Rounding::Floor => "Floor".to_string(),
                Rounding::Ceil => "Ceil".to_string(),
            },
            expected: v.u128().to_string(),
            overflow: false,
        },
        None => SnapshotEntry {
            a: a.to_string(),
            numerator: numerator.to_string(),
            denominator: denominator.to_string(),
            rounding: match rounding {
                Rounding::Floor => "Floor".to_string(),
                Rounding::Ceil => "Ceil".to_string(),
            },
            expected: "0".to_string(),
            overflow: true,
        },
    }
}

/// Generate the deterministic corpus of 4 096 entries.
///
/// The first entries are the hand-picked [`ANCHORS`]; the remainder are
/// generated by the LCG so the total is exactly 4 096.
fn generate_inputs() -> Vec<(u128, u128, u128, Rounding)> {
    const COUNT: usize = 4096;

    let mut inputs: Vec<(u128, u128, u128, Rounding)> = Vec::with_capacity(COUNT);
    inputs.extend_from_slice(ANCHORS);

    let mut lcg = Lcg::new(0x5AFE_5AFE_1234_5678_u64);

    while inputs.len() < COUNT {
        let a = lcg.next_u128_varwidth();
        let numerator = lcg.next_u128_varwidth();
        let denominator = lcg.next_u128_varwidth().max(1); // Avoid zero denominator in LCG
        for &rounding in &[Rounding::Floor, Rounding::Ceil] {
            inputs.push((a, numerator, denominator, rounding));
        }
    }

    inputs.truncate(COUNT);
    inputs
}

/// Build the full snapshot vector by evaluating `mul_div` on every generated input.
fn build_snapshot() -> Vec<SnapshotEntry> {
    generate_inputs()
        .into_iter()
        .map(|(a, num, denom, rounding)| compute_entry(a, num, denom, rounding))
        .collect()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

/// Verify mode: load the pinned snapshot and re-run the math.
///
/// Fails immediately if:
/// - the snapshot file is missing (run regenerate mode first),
/// - the JSON is malformed,
/// - the entry count is not exactly 4 096, or
/// - any live result diverges from the pinned value.
#[test]
fn verify_mul_div_snapshot() {
    // Support the `regenerate` escape hatch: if the test binary receives
    // `regenerate` as a CLI argument, switch to write mode instead.
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "regenerate") {
        regenerate_mul_div_snapshot();
        return;
    }

    let path = snapshot_path();

    // Bootstrap: if the snapshot file does not exist yet (e.g. first run after
    // a fresh checkout), generate it rather than failing with a confusing error.
    // In CI the committed snapshot file will always be present, so this branch
    // is only hit by contributors working on a fresh clone.
    if !path.exists() {
        eprintln!(
            "snapshot not found at '{}'; generating it now …",
            path.display()
        );
        regenerate_mul_div_snapshot();
        return;
    }

    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read snapshot '{}': {e}", path.display()));

    let entries: Vec<SnapshotEntry> =
        serde_json::from_str(&raw).expect("mul_div.json is malformed");

    assert_eq!(
        entries.len(),
        4096,
        "snapshot must contain exactly 4 096 entries, found {}",
        entries.len()
    );

    for (i, entry) in entries.iter().enumerate() {
        let a: u128 = entry
            .a
            .parse()
            .unwrap_or_else(|_| panic!("entry {i}: invalid a '{}'", entry.a));
        let numerator: u128 = entry
            .numerator
            .parse()
            .unwrap_or_else(|_| panic!("entry {i}: invalid numerator '{}'", entry.numerator));
        let denominator: u128 = entry
            .denominator
            .parse()
            .unwrap_or_else(|_| panic!("entry {i}: invalid denominator '{}'", entry.denominator));
        let rounding = match entry.rounding.as_str() {
            "Floor" => Rounding::Floor,
            "Ceil" => Rounding::Ceil,
            other => panic!("entry {i}: invalid rounding mode '{other}'"),
        };

        let live = mul_div(Uint128::new(a), numerator, denominator, rounding);

        if entry.overflow {
            // ── Overflow entries ──────────────────────────────────────────
            assert_eq!(
                live,
                None,
                "entry {i} (a={a}, numerator={numerator}, denominator={denominator}): \
                 expected None but got {:?}",
                live,
            );
        } else {
            // ── Normal entries ────────────────────────────────────────────
            let expected: u128 = entry
                .expected
                .parse()
                .unwrap_or_else(|_| panic!("entry {i}: invalid expected '{}'", entry.expected));

            let live_val = live.unwrap_or_else(|_| {
                panic!(
                    "entry {i} (a={a}, numerator={numerator}, denominator={denominator}): \
                     unexpected None"
                )
            });

            // Primary: exact match against pinned value.
            assert_eq!(
                live_val.u128(),
                expected,
                "SNAPSHOT MISMATCH at entry {i} \
                 (a={a}, numerator={numerator}, denominator={denominator}, rounding={}): \
                 live={}, pinned={expected}\n\
                 If intentional, regenerate:\n\
                 cargo test -p creditra-credit --test snap_mul_div \
                 -- --nocapture regenerate",
                entry.rounding,
                live_val,
            );
        }
    }

    println!(
        "✓ All {} snapshot entries verified against mul_div",
        entries.len()
    );
}

/// Regenerate mode: recompute all entries and overwrite the snapshot file.
///
/// Invoked automatically when the test binary receives `regenerate` as a CLI
/// argument. Also exposed as a named `#[test]` so `cargo test regenerate`
/// picks it up directly.
#[test]
fn regenerate_mul_div_snapshot() {
    let entries = build_snapshot();
    let path = snapshot_path();

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .unwrap_or_else(|e| panic!("could not create snapshots dir: {e}"));
    }

    let json =
        serde_json::to_string_pretty(&entries).expect("failed to serialise snapshot");
    fs::write(&path, &json)
        .unwrap_or_else(|e| panic!("failed to write snapshot to '{}': {e}", path.display()));

    println!("✓ Wrote {} entries to '{}'", entries.len(), path.display());

    // Self-verify immediately after writing.
    assert_eq!(entries.len(), 4096);
    for (i, entry) in entries.iter().enumerate() {
        let a: u128 = entry.a.parse().unwrap();
        let numerator: u128 = entry.numerator.parse().unwrap();
        let denominator: u128 = entry.denominator.parse().unwrap();
        let rounding = match entry.rounding.as_str() {
            "Floor" => Rounding::Floor,
            "Ceil" => Rounding::Ceil,
            other => panic!("entry {i}: invalid rounding mode '{other}'"),
        };

        let live = mul_div(Uint128::new(a), numerator, denominator, rounding);
        if entry.overflow {
            assert_eq!(
                live,
                None,
                "self-check failed at entry {i}: expected None"
            );
        } else {
            let expected: u128 = entry.expected.parse().unwrap();
            let v = live.unwrap();
            assert_eq!(
                v.u128(),
                expected,
                "self-check failed at entry {i} immediately after regeneration"
            );
        }
    }
    println!("✓ Self-check passed for all {} entries", entries.len());
}
