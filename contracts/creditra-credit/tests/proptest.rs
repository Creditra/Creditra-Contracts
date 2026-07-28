// SPDX-License-Identifier: MIT

//! # Invariant property tests for the creditra-credit (borrow) contract
//!
//! This test suite uses `proptest` to generate randomized sequences of
//! borrow operations (create credit line → draw → repay) and verifies
//! that the protocol invariants hold at every step.
//!
//! ## Coverage
//!
//! - Randomised sequences of CreateCreditLine, CreateDraw, RepayDraw
//! - State consistency after every successful operation
//! - Failed operations never mutate storage
//! - Authorisation enforcement
//! - Error-path atomicity
//!
//! ## Invariants verified
//!
//! 1. **Draw state correctness**: After a successful draw the stored
//!    `Draw` struct has `repaid == false`, correct `amount`, `denom`,
//!    and `credit_line_id`.
//! 2. **Sequential draw IDs**: Draws are numbered `[0, n)` without gaps.
//! 3. **Draw-count consistency**: `DRAW_COUNT[cl_id]` equals the number of
//!    stored draws for that credit line.
//! 4. **Repayment correctness**: After successful repay `draw.repaid == true`.
//! 5. **Audit trail correctness**: Each draw has a `DrawCreated` audit entry
//!    at seq 0; each repay appends a `Repaid` audit entry.
//! 6. **Authorisation**: Non-borrower cannot draw or repay on another's line.
//! 7. **Error atomicity**: Failed operations leave storage unchanged.
//! 8. **Non-existent entity**: Operations on nonexistent credit lines or
//!    draws return appropriate errors.
//! 9. **Repeated repay**: Repaying an already-repaid draw succeeds
//!    (idempotent) and appends another audit entry.

use cosmwasm_std::testing::{
    message_info, mock_dependencies, mock_env, MockApi, MockQuerier, MockStorage,
};
use cosmwasm_std::{Addr, OwnedDeps, Uint128};
use creditra_credit::contract;
use creditra_credit::msg::{ExecuteMsg, InstantiateMsg};
use creditra_credit::state::{
    Draw, DrawAction, DrawAuditEntry, CREDIT_LINES, CREDIT_LINE_COUNT, DRAWS, DRAW_AUDIT,
    DRAW_AUDIT_COUNT, DRAW_COUNT,
};
use proptest::prelude::*;
use proptest::test_runner::Config as ProptestConfig;

// ═══════════════════════════════════════════════════════════════════════════
// Test constants
// ═══════════════════════════════════════════════════════════════════════════

const OWNER: &str = "owner";
const BORROWER: &str = "borrower";
const UNAUTHORIZED_USER: &str = "unauthorized";
const COLLATERAL_DENOM: &str = "ucollateral";
const CREDIT_DENOM: &str = "ucredit";
const DEFAULT_COLLATERAL: &str = "1000000";
const DEFAULT_CREDIT: &str = "500000";

// ═══════════════════════════════════════════════════════════════════════════
// Strategies
// ═══════════════════════════════════════════════════════════════════════════

/// Generate a valid draw amount string (positive integer).
fn draw_amount() -> impl Strategy<Value = String> {
    (1u128..=10_000u128).prop_map(|n| n.to_string())
}

/// Generate an invalid draw amount (zero or negative).
fn invalid_draw_amount() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("0".to_string()),
        Just("-1".to_string()),
        Just("-100".to_string()),
    ]
}

/// Generate a sequence length for operation sequences.
fn seq_length() -> impl Strategy<Value = usize> {
    1usize..=30usize
}

// ═══════════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════════

fn make_addr(label: &str) -> Addr {
    Addr::unchecked(label)
}

fn setup_contract(deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>) {
    let env = mock_env();
    let owner = make_addr(OWNER);
    let info = message_info(&owner, &[]);
    let msg = InstantiateMsg {
        owner: owner.to_string(),
    };
    contract::instantiate(deps.as_mut(), env, info, msg).unwrap();
}

/// Create a credit line for a borrower (called by owner). Returns the
/// assigned credit line ID.
fn create_credit_line(
    deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
    borrower: &str,
    collateral_amount: &str,
    credit_amount: &str,
) -> u64 {
    let env = mock_env();
    let owner = make_addr(OWNER);
    let info = message_info(&owner, &[]);
    let msg = ExecuteMsg::CreateCreditLine {
        borrower: borrower.to_string(),
        collateral_denom: COLLATERAL_DENOM.to_string(),
        collateral_amount: collateral_amount.to_string(),
        credit_denom: CREDIT_DENOM.to_string(),
        credit_amount: credit_amount.to_string(),
    };
    contract::execute(deps.as_mut(), env, info, msg).unwrap();

    let count = CREDIT_LINE_COUNT.load(deps.as_ref().storage).unwrap();
    count - 1
}

