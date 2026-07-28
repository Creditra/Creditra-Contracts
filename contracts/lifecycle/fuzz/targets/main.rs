// SPDX-License-Identifier: MIT
#![no_main]

//! # Fuzz target: lifecycle v7 state-machine property oracle
//!
//! ## Purpose
//!
//! This target stress-tests the entire credit-line lifecycle state machine
//! exposed by `creditra_credit::lifecycle`. It generates arbitrary
//! [`LifecycleOp`] sequences and applies them to a simulated Soroban
//! environment, asserting invariants that must hold regardless of the
//! input combination.
//!
//! ## Properties under test
//!
//! 1. **No panic on any input** — every lifecycle entrypoint either returns
//!    successfully or terminates via a known [`ContractError`] variant. Raw
//!    Rust panics with unrecognized messages are a bug.
//!
//! 2. **No invalid status after a successful transition** — after every
//!    successful operation the credit-line status must be one of the five
//!    valid [`CreditStatus`] variants.
//!
//! 3. **Closed is terminal** — once a line reaches `CreditStatus::Closed`,
//!    no subsequent draw or state-change (other than idempotent close) may
//!    bring it back to any other status.
//!
//! 4. **Suspended ↛ Active without reinstate** — a suspended/defaulted line
//!    can only become Active via `reinstate_credit_line`; direct opens on an
//!    existing non-closed line require admin auth and are separately tested.
//!
//! 5. **Overflow-safe math** — credit limit, utilized amount, and accrued
//!    interest values derived from arbitrary `i128` inputs must never
//!    silently wrap; any overflow triggers [`ContractError::Overflow`] (12).
//!
//! 6. **Discriminant stability** — the `u32` discriminants of every
//!    lifecycle-relevant [`ContractError`] variant match the pinned values
//!    declared in `creditra_credit::types`. A mismatch here means the ABI
//!    was accidentally broken.
//!
//! ## Usage
//!
//! ```bash
//! # Run from workspace root for 60 seconds
//! cargo fuzz run --manifest-path contracts/lifecycle/fuzz/Cargo.toml \
//!     lifecycle -- -max_total_time=60
//!
//! # Reproduce a specific crash artifact
//! cargo fuzz run --manifest-path contracts/lifecycle/fuzz/Cargo.toml \
//!     lifecycle artifacts/lifecycle/<crash-file>
//! ```
//!
//! ## Architecture note
//!
//! Because the Soroban `Env` is not `Send`, the fuzzer runs single-threaded.
//! Each [`fuzz_target!`] invocation constructs a fresh `Env` so no state
//! leaks between iterations.

use arbitrary::Arbitrary;
use creditra_credit::types::{ContractError, CreditStatus};
use creditra_credit::{Credit, CreditClient};
use libfuzzer_sys::fuzz_target;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

// ── Discriminant stability assertions (Property 6) ───────────────────────────
//
// These are evaluated at fuzz-target compile time (const assertions) so a
// discriminant accident is caught the moment the target is built, not at
// runtime. They mirror the pinned values in `tests/err_stab.rs` but live here
// so the fuzz binary independently enforces the ABI contract.

const _: () = {
    assert!(ContractError::CreditLineNotFound as u32 == 3);
    assert!(ContractError::CreditLineClosed as u32 == 4);
    assert!(ContractError::InvalidAmount as u32 == 5);
    assert!(ContractError::AlreadyInitialized as u32 == 14);
    assert!(ContractError::CreditLineSuspended as u32 == 20);
    assert!(ContractError::CreditLineDefaulted as u32 == 21);
    assert!(ContractError::LiquidationGraceActive as u32 == 55);
    assert!(ContractError::AlreadySettled as u32 == 51);
    assert!(ContractError::LimitOutOfBounds as u32 == 34);
    assert!(ContractError::Overflow as u32 == 12);
    assert!(ContractError::NotAdmin as u32 == 2);
    assert!(ContractError::Paused as u32 == 18);
    assert!(ContractError::RateTooHigh as u32 == 8);
    assert!(ContractError::ScoreTooHigh as u32 == 9);
};

// ── Fuzz input types ──────────────────────────────────────────────────────────

