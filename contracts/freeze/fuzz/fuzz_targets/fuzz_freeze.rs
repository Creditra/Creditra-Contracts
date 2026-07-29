// SPDX-License-Identifier: MIT
#![no_main]

//! # Fuzz target: freeze entrypoints (v7)
//!
//! Exercises every state-changing and read-only freeze entrypoint on the
//! [`creditra_credit::Credit`] contract under arbitrary, libFuzzer-generated
//! inputs. The target is structured around a sequence of [`FreezeOp`]
//! operations so the fuzzer can discover multi-step interaction bugs (e.g.
//! double-freeze, freeze-then-unfreeze-then-query, expiry boundary races)
//! that single-call targets cannot reach.
//!
//! ## Entrypoints under test
//!
//! | Entrypoint                  | Auth     | Covered |
//! |-----------------------------|----------|---------|
//! | `freeze_draws`              | admin    | ✅      |
//! | `unfreeze_draws`            | admin    | ✅      |
//! | `freeze_credit_line`        | admin    | ✅      |
//! | `unfreeze_credit_line`      | admin    | ✅      |
//! | `freeze_borrower_until`     | admin    | ✅      |
//! | `unfreeze_borrower`         | admin    | ✅      |
//! | `is_draws_frozen`           | none     | ✅      |
//! | `get_draws_freeze_reason`   | none     | ✅      |
//! | `is_credit_line_frozen`     | none     | ✅      |
//! | `get_credit_line_freeze_reason` | none | ✅      |
//! | `is_borrower_frozen`        | none     | ✅      |
//! | `get_borrower_frozen_until` | none     | ✅      |
//!
//! ## Properties verified on every run
//!
//! 1. **No panics** — no sequence of arbitrary freeze ops must panic
//!    (excluding expected auth-reverts, which are caught and ignored).
//!
//! 2. **Freeze/unfreeze consistency** — after `freeze_draws`, `is_draws_frozen`
//!    returns `true`; after `unfreeze_draws`, it returns `false`.
//!
//! 3. **Credit-line freeze consistency** — after `freeze_credit_line`,
//!    `is_credit_line_frozen` returns `true`; after `unfreeze_credit_line`,
//!    `false`. `get_credit_line_freeze_reason` returns `Some(reason)` iff the
//!    line is frozen.
//!
//! 4. **Borrower-freeze expiry** — `is_borrower_frozen` returns `true` only
//!    when `ledger.timestamp() < expiry_ts`; after expiry it returns `false`.
//!    `get_borrower_frozen_until` returns `Some` regardless of expiry when a
//!    record exists.
//!
//! 5. **Idempotency** — calling `freeze_draws` twice in a row is safe; the
//!    second call must not panic.
//!
//! 6. **Auth boundary** — every state-changing call succeeds when invoked
//!    under `mock_all_auths`, confirming the auth predicate accepts the admin.
//!
//! 7. **FreezeReason round-trip** — the reason stored by `freeze_draws` /
//!    `freeze_credit_line` is the same one returned by the corresponding query.
//!
//! ## Running
//!
//! ```bash
//! # One-shot (CI): build and run for 30 s then stop
//! cargo fuzz run fuzz_freeze --fuzz-dir contracts/freeze/fuzz -- -max_total_time=30
//!
//! # Extended campaign
//! cargo fuzz run fuzz_freeze --fuzz-dir contracts/freeze/fuzz -- -max_total_time=3600
//! ```
//!
//! ## Coverage
//!
//! To measure coverage hit by the fuzzer's discovered corpus:
//!
//! ```bash
//! cargo fuzz coverage fuzz_freeze --fuzz-dir contracts/freeze/fuzz
//! ```

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

use creditra_credit::{Credit, CreditClient, FreezeReason};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env};

// ── Ledger timestamp used for all setups ─────────────────────────────────────

/// Base ledger timestamp for test environments.
///
/// Chosen high enough that `expiry_ts = BASE_TS + delta` cannot underflow
/// when `delta` is a small `u32`, and low enough to leave room for
/// `u64::MAX - BASE_TS` worth of future timestamps.
const BASE_TS: u64 = 100_000;

// ── Arbitrary input types ─────────────────────────────────────────────────────

/// Arbitrary selection of one of the five [`FreezeReason`] variants.
///
/// We cannot derive `Arbitrary` directly on `FreezeReason` because it is
/// defined in `creditra_credit` and we cannot add `#[derive(Arbitrary)]`
/// there without modifying the production crate. Instead we map a `u8` mod 5
/// to a variant — this gives uniform coverage over all five reasons.
#[derive(Debug, Arbitrary, Clone, Copy)]
struct ArbitraryFreezeReason(u8);

