// SPDX-License-Identifier: MIT
//! Replay-safe identifiers for draw and repayment records (Issue #1153).
//!
//! # What
//!
//! Before this module, a draw or a repayment left no per-operation record.
//! State moved (`utilized_amount`, `accrued_interest`) and an event was
//! emitted, but nothing on-chain identified *which* operation produced the
//! movement. Two consequences followed:
//!
//! 1. **Retries were unsafe.** A client whose `draw_credit` submission timed
//!    out could not tell whether the draw had landed. Re-submitting drew a
//!    second time; not re-submitting risked never drawing at all.
//! 2. **Movements were not reconcilable.** An indexer could see the aggregate
//!    change but had no stable handle to correlate it with the off-chain
//!    request that caused it.
//!
//! This module supplies both halves: a deterministic identifier for every
//! draw and repayment, and an idempotency barrier keyed on a caller-supplied
//! operation id.
//!
//! # Design
//!
//! ## Two id flavours, one record type
//!
//! * **Derived ids** (legacy entrypoints). `draw_credit` / `repay_credit`
//!   keep their existing signatures. Each call derives an id from
//!   `(kind, borrower, per-borrower sequence)`. The sequence is a monotonic
//!   counter bumped once per successful operation, so ids are unique and
//!   reproducible off-chain by anyone who can read the counter — but they
//!   provide no replay barrier, because the caller has no way to name the
//!   operation *before* it executes.
//! * **Caller-supplied ids** (`*_with_op_id` entrypoints). The caller passes
//!   a `BytesN<32>` it chose. The operation is rejected if that id was already
//!   consumed. This is the actual replay barrier: a retry after a timeout
//!   reuses the same id and is deterministically rejected rather than
//!   double-applying.
//!
//! Both flavours write the same [`DrawRecord`] / [`RepaymentRecord`], so a
//! reconciler reads one shape regardless of how the operation was submitted.
//!
//! ## Why reject a replay instead of returning the original result
//!
//! Returning the prior result would make a replay indistinguishable from a
//! fresh success at the call site, which hides a real client bug (two
//! *different* operations colliding on one id) behind a silent no-op. A
//! rejection is loud and diagnosable, and the client can still resolve its
//! uncertainty without guessing: [`get_draw_record`] and
//! [`get_repayment_record`] return the committed record for the id, so the
//! answer to "did my draw land?" is a free query rather than a second
//! state-changing call. This mirrors the settlement barrier in
//! [`crate::lifecycle`], which reverts `AlreadyInitialized` on a replayed
//! `(borrower, settlement_id)`.
//!
//! # Invariants
//!
//! * **OP-1 — At most one record per operation id.** A consumed id is never
//!   re-used for a second record, for either kind.
//! * **OP-2 — Ids are namespaced by kind.** The same 32 bytes may be used
//!   once as a draw id and once as a repayment id without colliding; the
//!   marker key includes the kind. Cross-kind collision would otherwise let a
//!   repayment silently block a legitimate draw.
//! * **OP-3 — Sequences are monotonic per borrower per kind.** A sequence is
//!   consumed only by a successful operation, so derived ids never repeat and
//!   the count of records equals the sequence.
//! * **OP-4 — The barrier is claimed before external effects.** The id is
//!   marked consumed *before* the token transfer, so a reentrant or
//!   concurrent call carrying the same id cannot slip past the check while
//!   the first call is still in flight.
//! * **OP-5 — Records are written only on success.** A rejected operation
//!   panics, which reverts the whole transaction including any marker or
//!   sequence write, so a failed attempt consumes nothing.
//!
//! # Storage
//!
//! * **Operation markers**: Persistent.
//!   Key: [`DataKey::OperationSeen`]`(kind, op_id)` → `bool` (presence =
//!   consumed).
//! * **Records**: Persistent.
//!   Key: [`DataKey::DrawRecordById`] / [`DataKey::RepaymentRecordById`].
//! * **Sequences**: Persistent, per borrower per kind.
//!   Key: [`DataKey::OperationSeq`]`(kind, borrower)` → `u64`.
//!
//! All three follow the credit-line TTL policy so a record cannot outlive
//! the line it describes and then resurface.
//!
//! # Security
//!
//! An operation id is *not* a capability: consuming one grants nothing, and
//! authorization is still enforced by `borrower.require_auth()` on the
//! entrypoint. A third party who front-runs a known id can therefore only
//! cause the legitimate call to be rejected, not redirect funds — and it
//! cannot even do that without the borrower's signature, because the auth
//! check precedes the barrier claim.
//!
//! Ids are opaque 32-byte values. They are echoed in events and records, so
//! callers must not encode sensitive data in them; a random value or a hash
//! of the client's own request id is the intended use.
use soroban_sdk::xdr::ToXdr;
use soroban_sdk::{contracttype, Address, Bytes, BytesN, Env};