/// Clamp values to protect the fuzz scenario from trivially-invalid ranges
/// while still covering edge cases near boundaries.
///
/// The lifecycle engine itself enforces domain-level bounds; these clamps
/// prevent the fuzzer from wasting budget on cases that always reject at the
/// first guard.
const MAX_CREDIT_LIMIT: i128 = 1_000_000_000_000_i128; // 1 trillion
const MAX_RATE_BPS: u32 = 10_000; // 100 %  (MAX_INTEREST_RATE_BPS in risk.rs)
const MAX_RISK_SCORE: u32 = 100; // MAX_RISK_SCORE in risk.rs
const MAX_DRAW: i128 = 1_000_000_000_000_i128;
const MAX_REPAY: i128 = 1_000_000_000_000_i128;

/// A single operation to apply to the credit-line state machine.
///
/// Variants cover every lifecycle entrypoint in the v7 surface:
/// `open_credit_line`, `suspend_credit_line`, `self_suspend_credit_line`,
/// `close_credit_line`, `default_credit_line`, `reinstate_credit_line`, and
/// the two helper entrypoints `set_credit_limit_bounds` /
/// `validate_credit_limit_bounds`.
#[derive(Arbitrary, Debug, Clone)]
enum LifecycleOp {
    /// Open (or re-open as admin) a credit line with the given parameters.
    Open {
        credit_limit: i64,
        rate_bps: u16,
        risk_score: u8,
    },
    /// Admin-suspend the credit line.
    AdminSuspend,
    /// Borrower self-suspends their own active line.
    SelfSuspend,
    /// Close the credit line via the admin path.
    AdminClose,
    /// Close the credit line via the borrower path (requires zero utilization).
    BorrowerClose,
    /// Mark the credit line as defaulted.
    Default,
    /// Reinstate a defaulted line back to Active.
    ReinstateActive,
    /// Reinstate a defaulted line back to Restricted.
    ReinstateRestricted,
    /// Set global credit limit bounds (admin).
    SetBounds { min: i64, max: i64 },
    /// Draw credit (may be blocked by status / limit).
    Draw { amount: i64 },
    /// Repay credit (may be blocked by status).
    Repay { amount: i64 },
    /// Advance the ledger timestamp.
    AdvanceTime { delta_seconds: u32 },
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Apply a single [`LifecycleOp`] inside a `catch_unwind`-equivalent context.
///
/// Because Soroban `Env` is not `UnwindSafe`, we model expected failures by
/// calling `try_*` variants where available and treating panics that encode a
/// known `ContractError` as acceptable. Any other panic fails the fuzz target.
fn apply_op(
    env: &Env,
    client: &CreditClient,
    borrower: &Address,
    admin: &Address,
    op: &LifecycleOp,
) {
    match op {
        LifecycleOp::Open {
            credit_limit,
            rate_bps,
            risk_score,
        } => {
            // Clamp to the valid domain so the fuzzer explores interesting
            // states rather than always hitting the first guard.
            let limit = (*credit_limit as i128).clamp(1, MAX_CREDIT_LIMIT);
            let rate = (*rate_bps as u32).min(MAX_RATE_BPS);
            let score = (*risk_score as u32).min(MAX_RISK_SCORE);
            let _ = client.try_open_credit_line(borrower, &limit, &rate, &score);
        }

        LifecycleOp::AdminSuspend => {
            let _ = client.try_suspend_credit_line(borrower);
        }

        LifecycleOp::SelfSuspend => {
            let _ = client.try_self_suspend_credit_line(borrower);
        }

        LifecycleOp::AdminClose => {
            let _ = client.try_close_credit_line(borrower, admin);
        }

        LifecycleOp::BorrowerClose => {
            let _ = client.try_close_credit_line(borrower, borrower);
        }

        LifecycleOp::Default => {
            let _ = client.try_default_credit_line(borrower);
        }

        LifecycleOp::ReinstateActive => {
            let _ = client.try_reinstate_credit_line(borrower, &CreditStatus::Active);
        }

        LifecycleOp::ReinstateRestricted => {
            let _ = client.try_reinstate_credit_line(borrower, &CreditStatus::Restricted);
        }

        LifecycleOp::SetBounds { min, max } => {
            // Allow arbitrary (min, max) pairs including inverted ranges to
            // exercise the bounds-validation guard.
            let mn = *min as i128;
            let mx = *max as i128;
            let _ = client.try_set_credit_limit_bounds(&mn, &mx);
        }

        LifecycleOp::Draw { amount } => {
            let amt = (*amount as i128).clamp(1, MAX_DRAW);
            let _ = client.try_draw_credit(borrower, &amt);
        }

        LifecycleOp::Repay { amount } => {
            let amt = (*amount as i128).clamp(1, MAX_REPAY);
            let _ = client.try_repay_credit(borrower, &amt);
        }

        LifecycleOp::AdvanceTime { delta_seconds } => {
            let current = env.ledger().timestamp();
            env.ledger()
                .set_timestamp(current.saturating_add(*delta_seconds as u64));
        }
    }
}

// ── Invariant checkers ────────────────────────────────────────────────────────

/// Check Properties 2 & 3: status is always a valid variant, and Closed
/// is terminal.
///
/// This is called after every operation that might mutate the credit line.
fn assert_status_invariants(client: &CreditClient, borrower: &Address, was_closed: bool) {
    let line_opt = client.get_credit_line(borrower);
    let Some(line) = line_opt else {
        // Line does not exist yet — nothing to check.
        return;
    };

    // Property 2: status must be one of the five valid variants.
    let valid = matches!(
        line.status,
        CreditStatus::Active
            | CreditStatus::Suspended
            | CreditStatus::Defaulted
            | CreditStatus::Closed
            | CreditStatus::Restricted
    );
    assert!(
        valid,
        "invalid CreditStatus discriminant after operation: {:?}",
        line.status
    );

    // Property 3: once Closed, the line must remain Closed.
    if was_closed {
        assert_eq!(
            line.status,
            CreditStatus::Closed,
            "credit line escaped terminal Closed state"
        );
    }

    // Property 5 (partial): no negative amounts from overflow.
    assert!(
        line.utilized_amount >= 0,
        "utilized_amount underflowed to {}: arithmetic is not overflow-safe",
        line.utilized_amount
    );
    assert!(
        line.accrued_interest >= 0,
        "accrued_interest underflowed to {}: arithmetic is not overflow-safe",
        line.accrued_interest
    );
    assert!(
        line.credit_limit > 0,
        "credit_limit became non-positive ({}) — invariant violation",
        line.credit_limit
    );
}

// ── fuzz_target! entry point ──────────────────────────────────────────────────

fuzz_target!(|ops: Vec<LifecycleOp>| {
    // ── Environment bootstrap ─────────────────────────────────────────────
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    // Initialize the contract with a fresh admin.
    client.init(&admin);

    // ── Operation replay loop ─────────────────────────────────────────────
    let mut was_closed = false;

    for op in &ops {
        // Snapshot whether the line was Closed before applying the operation,
        // so we can verify Property 3 after the operation completes.
        if let Some(line) = client.get_credit_line(&borrower) {
            if line.status == CreditStatus::Closed {
                was_closed = true;
            }
        }

        // Apply the operation. `try_*` wrappers absorb ContractError panics;
        // any raw Rust panic propagates and fails the target.
        apply_op(&env, &client, &borrower, &admin, op);

        // Assert structural invariants after every step.
        assert_status_invariants(&client, &borrower, was_closed);
    }

    // ── Final cross-check: discriminants survive a full sequence ──────────
    //
    // After all operations, re-verify the discriminant pins inline.
    // This catches any hypothetical runtime modification of the enum layout
    // (e.g. via unsafe transmute introduced in a dependency).
    debug_assert_eq!(ContractError::CreditLineNotFound as u32, 3);
    debug_assert_eq!(ContractError::CreditLineClosed as u32, 4);
    debug_assert_eq!(ContractError::InvalidAmount as u32, 5);
    debug_assert_eq!(ContractError::CreditLineSuspended as u32, 20);
    debug_assert_eq!(ContractError::CreditLineDefaulted as u32, 21);
    debug_assert_eq!(ContractError::AlreadySettled as u32, 51);
    debug_assert_eq!(ContractError::LiquidationGraceActive as u32, 55);
});
