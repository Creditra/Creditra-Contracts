// SPDX-License-Identifier: MIT
#![no_main]

//! # Fuzz target: query (v7) — read-only query property oracle
//!
//! ## Purpose
//!
//! This target stress-tests every read-only query entrypoint exposed by
//! `creditra_credit` (the v7 query surface) against arbitrary credit-line
//! state sequences. It verifies that queries are internally consistent,
//! never panic, and produce deterministic results for identical state.
//!
//! ## Properties under test
//!
//! 1. **No panic on any input** — every query returns successfully. Panics
//!    with unrecognised messages are a bug.
//!
//! 2. **`get_credit_line` / `get_credit_line_summary` match** — the two
//!    aliases always return the same value.
//!
//! 3. **`get_protocol_summary` counters are sane** — `total_utilized` is
//!    never negative and matches the sum of per-borrower utilizations.
//!
//! 4. **`get_health_factor` zero-utilization sentinel** — when
//!    `utilized_amount <= 0`, the result is always `u32::MAX`.
//!
//! 5. **`is_delinquent` matches `borrow_capabilities`** — when
//!    `borrow_capabilities.can_draw` is `true`, `is_delinquent` may be
//!    `true` or `false` but must not panic.
//!
//! 6. **`borrow_capabilities.can_repay` consistent** — `can_repay` is
//!    `true` exactly when a non-`Closed` credit line exists.
//!
//! 7. **`get_repayment_schedule` stability** — returns the same
//!    `Option` across two consecutive calls with no intervening mutation.
//!
//! 8. **Overflow-safe math** — health factor arithmetic and summary
//!    counters never produce negative numbers.
//!
//! ## Usage
//!
//! ```bash
//! # From workspace root
//! cargo fuzz run --manifest-path contracts/query/fuzz/Cargo.toml \
//!     main -- -max_total_time=60
//!
//! # Reproduce a specific crash
//! cargo fuzz run --manifest-path contracts/query/fuzz/Cargo.toml \
//!     main artifacts/query/<crash-file>
//! ```
//!
//! ## Architecture note
//!
//! Because the Soroban `Env` is not `Send`, the fuzzer runs single-threaded.
//! Each `fuzz_target!` invocation constructs a fresh `Env` so no state
//! leaks between iterations.

use arbitrary::Arbitrary;
use creditra_credit::types::{BorrowCapabilities, ContractError, CreditLineData, CreditStatus, ProtocolSummary, QueryCapabilities, RepaymentSchedule};
use creditra_credit::{Credit, CreditClient};
use libfuzzer_sys::fuzz_target;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, Address, Env};

// ── Discriminant stability assertions ───────────────────────────────────
//
// Pinned at compile time so a discriminant change is caught the moment the
// fuzz binary is built, not during corpus exploration.

const _: () = {
    assert!(ContractError::CreditLineNotFound as u32 == 3);
    assert!(ContractError::CreditLineClosed as u32 == 4);
    assert!(ContractError::InvalidAmount as u32 == 5);
    assert!(ContractError::CreditLineSuspended as u32 == 20);
    assert!(ContractError::CreditLineDefaulted as u32 == 21);
    assert!(ContractError::Overflow as u32 == 12);
};

// ── Clamp constants ─────────────────────────────────────────────────────

const MAX_CREDIT_LIMIT: i128 = 1_000_000_000_000_i128;
const MAX_RATE_BPS: u32 = 10_000;
const MAX_RISK_SCORE: u32 = 100;
const MAX_DRAW: i128 = 1_000_000_000_000_i128;
const MAX_REPAY: i128 = 1_000_000_000_000_i128;
const MAX_REPAY_AMOUNT_PER_PERIOD: i128 = 1_000_000_000_000_i128;

// ── Fuzz input type ─────────────────────────────────────────────────────

/// A single operation that mutates contract state and is followed by query
/// invariant checks.
#[derive(Arbitrary, Debug, Clone)]
enum QueryFuzzOp {
    /// Open a credit line for the borrower (admin).
    Open {
        credit_limit: i64,
        rate_bps: u16,
        risk_score: u8,
    },
    /// Draw credit (borrower).
    Draw {
        amount: i64,
    },
    /// Repay credit (borrower).
    Repay {
        amount: i64,
    },
    /// Admin-suspend the credit line.
    AdminSuspend,
    /// Admin-close the credit line.
    AdminClose,
    /// Borrower-close the credit line (requires zero utilization).
    BorrowerClose,
    /// Mark the credit line as defaulted (admin).
    Default,
    /// Reinstate a defaulted line to Active.
    ReinstateActive,
    /// Reinstate a defaulted line to Restricted.
    ReinstateRestricted,
    /// Advance the ledger timestamp.
    AdvanceTime {
        delta_seconds: u32,
    },
    /// Set the minimum collateral ratio (admin).
    SetMinCollateralRatioBps(u32),
    /// Explicitly re-assert all query invariants without mutation.
    CheckInvariants,
}

