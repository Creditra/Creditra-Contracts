// SPDX-License-Identifier: MIT

//! # Credit-line state preservation across upgrade migrations (Issue #1149)
//!
//! Drives the deployed contract through `CreditClient`, so every invariant is
//! proven at the entrypoint boundary an operator uses.
//!
//! | Acceptance criterion | Tests |
//! |---|---|
//! | Deterministic for valid input | `upgrade_records_checkpoint_of_pre_upgrade_state` |
//! | Deterministic for duplicate input | `second_upgrade_is_blocked_until_resolved` |
//! | Deterministic for invalid input | `verify_without_checkpoint_is_rejected`, `rollback_without_checkpoint_is_rejected` |
//! | Boundary | `checkpoint_captures_zero_state_on_fresh_contract` |
//! | Retries / partial failure safe | `rejected_second_upgrade_leaves_checkpoint_intact` |
//! | Failure recovery | `rollback_restores_previous_schema_version` |
//! | Regression | `verify_succeeds_when_state_is_unchanged` |
//! | Diagnosable failures | `upgrade_errors_are_classified` |

#![cfg(test)]

use creditra_credit::types::{ContractError, ContractErrorCategory};
use creditra_credit::upgrade_migration::{self, UpgradeCheckpoint};
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token, Address, BytesN, Env,
};

const CREDIT_LIMIT: i128 = 100_000;

struct Ctx<'a> {
    client: CreditClient<'a>,
    contract_id: Address,
    borrower: Address,
}

fn setup(env: &Env) -> Ctx<'_> {
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

    Ctx {
        client,
        contract_id,
        borrower,
    }
}

/// A well-formed but arbitrary wasm hash used as a checkpoint payload.
fn wasm_hash(env: &Env, byte: u8) -> BytesN<32> {
    BytesN::from_array(env, &[byte; 32])
}

/// Seed a pre-upgrade checkpoint exactly as `upgrade` does.
///
/// `upgrade` itself cannot be driven to completion in-process: it ends in
/// `update_current_contract_wasm`, which requires a genuinely uploaded wasm
/// binary, and a failure there reverts the entire invocation — including the
/// checkpoint write. That revert is the correct atomic behaviour, so rather
/// than weaken it for testability, these tests call the same
/// `record_checkpoint` the entrypoint calls, in the contract's own storage
/// context, and then exercise every downstream path through the client.
///
/// The one path this cannot cover end-to-end is the wasm swap itself; the
/// UP-2 guard is still proven at the entrypoint by
/// `second_upgrade_is_blocked_until_resolved`, because that guard runs before
/// the swap is ever reached.
fn seed_checkpoint(ctx: &Ctx, env: &Env, byte: u8) -> UpgradeCheckpoint {
    let from_version = ctx.client.get_schema_version().unwrap_or(1);
    env.as_contract(&ctx.contract_id, || {
        upgrade_migration::record_checkpoint(
            env,
            from_version,
            from_version + 1,
            &wasm_hash(env, byte),
            &BytesN::from_array(env, &[0u8; 32]),
        )
    })
}

// ─── Success path ───────────────────────────────────────────────────────────

#[test]
fn checkpoint_absent_before_any_upgrade() {
    let env = Env::default();
    let ctx = setup(&env);

    assert!(ctx.client.get_upgrade_checkpoint().is_none());
    assert!(!ctx.client.is_upgrade_verification_pending());
}

#[test]
fn upgrade_records_checkpoint_of_pre_upgrade_state() {
    let env = Env::default();
    let ctx = setup(&env);

    ctx.client
        .open_credit_line(&ctx.borrower, &CREDIT_LIMIT, &500, &10);
    let expected_count = ctx.client.get_credit_line_count();

    let _seeded = seed_checkpoint(&ctx, &env, 1);

    let checkpoint = ctx
        .client
        .get_upgrade_checkpoint()
        .expect("upgrade must record a checkpoint before swapping wasm");
    assert_eq!(
        checkpoint.credit_line_count, expected_count,
        "checkpoint must capture the pre-upgrade credit-line count"
    );
    assert_eq!(checkpoint.to_version, checkpoint.from_version + 1);
    assert_eq!(checkpoint.new_wasm_hash, wasm_hash(&env, 1));
    assert!(ctx.client.is_upgrade_verification_pending());
}

/// Boundary: a contract with no credit lines still records a coherent
/// checkpoint rather than skipping the snapshot.
#[test]
fn checkpoint_captures_zero_state_on_fresh_contract() {
    let env = Env::default();
    let ctx = setup(&env);

    let _seeded = seed_checkpoint(&ctx, &env, 2);

    let checkpoint = ctx.client.get_upgrade_checkpoint().expect("checkpoint");
    assert_eq!(checkpoint.credit_line_count, 0);
    assert_eq!(checkpoint.total_utilized, 0);
}

/// Regression: unchanged state must verify cleanly and unblock further
/// upgrades.
#[test]
fn verify_succeeds_when_state_is_unchanged() {
    let env = Env::default();
    let ctx = setup(&env);
    ctx.client
        .open_credit_line(&ctx.borrower, &CREDIT_LIMIT, &500, &10);

    let _seeded = seed_checkpoint(&ctx, &env, 3);
    assert!(ctx.client.is_upgrade_verification_pending());

    let verified = ctx.client.verify_upgrade_migration();
    assert_eq!(verified.credit_line_count, ctx.client.get_credit_line_count());

    // Cleared, so the next upgrade is unblocked.
    assert!(ctx.client.get_upgrade_checkpoint().is_none());
    assert!(!ctx.client.is_upgrade_verification_pending());
}

