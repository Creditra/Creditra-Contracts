// SPDX-License-Identifier: MIT

//! Property-based tests verifying the bijection invariant of the borrower-id
//! mapping maintained by `storage.rs`.
//!
//! # What
//!
//! `storage.rs` maintains two complementary persistent storage slots:
//!
//! - `CreditLineIdByBorrower(Address)  → u32`    — stable numeric id per borrower
//! - `CreditLineBorrowerById(u32)      → Address` — reverse lookup from id to address
//!
//! These form a **bijective** (one-to-one, onto) mapping for every id in
//! `0..CreditLineCount`.  [`ensure_credit_line_id`] writes both directions
//! atomically on first open and is **idempotent** on subsequent opens of the
//! same address.
//!
//! # Why a proptest?
//!
//! A single linear test can verify happy-path correctness but cannot efficiently
//! cover the space of interleaved open/close/default/reinstate sequences.
//! Proptest generates hundreds of randomised operation sequences, shrinks any
//! failing case to a minimal reproducer, and enforces all three invariants:
//!
//! 1. **Round-trip** — enumerating id `N` yields a borrower whose forward
//!    mapping `CreditLineIdByBorrower` also returns `N` (bijection).
//! 2. **Stable-id** — closing then reopening the same borrower keeps the same id.
//! 3. **Monotonic count** — `CreditLineCount` never decreases.
//!
//! # Acceptance criteria
//!
//! - Round-trip holds for ≥256 proptest cases (configured via `PROPTEST_CASES`).
//! - `Close` + reopen of the same borrower yields the same id.
//! - `CreditLineCount` is monotonically non-decreasing.
//!
//! # How
//!
//! A proptest generates a sequence of [`Op`] variants over a fixed pool of
//! `N_BORROWERS` (= 12) borrower addresses.  After every operation the full
//! `0..CreditLineCount` range is walked via `enumerate_credit_lines` and each
//! entry is cross-checked against the locally-tracked `id_to_borrower` list.

use proptest::prelude::*;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env};

use creditra_credit::{types::CreditStatus, Credit, CreditClient};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Number of distinct borrower addresses in the pool.  Must be >= 10 per the
/// issue acceptance criteria.
const N_BORROWERS: usize = 12;

// ---------------------------------------------------------------------------
// Test environment helpers
// ---------------------------------------------------------------------------

/// Shared initialisation: register contract, init admin, generate borrower pool.
///
/// Returns `(env, contract_id, admin, borrowers)`.
fn setup_env() -> (Env, Address, Address, Vec<Address>) {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());

    // Non-zero start timestamp prevents monotonicity violations on first write.
    env.ledger().with_mut(|li| li.timestamp = 1_000);

    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let borrowers: Vec<Address> = (0..N_BORROWERS)
        .map(|_| Address::generate(&env))
        .collect();

    (env, contract_id, admin, borrowers)
}

/// Open (or re-open) a credit line.  Returns `true` on success.
///
/// Uses a fixed credit-limit/rate/score that is well within bounds.
fn try_open(client: &CreditClient, borrower: &Address) -> bool {
    client
        .try_open_credit_line(borrower, &10_000_i128, &500_u32, &50_u32)
        .is_ok()
}

/// Admin-close a credit line.  Returns `true` on success.
fn try_close(client: &CreditClient, borrower: &Address, admin: &Address) -> bool {
    client.try_close_credit_line(borrower, admin).is_ok()
}

/// Admin-default a credit line.  Returns `true` on success.
fn try_default(client: &CreditClient, borrower: &Address) -> bool {
    client.try_default_credit_line(borrower).is_ok()
}

/// Reinstate a Defaulted credit line back to Active.  Returns `true` on success.
fn try_reinstate(client: &CreditClient, borrower: &Address) -> bool {
    client
        .try_reinstate_credit_line(borrower, &CreditStatus::Active)
        .is_ok()
}

// ---------------------------------------------------------------------------
// Operation model
// ---------------------------------------------------------------------------