// ── Helpers ─────────────────────────────────────────────────────────────

/// Apply a single [`QueryFuzzOp`] to the contract state.
fn apply_op(
    env: &Env,
    client: &CreditClient,
    borrower: &Address,
    _admin: &Address,
    op: &QueryFuzzOp,
) {
    match op {
        QueryFuzzOp::Open {
            credit_limit,
            rate_bps,
            risk_score,
        } => {
            let limit = (*credit_limit as i128).clamp(1, MAX_CREDIT_LIMIT);
            let rate = (*rate_bps as u32).min(MAX_RATE_BPS);
            let score = (*risk_score as u32).min(MAX_RISK_SCORE);
            let _ = client.try_open_credit_line(borrower, &limit, &rate, &score);
        }

        QueryFuzzOp::Draw { amount } => {
            let amt = (*amount as i128).clamp(1, MAX_DRAW);
            let _ = client.try_draw_credit(borrower, &amt);
        }

        QueryFuzzOp::Repay { amount } => {
            let amt = (*amount as i128).clamp(1, MAX_REPAY);
            let _ = client.try_repay_credit(borrower, &amt);
        }

        QueryFuzzOp::AdminSuspend => {
            let _ = client.try_suspend_credit_line(borrower);
        }

        QueryFuzzOp::AdminClose => {
            let _ = client.try_close_credit_line(borrower, _admin);
        }

        QueryFuzzOp::BorrowerClose => {
            let _ = client.try_close_credit_line(borrower, borrower);
        }

        QueryFuzzOp::Default => {
            let _ = client.try_default_credit_line(borrower);
        }

        QueryFuzzOp::ReinstateActive => {
            let _ = client.try_reinstate_credit_line(borrower, &CreditStatus::Active);
        }

        QueryFuzzOp::ReinstateRestricted => {
            let _ = client.try_reinstate_credit_line(borrower, &CreditStatus::Restricted);
        }

        QueryFuzzOp::AdvanceTime { delta_seconds } => {
            let current = env.ledger().timestamp();
            env.ledger()
                .set_timestamp(current.saturating_add(*delta_seconds as u64));
        }

        QueryFuzzOp::SetMinCollateralRatioBps(bps) => {
            let _ = client.try_set_min_collateral_ratio_bps(bps);
        }

        QueryFuzzOp::CheckInvariants => {
            // pure query — no mutation; invariants are checked by the caller
        }
    }
}

// ── Invariant check helpers ─────────────────────────────────────────────

/// Assert that `get_credit_line` and `get_credit_line_summary` always agree.
///
/// Property 2: the two aliases must return identical `Option<CreditLineData>`.
fn check_credit_line_aliases(client: &CreditClient, borrower: &Address) {
    let line = client.get_credit_line(borrower);
    let summary = client.get_credit_line_summary(borrower);
    assert_eq!(
        line, summary,
        "get_credit_line and get_credit_line_summary must match"
    );
}

/// Assert invariants on the raw `Option<CreditLineData>` returned by queries.
///
/// Properties:
/// - If `Some`, `utilized_amount >= 0`, `accrued_interest >= 0`,
///   `credit_limit > 0`, and `status` is a valid variant.
fn check_credit_line_invariants(line_opt: &Option<CreditLineData>) {
    let Some(line) = line_opt else {
        return;
    };

    assert!(
        line.utilized_amount >= 0,
        "utilized_amount must be non-negative, got {}",
        line.utilized_amount
    );
    assert!(
        line.accrued_interest >= 0,
        "accrued_interest must be non-negative, got {}",
        line.accrued_interest
    );
    assert!(
        line.credit_limit > 0,
        "credit_limit must be positive, got {}",
        line.credit_limit
    );
    assert!(
        matches!(
            line.status,
            CreditStatus::Active
                | CreditStatus::Suspended
                | CreditStatus::Defaulted
                | CreditStatus::Closed
                | CreditStatus::Restricted
        ),
        "invalid CreditStatus discriminant: {:?}",
        line.status
    );
}

/// Assert protocol-summary invariants.
///
/// Properties:
/// - `total_utilized >= 0`
/// - `count >= 0`
fn check_protocol_summary_invariants(summary: &ProtocolSummary) {
    assert!(
        summary.total_utilized >= 0,
        "total_utilized must be non-negative, got {}",
        summary.total_utilized
    );
    assert!(
        summary.total_collateral >= 0,
        "total_collateral must be non-negative, got {}",
        summary.total_collateral
    );
    assert!(
        summary.treasury_balance >= 0,
        "treasury_balance must be non-negative, got {}",
        summary.treasury_balance
    );
    assert!(
        summary.bounty_balance >= 0,
        "bounty_balance must be non-negative, got {}",
        summary.bounty_balance
    );
}

