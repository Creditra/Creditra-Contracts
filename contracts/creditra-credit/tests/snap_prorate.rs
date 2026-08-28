// SPDX-License-Identifier: MIT

//! # Snapshot-fuzz tests for `accrued_interest`
//!
//! This is the CosmWasm-side mirror of
//! `contracts/credit/tests/snapshot_prorate_interest.rs`, which pins the
//! Soroban `prorate_interest` function. Here we pin the equivalent
//! [`creditra_credit::accrual::accrued_interest`] — the 365-day simple-interest
//! accrual primitive used by the CosmWasm credit contract.
//!
//! ## Formula
//!
//! ```text
//! interest = principal · rate_bps · elapsed_seconds
//!            / (10_000 · SECONDS_PER_YEAR)     [floored]
//! ```
//!
//! where `SECONDS_PER_YEAR = 31_536_000` (365 × 86 400, **not** the Julian
//! 31 557 600 used by the Soroban twin).  The function always rounds down and
//! returns `Err(ContractError::Overflow)` rather than panicking when the
//! intermediate product overflows `Uint128`.
//!
//! ## Two modes
//!
//! ### Verify mode (default, CI)
//!
//! ```bash
//! cargo test -p creditra-credit --test snap_prorate
//! ```
//!
//! Loads `contracts/creditra-credit/tests/snapshots/accrued_interest.json`,
//! re-runs `accrued_interest` for every entry, and fails immediately on any
//! mismatch.
//!
//! ### Regenerate mode
//!
//! ```bash
//! cargo test -p creditra-credit --test snap_prorate -- --nocapture regenerate
//! ```
//!
//! Rewrites the snapshot file with freshly computed values. Run this after any
//! intentional change to `accrued_interest` and commit the updated JSON.
//! See `docs/contributing-tests.md` for the full regeneration workflow.
//!
//! ## Key differences from the Soroban twin
//!
//! | Property | Soroban (`prorate_interest`) | CosmWasm (`accrued_interest`) |
//! |---|---|---|
//! | Year length | 31 557 600 s (Julian) | 31 536 000 s (365-day) |
//! | Rounding | Caller-controlled `Floor`/`Ceil` | Always floor |
//! | Overflow | Panics | Returns `Err(ContractError::Overflow)` |
//! | Return type | `u128` | `Result<Uint128, ContractError>` |

use std::fs;
use std::path::PathBuf;

use cosmwasm_std::Uint128;
use creditra_credit::accrual::{accrued_interest, SECONDS_PER_YEAR};
use creditra_credit::error::ContractError;
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
        .join("accrued_interest.json")
}

// ─── Snapshot schema ──────────────────────────────────────────────────────────

/// One row in the pinned snapshot JSON array.
///
/// `principal` and `expected` are stored as decimal strings to preserve the
/// full `u128` range across JSON serialisers that cap integers at 2^53.
/// `overflow` is `true` when the entry is expected to return
/// `Err(ContractError::Overflow)`.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct SnapshotEntry {
    /// Outstanding principal (u128 as decimal string).
    principal: String,
    /// Annual interest rate in basis points (0 ..= 10 000).
    rate_bps: u32,
    /// Elapsed seconds since last accrual (stored as u32; cast to u64 on use).
    seconds: u32,
    /// Floor-rounded expected output when `overflow == false` (u128 as decimal
    /// string); `"0"` when `overflow == true` (field is ignored in that case).
    expected: String,
    /// `true` when `accrued_interest` is expected to return
    /// `Err(ContractError::Overflow)` for this input.
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
}

/// Maximum principal value used in the LCG corpus.
///
/// The bound is chosen so that `principal × 10_000 × u32::MAX` fits inside
/// `Uint128` (`u128::MAX ≈ 3.4 × 10^38`; the product at this cap is
/// `10^24 × 10^4 × ~4.3 × 10^9 ≈ 4.3 × 10^37 < u128::MAX`). Values above
/// this cap trigger `Overflow`; those entries are marked `overflow: true` in
/// the snapshot.
const PRINCIPAL_MAX: u128 = 1_000_000_000_000_000_000_000_000_u128; // 10^24

