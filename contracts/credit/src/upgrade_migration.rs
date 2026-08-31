// SPDX-License-Identifier: MIT
//! Credit-line state preservation across upgrade migrations (Issue #1149).
//!
//! # What
//!
//! `upgrade` swaps the contract WASM and bumps the schema version. Before
//! this module it did so blind:
//!
//! * The schema version was incremented unconditionally, with **no record of
//!   what the state looked like beforehand**, so nothing could tell whether
//!   credit-line records survived the swap or were silently re-interpreted by
//!   a layout the new binary reads differently.
//! * The "previous" WASM hash written to the upgrade event was a zero
//!   sentinel, so a bad upgrade had **no recorded rollback target**.
//! * Nothing prevented a second upgrade being stacked on top of a migration
//!   that had already gone wrong.
//!
//! This module adds the missing half: a checkpoint taken *before* the swap, a
//! verification step that proves state survived, and a rollback that restores
//! the previous schema version when it did not.
//!
//! # Model
//!
//! ```text
//!   upgrade(new_wasm)
//!        │  writes UpgradeCheckpoint {counts, versions, prev_wasm}
//!        │  swaps WASM
//!        ▼
//!   ┌──────────────────────┐
//!   │ checkpoint pending   │  ← further upgrades rejected here
//!   └──────────┬───────────┘
//!    verify    │    rollback
//!     ┌────────┴────────┐
//!     ▼                 ▼
//!  cleared          cleared, schema version restored
//! ```
//!
//! # Invariants
//!
//! * **UP-1 — Every upgrade records a checkpoint before mutating anything
//!   observable.** The counts captured are the pre-swap truth; capturing them
//!   afterwards would compare the new binary against itself and prove
//!   nothing.
//! * **UP-2 — At most one checkpoint exists at a time.** A second `upgrade`
//!   while one is outstanding is rejected with `UpgradeVerificationPending`,
//!   so a failed migration cannot be compounded or its evidence overwritten.
//! * **UP-3 — Verification is a pure comparison.** It reads
//!   `CreditLineCount` and `TotalUtilized` and compares them to the
//!   checkpoint; it never repairs state, because silently "fixing" a mismatch
//!   would destroy the evidence that a migration lost data.
//! * **UP-4 — A failed verification is recoverable.** The checkpoint is
//!   retained on mismatch precisely so `rollback_upgrade` still has a target.
//!   Only a successful verification or an explicit rollback clears it.
//! * **UP-5 — Rejected calls mutate nothing.** Every entrypoint validates
//!   before its first write, and a Soroban panic reverts the transaction, so
//!   a rejected upgrade, verification, or rollback leaves storage unchanged.
//!
//! # What rollback does and does not do
//!
//! `rollback_upgrade` restores the **schema version** and clears the
//! checkpoint. It deliberately does **not** re-swap the WASM: the running
//! binary cannot be trusted to deploy its own replacement after a failed
//! migration, and Soroban has no atomic "revert to previous hash" primitive.
//! The checkpoint carries `previous_wasm_hash` so an operator can re-deploy
//! the known-good binary explicitly. Documenting this boundary is the point —
//! an operator must not believe rollback is a complete undo.
//!
//! # Security
//!
//! Every entrypoint is admin-gated by the caller (`require_admin_auth` in
//! `lib.rs`). The checkpoint contains only aggregate counters and WASM
//! hashes — no borrower addresses, amounts, or other per-borrower data — so
//! exposing it through a view leaks nothing about individual positions.

use soroban_sdk::{contracttype, BytesN, Env};

use crate::storage::DataKey;
use crate::types::ContractError;

/// A pre-upgrade snapshot of the state that must survive a WASM swap.
///
/// Aggregates are used rather than a per-borrower scan because the scan cost
/// is unbounded in the number of credit lines, and these two accumulators are
/// already maintained as invariants over every credit-line mutation: if any
/// record were lost or misread, at least one of them moves.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpgradeCheckpoint {
    /// Schema version committed before the upgrade; the rollback target.
    pub from_version: u32,
    /// Schema version written by the upgrade.
    pub to_version: u32,
    /// `CreditLineCount` immediately before the WASM swap.
    pub credit_line_count: u32,
    /// `TotalUtilized` immediately before the WASM swap.
    pub total_utilized: i128,
    /// WASM hash the contract is being upgraded *to*.
    pub new_wasm_hash: BytesN<32>,
    /// WASM hash recorded as the known-good binary to redeploy on rollback.
    pub previous_wasm_hash: BytesN<32>,
    /// Ledger timestamp the checkpoint was taken.
    pub recorded_at: u64,
    /// Ledger sequence the checkpoint was taken.
    pub recorded_at_ledger: u32,
}

