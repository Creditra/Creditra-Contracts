// SPDX-License-Identifier: MIT

//! # Replay-safe identifiers for draw and repayment records (Issue #1153)
//!
//! These tests drive the deployed contract through `CreditClient`, so every
//! invariant is proven at the entrypoint boundary a real caller uses.
//!
//! ## Coverage map
//!
//! | Acceptance criterion | Tests |
//! |---|---|
//! | Deterministic for valid input | `draw_records_*`, `repay_records_*` |
//! | Deterministic for duplicate input | `replayed_*_op_id_is_rejected` |
//! | Deterministic for invalid input | `rejected_*_consumes_nothing` |
//! | Boundary cases | `sequences_are_independent_*`, `same_bytes_usable_once_per_kind` |
//! | State-transition invariants preserved | `rejected_*_consumes_nothing`, `replay_does_not_move_funds` |
//! | Retries cannot double-apply | `replayed_draw_op_id_is_rejected`, `replayed_repay_op_id_is_rejected` |
//! | Existing callers compatible | `legacy_entrypoints_still_work_and_record` |
//! | Failures diagnosable | `replayed_*` assert the specific error code |

#![cfg(test)]

use creditra_credit::operation_id::{OperationIdSource, OperationKind};
use creditra_credit::types::{ContractError, ContractErrorCategory};
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token, Address, BytesN, Env,
};

const CREDIT_LIMIT: i128 = 100_000;
const COLLATERAL: i128 = 50_000;

struct Ctx<'a> {
    client: CreditClient<'a>,
    borrower: Address,
    token: Address,
}

fn setup(env: &Env) -> Ctx<'_> {
    // The draw path transfers from the liquidity reserve inside the contract
    // invocation, which is a non-root authorization. Matches the setup used by
    // circuit_breaker.rs and conservation_cross_contract.rs.
    env.mock_all_auths_allowing_non_root_auth();
    env.ledger().with_mut(|li| li.timestamp = 1_000);

    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&token);

    let asset = token::StellarAssetClient::new(env, &token);
    asset.mint(&borrower, &200_000);
    asset.mint(&token, &500_000);

    client.open_credit_line(&borrower, &CREDIT_LIMIT, &500, &10);
    client.deposit_collateral(&borrower, &COLLATERAL);

    token::Client::new(env, &token).approve(&borrower, &contract_id, &200_000, &6_000_000_u32);

    Ctx {
        client,
        borrower,
        token,
    }
}

fn op_id(env: &Env, byte: u8) -> BytesN<32> {
    BytesN::from_array(env, &[byte; 32])
}

// ─── Legacy entrypoints stay compatible and now record ──────────────────────

/// Existing callers keep their signatures and behaviour, and the movement is
/// now recorded under a derived id.
#[test]
fn legacy_entrypoints_still_work_and_record() {
    let env = Env::default();
    let ctx = setup(&env);

    ctx.client.draw_credit(&ctx.borrower, &1_000);

    let credit = ctx.client.get_credit_line(&ctx.borrower).unwrap();
    assert_eq!(credit.utilized_amount, 1_000);

    // Sequence advanced, so the derived id for sequence 0 is now resolvable.
    assert_eq!(
        ctx.client
            .get_operation_sequence(&OperationKind::Draw, &ctx.borrower),
        1
    );
    let derived = ctx
        .client
        .derive_operation_id(&OperationKind::Draw, &ctx.borrower, &0);
    let record = ctx.client.get_draw_record(&derived).expect("draw recorded");
    assert_eq!(record.amount, 1_000);
    assert_eq!(record.utilized_after, 1_000);
    assert_eq!(record.sequence, 0);
    assert_eq!(record.borrower, ctx.borrower);
    assert_eq!(record.id_source, OperationIdSource::Derived);
}

/// The derivation exposed to callers matches the id the contract actually
/// recorded, so an off-chain reconciler can recompute historical ids.
#[test]
fn derived_ids_are_reproducible_and_unique_per_sequence() {
    let env = Env::default();
    let ctx = setup(&env);

    ctx.client.draw_credit(&ctx.borrower, &1_000);
    ctx.client.draw_credit(&ctx.borrower, &2_000);

    let first = ctx
        .client
        .derive_operation_id(&OperationKind::Draw, &ctx.borrower, &0);
    let second = ctx
        .client
        .derive_operation_id(&OperationKind::Draw, &ctx.borrower, &1);
    assert_ne!(first, second, "each sequence must derive a distinct id");

    assert_eq!(ctx.client.get_draw_record(&first).unwrap().amount, 1_000);
    assert_eq!(ctx.client.get_draw_record(&second).unwrap().amount, 2_000);

    // Deterministic: recomputing yields the same bytes.
    assert_eq!(
        first,
        ctx.client
            .derive_operation_id(&OperationKind::Draw, &ctx.borrower, &0)
    );
}

// ─── Caller-supplied ids: the replay barrier ────────────────────────────────