/// Fixed interesting anchors that seed the corpus before the LCG fills it.
///
/// These are chosen to exercise:
/// - Zero-input short-circuit paths.
/// - Exact year / half-year / quarter-year boundaries.
/// - Minimum and maximum bps.
/// - Very small and very large principals.
/// - Exact-division cases (`principal = BPS_YEAR_DENOM` equivalent).
const ANCHORS: &[(u128, u32, u32)] = &[
    // ── Zero-input short-circuits ───────────────────────────────────────────
    (0, 500, 86_400),
    (1_000_000, 0, 86_400),
    (1_000_000, 500, 0),
    // ── Exact year boundaries ───────────────────────────────────────────────
    // 10 000 tokens @ 300 bps for 1 year → 300
    (10_000, 300, SECONDS_PER_YEAR as u32),
    // 10 000 tokens @ 300 bps for half year → 150
    (10_000, 300, SECONDS_PER_YEAR as u32 / 2),
    // 10 000 tokens @ 300 bps for quarter year → 75
    (10_000, 300, SECONDS_PER_YEAR as u32 / 4),
    // ── Maximum rate ────────────────────────────────────────────────────────
    (10_000, 10_000, SECONDS_PER_YEAR as u32),
    // ── Minimum non-zero inputs ─────────────────────────────────────────────
    (1, 1, 1),
    // Very small principal, maximum rate, maximum time
    (1, 10_000, u32::MAX),
    // ── Large principals at the corpus cap ──────────────────────────────────
    (PRINCIPAL_MAX, 10_000, u32::MAX),
    (PRINCIPAL_MAX, 1, 1),
    // ── Exact-divisibility boundary ─────────────────────────────────────────
    // principal = BPS_DENOMINATOR × SECONDS_PER_YEAR = 315_360_000_000
    // → exact result with no remainder
    (315_360_000_000_u128, 10_000, SECONDS_PER_YEAR as u32),
    // ── Common human-scale time deltas ──────────────────────────────────────
    (10_000, 300, 86_400),          // 1 day
    (1_000_000, 500, 3_600),        // 1 hour
    (1_000_000_000, 9_999, 1),      // 1 second, near-max rate
    // ── Large principal, moderate rate, 1 year ──────────────────────────────
    (1_000_000_000, 500, SECONDS_PER_YEAR as u32),
    // ── Floor-to-zero cases (result < 1 token) ──────────────────────────────
    (1, 1, SECONDS_PER_YEAR as u32),  // 1 · 1 / 315_360_000_000 → 0
    (100, 50, 3_600),
    // ── Multi-year time deltas (u32-representable) ───────────────────────────
    (10_000, 300, SECONDS_PER_YEAR as u32 * 2),
    (10_000, 300, SECONDS_PER_YEAR as u32 * 10),
    // ── Protocol-representative scale (1 M tokens, 5%, 30 days) ─────────────
    (1_000_000, 500, 30 * 86_400),
];

/// Compute `accrued_interest` for one entry, returning an `SnapshotEntry`.
fn compute_entry(principal: u128, rate_bps: u32, seconds: u32) -> SnapshotEntry {
    match accrued_interest(Uint128::new(principal), rate_bps, seconds as u64) {
        Ok(v) => SnapshotEntry {
            principal: principal.to_string(),
            rate_bps,
            seconds,
            expected: v.u128().to_string(),
            overflow: false,
        },
        Err(ContractError::Overflow) => SnapshotEntry {
            principal: principal.to_string(),
            rate_bps,
            seconds,
            expected: "0".to_string(),
            overflow: true,
        },
        Err(e) => panic!("unexpected error for principal={principal} rate={rate_bps} sec={seconds}: {e}"),
    }
}