use crate::storage::{DataKey, LEDGER_BUMP_AMOUNT, LEDGER_BUMP_THRESHOLD};
use crate::types::ContractError;

/// Refresh the persistent TTL for an operation key.
///
/// Uses the same ledger bounds as credit-line entries so a record or marker
/// cannot expire before the line it describes — an expired marker would
/// silently re-open the replay window it exists to close.
fn bump_operation_ttl<K>(env: &Env, key: &K)
where
    K: soroban_sdk::IntoVal<Env, soroban_sdk::Val>,
{
    env.storage()
        .persistent()
        .extend_ttl(key, LEDGER_BUMP_THRESHOLD, LEDGER_BUMP_AMOUNT);
}

/// Which ledger-movement family an operation id belongs to.
///
/// Part of every marker, sequence, and record key so the two families cannot
/// collide (invariant OP-2).
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationKind {
    /// A `draw_credit` family operation.
    Draw = 0,
    /// A `repay_credit` family operation.
    Repayment = 1,
}

impl OperationKind {
    /// Domain-separation byte mixed into a derived id.
    ///
    /// Without this, a draw and a repayment at the same sequence for the same
    /// borrower would derive identical ids.
    fn domain_byte(self) -> u8 {
        match self {
            OperationKind::Draw => 0x01,
            OperationKind::Repayment => 0x02,
        }
    }
}

/// How the operation's identifier was established.
///
/// Recorded so a reviewer can tell, from the record alone, whether the
/// operation carried a real replay barrier or merely a derived label.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationIdSource {
    /// Derived from `(kind, borrower, sequence)` by the contract. Unique and
    /// reproducible, but not replay-protected — the caller could not name the
    /// operation in advance.
    Derived = 0,
    /// Supplied by the caller and enforced against the replay barrier.
    CallerSupplied = 1,
}

/// An immutable record of one completed draw.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DrawRecord {
    /// Replay-safe identifier; unique across all draws.
    pub id: BytesN<32>,
    pub borrower: Address,
    /// Amount drawn, in token base units. Always `> 0`.
    pub amount: i128,
    /// `utilized_amount` after the draw was applied.
    pub utilized_after: i128,
    /// Per-borrower draw sequence this record consumed (0-based).
    pub sequence: u64,
    pub timestamp: u64,
    pub ledger_sequence: u32,
    pub id_source: OperationIdSource,
}

/// An immutable record of one completed repayment.
///
/// The interest/principal split is captured because repayment allocation is
/// order-dependent: reconstructing it later from aggregates alone is not
/// possible once further accrual has occurred.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepaymentRecord {
    /// Replay-safe identifier; unique across all repayments.
    pub id: BytesN<32>,
    pub borrower: Address,
    /// Amount the caller submitted.
    pub amount_submitted: i128,
    /// Amount actually applied after capping at total owed.
    pub amount_applied: i128,
    /// Portion of `amount_applied` that retired accrued interest.
    pub interest_repaid: i128,
    /// Portion of `amount_applied` that retired principal.
    pub principal_repaid: i128,
    /// `utilized_amount` after the repayment was applied.
    pub utilized_after: i128,
    /// Per-borrower repayment sequence this record consumed (0-based).
    pub sequence: u64,
    pub timestamp: u64,
    pub ledger_sequence: u32,
    pub id_source: OperationIdSource,
}

/// Read the next unconsumed sequence for `(kind, borrower)`.
///
/// Starts at 0 for a borrower that has never transacted, so the value doubles
/// as "number of recorded operations of this kind".
pub fn next_sequence(env: &Env, kind: OperationKind, borrower: &Address) -> u64 {
    env.storage()
        .persistent()
        .get(&DataKey::OperationSeq(kind, borrower.clone()))
        .unwrap_or(0u64)
}