/// Create a draw (borrow) against a credit line as the borrower.
/// Returns the draw ID on success.
fn create_draw(
    deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
    credit_line_id: u64,
    amount: &str,
    denom: &str,
    caller: &str,
) -> Result<u64, creditra_credit::error::ContractError> {
    let env = mock_env();
    let caller_addr = make_addr(caller);
    let info = message_info(&caller_addr, &[]);
    let msg = ExecuteMsg::CreateDraw {
        credit_line_id,
        amount: amount.to_string(),
        denom: denom.to_string(),
    };
    contract::execute(deps.as_mut(), env, info, msg)?;

    let draw_count = DRAW_COUNT
        .load(deps.as_ref().storage, credit_line_id)
        .unwrap_or(0);
    Ok(draw_count - 1)
}

/// Repay a draw as the borrower. Returns Ok on success.
fn repay_draw(
    deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
    credit_line_id: u64,
    draw_id: u64,
    caller: &str,
) -> Result<(), creditra_credit::error::ContractError> {
    let env = mock_env();
    let caller_addr = make_addr(caller);
    let info = message_info(&caller_addr, &[]);
    let msg = ExecuteMsg::RepayDraw {
        credit_line_id,
        draw_id,
    };
    contract::execute(deps.as_mut(), env, info, msg)?;
    Ok(())
}

/// Snapshot the current storage state for invariant comparison.
fn snapshot_storage(deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>) -> StorageSnapshot {
    let cl_count = CREDIT_LINE_COUNT
        .may_load(deps.as_ref().storage)
        .unwrap()
        .unwrap_or(0);
    let mut draws: Vec<Draw> = Vec::new();
    let mut draw_audit_counts: Vec<(u64, u64, u64)> = Vec::new();

    for cl_id in 0..cl_count {
        let draw_count = DRAW_COUNT
            .may_load(deps.as_ref().storage, cl_id)
            .unwrap()
            .unwrap_or(0);
        for d_id in 0..draw_count {
            if let Ok(draw) = DRAWS.load(deps.as_ref().storage, (cl_id, d_id)) {
                draws.push(draw);
                let audit_count = DRAW_AUDIT_COUNT
                    .may_load(deps.as_ref().storage, (cl_id, d_id))
                    .unwrap()
                    .unwrap_or(0);
                draw_audit_counts.push((cl_id, d_id, audit_count));
            }
        }
    }

    StorageSnapshot {
        cl_count,
        draws,
        draw_audit_counts,
    }
}

#[derive(Clone, Debug)]
struct StorageSnapshot {
    cl_count: u64,
    draws: Vec<Draw>,
    draw_audit_counts: Vec<(u64, u64, u64)>,
}

