// SPDX-License-Identifier: MIT

//! # Fuzz target: `prorate_interest` snapshot verifier
//!
//! This target loads the pinned deterministic snapshot
//! `contracts/credit/test_snapshots/prorate_interest.json` and re-executes
//! [`creditra_credit::math_utils::prorate_interest`] for every recorded
//! `(principal, rate_bps, seconds)` triple, asserting that the live
//! implementation produces the same floor-rounded output that was captured
//! when the snapshot was generated.
//!
//! ## Purpose
//!
//! `prorate_interest` is the single rounding-floor primitive hit by every
//! interest accrual in the protocol. Any change to its rounding direction,
//! overflow handling, or constant values (`BPS_YEAR_DENOM`, `SECONDS_PER_YEAR`)
//! will cause a divergence between the live result and the pinned snapshot,
//! turning this target red immediately at PR time.
//!
//! ## Running modes
//!
//! ### Snapshot verification (CI / normal fuzzing)
//!
//! ```bash
//! cargo fuzz run prorate_interest_snapshot -- -max_total_time=60
//! ```
//!
//! libFuzzer will call `fuzz_target!` with arbitrary byte slices; the target
//! ignores the mutated bytes entirely and instead loads + verifies the pinned
//! snapshot on **every invocation**. This means any corpus entry triggers a
//! full 4 096-case regression sweep.
//!
//! ### Snapshot regeneration
//!
//! See `docs/contributing-tests.md` — "Regenerating the prorate_interest snapshot".
//! In short:
//!
//! ```bash
//! cargo test -p creditra-credit --test snapshot_prorate_interest -- --nocapture regenerate
//! ```
//!
//! ## Properties verified per entry
//!
//! 1. **Exact match**: `live_result == snapshot_output` — the primary regression gate.
//! 2. **Floor ≤ ceil**: `prorate_interest(..., Floor) ≤ prorate_interest(..., Ceil)`.
//! 3. **Ceil − floor ∈ {0, 1}**: rounding never moves by more than 1 ulp.
//! 4. **Zero-input short-circuits**: any zero input yields zero output.

#![no_main]

use libfuzzer_sys::fuzz_target;
use serde::Deserialize;

use creditra_credit::math_utils::{prorate_interest, Rounding};

/// One row in the snapshot JSON array.
#[derive(Debug, Deserialize)]
struct SnapshotEntry {
    /// Outstanding principal (u128, serialised as decimal string to avoid
    /// JSON number precision loss for values > 2^53).
    principal: String,
    /// Annual interest rate in basis points (0 ..= 10_000).
    rate_bps: u32,
    /// Elapsed seconds since last accrual (0 ..= u32::MAX, stored as u32 to
    /// keep the snapshot compact; cast to u64 on use).
    seconds: u32,
    /// Expected floor-rounded interest output (u128, decimal string).
    expected_floor: String,
}

/// Path to the pinned snapshot, relative to the workspace root.
///
/// `cargo fuzz run` is invoked from the workspace root by convention.
const SNAPSHOT_PATH: &str =
    "contracts/credit/test_snapshots/prorate_interest.json";

/// Load and verify all 4 096 snapshot entries.
///
/// This function is called on every libFuzzer iteration regardless of the
/// fuzz input (which is intentionally ignored). The snapshot is re-read from
/// disk each call so that an on-disk edit is caught immediately without
/// recompiling.
fn verify_snapshot() {
    let raw = std::fs::read_to_string(SNAPSHOT_PATH).unwrap_or_else(|e| {
        panic!(
            "snapshot not found at '{SNAPSHOT_PATH}': {e}\n\
             Run `cargo test -p creditra-credit --test snapshot_prorate_interest \
             -- regenerate` to generate it."
        )
    });

    let entries: Vec<SnapshotEntry> =
        serde_json::from_str(&raw).expect("snapshot JSON is malformed");

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
            .unwrap_or_else(|_| panic!("entry {i}: invalid expected_floor '{}'", entry.expected_floor));
        let time_delta = entry.seconds as u64;

        // ── Property 1: exact match against pinned output ─────────────────
        let live_floor = prorate_interest(principal, entry.rate_bps, time_delta, Rounding::Floor);
        assert_eq!(
            live_floor,
            expected_floor,
            "SNAPSHOT MISMATCH at entry {i}: \
             principal={principal}, rate_bps={}, seconds={} \
             → live={live_floor}, pinned={expected_floor}\n\
             A rounding-direction or constant change has broken the snapshot. \
             If intentional, regenerate with:\n\
             cargo test -p creditra-credit --test snapshot_prorate_interest \
             -- regenerate",
            entry.rate_bps,
            entry.seconds,
        );

        // ── Property 2: zero inputs always yield zero ─────────────────────
        if principal == 0 || entry.rate_bps == 0 || entry.seconds == 0 {
            assert_eq!(
                live_floor, 0,
                "entry {i}: zero input must yield zero, got {live_floor}"
            );
        }

        // ── Property 3: floor ≤ ceil ──────────────────────────────────────
        let live_ceil = prorate_interest(principal, entry.rate_bps, time_delta, Rounding::Ceil);
        assert!(
            live_floor <= live_ceil,
            "entry {i}: floor ({live_floor}) > ceil ({live_ceil}) — \
             principal={principal}, rate_bps={}, seconds={}",
            entry.rate_bps,
            entry.seconds,
        );

        // ── Property 4: ceil − floor ∈ {0, 1} ────────────────────────────
        assert!(
            live_ceil - live_floor <= 1,
            "entry {i}: ceil − floor = {} (must be 0 or 1) — \
             principal={principal}, rate_bps={}, seconds={}",
            live_ceil - live_floor,
            entry.rate_bps,
            entry.seconds,
        );
    }
}

// libFuzzer entry-point.  The fuzz input (`data`) is intentionally unused:
// this target is a *snapshot verifier*, not a generative fuzzer.  Every
// corpus byte-string that libFuzzer feeds us triggers the full 4 096-case
// regression sweep, which is exactly what we want.
fuzz_target!(|_data: &[u8]| {
    verify_snapshot();
});