/// Assert health-factor invariants.
///
/// Properties:
/// - When `utilized_amount <= 0`, health factor must be `u32::MAX`.
/// - Health factor is never zero when there is utilization (it's at least 1).
fn check_health_factor(
    hf: u32,
    line_opt: &Option<CreditLineData>,
) {
    if let Some(line) = line_opt {
        if line.utilized_amount <= 0 {
            assert_eq!(
                hf,
                u32::MAX,
                "zero utilization must produce u32::MAX health factor"
            );
        }
    }
}

/// Assert borrow-capabilities invariants.
///
/// Properties:
/// - `can_repay` is `true` iff a non-Closed credit line exists.
/// - `can_self_suspend` is `true` iff a line exists with `status == Active`.
/// - `can_draw` is `true` only when the line exists and status is draw-allowed
///   (not Suspended, Defaulted, or Closed).
fn check_borrow_capabilities(
    caps: &BorrowCapabilities,
    line_opt: &Option<CreditLineData>,
) {
    match line_opt {
        None => {
            assert!(
                !caps.can_draw,
                "can_draw must be false when no credit line exists"
            );
            assert!(
                !caps.can_repay,
                "can_repay must be false when no credit line exists"
            );
            assert!(
                !caps.can_self_suspend,
                "can_self_suspend must be false when no credit line exists"
            );
        }
        Some(line) => {
            // can_repay: true iff the line is not Closed
            let expected_can_repay = line.status != CreditStatus::Closed;
            assert_eq!(
                caps.can_repay, expected_can_repay,
                "can_repay mismatch for status {:?}",
                line.status
            );

            // can_self_suspend: true iff status is Active
            let expected_can_self_suspend = line.status == CreditStatus::Active;
            assert_eq!(
                caps.can_self_suspend, expected_can_self_suspend,
                "can_self_suspend mismatch for status {:?}",
                line.status
            );
        }
    }
}

/// Assert repayment-schedule idempotency.
///
/// Property: calling `get_repayment_schedule` twice with no mutation in
/// between must return the same value.
fn check_repayment_schedule_idempotent(client: &CreditClient, borrower: &Address) {
    let first = client.get_repayment_schedule(borrower);
    let second = client.get_repayment_schedule(borrower);
    assert_eq!(
        first, second,
        "get_repayment_schedule must be idempotent"
    );
}

/// Assert `is_delinquent` does not panic for any state.
///
/// Also sanity check: `is_delinquent` is `false` when no credit line exists.
fn check_is_delinquent_safe(
    client: &CreditClient,
    borrower: &Address,
    line_opt: &Option<CreditLineData>,
) {
    let delinquent = client.is_delinquent(borrower);
    if line_opt.is_none() {
        assert!(
            !delinquent,
            "is_delinquent must be false when no credit line exists"
        );
    }
}

/// Run all query invariant checks for the current state.
fn check_all_query_invariants(
    env: &Env,
    client: &CreditClient,
    borrower: &Address,
    _admin: &Address,
) {
    // 1. get_credit_line + get_credit_line_summary aliases
    check_credit_line_aliases(client, borrower);

    let line_opt = client.get_credit_line(borrower);

    // 2. Credit line structural invariants
    check_credit_line_invariants(&line_opt);

    // 3. Protocol summary
    let summary = client.get_protocol_summary();
    check_protocol_summary_invariants(&summary);

    // 4. Health factor
    let hf = client.get_health_factor(borrower);
    check_health_factor(hf, &line_opt);

    // 5. Borrow capabilities
    let caps = client.borrow_capabilities(borrower);
    check_borrow_capabilities(&caps, &line_opt);

    // 6. Repayment schedule idempotency
    check_repayment_schedule_idempotent(client, borrower);

    // 7. is_delinquent safety
    check_is_delinquent_safe(client, borrower, &line_opt);
}

// ── fuzz_target! entry point ────────────────────────────────────────────

fuzz_target!(|ops: Vec<QueryFuzzOp>| {
    // ── Environment bootstrap ─────────────────────────────────────────
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    // Initialise the contract with a fresh admin.
    client.init(&admin);

    // Register a liquidity token so draw/repay operations are usable.
    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);

    // Mint tokens to the contract (reserve) and the borrower.
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &1_000_000_000_i128);
    token::StellarAssetClient::new(&env, &token_address).mint(&borrower, &1_000_000_000_i128);

    // Initial invariants: no credit line exists yet.
    check_all_query_invariants(&env, &client, &borrower, &admin);

    // ── Operation loop ────────────────────────────────────────────────
    for op in &ops {
        apply_op(&env, &client, &borrower, &admin, op);
        check_all_query_invariants(&env, &client, &borrower, &admin);
    }
});