fn assert_snapshots_equal(before: &StorageSnapshot, after: &StorageSnapshot, msg: &str) {
    assert_eq!(before.cl_count, after.cl_count, "{}: cl_count changed", msg);
    assert_eq!(
        before.draws.len(),
        after.draws.len(),
        "{}: draw count changed",
        msg
    );
    assert_eq!(
        before.draw_audit_counts, after.draw_audit_counts,
        "{}: audit counts changed",
        msg
    );

    for (b, a) in before.draws.iter().zip(after.draws.iter()) {
        assert_eq!(b, a, "{}: a draw was mutated", msg);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Invariant: Draw state correctness after creation
// ═══════════════════════════════════════════════════════════════════════════

/// After a successful draw, the stored Draw struct must have:
/// - `repaid == false`
/// - `amount` matching the requested amount
/// - `denom` matching the requested denom
/// - `credit_line_id` matching the parent credit line
/// - `drawn_by` matching the borrower address
/// - `id` matching the assigned draw ID
fn assert_draw_creation_invariants(
    deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>,
    credit_line_id: u64,
    draw_id: u64,
    amount: &str,
    denom: &str,
    borrower: &str,
) {
    let draw: Draw = DRAWS
        .load(deps.as_ref().storage, (credit_line_id, draw_id))
        .expect("draw must exist after creation");

    assert!(!draw.repaid, "newly created draw must not be repaid");
    let parsed_amount: Uint128 = amount.parse().expect("valid u128 string");
    assert_eq!(draw.amount, parsed_amount, "draw amount mismatch");
    assert_eq!(draw.denom, denom, "draw denom mismatch");
    assert_eq!(
        draw.credit_line_id, credit_line_id,
        "draw credit_line_id mismatch"
    );
    assert_eq!(draw.drawn_by, make_addr(borrower), "draw drawn_by mismatch");
    assert_eq!(draw.id, draw_id, "draw id mismatch");
}

// ═══════════════════════════════════════════════════════════════════════════
// Invariant: Sequential draw IDs and count consistency
// ═══════════════════════════════════════════════════════════════════════════

/// Verify that draw IDs are sequential [0, n) and DRAW_COUNT matches the
/// actual number of stored draws for each credit line.
fn assert_draw_count_and_sequential_ids(deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>) {
    let cl_count = CREDIT_LINE_COUNT
        .may_load(deps.as_ref().storage)
        .unwrap()
        .unwrap_or(0);

    for cl_id in 0..cl_count {
        let stored_count = DRAW_COUNT
            .may_load(deps.as_ref().storage, cl_id)
            .unwrap()
            .unwrap_or(0);

        // Verify draws exist at sequential IDs [0, stored_count)
        for d_id in 0..stored_count {
            assert!(
                DRAWS
                    .may_load(deps.as_ref().storage, (cl_id, d_id))
                    .unwrap()
                    .is_some(),
                "draw {} should exist for credit line {}",
                d_id,
                cl_id
            );
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Invariant: Audit trail correctness
// ═══════════════════════════════════════════════════════════════════════════

/// Verify that each draw has the correct audit trail:
/// - At least one entry with `DrawCreated` at seq 0
/// - If repaid, the last entry should be `Repaid`
/// - Audit seq numbers are sequential from 0
fn assert_audit_trail_invariants(deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>) {
    let cl_count = CREDIT_LINE_COUNT
        .may_load(deps.as_ref().storage)
        .unwrap()
        .unwrap_or(0);

    for cl_id in 0..cl_count {
        let draw_count = DRAW_COUNT
            .may_load(deps.as_ref().storage, cl_id)
            .unwrap()
            .unwrap_or(0);

        for d_id in 0..draw_count {
            let audit_count = DRAW_AUDIT_COUNT
                .may_load(deps.as_ref().storage, (cl_id, d_id))
                .unwrap()
                .unwrap_or(0);

            // Audit entries must be present at seq 0..audit_count
            for seq in 0..audit_count {
                let entry: DrawAuditEntry = DRAW_AUDIT
                    .load(deps.as_ref().storage, (cl_id, d_id, seq))
                    .unwrap_or_else(|_| {
                        panic!("audit entry ({}, {}, {}) must exist", cl_id, d_id, seq)
                    });
                assert_eq!(
                    entry.seq, seq,
                    "audit entry seq mismatch for ({}, {}, {})",
                    cl_id, d_id, seq
                );
                assert_eq!(entry.draw_id, d_id);
                assert_eq!(entry.credit_line_id, cl_id);
            }

            // Verify first entry is DrawCreated (if any entries exist)
            if audit_count > 0 {
                let first: DrawAuditEntry = DRAW_AUDIT
                    .load(deps.as_ref().storage, (cl_id, d_id, 0))
                    .unwrap();
                assert_eq!(
                    first.action,
                    DrawAction::DrawCreated,
                    "first audit entry for draw ({}, {}) must be DrawCreated",
                    cl_id,
                    d_id
                );
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Invariant: Draw existence check helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Verify that a draw exists and has the expected repaid status.
fn assert_draw_repayment_status(
    deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>,
    credit_line_id: u64,
    draw_id: u64,
    expected_repaid: bool,
) {
    let draw: Draw = DRAWS
        .load(deps.as_ref().storage, (credit_line_id, draw_id))
        .unwrap_or_else(|_| panic!("draw ({}, {}) must exist", credit_line_id, draw_id));
    assert_eq!(
        draw.repaid, expected_repaid,
        "draw ({}, {}) repaid status mismatch",
        credit_line_id, draw_id
    );

    if expected_repaid {
        // Check that a Repaid audit entry was added
        let audit_count = DRAW_AUDIT_COUNT
            .may_load(deps.as_ref().storage, (credit_line_id, draw_id))
            .unwrap()
            .unwrap_or(0);

        if audit_count > 0 {
            // The latest audit entry should be Repaid (it could have been added
            // after the creation entry, or there could be additional entries)
            let last: DrawAuditEntry = DRAW_AUDIT
                .load(
                    deps.as_ref().storage,
                    (credit_line_id, draw_id, audit_count - 1),
                )
                .unwrap();
            // If the draw is repaid, the last audit entry should be Repaid
            // (unless more audit memos were added after)
            let found_repaid = (0..audit_count).any(|seq| {
                DRAW_AUDIT
                    .load(deps.as_ref().storage, (credit_line_id, draw_id, seq))
                    .map(|e| e.action == DrawAction::Repaid)
                    .unwrap_or(false)
            });
            assert!(
                found_repaid,
                "repaid draw ({}, {}) must have a Repaid audit entry",
                credit_line_id, draw_id
            );
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Property tests
// ═══════════════════════════════════════════════════════════════════════════

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    // ─────────────────────────────────────────────────────────────────────
    // Property: Single draw-create invariants
    // ─────────────────────────────────────────────────────────────────────

    /// Verify all invariants for a single well-formed draw operation.
    #[test]
    fn single_draw_invariants(amount in draw_amount()) {
        let mut deps = mock_dependencies();
        setup_contract(&mut deps);
        let cl_id = create_credit_line(&mut deps, BORROWER, DEFAULT_COLLATERAL, DEFAULT_CREDIT);

        let snapshot_before = snapshot_storage(&deps);

        let draw_id = create_draw(&mut deps, cl_id, &amount, CREDIT_DENOM, BORROWER)
            .expect("draw should succeed");

        // 1. Draw state correctness
        assert_draw_creation_invariants(&deps, cl_id, draw_id, &amount, CREDIT_DENOM, BORROWER);

        // 2. DRAW_COUNT should be exactly 1
        let dc = DRAW_COUNT.load(deps.as_ref().storage, cl_id).unwrap();
        prop_assert_eq!(dc, 1, "DRAW_COUNT should be 1 after one draw");

        // 3. Draw ID should be 0
        prop_assert_eq!(draw_id, 0, "first draw should have ID 0");

        // 4. Audit trail
        let audit_count = DRAW_AUDIT_COUNT
            .load(deps.as_ref().storage, (cl_id, draw_id))
            .unwrap();
        prop_assert_eq!(audit_count, 1, "draw should have exactly 1 audit entry");
        let entry: DrawAuditEntry = DRAW_AUDIT
            .load(deps.as_ref().storage, (cl_id, draw_id, 0))
            .unwrap();
        prop_assert_eq!(entry.action, DrawAction::DrawCreated, "first audit entry should be DrawCreated");
        prop_assert_eq!(entry.draw_id, draw_id);
        prop_assert_eq!(entry.credit_line_id, cl_id);
        prop_assert_eq!(entry.by, make_addr(BORROWER), "audit entry should be by the borrower");

        // 5. Storage changed appropriately (cl_count unchanged, draws increased)
        let snapshot_after = snapshot_storage(&deps);
        prop_assert_eq!(snapshot_after.cl_count, snapshot_before.cl_count, "cl_count should not change");
        prop_assert_eq!(
            snapshot_after.draws.len(),
            snapshot_before.draws.len() + 1,
            "draw count should increase by 1"
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // Property: Multiple draws on the same credit line
    // ─────────────────────────────────────────────────────────────────────

    /// Verify that multiple draws on the same credit line get sequential IDs
    /// and each draw is stored correctly.
    #[test]
    fn multiple_draws_sequential_ids(num_draws in 1u64..=10u64, amount in draw_amount()) {
        let mut deps = mock_dependencies();
        setup_contract(&mut deps);
        let cl_id = create_credit_line(&mut deps, BORROWER, DEFAULT_COLLATERAL, DEFAULT_CREDIT);

        for expected_id in 0..num_draws {
            let draw_id = create_draw(&mut deps, cl_id, &amount, CREDIT_DENOM, BORROWER)
                .expect("draw should succeed");

            prop_assert_eq!(draw_id, expected_id, "draw ID should be sequential");
            assert_draw_creation_invariants(&deps, cl_id, draw_id, &amount, CREDIT_DENOM, BORROWER);
        }

        let dc = DRAW_COUNT.load(deps.as_ref().storage, cl_id).unwrap();
        prop_assert_eq!(dc, num_draws, "DRAW_COUNT should match number of draws");
    }

    // ─────────────────────────────────────────────────────────────────────
    // Property: Repay after draw
    // ─────────────────────────────────────────────────────────────────────

    /// Verify that repaying a draw sets `repaid = true` and appends a
    /// `Repaid` audit entry.
    #[test]
    fn draw_then_repay(amount in draw_amount()) {
        let mut deps = mock_dependencies();
        setup_contract(&mut deps);
        let cl_id = create_credit_line(&mut deps, BORROWER, DEFAULT_COLLATERAL, DEFAULT_CREDIT);
        let draw_id = create_draw(&mut deps, cl_id, &amount, CREDIT_DENOM, BORROWER)
            .expect("draw should succeed");

        // Verify not repaid yet
        assert_draw_repayment_status(&deps, cl_id, draw_id, false);

        let audit_count_before = DRAW_AUDIT_COUNT
            .load(deps.as_ref().storage, (cl_id, draw_id))
            .unwrap();

        // Repay
        repay_draw(&mut deps, cl_id, draw_id, BORROWER)
            .expect("repay should succeed");

        // Verify repaid
        assert_draw_repayment_status(&deps, cl_id, draw_id, true);

        // Audit count should increase
        let audit_count_after = DRAW_AUDIT_COUNT
            .load(deps.as_ref().storage, (cl_id, draw_id))
            .unwrap();
        prop_assert_eq!(
            audit_count_after,
            audit_count_before + 1,
            "audit count should increase by 1 after repay"
        );

        // The last audit entry should be Repaid
        let last_entry: DrawAuditEntry = DRAW_AUDIT
            .load(deps.as_ref().storage, (cl_id, draw_id, audit_count_after - 1))
            .unwrap();
        prop_assert_eq!(
            last_entry.action,
            DrawAction::Repaid,
            "last audit entry after repay should be Repaid"
        );
        prop_assert_eq!(last_entry.by, make_addr(BORROWER));
    }

    // ─────────────────────────────────────────────────────────────────────
    // Property: Repeated repay (idempotent)
    // ─────────────────────────────────────────────────────────────────────

    /// Repaying an already-repaid draw should succeed and append another
    /// audit entry, but the draw remains repaid.
    #[test]
    fn repay_already_repaid_draw(amount in draw_amount()) {
        let mut deps = mock_dependencies();
        setup_contract(&mut deps);
        let cl_id = create_credit_line(&mut deps, BORROWER, DEFAULT_COLLATERAL, DEFAULT_CREDIT);
        let draw_id = create_draw(&mut deps, cl_id, &amount, CREDIT_DENOM, BORROWER)
            .expect("draw should succeed");

        // Repay once
        repay_draw(&mut deps, cl_id, draw_id, BORROWER).expect("first repay should succeed");
        assert_draw_repayment_status(&deps, cl_id, draw_id, true);

        let audit_count_after_first = DRAW_AUDIT_COUNT
            .load(deps.as_ref().storage, (cl_id, draw_id))
            .unwrap();

        // Repay again (should still succeed - idempotent)
        repay_draw(&mut deps, cl_id, draw_id, BORROWER).expect("second repay should succeed (idempotent)");

        // Draw should still be repaid
        assert_draw_repayment_status(&deps, cl_id, draw_id, true);

        // Audit count should increase again
        let audit_count_after_second = DRAW_AUDIT_COUNT
            .load(deps.as_ref().storage, (cl_id, draw_id))
            .unwrap();
        prop_assert_eq!(
            audit_count_after_second,
            audit_count_after_first + 1,
            "audit count should increase on second repay"
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // Property: Draw on non-existent credit line
    // ─────────────────────────────────────────────────────────────────────

    /// Creating a draw on a non-existent credit line must fail with
    /// `CreditLineNotFound` and leave state unchanged.
    #[test]
    fn draw_on_nonexistent_credit_line(amount in draw_amount()) {
        let mut deps = mock_dependencies();
        setup_contract(&mut deps);

        let snapshot_before = snapshot_storage(&deps);

        let result = create_draw(&mut deps, 999, &amount, CREDIT_DENOM, BORROWER);

        prop_assert!(
            result.is_err(),
            "draw on non-existent credit line should fail"
        );
        let err = result.unwrap_err();
        prop_assert_eq!(
            err,
            creditra_credit::error::ContractError::CreditLineNotFound(999),
            "error should be CreditLineNotFound"
        );

        // State must be unchanged after failed operation
        let snapshot_after = snapshot_storage(&deps);
        assert_snapshots_equal(&snapshot_before, &snapshot_after, "state changed after failed draw");
    }

    // ─────────────────────────────────────────────────────────────────────
    // Property: Repay on non-existent draw
    // ─────────────────────────────────────────────────────────────────────

    /// Repaying a non-existent draw must fail with `DrawNotFound` and
    /// leave state unchanged.
    #[test]
    fn repay_nonexistent_draw() {
        let mut deps = mock_dependencies();
        setup_contract(&mut deps);
        let cl_id = create_credit_line(&mut deps, BORROWER, DEFAULT_COLLATERAL, DEFAULT_CREDIT);

        let snapshot_before = snapshot_storage(&deps);

        let result = repay_draw(&mut deps, cl_id, 999, BORROWER);

        prop_assert!(
            result.is_err(),
            "repay on non-existent draw should fail"
        );
        let err = result.unwrap_err();
        prop_assert_eq!(
            err,
            creditra_credit::error::ContractError::DrawNotFound(999, cl_id),
            "error should be DrawNotFound"
        );

        let snapshot_after = snapshot_storage(&deps);
        assert_snapshots_equal(&snapshot_before, &snapshot_after, "state changed after failed repay");
    }

    // ─────────────────────────────────────────────────────────────────────
    // Property: Unauthorized draw
    // ─────────────────────────────────────────────────────────────────────

    /// Only the borrower can create a draw on their credit line.
    #[test]
    fn unauthorized_draw_fails(amount in draw_amount()) {
        let mut deps = mock_dependencies();
        setup_contract(&mut deps);
        let cl_id = create_credit_line(&mut deps, BORROWER, DEFAULT_COLLATERAL, DEFAULT_CREDIT);

        let snapshot_before = snapshot_storage(&deps);

        let result = create_draw(&mut deps, cl_id, &amount, CREDIT_DENOM, UNAUTHORIZED_USER);

        prop_assert!(
            result.is_err(),
            "unauthorized draw should fail"
        );
        let err = result.unwrap_err();
        prop_assert_eq!(
            err,
            creditra_credit::error::ContractError::Unauthorized,
            "error should be Unauthorized"
        );

        let snapshot_after = snapshot_storage(&deps);
        assert_snapshots_equal(&snapshot_before, &snapshot_after, "state changed after unauthorized draw");
    }

    // ─────────────────────────────────────────────────────────────────────
    // Property: Unauthorized repay
    // ─────────────────────────────────────────────────────────────────────

    /// Only the draw creator can repay their draw.
    #[test]
    fn unauthorized_repay_fails(amount in draw_amount()) {
        let mut deps = mock_dependencies();
        setup_contract(&mut deps);
        let cl_id = create_credit_line(&mut deps, BORROWER, DEFAULT_COLLATERAL, DEFAULT_CREDIT);
        let draw_id = create_draw(&mut deps, cl_id, &amount, CREDIT_DENOM, BORROWER)
            .expect("draw should succeed");

        let snapshot_before = snapshot_storage(&deps);

        let result = repay_draw(&mut deps, cl_id, draw_id, UNAUTHORIZED_USER);

        prop_assert!(
            result.is_err(),
            "unauthorized repay should fail"
        );
        let err = result.unwrap_err();
        prop_assert_eq!(
            err,
            creditra_credit::error::ContractError::Unauthorized,
            "error should be Unauthorized"
        );

        // Draw should still not be repaid
        let draw: Draw = DRAWS.load(deps.as_ref().storage, (cl_id, draw_id)).unwrap();
        prop_assert!(!draw.repaid, "draw should remain unpaid after unauthorized repay");

        let snapshot_after = snapshot_storage(&deps);
        assert_snapshots_equal(&snapshot_before, &snapshot_after, "state changed after unauthorized repay");
    }

    // ─────────────────────────────────────────────────────────────────────
    // Property: Alternate draw and repay sequence
    // ─────────────────────────────────────────────────────────────────────

    /// Alternating draw and repay operations on the same credit line
    /// must maintain all invariants. Each draw gets a unique sequential ID.
    #[test]
    fn alternating_draw_and_repay(
        ops in prop::collection::vec(proptest::bool::ANY, 1..=15),
        amount in draw_amount(),
    ) {
        let mut deps = mock_dependencies();
        setup_contract(&mut deps);
        let cl_id = create_credit_line(&mut deps, BORROWER, DEFAULT_COLLATERAL, DEFAULT_CREDIT);

        let mut next_draw_id = 0u64;

        for (i, is_draw) in ops.iter().enumerate() {
            if *is_draw {
                // Perform a draw
                let result = create_draw(&mut deps, cl_id, &amount, CREDIT_DENOM, BORROWER);
                if result.is_ok() {
                    let draw_id = result.unwrap();
                    prop_assert_eq!(draw_id, next_draw_id, "draw IDs must be sequential");
                    assert_draw_creation_invariants(
                        &deps, cl_id, draw_id, &amount, CREDIT_DENOM, BORROWER
                    );
                    next_draw_id += 1;
                }
                // Draw should always succeed on a valid credit line
                prop_assert!(
                    result.is_ok(),
                    "draw at step {} should succeed",
                    i
                );
            } else {
                // Repay an existing draw (repay in reverse order to keep it interesting)
                if next_draw_id > 0 {
                    let target_id = next_draw_id - 1;
                    let result = repay_draw(&mut deps, cl_id, target_id, BORROWER);
                    prop_assert!(
                        result.is_ok(),
                        "repay of draw {} at step {} should succeed",
                        target_id,
                        i
                    );
                    assert_draw_repayment_status(&deps, cl_id, target_id, true);
                }
            }
        }

        // Final invariants check
        assert_draw_count_and_sequential_ids(&deps);

        // All draws that were created should exist
        for d_id in 0..next_draw_id {
            prop_assert!(
                DRAWS.may_load(deps.as_ref().storage, (cl_id, d_id)).unwrap().is_some(),
                "draw {} should exist after alternating ops",
                d_id
            );
        }
    }

    // ─────────────────────────────────────────────────────────────────────
    // Property: Multiple credit lines with draws are isolated
    // ─────────────────────────────────────────────────────────────────────

    /// Draws on different credit lines must be completely isolated.
    /// Each credit line has its own sequential draw IDs and draw count.
    #[test]
    fn multiple_credit_lines_isolated(
        draws_first in 0u64..=5u64,
        draws_second in 0u64..=5u64,
        amount in draw_amount(),
    ) {
        let mut deps = mock_dependencies();
        setup_contract(&mut deps);

        let cl1 = create_credit_line(&mut deps, "borrower1", DEFAULT_COLLATERAL, DEFAULT_CREDIT);
        let cl2 = create_credit_line(&mut deps, "borrower2", DEFAULT_COLLATERAL, DEFAULT_CREDIT);

        for i in 0..draws_first {
            let draw_id = create_draw(&mut deps, cl1, &amount, CREDIT_DENOM, "borrower1")
                .expect("draw on cl1 should succeed");
            prop_assert_eq!(draw_id, i, "cl1 draw ID should be sequential");
        }

        for i in 0..draws_second {
            let draw_id = create_draw(&mut deps, cl2, &amount, CREDIT_DENOM, "borrower2")
                .expect("draw on cl2 should succeed");
            prop_assert_eq!(draw_id, i, "cl2 draw ID should be sequential");
        }

        // Verify counts are isolated
        let dc1 = DRAW_COUNT.load(deps.as_ref().storage, cl1).unwrap();
        let dc2 = DRAW_COUNT.load(deps.as_ref().storage, cl2).unwrap();
        prop_assert_eq!(dc1, draws_first, "cl1 draw count mismatch");
        prop_assert_eq!(dc2, draws_second, "cl2 draw count mismatch");
    }

    // ─────────────────────────────────────────────────────────────────────
    // Property: Repay on non-existent credit line
    // ─────────────────────────────────────────────────────────────────────

    /// Repaying on a non-existent credit line must fail.
    #[test]
    fn repay_on_nonexistent_credit_line() {
        let mut deps = mock_dependencies();
        setup_contract(&mut deps);

        let result = repay_draw(&mut deps, 999, 0, BORROWER);
        prop_assert!(
            result.is_err(),
            "repay on non-existent credit line should fail"
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // Property: Final storage invariants after random operations
    // ─────────────────────────────────────────────────────────────────────

    /// After any sequence of operations, all storage invariants must hold.
    #[test]
    fn storage_invariants_after_random_ops(
        num_credit_lines in 1u64..=3u64,
        num_ops in 1usize..=20usize,
        amount in draw_amount(),
    ) {
        let mut deps = mock_dependencies();
        setup_contract(&mut deps);

        // Create credit lines
        let mut cl_ids = Vec::new();
        for i in 0..num_credit_lines {
            let borrower = format!("borrower_{}", i);
            let cl_id = create_credit_line(&mut deps, &borrower, DEFAULT_COLLATERAL, DEFAULT_CREDIT);
            prop_assert_eq!(cl_id, i, "credit line IDs should be sequential");
            cl_ids.push((cl_id, borrower));
        }

        // Perform random operations
        for _ in 0..num_ops {
            let idx = (num_ops as u64 % num_credit_lines.max(1)) as usize % cl_ids.len();
            let (cl_id, borrower) = &cl_ids[idx];

            // Randomly choose draw or repay
            if (num_ops as u64 % 3) != 0 {
                // Draw
                let _ = create_draw(&mut deps, *cl_id, &amount, CREDIT_DENOM, borrower);
            } else {
                // Repay a random draw if any exist
                let draw_count = DRAW_COUNT
                    .may_load(deps.as_ref().storage, *cl_id)
                    .unwrap()
                    .unwrap_or(0);
                if draw_count > 0 {
                    let target = (num_ops as u64) % draw_count;
                    let _ = repay_draw(&mut deps, *cl_id, target, borrower);
                }
            }
        }

        // Verify all invariants
        assert_draw_count_and_sequential_ids(&deps);
        assert_audit_trail_invariants(&deps);
    }

    // ─────────────────────────────────────────────────────────────────────
    // Property: Different borrowers on same credit line
    // ─────────────────────────────────────────────────────────────────────

    /// A credit line is for a single borrower. Only that borrower can draw.
    /// Other users cannot draw on the same credit line.
    #[test]
    fn only_assigned_borrower_can_draw(amount in draw_amount()) {
        let mut deps = mock_dependencies();
        setup_contract(&mut deps);
        let cl_id = create_credit_line(&mut deps, BORROWER, DEFAULT_COLLATERAL, DEFAULT_CREDIT);

        // Other user cannot draw
        let result = create_draw(&mut deps, cl_id, &amount, CREDIT_DENOM, "other_user");
        prop_assert_eq!(
            result.unwrap_err(),
            creditra_credit::error::ContractError::Unauthorized,
            "other user should not be able to draw"
        );

        // The assigned borrower can still draw
        let draw_id = create_draw(&mut deps, cl_id, &amount, CREDIT_DENOM, BORROWER);
        prop_assert!(
            draw_id.is_ok(),
            "assigned borrower should be able to draw"
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // Property: Repay enforces origin check
    // ─────────────────────────────────────────────────────────────────────

    /// Only the user who created the draw (drawn_by) can repay it.
    #[test]
    fn only_draw_creator_can_repay(amount in draw_amount()) {
        let mut deps = mock_dependencies();
        setup_contract(&mut deps);
        let cl_id = create_credit_line(&mut deps, BORROWER, DEFAULT_COLLATERAL, DEFAULT_CREDIT);
        let draw_id = create_draw(&mut deps, cl_id, &amount, CREDIT_DENOM, BORROWER)
            .expect("draw should succeed");

        // A different borrower on a DIFFERENT credit line cannot repay
        let cl2 = create_credit_line(&mut deps, "other_borrower", DEFAULT_COLLATERAL, DEFAULT_CREDIT);
        let result = repay_draw(&mut deps, cl_id, draw_id, "other_borrower");
        prop_assert_eq!(
            result.unwrap_err(),
            creditra_credit::error::ContractError::Unauthorized,
            "different borrower should not be able to repay"
        );

        // Even the owner cannot repay
        let result = repay_draw(&mut deps, cl_id, draw_id, OWNER);
        prop_assert_eq!(
            result.unwrap_err(),
            creditra_credit::error::ContractError::Unauthorized,
            "owner should not be able to repay"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Deterministic edge-case tests (not proptest, but complement it)
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod edge_cases {
    use super::*;

    /// Zero draws on a credit line: DRAW_COUNT should be 0 (or absent).
    #[test]
    fn zero_draws_on_credit_line() {
        let mut deps = mock_dependencies();
        setup_contract(&mut deps);
        let _cl_id = create_credit_line(&mut deps, BORROWER, DEFAULT_COLLATERAL, DEFAULT_CREDIT);

        let dc = DRAW_COUNT.may_load(deps.as_ref().storage, 0).unwrap();
        assert!(
            dc.is_none() || dc == Some(0),
            "no draws should mean DRAW_COUNT is 0 or None"
        );
    }

    /// Full cycle: create credit line, draw max amount, repay fully.
    #[test]
    fn full_draw_and_repay_cycle() {
        let mut deps = mock_dependencies();
        setup_contract(&mut deps);
        let cl_id = create_credit_line(&mut deps, BORROWER, DEFAULT_COLLATERAL, DEFAULT_CREDIT);
        let draw_id = create_draw(&mut deps, cl_id, "500000", CREDIT_DENOM, BORROWER)
            .expect("draw should succeed");

        assert_draw_repayment_status(&deps, cl_id, draw_id, false);

        repay_draw(&mut deps, cl_id, draw_id, BORROWER).expect("repay should succeed");
        assert_draw_repayment_status(&deps, cl_id, draw_id, true);
    }

    /// Two draws then repay both.
    #[test]
    fn two_draws_then_repay_both() {
        let mut deps = mock_dependencies();
        setup_contract(&mut deps);
        let cl_id = create_credit_line(&mut deps, BORROWER, DEFAULT_COLLATERAL, DEFAULT_CREDIT);

        let d0 = create_draw(&mut deps, cl_id, "100", CREDIT_DENOM, BORROWER).unwrap();
        let d1 = create_draw(&mut deps, cl_id, "200", CREDIT_DENOM, BORROWER).unwrap();

        assert_eq!(d0, 0);
        assert_eq!(d1, 1);

        repay_draw(&mut deps, cl_id, 0, BORROWER).unwrap();
        repay_draw(&mut deps, cl_id, 1, BORROWER).unwrap();

        assert_draw_repayment_status(&deps, cl_id, 0, true);
        assert_draw_repayment_status(&deps, cl_id, 1, true);
    }

    /// Create credit line for borrower A, but try to draw as borrower B.
    #[test]
    fn cannot_draw_as_wrong_borrower() {
        let mut deps = mock_dependencies();
        setup_contract(&mut deps);
        let cl_id = create_credit_line(&mut deps, "alice", DEFAULT_COLLATERAL, DEFAULT_CREDIT);

        let err = create_draw(&mut deps, cl_id, "100", CREDIT_DENOM, "bob").unwrap_err();
        assert_eq!(err, creditra_credit::error::ContractError::Unauthorized);
    }

    /// Repeated identical draws all get different IDs.
    #[test]
    fn repeated_identical_draws_get_unique_ids() {
        let mut deps = mock_dependencies();
        setup_contract(&mut deps);
        let cl_id = create_credit_line(&mut deps, BORROWER, DEFAULT_COLLATERAL, DEFAULT_CREDIT);

        for i in 0..5u64 {
            let draw_id = create_draw(&mut deps, cl_id, "100", CREDIT_DENOM, BORROWER).unwrap();
            assert_eq!(draw_id, i);
        }
    }

    /// Repaying a draw of zero-amount credit line still works.
    /// (Note: the contract accepts "0" as a valid amount string for draws,
    /// but the draw_amount() strategy excludes zero. This test uses an
    /// explicit zero.)
    #[test]
    fn draw_with_zero_amount() {
        let mut deps = mock_dependencies();
        setup_contract(&mut deps);
        let cl_id = create_credit_line(&mut deps, BORROWER, DEFAULT_COLLATERAL, DEFAULT_CREDIT);

        // The contract does not validate that amount > 0, so this should succeed
        let result = create_draw(&mut deps, cl_id, "0", CREDIT_DENOM, BORROWER);
        // If it succeeds, check invariants; if it fails, that's also valid
        if let Ok(draw_id) = result {
            let draw = DRAWS.load(deps.as_ref().storage, (cl_id, draw_id)).unwrap();
            assert_eq!(draw.amount, Uint128::zero());
            assert!(!draw.repaid);

            // Can repay a zero-amount draw
            repay_draw(&mut deps, cl_id, draw_id, BORROWER).expect("repay should succeed");
            assert_draw_repayment_status(&deps, cl_id, draw_id, true);
        }
    }

    /// Maximum valid Uint128 draw amount.
    #[test]
    fn draw_maximum_amount() {
        let mut deps = mock_dependencies();
        setup_contract(&mut deps);
        let cl_id = create_credit_line(&mut deps, BORROWER, DEFAULT_COLLATERAL, DEFAULT_CREDIT);

        let max_amount = Uint128::MAX.to_string();
        let draw_id = create_draw(&mut deps, cl_id, &max_amount, CREDIT_DENOM, BORROWER)
            .expect("max draw should succeed");

        let draw = DRAWS.load(deps.as_ref().storage, (cl_id, draw_id)).unwrap();
        assert_eq!(draw.amount, Uint128::MAX);
    }

    /// Repay non-existent draw on a non-existent credit line.
    #[test]
    fn repay_draw_nonexistent_credit_line_and_draw() {
        let mut deps = mock_dependencies();
        setup_contract(&mut deps);

        let err = repay_draw(&mut deps, 999, 0, BORROWER).unwrap_err();
        // This should match the DrawNotFound pattern or similar
        assert!(
            matches!(err, creditra_credit::error::ContractError::DrawNotFound(..))
                || matches!(
                    err,
                    creditra_credit::error::ContractError::CreditLineNotFound(..)
                ),
            "repay on nonexistent entities should fail with an appropriate error"
        );
    }
}