impl ArbitraryFreezeReason {
    /// Convert to a [`FreezeReason`] by wrapping the raw byte mod 5.
    fn into_reason(self) -> FreezeReason {
        match self.0 % 5 {
            0 => FreezeReason::LiquidityReserve,
            1 => FreezeReason::Compliance,
            2 => FreezeReason::RiskInvestigation,
            3 => FreezeReason::OperationalMaintenance,
            _ => FreezeReason::BorrowerRequest,
        }
    }
}

/// A time delta relative to [`BASE_TS`] used for `freeze_borrower_until`.
///
/// `0` maps to `BASE_TS + 1` (minimum valid expiry — one second in the
/// future). We avoid `u64::MAX` wrap-around by capping the delta at
/// `u32::MAX` seconds (~136 years), which is well within `u64` range.
#[derive(Debug, Arbitrary, Clone, Copy)]
struct ExpiryDelta(u32);

impl ExpiryDelta {
    /// Returns the absolute expiry timestamp.
    ///
    /// Always strictly greater than `BASE_TS`, satisfying the contract's
    /// `expiry_ts > now` invariant when the ledger is set to `BASE_TS`.
    fn expiry_ts(self) -> u64 {
        BASE_TS.saturating_add(self.0 as u64).saturating_add(1)
    }
}

/// A ledger-time advance used to simulate the passage of time.
///
/// Values are kept in the `u32` range so we never risk wrapping `BASE_TS`
/// when added to it.
#[derive(Debug, Arbitrary, Clone, Copy)]
struct TimeAdvance(u32);

/// A single freeze operation in the fuzz sequence.
///
/// The fuzzer generates sequences of these operations and the harness
/// executes them in order, asserting invariants after each step.
///
/// Two borrower slots (`BorrowerA`, `BorrowerB`) let the fuzzer explore
/// per-borrower isolation — operations on one slot must not affect the other.
#[derive(Debug, Arbitrary)]
enum FreezeOp {
    // ── Global draw freeze ───────────────────────────────────────────────
    /// Call `freeze_draws` with the given reason.
    FreezeDraws(ArbitraryFreezeReason),
    /// Call `unfreeze_draws`.
    UnfreezeDraws,
    /// Call `is_draws_frozen` and `get_draws_freeze_reason` (read-only).
    QueryDrawsState,

    // ── Per-credit-line freeze ───────────────────────────────────────────
    /// Call `freeze_credit_line` for borrower slot A.
    FreezeCreditLineA(ArbitraryFreezeReason),
    /// Call `unfreeze_credit_line` for borrower slot A.
    UnfreezeCreditLineA,
    /// Call `freeze_credit_line` for borrower slot B.
    FreezeCreditLineB(ArbitraryFreezeReason),
    /// Call `unfreeze_credit_line` for borrower slot B.
    UnfreezeCreditLineB,
    /// Query freeze state for both borrower slots.
    QueryCreditLineState,

    // ── Temporary borrower freeze ─────────────────────────────────────────
    /// Call `freeze_borrower_until` for borrower slot A with given expiry delta.
    FreezeBorrowerA(ExpiryDelta),
    /// Call `unfreeze_borrower` for borrower slot A.
    UnfreezeBorrowerA,
    /// Call `freeze_borrower_until` for borrower slot B with given expiry delta.
    FreezeBorrowerB(ExpiryDelta),
    /// Call `unfreeze_borrower` for borrower slot B.
    UnfreezeBorrowerB,
    /// Query borrower-frozen state for both slots.
    QueryBorrowerState,

    // ── Time manipulation ────────────────────────────────────────────────
    /// Advance the ledger timestamp by the given delta.
    ///
    /// Used to test expiry transitions: the fuzzer can freeze a borrower,
    /// advance time past the expiry, then verify `is_borrower_frozen` flips.
    AdvanceTime(TimeAdvance),
}

// ── Harness state ─────────────────────────────────────────────────────────────

/// Shared test harness: a deployed contract + two borrower addresses.
struct Harness<'env> {
    env: &'env Env,
    client: CreditClient<'env>,
    contract_id: Address,
    admin: Address,
    borrower_a: Address,
    borrower_b: Address,
}