/// Read the outstanding checkpoint, if any.
pub fn get_checkpoint(env: &Env) -> Option<UpgradeCheckpoint> {
    env.storage().instance().get(&DataKey::UpgradeCheckpoint)
}

/// Whether an upgrade is awaiting verification or rollback.
pub fn is_verification_pending(env: &Env) -> bool {
    get_checkpoint(env).is_some()
}

/// Reject an upgrade while a previous one is unresolved (invariant UP-2).
pub fn require_no_pending_verification(env: &Env) {
    if is_verification_pending(env) {
        env.panic_with_error(ContractError::UpgradeVerificationPending);
    }
}

/// Capture the pre-upgrade state snapshot (invariant UP-1).
///
/// Called by `upgrade` *before* the schema bump and the WASM swap.
pub fn record_checkpoint(
    env: &Env,
    from_version: u32,
    to_version: u32,
    new_wasm_hash: &BytesN<32>,
    previous_wasm_hash: &BytesN<32>,
) -> UpgradeCheckpoint {
    let checkpoint = UpgradeCheckpoint {
        from_version,
        to_version,
        credit_line_count: crate::storage::get_credit_line_count(env),
        total_utilized: crate::storage::get_total_utilized(env),
        new_wasm_hash: new_wasm_hash.clone(),
        previous_wasm_hash: previous_wasm_hash.clone(),
        recorded_at: env.ledger().timestamp(),
        recorded_at_ledger: env.ledger().sequence(),
    };
    env.storage()
        .instance()
        .set(&DataKey::UpgradeCheckpoint, &checkpoint);
    checkpoint
}

/// Compare current aggregates against the checkpoint (invariant UP-3).
///
/// Returns the verified checkpoint and clears it on success. On mismatch the
/// checkpoint is **retained** (invariant UP-4) so a rollback is still
/// possible, and the call reverts with `UpgradeStateMismatch`.
///
/// # Errors
/// - [`ContractError::NoUpgradeCheckpoint`] — nothing to verify.
/// - [`ContractError::UpgradeStateMismatch`] — state did not survive.
pub fn verify(env: &Env) -> Result<UpgradeCheckpoint, ContractError> {
    let checkpoint = match get_checkpoint(env) {
        Some(c) => c,
        None => env.panic_with_error(ContractError::NoUpgradeCheckpoint),
    };

    let count_now = crate::storage::get_credit_line_count(env);
    let utilized_now = crate::storage::get_total_utilized(env);

    if count_now != checkpoint.credit_line_count || utilized_now != checkpoint.total_utilized {
        // Deliberately not cleared: the checkpoint is the evidence and the
        // rollback target (UP-4).
        env.panic_with_error(ContractError::UpgradeStateMismatch);
    }

    env.storage().instance().remove(&DataKey::UpgradeCheckpoint);
    Ok(checkpoint)
}

/// Restore the pre-upgrade schema version and clear the checkpoint.
///
/// Does **not** re-swap the WASM — see the module docs for why. The returned
/// checkpoint carries `previous_wasm_hash` so the operator can redeploy the
/// known-good binary explicitly.
///
/// # Errors
/// - [`ContractError::NoUpgradeCheckpoint`] — nothing to roll back.
pub fn rollback(env: &Env) -> UpgradeCheckpoint {
    let checkpoint = match get_checkpoint(env) {
        Some(c) => c,
        None => env.panic_with_error(ContractError::NoUpgradeCheckpoint),
    };
    crate::storage::set_schema_version(env, checkpoint.from_version);
    env.storage().instance().remove(&DataKey::UpgradeCheckpoint);
    checkpoint
}