/// Generate the deterministic corpus of 4 096 entries.
///
/// The first entries are the hand-picked [`ANCHORS`]; the remainder are
/// generated by the LCG so the total is exactly 4 096.
fn generate_inputs() -> Vec<(u128, u32, u32)> {
    const COUNT: usize = 4096;

    let mut inputs: Vec<(u128, u32, u32)> = Vec::with_capacity(COUNT);
    inputs.extend_from_slice(ANCHORS);

    let mut lcg = Lcg::new(0xFEED_FACE_DEAD_BEEF_u64);

    while inputs.len() < COUNT {
        let principal = (lcg.next_u64() as u128) % (PRINCIPAL_MAX + 1);
        let rate_bps = (lcg.next_u64() % 10_001) as u32;
        let seconds = (lcg.next_u64() % (u32::MAX as u64 + 1)) as u32;
        inputs.push((principal, rate_bps, seconds));
    }

    inputs
}

/// Build the full snapshot vector by evaluating `accrued_interest` on every
/// generated input.
fn build_snapshot() -> Vec<SnapshotEntry> {
    generate_inputs()
        .into_iter()
        .map(|(p, r, s)| compute_entry(p, r, s))
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
///
/// # Secondary assertions (checked for every non-overflow entry)
///
/// 1. **Zero-input short-circuit** — any entry with `principal = 0`,
///    `rate_bps = 0`, or `seconds = 0` must yield exactly `0`.
/// 2. **Non-negativity** — the result is always `≥ 0` (guaranteed by
///    `Uint128`, asserted explicitly for documentation).
/// 3. **Monotone upper bound** — `interest ≤ principal` for any elapsed
///    time ≤ one year at any rate ≤ 10 000 bps.
#[test]
fn verify_accrued_interest_snapshot() {
    // Support the `regenerate` escape hatch: if the test binary receives
    // `regenerate` as a CLI argument, switch to write mode instead.
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "regenerate") {
        regenerate_accrued_interest_snapshot();
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
        regenerate_accrued_interest_snapshot();
        return;
    }

    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read snapshot '{}': {e}", path.display()));

    let entries: Vec<SnapshotEntry> =
        serde_json::from_str(&raw).expect("accrued_interest.json is malformed");

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
        let elapsed = entry.seconds as u64;

        let live = accrued_interest(Uint128::new(principal), entry.rate_bps, elapsed);

        if entry.overflow {
            // ── Overflow entries ──────────────────────────────────────────
            assert_eq!(
                live,
                Err(ContractError::Overflow),
                "entry {i} (principal={principal}, rate={}, sec={}): \
                 expected Overflow but got {:?}",
                entry.rate_bps,
                entry.seconds,
                live,
            );
        } else {
            // ── Normal entries ────────────────────────────────────────────
            let expected: u128 = entry
                .expected
                .parse()
                .unwrap_or_else(|_| panic!("entry {i}: invalid expected '{}'", entry.expected));

            let live_val = live.unwrap_or_else(|e| {
                panic!(
                    "entry {i} (principal={principal}, rate={}, sec={}): \
                     unexpected error {e}",
                    entry.rate_bps, entry.seconds,
                )
            });

            // Primary: exact match against pinned value.
            assert_eq!(
                live_val.u128(),
                expected,
                "SNAPSHOT MISMATCH at entry {i} \
                 (principal={principal}, rate_bps={}, seconds={}): \
                 live={}, pinned={expected}\n\
                 If intentional, regenerate:\n\
                 cargo test -p creditra-credit --test snap_prorate \
                 -- --nocapture regenerate",
                entry.rate_bps,
                entry.seconds,
                live_val,
            );

            // Secondary 1: zero-input short-circuit.
            if principal == 0 || entry.rate_bps == 0 || entry.seconds == 0 {
                assert_eq!(
                    live_val,
                    Uint128::zero(),
                    "entry {i}: zero input must yield 0, got {live_val}"
                );
            }

            // Secondary 2: non-negativity (Uint128 ensures this; explicit for
            // documentation purposes).
            assert!(live_val.u128() < u128::MAX);

            // Secondary 3: interest ≤ principal when elapsed ≤ 1 year and
            // rate ≤ 10 000 bps (100 % per year cannot double the principal
            // in one year or less).
            if entry.seconds <= SECONDS_PER_YEAR as u32 && entry.rate_bps <= 10_000 {
                assert!(
                    live_val.u128() <= principal,
                    "entry {i}: interest ({live_val}) exceeds principal ({principal}) \
                     for elapsed={} sec, rate={} bps",
                    entry.seconds,
                    entry.rate_bps,
                );
            }
        }
    }

    println!(
        "✓ All {} snapshot entries verified against accrued_interest",
        entries.len()
    );
}