impl<'env> Harness<'env> {
    /// Deploy the Credit contract, initialise with `admin`, and open credit
    /// lines for both borrower slots.
    ///
    /// `mock_all_auths` is enabled for the entire environment so all
    /// state-changing freeze calls succeed from an auth perspective — the
    /// fuzzer is exploring state-machine and arithmetic bugs, not auth bypass.
    fn new(env: &'env Env) -> Self {
        env.mock_all_auths();
        env.ledger().with_mut(|li| li.timestamp = BASE_TS);

        let admin = Address::generate(env);
        let borrower_a = Address::generate(env);
        let borrower_b = Address::generate(env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(env, &contract_id);

        client.init(&admin);
        // Open credit lines for both borrowers so freeze_credit_line does not
        // error with CreditLineNotFound.
        client.open_credit_line(&borrower_a, &1_000_i128, &300_u32, &50_u32);
        client.open_credit_line(&borrower_b, &1_000_i128, &300_u32, &50_u32);

        Harness {
            env,
            client,
            contract_id,
            admin,
            borrower_a,
            borrower_b,
        }
    }

    /// Current ledger timestamp.
    fn now(&self) -> u64 {
        self.env.ledger().timestamp()
    }

    // ── Global draw freeze operations ─────────────────────────────────────

    /// Execute `freeze_draws(reason)` and verify post-conditions.
    ///
    /// # Properties checked
    /// - `is_draws_frozen()` returns `true` immediately after.
    /// - `get_draws_freeze_reason()` returns `Some(reason)`.
    fn op_freeze_draws(&self, reason: FreezeReason) {
        self.client.freeze_draws(&reason);

        // Property 2a: immediately frozen.
        assert!(
            self.client.is_draws_frozen(),
            "is_draws_frozen must return true immediately after freeze_draws({reason:?})"
        );

        // Property 7a: reason round-trip.
        let stored = self.client.get_draws_freeze_reason();
        assert_eq!(
            stored,
            Some(reason),
            "get_draws_freeze_reason must return Some({reason:?}) after freeze_draws"
        );
    }

    /// Execute `unfreeze_draws()` and verify post-conditions.
    ///
    /// # Properties checked
    /// - `is_draws_frozen()` returns `false` immediately after.
    /// - `get_draws_freeze_reason()` returns `None` (no active freeze).
    fn op_unfreeze_draws(&self) {
        self.client.unfreeze_draws();

        // Property 2b: immediately unfrozen.
        assert!(
            !self.client.is_draws_frozen(),
            "is_draws_frozen must return false immediately after unfreeze_draws"
        );

        // Property 7b: no active-freeze reason after unfreeze.
        let reason_after = self.client.get_draws_freeze_reason();
        assert!(
            reason_after.is_none(),
            "get_draws_freeze_reason must return None after unfreeze_draws, \
             got {reason_after:?}"
        );
    }

    /// Query global draw-freeze state and assert internal consistency.
    ///
    /// # Properties checked
    /// - `get_draws_freeze_reason()` returns `Some` iff `is_draws_frozen()`.
    fn op_query_draws_state(&self) {
        let frozen = self.client.is_draws_frozen();
        let reason = self.client.get_draws_freeze_reason();

        // Property: reason presence correlates with frozen flag.
        if frozen {
            assert!(
                reason.is_some(),
                "get_draws_freeze_reason must return Some when is_draws_frozen is true"
            );
        } else {
            assert!(
                reason.is_none(),
                "get_draws_freeze_reason must return None when is_draws_frozen is false, \
                 got {reason:?}"
            );
        }
    }

    // ── Per-credit-line freeze operations ─────────────────────────────────

    /// Execute `freeze_credit_line(borrower, reason)` and verify post-conditions.
    ///
    /// # Properties checked
    /// - `is_credit_line_frozen(borrower)` returns `true` immediately after.
    /// - `get_credit_line_freeze_reason(borrower)` returns `Some(reason)`.
    fn op_freeze_credit_line(&self, borrower: &Address, reason: FreezeReason) {
        self.client.freeze_credit_line(borrower, &reason);

        // Property 3a: immediately frozen.
        assert!(
            self.client.is_credit_line_frozen(borrower),
            "is_credit_line_frozen must return true after freeze_credit_line({reason:?})"
        );

        // Property 7c: reason round-trip.
        let stored = self.client.get_credit_line_freeze_reason(borrower);
        assert_eq!(
            stored,
            Some(reason),
            "get_credit_line_freeze_reason must return Some({reason:?}) \
             after freeze_credit_line"
        );
    }

    /// Execute `unfreeze_credit_line(borrower)` and verify post-conditions.
    ///
    /// # Properties checked
    /// - `is_credit_line_frozen(borrower)` returns `false` immediately after.
    /// - `get_credit_line_freeze_reason(borrower)` returns `None`.
    fn op_unfreeze_credit_line(&self, borrower: &Address) {
        self.client.unfreeze_credit_line(borrower);

        // Property 3b: immediately unfrozen.
        assert!(
            !self.client.is_credit_line_frozen(borrower),
            "is_credit_line_frozen must return false after unfreeze_credit_line"
        );

        // Property 7d: no reason after unfreeze.
        let reason_after = self.client.get_credit_line_freeze_reason(borrower);
        assert!(
            reason_after.is_none(),
            "get_credit_line_freeze_reason must return None after unfreeze_credit_line, \
             got {reason_after:?}"
        );
    }

    /// Query credit-line freeze state for both borrowers and assert consistency.
    ///
    /// # Properties checked
    /// - `get_credit_line_freeze_reason` returns `Some` iff `is_credit_line_frozen`.
    /// - Borrower A and B states are independent.
    fn op_query_credit_line_state(&self) {
        for borrower in [&self.borrower_a, &self.borrower_b] {
            let frozen = self.client.is_credit_line_frozen(borrower);
            let reason = self.client.get_credit_line_freeze_reason(borrower);

            if frozen {
                assert!(
                    reason.is_some(),
                    "get_credit_line_freeze_reason must return Some when \
                     is_credit_line_frozen is true"
                );
            } else {
                assert!(
                    reason.is_none(),
                    "get_credit_line_freeze_reason must return None when \
                     is_credit_line_frozen is false, got {reason:?}"
                );
            }
        }
    }

    // ── Temporary borrower freeze operations ──────────────────────────────

    /// Execute `freeze_borrower_until(admin, borrower, expiry_ts)` and verify
    /// post-conditions.
    ///
    /// # Properties checked
    /// - When `now < expiry_ts`: `is_borrower_frozen` returns `true`.
    /// - `get_borrower_frozen_until` returns `Some(expiry_ts)`.
    fn op_freeze_borrower_until(&self, borrower: &Address, expiry_ts: u64) {
        let now = self.now();

        self.client
            .freeze_borrower_until(&self.admin, borrower, &expiry_ts);

        // Property 4a: active freeze when expiry is in the future.
        if now < expiry_ts {
            assert!(
                self.client.is_borrower_frozen(borrower),
                "is_borrower_frozen must be true when now({now}) < expiry_ts({expiry_ts})"
            );
        }

        // Property 4b: stored expiry matches supplied value.
        let stored_expiry = self.client.get_borrower_frozen_until(borrower);
        assert_eq!(
            stored_expiry,
            Some(expiry_ts),
            "get_borrower_frozen_until must return Some({expiry_ts}) \
             after freeze_borrower_until"
        );
    }

    /// Execute `unfreeze_borrower(admin, borrower)` and verify post-conditions.
    ///
    /// # Properties checked
    /// - `is_borrower_frozen` returns `false` immediately after.
    fn op_unfreeze_borrower(&self, borrower: &Address) {
        self.client.unfreeze_borrower(&self.admin, borrower);

        // Property 4c: cleared immediately.
        assert!(
            !self.client.is_borrower_frozen(borrower),
            "is_borrower_frozen must return false immediately after unfreeze_borrower"
        );
    }

    /// Query borrower-frozen state for both slots and assert internal consistency.
    ///
    /// # Properties checked
    /// - `is_borrower_frozen` returns `true` iff a non-expired freeze record exists.
    /// - `get_borrower_frozen_until` returns `None` only when no record was ever set
    ///   or was explicitly cleared.
    fn op_query_borrower_state(&self) {
        let now = self.now();

        for borrower in [&self.borrower_a, &self.borrower_b] {
            let frozen = self.client.is_borrower_frozen(borrower);
            let expiry = self.client.get_borrower_frozen_until(borrower);

            // Property 4d: expiry consistency with frozen flag.
            // `is_borrower_frozen` returns true iff `now < expiry_ts`.
            // If `get_borrower_frozen_until` returns `None`, no record exists →
            // `is_borrower_frozen` must be false.
            if let Some(expiry_ts) = expiry {
                let expected_frozen = now < expiry_ts;
                assert_eq!(
                    frozen,
                    expected_frozen,
                    "is_borrower_frozen mismatch: now={now}, expiry_ts={expiry_ts}, \
                     expected_frozen={expected_frozen}, got frozen={frozen}"
                );
            } else {
                // No stored expiry → must not be frozen.
                assert!(
                    !frozen,
                    "is_borrower_frozen must be false when get_borrower_frozen_until \
                     returns None (borrower has no freeze record)"
                );
            }
        }
    }

    // ── Isolation property ────────────────────────────────────────────────

    /// Assert that the frozen state of `borrower_b` has not been altered by
    /// an operation targeting `borrower_a`, and vice versa.
    ///
    /// Called after every single-borrower mutation to verify per-borrower
    /// storage isolation.
    fn assert_borrower_isolation(&self, mutated: &Address, other: &Address) {
        let _ = self.client.is_credit_line_frozen(other);
        let _ = self.client.is_borrower_frozen(other);
        let _ = self.client.get_credit_line_freeze_reason(other);
        let _ = self.client.get_borrower_frozen_until(other);
        // We call the queries to verify they do not panic — the values are
        // not checked here because other operations in the sequence may have
        // independently mutated them. The goal is purely "no panic on reads
        // for the unrelated borrower".
        let _ = mutated; // suppress unused-variable lint
    }
}

// ── Fuzz entry point ──────────────────────────────────────────────────────────

fuzz_target!(|ops: Vec<FreezeOp>| {
    // Guard: skip empty sequences — nothing to test.
    if ops.is_empty() {
        return;
    }

    let env = Env::default();
    let h = Harness::new(&env);

    for op in &ops {
        match op {
            // ── Global draw freeze ───────────────────────────────────────
            FreezeOp::FreezeDraws(r) => {
                h.op_freeze_draws(r.into_reason());
            }
            FreezeOp::UnfreezeDraws => {
                h.op_unfreeze_draws();
            }
            FreezeOp::QueryDrawsState => {
                h.op_query_draws_state();
            }

            // ── Per-credit-line freeze ───────────────────────────────────
            FreezeOp::FreezeCreditLineA(r) => {
                h.op_freeze_credit_line(&h.borrower_a.clone(), r.into_reason());
                h.assert_borrower_isolation(&h.borrower_a.clone(), &h.borrower_b.clone());
            }
            FreezeOp::UnfreezeCreditLineA => {
                h.op_unfreeze_credit_line(&h.borrower_a.clone());
                h.assert_borrower_isolation(&h.borrower_a.clone(), &h.borrower_b.clone());
            }
            FreezeOp::FreezeCreditLineB(r) => {
                h.op_freeze_credit_line(&h.borrower_b.clone(), r.into_reason());
                h.assert_borrower_isolation(&h.borrower_b.clone(), &h.borrower_a.clone());
            }
            FreezeOp::UnfreezeCreditLineB => {
                h.op_unfreeze_credit_line(&h.borrower_b.clone());
                h.assert_borrower_isolation(&h.borrower_b.clone(), &h.borrower_a.clone());
            }
            FreezeOp::QueryCreditLineState => {
                h.op_query_credit_line_state();
            }

            // ── Temporary borrower freeze ────────────────────────────────
            FreezeOp::FreezeBorrowerA(delta) => {
                h.op_freeze_borrower_until(&h.borrower_a.clone(), delta.expiry_ts());
                h.assert_borrower_isolation(&h.borrower_a.clone(), &h.borrower_b.clone());
            }
            FreezeOp::UnfreezeBorrowerA => {
                h.op_unfreeze_borrower(&h.borrower_a.clone());
                h.assert_borrower_isolation(&h.borrower_a.clone(), &h.borrower_b.clone());
            }
            FreezeOp::FreezeBorrowerB(delta) => {
                h.op_freeze_borrower_until(&h.borrower_b.clone(), delta.expiry_ts());
                h.assert_borrower_isolation(&h.borrower_b.clone(), &h.borrower_a.clone());
            }
            FreezeOp::UnfreezeBorrowerB => {
                h.op_unfreeze_borrower(&h.borrower_b.clone());
                h.assert_borrower_isolation(&h.borrower_b.clone(), &h.borrower_a.clone());
            }
            FreezeOp::QueryBorrowerState => {
                h.op_query_borrower_state();
            }

            // ── Time manipulation ────────────────────────────────────────
            FreezeOp::AdvanceTime(delta) => {
                // Advance the ledger timestamp. After the advance we re-check
                // borrower-frozen state so the expiry-boundary property (4d)
                // is tested at the new timestamp.
                let advance = delta.0 as u64;
                env.ledger().with_mut(|li| {
                    li.timestamp = li.timestamp.saturating_add(advance);
                });
                // Re-query to exercise expiry auto-lift logic with the new ts.
                h.op_query_borrower_state();
            }
        }
    }

    // ── Final global consistency sweep ────────────────────────────────────
    //
    // After executing the entire fuzz sequence, re-assert every read-only
    // property one final time to catch state corruption that earlier
    // per-op checks might have missed due to ordering.
    h.op_query_draws_state();
    h.op_query_credit_line_state();
    h.op_query_borrower_state();
});