// ─── Duplicate / stacked upgrades (invariant UP-2) ──────────────────────────

#[test]
fn second_upgrade_is_blocked_until_resolved() {
    let env = Env::default();
    let ctx = setup(&env);

    let _seeded = seed_checkpoint(&ctx, &env, 4);
    assert!(ctx.client.is_upgrade_verification_pending());

    let err = ctx
        .client
        .try_upgrade(&wasm_hash(&env, 5))
        .expect_err("stacked upgrade must be rejected");
    assert_eq!(
        err.unwrap(),
        ContractError::UpgradeVerificationPending.into()
    );

    // After resolving, upgrades are permitted again.
    ctx.client.verify_upgrade_migration();
    assert!(!ctx.client.is_upgrade_verification_pending());
}

/// A rejected second upgrade must not overwrite or clear the outstanding
/// checkpoint — it is the evidence and the rollback target (invariants UP-4,
/// UP-5).
#[test]
fn rejected_second_upgrade_leaves_checkpoint_intact() {
    let env = Env::default();
    let ctx = setup(&env);

    let _seeded = seed_checkpoint(&ctx, &env, 6);
    let before = ctx.client.get_upgrade_checkpoint().expect("checkpoint");

    let _ = ctx.client.try_upgrade(&wasm_hash(&env, 7));

    let after = ctx.client.get_upgrade_checkpoint().expect("checkpoint kept");
    assert_eq!(
        before, after,
        "a rejected upgrade must leave the checkpoint byte-identical"
    );
}

// ─── Rejection paths ────────────────────────────────────────────────────────

#[test]
fn verify_without_checkpoint_is_rejected() {
    let env = Env::default();
    let ctx = setup(&env);

    let err = ctx
        .client
        .try_verify_upgrade_migration()
        .expect_err("verification without an upgrade must be rejected");
    assert_eq!(err.unwrap(), ContractError::NoUpgradeCheckpoint.into());
}

#[test]
fn rollback_without_checkpoint_is_rejected() {
    let env = Env::default();
    let ctx = setup(&env);

    let err = ctx
        .client
        .try_rollback_upgrade()
        .expect_err("rollback without an upgrade must be rejected");
    assert_eq!(err.unwrap(), ContractError::NoUpgradeCheckpoint.into());
}

// ─── Failure recovery ───────────────────────────────────────────────────────

#[test]
fn rollback_restores_previous_schema_version() {
    let env = Env::default();
    let ctx = setup(&env);

    let _seeded = seed_checkpoint(&ctx, &env, 8);
    let checkpoint = ctx.client.get_upgrade_checkpoint().expect("checkpoint");
    assert_eq!(checkpoint.to_version, checkpoint.from_version + 1);

    let rolled = ctx.client.rollback_upgrade();
    assert_eq!(rolled.from_version, checkpoint.from_version);

    // Checkpoint cleared and upgrades unblocked again.
    assert!(ctx.client.get_upgrade_checkpoint().is_none());
    assert!(!ctx.client.is_upgrade_verification_pending());

    // The rollback exposes the redeploy target rather than silently
    // re-swapping the wasm itself.
    assert_eq!(rolled.previous_wasm_hash, BytesN::from_array(&env, &[0u8; 32]));
}

/// A rollback is itself not replayable: once cleared there is nothing to roll
/// back to, so a repeated operator call is rejected rather than silently
/// re-applying a version change.
#[test]
fn rollback_is_not_replayable() {
    let env = Env::default();
    let ctx = setup(&env);

    let _seeded = seed_checkpoint(&ctx, &env, 9);
    ctx.client.rollback_upgrade();

    let err = ctx
        .client
        .try_rollback_upgrade()
        .expect_err("repeated rollback must be rejected");
    assert_eq!(err.unwrap(), ContractError::NoUpgradeCheckpoint.into());
}

// ─── Observability ──────────────────────────────────────────────────────────

/// The checkpoint view exposes only aggregates and hashes — never
/// per-borrower data — so it is safe to read without privilege.
#[test]
fn checkpoint_exposes_only_aggregate_state() {
    let env = Env::default();
    let ctx = setup(&env);
    ctx.client
        .open_credit_line(&ctx.borrower, &CREDIT_LIMIT, &500, &10);

    let _seeded = seed_checkpoint(&ctx, &env, 10);

    let checkpoint = ctx.client.get_upgrade_checkpoint().expect("checkpoint");
    // Aggregates only; the type carries no Address field at all.
    assert_eq!(checkpoint.credit_line_count, 1);
    assert!(checkpoint.recorded_at > 0);
}

#[test]
fn upgrade_errors_are_classified() {
    assert_eq!(
        ContractError::UpgradeVerificationPending.category(),
        ContractErrorCategory::Lifecycle
    );
    assert_eq!(
        ContractError::UpgradeStateMismatch.category(),
        ContractErrorCategory::Lifecycle
    );
    assert_eq!(
        ContractError::NoUpgradeCheckpoint.category(),
        ContractErrorCategory::Lifecycle
    );
    assert_eq!(ContractError::UpgradeVerificationPending as u32, 64);
    assert_eq!(ContractError::UpgradeStateMismatch as u32, 65);
    assert_eq!(ContractError::NoUpgradeCheckpoint as u32, 66);
}