/// Regenerate mode: recompute all entries and overwrite the snapshot file.
///
/// Invoked automatically when the test binary receives `regenerate` as a CLI
/// argument.  Also exposed as a named `#[test]` so `cargo test regenerate`
/// picks it up directly.
#[test]
fn regenerate_accrued_interest_snapshot() {
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
        let principal: u128 = entry.principal.parse().unwrap();
        let live = accrued_interest(Uint128::new(principal), entry.rate_bps, entry.seconds as u64);
        if entry.overflow {
            assert_eq!(
                live,
                Err(ContractError::Overflow),
                "self-check failed at entry {i}: expected Overflow"
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

// ─── Property / fuzz tests ────────────────────────────────────────────────────

/// Deterministic regression suite — hand-picked inputs with pre-computed
/// expected values that don't require the snapshot file.
///
/// Each case documents *why* it is interesting and what the expected result is.
#[cfg(test)]
mod deterministic {
    use super::*;

    // ── Zero-input short-circuits ─────────────────────────────────────────

    #[test]
    fn zero_principal_returns_zero() {
        assert_eq!(
            accrued_interest(Uint128::zero(), 500, 86_400).unwrap(),
            Uint128::zero()
        );
    }

    #[test]
    fn zero_rate_returns_zero() {
        assert_eq!(
            accrued_interest(Uint128::new(1_000_000), 0, 86_400).unwrap(),
            Uint128::zero()
        );
    }

    #[test]
    fn zero_elapsed_returns_zero() {
        assert_eq!(
            accrued_interest(Uint128::new(1_000_000), 500, 0).unwrap(),
            Uint128::zero()
        );
    }

    // ── Exact-year results ────────────────────────────────────────────────

    #[test]
    fn three_percent_for_one_full_year() {
        // 10 000 · 300 · 31_536_000 / (10_000 · 31_536_000) = 300
        assert_eq!(
            accrued_interest(Uint128::new(10_000), 300, SECONDS_PER_YEAR).unwrap(),
            Uint128::new(300)
        );
    }

    #[test]
    fn five_percent_for_one_full_year() {
        // 1_000_000 · 500 / 10_000 = 50_000
        assert_eq!(
            accrued_interest(Uint128::new(1_000_000), 500, SECONDS_PER_YEAR).unwrap(),
            Uint128::new(50_000)
        );
    }

    #[test]
    fn one_hundred_percent_for_one_full_year() {
        // 10 000 · 10 000 · SECONDS_PER_YEAR / (10_000 · SECONDS_PER_YEAR) = 10 000
        assert_eq!(
            accrued_interest(Uint128::new(10_000), 10_000, SECONDS_PER_YEAR).unwrap(),
            Uint128::new(10_000)
        );
    }

    // ── Sub-year time deltas ──────────────────────────────────────────────

    #[test]
    fn half_year_is_half_annual() {
        let full = accrued_interest(Uint128::new(1_000_000), 400, SECONDS_PER_YEAR).unwrap();
        let half = accrued_interest(Uint128::new(1_000_000), 400, SECONDS_PER_YEAR / 2).unwrap();
        assert_eq!(full, Uint128::new(40_000));
        assert_eq!(half, Uint128::new(20_000));
    }

    #[test]
    fn one_day_at_five_percent() {
        // 1_000_000 · 500 · 86_400 / 315_360_000_000
        // = 43_200_000_000_000 / 315_360_000_000 = 136.98… → 136
        assert_eq!(
            accrued_interest(Uint128::new(1_000_000), 500, 86_400).unwrap(),
            Uint128::new(136)
        );
    }

    #[test]
    fn one_hour_at_five_percent() {
        // 1_000_000 · 500 · 3_600 / 315_360_000_000 = 1_800_000_000_000 / 315_360_000_000 = 5.7… → 5
        assert_eq!(
            accrued_interest(Uint128::new(1_000_000), 500, 3_600).unwrap(),
            Uint128::new(5)
        );
    }

    // ── Floor behaviour ───────────────────────────────────────────────────

    #[test]
    fn sub_unit_principal_floors_to_zero() {
        // 1 · 1 · 1 / 315_360_000_000 = 0
        assert_eq!(
            accrued_interest(Uint128::new(1), 1, 1).unwrap(),
            Uint128::zero()
        );
    }

    #[test]
    fn small_principal_floors_to_zero_for_one_year() {
        // 1 · 1 · 31_536_000 / 315_360_000_000 = 0 (denominator > numerator)
        assert_eq!(
            accrued_interest(Uint128::new(1), 1, SECONDS_PER_YEAR).unwrap(),
            Uint128::zero()
        );
    }

    #[test]
    fn threshold_principal_yields_one_token() {
        // 10_000 · 1 · 31_536_000 / 315_360_000_000 = 1 (exact)
        assert_eq!(
            accrued_interest(Uint128::new(10_000), 1, SECONDS_PER_YEAR).unwrap(),
            Uint128::new(1)
        );
    }

    // ── Exact-division (no remainder) ─────────────────────────────────────

    #[test]
    fn exact_division_no_floor_error() {
        // principal = BPS_DENOM × SPY = 315_360_000_000; rate = 10_000; t = SPY
        // → numerator = 315_360_000_000 · 10_000 · 31_536_000
        //             = 99_457_690_000_000_000_000_000 (exact multiple of denom)
        let principal = Uint128::new(315_360_000_000_u128);
        let result = accrued_interest(principal, 10_000, SECONDS_PER_YEAR).unwrap();
        assert_eq!(result, Uint128::new(315_360_000_000_u128));
    }

    // ── Overflow path ─────────────────────────────────────────────────────

    #[test]
    fn overflow_returns_err_not_panic() {
        // Uint128::MAX as principal with any nonzero rate/time overflows the
        // intermediate product → must return Err(Overflow), never panic.
        assert_eq!(
            accrued_interest(Uint128::MAX, 10_000, SECONDS_PER_YEAR),
            Err(ContractError::Overflow)
        );
    }

    #[test]
    fn large_but_representable_principal_ok() {
        // Choose the largest principal that still fits inside Uint128 after
        // multiplication: floor(Uint128::MAX / (10_000 × SECONDS_PER_YEAR)).
        let denom = Uint128::new(10_000u128 * SECONDS_PER_YEAR as u128);
        let max_ok = Uint128::MAX.checked_div(denom).unwrap();
        let result = accrued_interest(max_ok, 10_000, SECONDS_PER_YEAR);
        assert!(result.is_ok(), "expected Ok, got {result:?}");
    }

    // ── Monotonicity ──────────────────────────────────────────────────────

    #[test]
    fn monotone_in_time_over_daily_series() {
        let principal = Uint128::new(1_000_000);
        let rate = 500u32;
        let mut prev = Uint128::zero();
        for days in 0u64..=365 {
            let cur = accrued_interest(principal, rate, days * 86_400).unwrap();
            assert!(
                cur >= prev,
                "day {days}: accrued {cur} < previous {prev}"
            );
            prev = cur;
        }
        // Sanity-check the final value matches the direct 1-year call.
        let year_direct = accrued_interest(principal, rate, SECONDS_PER_YEAR).unwrap();
        assert_eq!(prev, year_direct);
    }

    #[test]
    fn interest_grows_strictly_once_floor_clears() {
        let principal = Uint128::new(1_000_000);
        let rate = 500u32;
        // 1 day floors to 136; 2 days floors to 273 > 136.
        let one_day = accrued_interest(principal, rate, 86_400).unwrap();
        let two_days = accrued_interest(principal, rate, 2 * 86_400).unwrap();
        assert!(one_day > Uint128::zero());
        assert!(two_days > one_day);
    }

    // ── SECONDS_PER_YEAR constant ─────────────────────────────────────────

    #[test]
    fn seconds_per_year_is_365_day_year() {
        // The CosmWasm crate uses a 365-day year (not the Julian 365.25-day
        // year used by the Soroban twin). This guards against accidental
        // constant drift.
        assert_eq!(SECONDS_PER_YEAR, 365 * 86_400);
    }
}

// ─── proptest suite ───────────────────────────────────────────────────────────

#[cfg(test)]
mod fuzz {
    use super::*;
    use proptest::prelude::*;
    use proptest::test_runner::Config as ProptestConfig;

    /// Principal range sized to stay well below the overflow boundary so the
    /// majority of proptest runs produce `Ok` results. Overflow cases are
    /// exercised separately.
    fn safe_principal() -> impl Strategy<Value = u128> {
        0u128..=PRINCIPAL_MAX
    }

    fn rate_bps() -> impl Strategy<Value = u32> {
        0u32..=10_000u32
    }

    /// Elapsed seconds, 0 to 100 years.
    fn elapsed_secs() -> impl Strategy<Value = u64> {
        0u64..=(SECONDS_PER_YEAR as u64 * 100)
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 512, .. ProptestConfig::default() })]

        /// **Monotone in time.** For `t1 ≤ t2`, the interest at `t2` is `≥`
        /// the interest at `t1` whenever both compute without overflow.
        #[test]
        fn monotone_in_time(
            p in safe_principal(),
            r in rate_bps(),
            t1 in elapsed_secs(),
            t2 in elapsed_secs(),
        ) {
            let (lo, hi) = if t1 <= t2 { (t1, t2) } else { (t2, t1) };
            let principal = Uint128::new(p);
            if let (Ok(a), Ok(b)) = (
                accrued_interest(principal, r, lo),
                accrued_interest(principal, r, hi),
            ) {
                prop_assert!(
                    b >= a,
                    "time monotonicity violated: interest({lo}s)={a} > interest({hi}s)={b} \
                     (principal={p}, rate={r})"
                );
            }
        }

        /// **Monotone in principal.** A larger balance accrues `≥` interest.
        #[test]
        fn monotone_in_principal(
            p1 in safe_principal(),
            p2 in safe_principal(),
            r in rate_bps(),
            t in elapsed_secs(),
        ) {
            let (lo, hi) = if p1 <= p2 { (p1, p2) } else { (p2, p1) };
            if let (Ok(a), Ok(b)) = (
                accrued_interest(Uint128::new(lo), r, t),
                accrued_interest(Uint128::new(hi), r, t),
            ) {
                prop_assert!(b >= a, "principal monotonicity violated: {a} > {b}");
            }
        }

        /// **Monotone in rate.** A higher rate accrues `≥` interest.
        #[test]
        fn monotone_in_rate(
            p in safe_principal(),
            r1 in rate_bps(),
            r2 in rate_bps(),
            t in elapsed_secs(),
        ) {
            let (lo, hi) = if r1 <= r2 { (r1, r2) } else { (r2, r1) };
            let principal = Uint128::new(p);
            if let (Ok(a), Ok(b)) = (
                accrued_interest(principal, lo, t),
                accrued_interest(principal, hi, t),
            ) {
                prop_assert!(b >= a, "rate monotonicity violated: {a} > {b}");
            }
        }

        /// **Zero boundary.** Any zero input produces exactly zero.
        #[test]
        fn zero_input_yields_zero(
            p in safe_principal(),
            r in rate_bps(),
            t in elapsed_secs(),
        ) {
            prop_assert_eq!(
                accrued_interest(Uint128::zero(), r, t).unwrap(),
                Uint128::zero()
            );
            prop_assert_eq!(
                accrued_interest(Uint128::new(p), 0, t).unwrap(),
                Uint128::zero()
            );
            prop_assert_eq!(
                accrued_interest(Uint128::new(p), r, 0).unwrap(),
                Uint128::zero()
            );
        }

        /// **Total / panic-free.** Over the full unrestricted input domain the
        /// function returns `Ok(_)` or `Err(Overflow)` — never panics or wraps.
        #[test]
        fn never_panics(
            p in any::<u128>(),
            r in any::<u32>(),
            t in any::<u64>(),
        ) {
            match accrued_interest(Uint128::new(p), r, t) {
                Ok(_) => {}
                Err(e) => prop_assert_eq!(e, ContractError::Overflow),
            }
        }

        /// **Overflow region is upward-closed in time.** If a smaller elapsed
        /// time overflows, every larger time also overflows.
        #[test]
        fn overflow_upward_closed_in_time(
            p in any::<u128>(),
            r in 1u32..=10_000u32,
            t1 in any::<u64>(),
            t2 in any::<u64>(),
        ) {
            let (lo, hi) = if t1 <= t2 { (t1, t2) } else { (t2, t1) };
            let principal = Uint128::new(p);
            if accrued_interest(principal, r, lo).is_err() && lo > 0 {
                prop_assert!(
                    accrued_interest(principal, r, hi).is_err(),
                    "overflow at {lo}s but not at {hi}s (principal={p}, rate={r})"
                );
            }
        }

        /// **Interest ≤ principal within one year.** At any rate ≤ 100 %
        /// the accrued interest for up to one year never exceeds the principal.
        #[test]
        fn interest_bounded_by_principal_within_one_year(
            p in safe_principal(),
            r in rate_bps(),
            t in 0u64..=(SECONDS_PER_YEAR as u64),
        ) {
            if let Ok(interest) = accrued_interest(Uint128::new(p), r, t) {
                prop_assert!(
                    interest.u128() <= p,
                    "interest ({interest}) > principal ({p}) at rate={r} bps, elapsed={t}s"
                );
            }
        }

        /// **Additive consistency.** Two consecutive sub-periods together
        /// accrue at most the amount for the combined period (flooring means
        /// split periods accrue ≤ the unsplit period).
        #[test]
        fn split_period_accrues_le_combined(
            p in safe_principal(),
            r in rate_bps(),
            t1 in 0u64..=(SECONDS_PER_YEAR as u64),
            t2 in 0u64..=(SECONDS_PER_YEAR as u64),
        ) {
            let principal = Uint128::new(p);
            let combined = t1.saturating_add(t2);
            if let (Ok(a), Ok(b), Ok(c)) = (
                accrued_interest(principal, r, t1),
                accrued_interest(principal, r, t2),
                accrued_interest(principal, r, combined),
            ) {
                // Floor rounding means a + b ≤ c (two floors lose at most 1 each).
                let sum = a.u128().saturating_add(b.u128());
                prop_assert!(
                    sum <= c.u128() + 2,
                    "split periods ({t1}+{t2}) accrued {sum} > combined {c} + 2 \
                     (principal={p}, rate={r})"
                );
            }
        }
    }
}