#[test]
fn draw_with_op_id_records_caller_supplied_source() {
    let env = Env::default();
    let ctx = setup(&env);
    let id = op_id(&env, 7);

    assert!(!ctx
        .client
        .is_operation_id_consumed(&OperationKind::Draw, &id));

    ctx.client
        .draw_credit_with_op_id(&ctx.borrower, &1_500, &id);

    assert!(ctx
        .client
        .is_operation_id_consumed(&OperationKind::Draw, &id));
    let record = ctx.client.get_draw_record(&id).expect("draw recorded");
    assert_eq!(record.id, id);
    assert_eq!(record.amount, 1_500);
    assert_eq!(record.id_source, OperationIdSource::CallerSupplied);
}

/// The core acceptance criterion: a retried draw carrying the same id is
/// rejected rather than drawing twice.
#[test]
fn replayed_draw_op_id_is_rejected() {
    let env = Env::default();
    let ctx = setup(&env);
    let id = op_id(&env, 7);

    ctx.client
        .draw_credit_with_op_id(&ctx.borrower, &1_500, &id);
    let after_first = ctx
        .client
        .get_credit_line(&ctx.borrower)
        .unwrap()
        .utilized_amount;

    let err = ctx
        .client
        .try_draw_credit_with_op_id(&ctx.borrower, &1_500, &id)
        .expect_err("replayed op id must be rejected");
    assert_eq!(err.unwrap(), ContractError::DuplicateOperationId.into());

    // No second draw: utilization is unchanged and the record still describes
    // the original operation.
    assert_eq!(
        ctx.client
            .get_credit_line(&ctx.borrower)
            .unwrap()
            .utilized_amount,
        after_first
    );
    assert_eq!(ctx.client.get_draw_record(&id).unwrap().sequence, 0);
}

/// A replay must not move funds — the borrower's balance is the ground truth
/// that no second transfer occurred.
#[test]
fn replay_does_not_move_funds() {
    let env = Env::default();
    let ctx = setup(&env);
    let id = op_id(&env, 9);

    ctx.client
        .draw_credit_with_op_id(&ctx.borrower, &2_500, &id);
    let balance_after_first = token::Client::new(&env, &ctx.token).balance(&ctx.borrower);

    let _ = ctx
        .client
        .try_draw_credit_with_op_id(&ctx.borrower, &2_500, &id);

    assert_eq!(
        token::Client::new(&env, &ctx.token).balance(&ctx.borrower),
        balance_after_first,
        "a replayed draw must not transfer a second time"
    );
}

#[test]
fn replayed_repay_op_id_is_rejected() {
    let env = Env::default();
    let ctx = setup(&env);
    ctx.client.draw_credit(&ctx.borrower, &5_000);

    let id = op_id(&env, 11);
    ctx.client
        .repay_credit_with_op_id(&ctx.borrower, &1_000, &id);
    let after_first = ctx
        .client
        .get_credit_line(&ctx.borrower)
        .unwrap()
        .utilized_amount;

    let err = ctx
        .client
        .try_repay_credit_with_op_id(&ctx.borrower, &1_000, &id)
        .expect_err("replayed repayment must be rejected");
    assert_eq!(err.unwrap(), ContractError::DuplicateOperationId.into());

    // The borrower is not debited twice.
    assert_eq!(
        ctx.client
            .get_credit_line(&ctx.borrower)
            .unwrap()
            .utilized_amount,
        after_first
    );
}

/// The repayment record captures the interest/principal split, which cannot be
/// reconstructed from aggregates once further interest accrues.
#[test]
fn repayment_record_captures_allocation_split() {
    let env = Env::default();
    let ctx = setup(&env);
    ctx.client.draw_credit(&ctx.borrower, &5_000);

    let id = op_id(&env, 13);
    ctx.client
        .repay_credit_with_op_id(&ctx.borrower, &1_200, &id);

    let record = ctx
        .client
        .get_repayment_record(&id)
        .expect("repayment recorded");
    assert_eq!(record.amount_submitted, 1_200);
    assert_eq!(
        record.interest_repaid + record.principal_repaid,
        record.amount_applied,
        "the split must account for exactly the applied amount"
    );
    assert_eq!(
        record.utilized_after,
        ctx.client
            .get_credit_line(&ctx.borrower)
            .unwrap()
            .utilized_amount
    );
    assert_eq!(record.id_source, OperationIdSource::CallerSupplied);
}

// ─── Boundary: namespacing and sequence independence ────────────────────────

/// Invariant OP-2: the same 32 bytes are usable once per kind. A repayment id
/// must not block a legitimate draw.
#[test]
fn same_bytes_usable_once_per_kind() {
    let env = Env::default();
    let ctx = setup(&env);
    ctx.client.draw_credit(&ctx.borrower, &5_000);

    let shared = op_id(&env, 42);
    ctx.client
        .draw_credit_with_op_id(&ctx.borrower, &1_000, &shared);

    // Same bytes, different kind — must be accepted.
    ctx.client
        .repay_credit_with_op_id(&ctx.borrower, &500, &shared);

    assert!(ctx
        .client
        .is_operation_id_consumed(&OperationKind::Draw, &shared));
    assert!(ctx
        .client
        .is_operation_id_consumed(&OperationKind::Repayment, &shared));
    assert!(ctx.client.get_draw_record(&shared).is_some());
    assert!(ctx.client.get_repayment_record(&shared).is_some());
}

