// SPDX-License-Identifier: MIT

//! # Integration test: `prorate_interest` snapshot
//!
//! Two modes, selected by whether the string `"regenerate"` appears in
//! `CARGO_TEST_ARGS` (passed via `-- regenerate` on the CLI):
//!
//! ## Verify mode (default, CI)
//!
//! ```bash
//! cargo test -p creditra-credit --test snapshot_prorate_interest
//! ```
//!
//! Loads `contracts/credit/test_snapshots/prorate_interest.json`, re-runs
//! `prorate_interest` for every entry, and fails immediately on any mismatch.
//!
//! ## Regenerate mode
//!
//! ```bash
//! cargo test -p creditra-credit --test snapshot_prorate_interest \
//!     -- --nocapture regenerate
//! ```
//!
//! Rewrites the snapshot file with freshly computed values.  Run this after
//! any intentional change to `prorate_interest` and commit the updated JSON.
//! See `docs/contributing-tests.md` for the full regeneration workflow.

use std::fs;
use std::path::PathBuf;

use creditra_credit::math_utils::{prorate_interest, Rounding};
use serde::{Deserialize, Serialize};

// ─── Snapshot path ────────────────────────────────────────────────────────────

/// Resolves the snapshot path relative to the workspace root.
///
/// `CARGO_MANIFEST_DIR` points to `contracts/credit`; the snapshot lives
/// two levels up in `test_snapshots/` inside that same directory.
fn snapshot_path() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("test_snapshots")
        .join("prorate_interest.json")
}

// ─── Snapshot schema ──────────────────────────────────────────────────────────

/// One row in the pinned snapshot JSON array.
///
/// `principal` and `expected_floor` are decimal strings to preserve full
/// u128 precision across JSON serialisers that cap integers at 2^53.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct SnapshotEntry {
    /// Outstanding principal (u128 as decimal string).
    principal: String,
    /// Annual interest rate in basis points (0 ..= 10_000).
    rate_bps: u32,
    /// Elapsed seconds since last accrual (stored as u32; cast to u64 on use).
    seconds: u32,
    /// Floor-rounded expected output (u128 as decimal string).
    expected_floor: String,
}

// ─── Deterministic input generation ──────────────────────────────────────────

/// Minimal 64-bit LCG (Knuth / MMIX parameters).
///
/// No external crate required; reproducible on every platform.
/// State is public only for testing; callers should use [`Lcg::next_u64`].
struct Lcg {
    state: u64,
}

impl Lcg {
    /// Create a new LCG seeded with `seed`.  The same seed always produces
    /// the same sequence.
    const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Advance the state and return the next pseudo-random `u64`.
    fn next_u64(&mut self) -> u64 {
        // Knuth multiplicative LCG — period 2^64.
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.state
    }
}

/// Upper bound for `principal` such that `principal × 10_000 × u32::MAX`
/// never overflows `u128`.
///
/// Proof:
///   u128::MAX  ≈ 3.402 × 10^38
///   10_000 × u32::MAX ≈ 4.295 × 10^13
///   PRINCIPAL_MAX = floor(u128::MAX / (10_000 × u32::MAX))
///               ≈ 7.922 × 10^24
///
/// We cap at 10^24 for a round number safely below that ceiling.
const PRINCIPAL_MAX: u128 = 1_000_000_000_000_000_000_000_000_u128; // 10^24

/// Generate the deterministic 4 096-input corpus.
///
/// Input ranges:
/// - `principal` ∈ [0, PRINCIPAL_MAX]  (skewed toward interesting values)
/// - `rate_bps`  ∈ [0, 10_000]
/// - `seconds`   ∈ [0, u32::MAX]
///
/// The corpus is built to maximise coverage of:
/// - zero inputs (short-circuit paths)
/// - exact-year boundaries (`SECONDS_PER_YEAR`, half-year, quarter-year)
/// - maximum rate (10 000 bps = 100 %)
/// - very small and very large principals
/// - arbitrary interior points via the LCG
fn generate_inputs() -> Vec<(u128, u32, u32)> {
    const COUNT: usize = 4096;

    // Fixed interesting anchors added first (< COUNT so LCG fills the rest).
    let anchors: &[(u128, u32, u32)] = &[
        // Zero-input triples (short-circuit)
        (0, 500, 86_400),
        (1_000_000, 0, 86_400),
        (1_000_000, 500, 0),
        // Exact year
        (10_000, 300, 31_557_600),
        // Half year
        (10_000, 300, 15_778_800),
        // Quarter year
        (10_000, 300, 7_889_400),
        // Max rate, one year
        (10_000, 10_000, 31_557_600),
        // Very small principal
        (1, 1, 1),
        (1, 10_000, u32::MAX),
        // Very large principal (at cap)
        (PRINCIPAL_MAX, 10_000, u32::MAX),
        (PRINCIPAL_MAX, 1, 1),
        // Boundary: principal = BPS_YEAR_DENOM (exact divisibility)
        (315_576_000_000_u128, 10_000, 31_557_600),
        // One day
        (10_000, 300, 86_400),
        // One hour
        (1_000_000, 500, 3_600),
        // One second
        (1_000_000_000, 9_999, 1),
    ];

    let mut inputs: Vec<(u128, u32, u32)> = Vec::with_capacity(COUNT);
    inputs.extend_from_slice(anchors);

    let mut lcg = Lcg::new(0xDEAD_BEEF_CAFE_1234_u64);

    while inputs.len() < COUNT {
        // principal: map a u64 into [0, PRINCIPAL_MAX] via modulo reduction.
        // The slight modulo bias is acceptable for a test corpus.
        let principal = (lcg.next_u64() as u128) % (PRINCIPAL_MAX + 1);

        // rate_bps: [0, 10_000]
        let rate_bps = (lcg.next_u64() % 10_001) as u32;

        // seconds: full u32 range [0, u32::MAX]
        let seconds = (lcg.next_u64() % (u32::MAX as u64 + 1)) as u32;

        inputs.push((principal, rate_bps, seconds));
    }

    inputs
}