/// Consume and return the current sequence, advancing the counter.
///
/// Saturating rather than wrapping: at `u64::MAX` the counter pins instead of
/// rolling over to 0, which would violate OP-3 by repeating derived ids. That
/// bound is unreachable in practice (it would take more operations than the
/// ledger can hold), but pinning fails safe rather than silently colliding.
fn consume_sequence(env: &Env, kind: OperationKind, borrower: &Address) -> u64 {
    let current = next_sequence(env, kind, borrower);
    let key = DataKey::OperationSeq(kind, borrower.clone());
    env.storage()
        .persistent()
        .set(&key, &current.saturating_add(1));
    bump_operation_ttl(env, &key);
    current
}

/// Derive a deterministic id from `(kind, borrower, sequence)`.
///
/// Reproducible off-chain: the same borrower, kind, and sequence always yield
/// the same 32 bytes, so an indexer can compute the id of a historical
/// operation without having observed the event that carried it.
pub fn derive_operation_id(
    env: &Env,
    kind: OperationKind,
    borrower: &Address,
    sequence: u64,
) -> BytesN<32> {
    let mut preimage = Bytes::new(env);
    preimage.push_back(kind.domain_byte());
    preimage.append(&borrower.clone().to_xdr(env));
    for byte in sequence.to_be_bytes() {
        preimage.push_back(byte);
    }
    env.crypto().sha256(&preimage).into()
}

/// Whether `op_id` has already been consumed for `kind`.
pub fn is_operation_consumed(env: &Env, kind: OperationKind, op_id: &BytesN<32>) -> bool {
    env.storage()
        .persistent()
        .has(&DataKey::OperationSeen(kind, op_id.clone()))
}

/// Claim `op_id` for `kind`, rejecting a replay.
///
/// # Invariant OP-4
/// Callers must invoke this **before** any external effect (token transfer,
/// cross-contract call), so a duplicate submission cannot pass the check while
/// the first is still in flight.
///
/// # Errors
/// - [`ContractError::DuplicateOperationId`] — the id was already consumed.
pub fn claim_operation_id(env: &Env, kind: OperationKind, op_id: &BytesN<32>) {
    if is_operation_consumed(env, kind, op_id) {
        env.panic_with_error(ContractError::DuplicateOperationId);
    }
    let key = DataKey::OperationSeen(kind, op_id.clone());
    env.storage().persistent().set(&key, &true);
    bump_operation_ttl(env, &key);
}

/// Resolve the id for an operation, claiming the barrier when one was supplied.
///
/// Returns the id to record and how it was established. A supplied id is
/// claimed here (rejecting replays); a derived id needs no claim because it is
/// unique by construction — the sequence it is derived from has just been
/// consumed and can never be issued again.
pub fn resolve_operation_id(
    env: &Env,
    kind: OperationKind,
    borrower: &Address,
    supplied: Option<BytesN<32>>,
) -> (BytesN<32>, u64, OperationIdSource) {
    let sequence = consume_sequence(env, kind, borrower);
    match supplied {
        Some(op_id) => {
            claim_operation_id(env, kind, &op_id);
            (op_id, sequence, OperationIdSource::CallerSupplied)
        }
        None => (
            derive_operation_id(env, kind, borrower, sequence),
            sequence,
            OperationIdSource::Derived,
        ),
    }
}

/// Persist a completed draw record.
pub fn put_draw_record(env: &Env, record: &DrawRecord) {
    let key = DataKey::DrawRecordById(record.id.clone());
    env.storage().persistent().set(&key, record);
    bump_operation_ttl(env, &key);
}

/// Persist a completed repayment record.
pub fn put_repayment_record(env: &Env, record: &RepaymentRecord) {
    let key = DataKey::RepaymentRecordById(record.id.clone());
    env.storage().persistent().set(&key, record);
    bump_operation_ttl(env, &key);
}

/// Fetch a draw record by id, if one was committed.
pub fn get_draw_record(env: &Env, op_id: &BytesN<32>) -> Option<DrawRecord> {
    env.storage()
        .persistent()
        .get(&DataKey::DrawRecordById(op_id.clone()))
}

/// Fetch a repayment record by id, if one was committed.
pub fn get_repayment_record(env: &Env, op_id: &BytesN<32>) -> Option<RepaymentRecord> {
    env.storage()
        .persistent()
        .get(&DataKey::RepaymentRecordById(op_id.clone()))
}