/// Invariant OP-3: sequences are per borrower and per kind.
#[test]
fn sequences_are_independent_per_borrower_and_kind() {
    let env = Env::default();
    let ctx = setup(&env);

    assert_eq!(
        ctx.client
            .get_operation_sequence(&OperationKind::Draw, &ctx.borrower),
        0
    );
    assert_eq!(
        ctx.client
            .get_operation_sequence(&OperationKind::Repayment, &ctx.borrower),
        0
    );

    ctx.client.draw_credit(&ctx.borrower, &5_000);
    ctx.client.draw_credit(&ctx.borrower, &1_000);
    ctx.client.repay_credit(&ctx.borrower, &500);

    assert_eq!(
        ctx.client
            .get_operation_sequence(&OperationKind::Draw, &ctx.borrower),
        2
    );
    assert_eq!(
        ctx.client
            .get_operation_sequence(&OperationKind::Repayment, &ctx.borrower),
        1,
        "repayment sequence must not be advanced by draws"
    );

    // A second borrower starts from zero.
    let other = Address::generate(&env);
    assert_eq!(
        ctx.client
            .get_operation_sequence(&OperationKind::Draw, &other),
        0
    );
}

// ─── Rejected operations consume nothing (invariant OP-5) ───────────────────

/// A draw rejected on amount validation must not consume a sequence or an id;
/// the whole transaction reverts.
#[test]
fn rejected_draw_consumes_nothing() {
    let env = Env::default();
    let ctx = setup(&env);
    let id = op_id(&env, 21);

    let err = ctx
        .client
        .try_draw_credit_with_op_id(&ctx.borrower, &0, &id)
        .expect_err("zero amount must be rejected");
    assert_eq!(err.unwrap(), ContractError::InvalidAmount.into());

    assert_eq!(
        ctx.client
            .get_operation_sequence(&OperationKind::Draw, &ctx.borrower),
        0,
        "a rejected draw must not consume a sequence"
    );
    assert!(
        !ctx.client
            .is_operation_id_consumed(&OperationKind::Draw, &id),
        "a rejected draw must not consume its op id"
    );

    // The id is therefore still usable for a real draw.
    ctx.client
        .draw_credit_with_op_id(&ctx.borrower, &1_000, &id);
    assert_eq!(ctx.client.get_draw_record(&id).unwrap().amount, 1_000);
}

/// An over-limit draw is rejected after the barrier would have been reached,
/// proving the revert unwinds the claim.
#[test]
fn rejected_over_limit_draw_consumes_nothing() {
    let env = Env::default();
    let ctx = setup(&env);
    let id = op_id(&env, 23);

    let err = ctx
        .client
        .try_draw_credit_with_op_id(&ctx.borrower, &(CREDIT_LIMIT + 1), &id)
        .expect_err("over-limit draw must be rejected");
    assert_eq!(err.unwrap(), ContractError::OverLimit.into());

    assert!(!ctx
        .client
        .is_operation_id_consumed(&OperationKind::Draw, &id));
    assert_eq!(
        ctx.client
            .get_operation_sequence(&OperationKind::Draw, &ctx.borrower),
        0
    );
}

#[test]
fn rejected_repayment_consumes_nothing() {
    let env = Env::default();
    let ctx = setup(&env);
    ctx.client.draw_credit(&ctx.borrower, &5_000);
    let id = op_id(&env, 25);

    let err = ctx
        .client
        .try_repay_credit_with_op_id(&ctx.borrower, &0, &id)
        .expect_err("zero repayment must be rejected");
    assert_eq!(err.unwrap(), ContractError::InvalidAmount.into());

    assert!(!ctx
        .client
        .is_operation_id_consumed(&OperationKind::Repayment, &id));
    assert_eq!(
        ctx.client
            .get_operation_sequence(&OperationKind::Repayment, &ctx.borrower),
        0
    );
}

// ─── Observability ──────────────────────────────────────────────────────────

/// An unknown id returns `None` rather than panicking, so a client that timed
/// out can safely ask "did my operation land?".
#[test]
fn unknown_ids_return_none() {
    let env = Env::default();
    let ctx = setup(&env);
    let unknown = op_id(&env, 99);

    assert!(ctx.client.get_draw_record(&unknown).is_none());
    assert!(ctx.client.get_repayment_record(&unknown).is_none());
    assert!(!ctx
        .client
        .is_operation_id_consumed(&OperationKind::Draw, &unknown));
}

/// The rejection is diagnosable: a stable code in the documented category.
#[test]
fn duplicate_error_is_classified_for_diagnostics() {
    assert_eq!(
        ContractError::DuplicateOperationId.category(),
        ContractErrorCategory::Lifecycle
    );
    assert_eq!(ContractError::DuplicateOperationId as u32, 64);
}