// ─── Snapshot generation ──────────────────────────────────────────────────────

/// Compute every entry and serialise to JSON.
fn build_snapshot() -> Vec<SnapshotEntry> {
    generate_inputs()
        .into_iter()
        .map(|(principal, rate_bps, seconds)| {
            let floor = prorate_interest(principal, rate_bps, seconds as u64, Rounding::Floor);
            SnapshotEntry {
                principal: principal.to_string(),
                rate_bps,
                seconds,
                expected_floor: floor.to_string(),
            }
        })
        .collect()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

/// Verify mode: load the snapshot from disk and re-run the math.
///
/// Fails immediately if:
/// - the snapshot file is missing (run regenerate first)
/// - the JSON is malformed
/// - the entry count is not exactly 4 096
/// - any live result diverges from the pinned value
#[test]
fn verify_prorate_interest_snapshot() {
    // Allow the test runner to skip straight to regeneration.
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "regenerate") {
        regenerate_prorate_interest_snapshot();
        return;
    }

    let path = snapshot_path();
    let raw = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "snapshot not found at '{}': {e}\n\
             Run: cargo test -p creditra-credit --test snapshot_prorate_interest \
             -- --nocapture regenerate",
            path.display()
        )
    });

    let entries: Vec<SnapshotEntry> =
        serde_json::from_str(&raw).expect("prorate_interest.json is malformed");

    assert_eq!(
        entries.len(),
        4096,
        "snapshot must contain exactly 4 096 entries, found {}",
        entries.len()
    );

    for (i, entry) in entries.iter().enumerate() {
        let principal: u128 = entry
            .principal
            .parse()
            .unwrap_or_else(|_| panic!("entry {i}: invalid principal '{}'", entry.principal));
        let expected_floor: u128 = entry
            .expected_floor
            .parse()
            .unwrap_or_else(|_| panic!("entry {i}: invalid expected_floor"));
        let time_delta = entry.seconds as u64;

        // ── Primary assertion: exact match ────────────────────────────────
        let live_floor = prorate_interest(principal, entry.rate_bps, time_delta, Rounding::Floor);
        assert_eq!(
            live_floor, expected_floor,
            "SNAPSHOT MISMATCH at entry {i} (principal={principal}, \
             rate_bps={}, seconds={}): live={live_floor}, pinned={expected_floor}\n\
             If this change is intentional, regenerate the snapshot:\n\
             cargo test -p creditra-credit --test snapshot_prorate_interest \
             -- --nocapture regenerate",
            entry.rate_bps, entry.seconds,
        );

        // ── Secondary: zero-input short-circuit ───────────────────────────
        if principal == 0 || entry.rate_bps == 0 || entry.seconds == 0 {
            assert_eq!(
                live_floor, 0,
                "entry {i}: zero input must yield 0, got {live_floor}"
            );
        }

        // ── Secondary: floor ≤ ceil ───────────────────────────────────────
        let live_ceil = prorate_interest(principal, entry.rate_bps, time_delta, Rounding::Ceil);
        assert!(
            live_floor <= live_ceil,
            "entry {i}: floor ({live_floor}) > ceil ({live_ceil})"
        );

        // ── Secondary: ceil − floor ∈ {0, 1} ─────────────────────────────
        assert!(
            live_ceil - live_floor <= 1,
            "entry {i}: ceil − floor = {} (must be 0 or 1)",
            live_ceil - live_floor
        );
    }

    println!(
        "✓ All {} snapshot entries verified against prorate_interest",
        entries.len()
    );
}

/// Regenerate mode: recompute all entries and overwrite the snapshot file.
///
/// Invoked automatically when the test binary receives `regenerate` as an
/// argument, or can be called directly from other test code.
#[test]
fn regenerate_prorate_interest_snapshot() {
    let entries = build_snapshot();
    let path = snapshot_path();

    // Ensure the directory exists (it should, but be defensive).
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("could not create test_snapshots dir");
    }

    let json = serde_json::to_string_pretty(&entries).expect("failed to serialise snapshot");
    fs::write(&path, json)
        .unwrap_or_else(|e| panic!("failed to write snapshot to '{}': {e}", path.display()));

    println!("✓ Wrote {} entries to '{}'", entries.len(), path.display());

    // Self-verify immediately after writing.
    assert_eq!(entries.len(), 4096);
    for (i, entry) in entries.iter().enumerate() {
        let principal: u128 = entry.principal.parse().unwrap();
        let expected_floor: u128 = entry.expected_floor.parse().unwrap();
        let live = prorate_interest(
            principal,
            entry.rate_bps,
            entry.seconds as u64,
            Rounding::Floor,
        );
        assert_eq!(
            live, expected_floor,
            "self-check failed at entry {i} immediately after regeneration"
        );
    }
    println!("✓ Self-check passed for all {} entries", entries.len());
}