/// Operations that can be applied to the borrower at `idx` in the pool.
#[derive(Debug, Clone)]
enum Op {
    /// Open (or re-open after Close/Default) a credit line.
    Open { idx: usize },
    /// Force-close a credit line (admin path — works from any non-Closed status).
    Close { idx: usize },
    /// Admin-default a credit line (Active/Suspended/Restricted → Defaulted).
    Default { idx: usize },
    /// Reinstate a Defaulted credit line to Active.
    Reinstate { idx: usize },
}

/// Proptest strategy that generates a random [`Op`] referencing a valid
/// borrower pool index.
///
/// `Open` is weighted 4× relative to `Close`/`Default`/`Reinstate` so that
/// the pool fills up quickly and the bijection sweep exercises many ids.
fn arb_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (0..N_BORROWERS).prop_map(|idx| Op::Open { idx }),
        2 => (0..N_BORROWERS).prop_map(|idx| Op::Close { idx }),
        1 => (0..N_BORROWERS).prop_map(|idx| Op::Default { idx }),
        1 => (0..N_BORROWERS).prop_map(|idx| Op::Reinstate { idx }),
    ]
}

// ---------------------------------------------------------------------------
// Bijection sweep
// ---------------------------------------------------------------------------

/// Walk every id in `0..count` via `enumerate_credit_lines` and verify:
///
/// 1. The enumeration returns contiguous ids `0, 1, 2, …, count-1`.
/// 2. For each id the embedded `borrower` address matches `id_to_borrower[id]`.
///
/// Returns a `TestCaseError` via `prop_assert*!` on any violation.
fn sweep_bijection(
    client: &CreditClient,
    id_to_borrower: &[Address],
    count: u32,
    label: &str,
) -> Result<(), TestCaseError> {
    let mut cursor: Option<u32> = None;
    let mut walked = 0_u32;

    loop {
        let page = client.enumerate_credit_lines(&cursor, &100_u32);
        if page.len() == 0 {
            break;
        }

        for i in 0..page.len() {
            let (id, line) = page.get(i).expect("index within page bounds");

            // ── Contiguity ────────────────────────────────────────────────
            prop_assert_eq!(
                id,
                walked,
                "[{label}] enumerate yielded non-contiguous id={id}, expected {walked}"
            );

            // ── Known-address cross-check ─────────────────────────────────
            // If the ID is within the range of our recorded first-opens, the
            // borrower address must match.
            if (id as usize) < id_to_borrower.len() {
                let expected_addr = &id_to_borrower[id as usize];
                prop_assert_eq!(
                    &line.borrower,
                    expected_addr,
                    "[{label}] id={id}: enumerated borrower ≠ recorded borrower"
                );
            } else {
                return Err(TestCaseError::fail(format!(
                    "[{label}] id={id} is outside recorded first-opens length {}",
                    id_to_borrower.len()
                )));
            }

            walked += 1;
            cursor = Some(id);
        }

        if page.len() < 100 {
            break;
        }
    }

    // Enumeration must cover exactly `count` ids (no gaps, no extras).
    prop_assert_eq!(
        walked,
        count,
        "[{label}] enumeration covered {walked} ids but CreditLineCount={count}"
    );

    // Locally tracked first-opens must match the contract's count.
    prop_assert_eq!(
        id_to_borrower.len(),
        count as usize,
        "[{label}] local first-opens length {} ≠ CreditLineCount={count}",
        id_to_borrower.len()
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Deterministic unit tests (fast baseline)
// ---------------------------------------------------------------------------

/// Open all N_BORROWERS lines and verify the bijection once.
///
/// This deterministic test is a fast sanity check on the harness itself before
/// the proptest exercises random interleaving.
#[test]
fn bijection_baseline_open_all() {
    let (env, contract_id, _admin, borrowers) = setup_env();
    let _ = &env;
    let client = CreditClient::new(&env, &contract_id);

    for borrower in &borrowers {
        let ok = try_open(&client, borrower);
        assert!(ok, "open must succeed for a fresh borrower");
    }

    let count = client.get_credit_line_count();
    assert_eq!(count, N_BORROWERS as u32, "count must equal N_BORROWERS");

    // Walk enumeration and verify contiguous ids.
    let mut cursor: Option<u32> = None;
    let mut walked = 0_u32;
    loop {
        let page = client.enumerate_credit_lines(&cursor, &100_u32);
        if page.len() == 0 {
            break;
        }
        for i in 0..page.len() {
            let (id, _) = page.get(i).unwrap();
            assert_eq!(id, walked, "id must be contiguous");
            walked += 1;
            cursor = Some(id);
        }
        if page.len() < 100 {
            break;
        }
    }
    assert_eq!(walked, count, "enumeration must cover all ids");
}

/// Close then reopen the same borrower; the stable id must be unchanged.
#[test]
fn stable_id_after_close_reopen() {
    let (env, contract_id, admin, borrowers) = setup_env();
    let client = CreditClient::new(&env, &contract_id);

    let borrower = &borrowers[0];

    // First open.
    assert!(try_open(&client, borrower), "first open must succeed");
    let count_first = client.get_credit_line_count();
    assert_eq!(count_first, 1);

    // Read the assigned id.
    let page = client.enumerate_credit_lines(&None, &1_u32);
    let (id_before, _) = page.get(0).expect("must have one entry");

    // Admin-close.
    assert!(try_close(&client, borrower, &admin), "close must succeed");

    // Reopen (admin re-opens a Closed line).
    assert!(try_open(&client, borrower), "re-open must succeed");

    // Count must not have grown (same borrower → same slot).
    let count_after = client.get_credit_line_count();
    assert_eq!(
        count_after, count_first,
        "CreditLineCount must not grow on re-open of same borrower"
    );

    // Verify id is stable.
    let page2 = client.enumerate_credit_lines(&None, &1_u32);
    let (id_after, _) = page2.get(0).expect("must have one entry");
    assert_eq!(
        id_after, id_before,
        "stable id must be preserved across close+reopen"
    );
}

/// `CreditLineCount` must never decrease, even after many closes.
#[test]
fn count_monotonically_increases() {
    let (env, contract_id, admin, borrowers) = setup_env();
    let client = CreditClient::new(&env, &contract_id);

    let mut max_count = 0_u32;

    for borrower in &borrowers {
        // Advance timestamp to avoid monotonicity guard issues.
        env.ledger().with_mut(|li| li.timestamp += 60);

        try_open(&client, borrower);
        let c = client.get_credit_line_count();
        assert!(c >= max_count, "count regressed after open: {c} < {max_count}");
        max_count = c;

        try_close(&client, borrower, &admin);
        let c2 = client.get_credit_line_count();
        assert!(
            c2 >= max_count,
            "count regressed after close: {c2} < {max_count}"
        );
        max_count = c2;
    }

    // Sanity: all N_BORROWERS slots were issued.
    assert_eq!(max_count, N_BORROWERS as u32);
}

// ---------------------------------------------------------------------------
// Main property-based test
// ---------------------------------------------------------------------------

proptest! {
    // Run 256 cases to satisfy the acceptance criterion.
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// For ≥256 randomly-generated operation sequences spanning ≥12 distinct
    /// borrowers, assert after every operation:
    ///
    /// 1. **Round-trip / bijection** — enumerating the full range
    ///    `0..CreditLineCount` via `enumerate_credit_lines` returns contiguous
    ///    ids and each entry's embedded borrower address matches the address
    ///    that was recorded when it was first opened.
    ///
    /// 2. **Monotonic count** — `CreditLineCount` never decreases across any op.
    ///
    /// 3. **Stable id** — when a borrower is reopened after a close, the id
    ///    recovered from enumeration equals the id recorded at first open.
    #[test]
    fn prop_bijection_survives_open_close_churn(
        ops in proptest::collection::vec(arb_op(), 40..100),
    ) {
        let (env, contract_id, admin, borrowers) = setup_env();
        let client = CreditClient::new(&env, &contract_id);

        // The list of borrower addresses in the order they were first opened.
        // The index in this vector is the contract-assigned ID.
        let mut id_to_borrower: Vec<Address> = Vec::new();

        // Track the ID of each borrower in the pool (indexed by borrower pool index `idx`).
        // `borrower_to_id[idx]` is `Some(id)` if the borrower was opened, `None` otherwise.
        let mut borrower_to_id: Vec<Option<u32>> = vec![None; N_BORROWERS];

        // Track the previous count for monotonicity verification.
        let mut prev_count = 0_u32;

        for op in &ops {
            // Advance ledger time by a small positive delta before every op so
            // that the monotonic-timestamp guard in `assert_ts_monotonic` never
            // fires for the operations that write timestamps.
            env.ledger().with_mut(|li| li.timestamp += 10);

            match op {
                Op::Open { idx } => {
                    let borrower = &borrowers[*idx];
                    let ok = try_open(&client, borrower);

                    if ok {
                        let count = client.get_credit_line_count();

                        // Monotonicity check.
                        prop_assert!(
                            count >= prev_count,
                            "CreditLineCount regressed after Open: {count} < {prev_count}"
                        );
                        prev_count = count;

                        // Find the current id of this borrower via enumeration.
                        // We scan at most `count` entries (capped at 100 per page).
                        let mut found_id: Option<u32> = None;
                        let mut cursor2: Option<u32> = None;
                        'outer: loop {
                            let pg = client.enumerate_credit_lines(&cursor2, &100_u32);
                            if pg.len() == 0 {
                                break;
                            }
                            for j in 0..pg.len() {
                                let (id, line) = pg.get(j).unwrap();
                                if line.borrower == *borrower {
                                    found_id = Some(id);
                                    break 'outer;
                                }
                                cursor2 = Some(id);
                            }
                            if pg.len() < 100 {
                                break;
                            }
                        }

                        let id = match found_id {
                            Some(i) => i,
                            None => {
                                // The borrower must appear after a successful open.
                                return Err(TestCaseError::fail(
                                    format!("borrower[{idx}] not found in enumerate after open")
                                ));
                            }
                        };

                        // Stable-id check: if we have a recorded first-open id,
                        // it must equal the current id.
                        if let Some(prev_id) = borrower_to_id[*idx] {
                            prop_assert_eq!(
                                id,
                                prev_id,
                                "stable-id violated for borrowers[{idx}]: \
                                 id after reopen={id}, expected {prev_id}"
                            );
                        } else {
                            borrower_to_id[*idx] = Some(id);
                            // The ID assigned must be the next index in id_to_borrower.
                            prop_assert_eq!(
                                id as usize,
                                id_to_borrower.len(),
                                "first-open ID={id} does not match expected sequential ID={}",
                                id_to_borrower.len()
                            );
                            id_to_borrower.push(borrower.clone());
                        }
                    }
                    // If open failed (e.g. line is already Active), skip silently.
                }

                Op::Close { idx } => {
                    let _ = try_close(&client, &borrowers[*idx], &admin);
                    let count = client.get_credit_line_count();
                    prop_assert!(
                        count >= prev_count,
                        "CreditLineCount regressed after Close: {count} < {prev_count}"
                    );
                    prev_count = count;
                }

                Op::Default { idx } => {
                    let _ = try_default(&client, &borrowers[*idx]);
                    let count = client.get_credit_line_count();
                    prop_assert!(
                        count >= prev_count,
                        "CreditLineCount regressed after Default: {count} < {prev_count}"
                    );
                    prev_count = count;
                }

                Op::Reinstate { idx } => {
                    let _ = try_reinstate(&client, &borrowers[*idx]);
                    let count = client.get_credit_line_count();
                    prop_assert!(
                        count >= prev_count,
                        "CreditLineCount regressed after Reinstate: {count} < {prev_count}"
                    );
                    prev_count = count;
                }
            }

            // ── Bijection sweep after every operation ─────────────────────
            let count = client.get_credit_line_count();
            sweep_bijection(&client, &id_to_borrower, count, "post-op sweep")?;
        }
    }
}
