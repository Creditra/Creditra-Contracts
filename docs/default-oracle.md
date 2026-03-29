# Default Oracle Design (Stellar/Soroban)

## Goal

Define how verified default signals can trigger or assist `default_credit_line` while avoiding blind trust in unbounded external calls.

This is a design note for a staged integration. Current behavior remains admin-driven defaulting.

## Stellar-Specific Constraints

- Soroban contracts cannot call arbitrary internet services or webhooks.
- Any oracle signal must arrive as on-chain transaction input.
- Cross-contract calls are synchronous and metered, so verification logic must be bounded and deterministic.
- Signature verification and storage reads/writes consume CPU/memory budget per invocation.
- Replay and freshness checks should rely on ledger timestamp/sequence and contract storage.

## Current Admin Workflow

Current default lifecycle controls:

- `default_credit_line` marks line as `Defaulted`.
- `reinstate_credit_line` restores a defaulted line to `Active`.
- `suspend_credit_line` can freeze lines before/after risk changes.
- `close_credit_line` handles final closure paths.

See `docs/credit.md` method sections for exact behavior.

## Trust Boundaries

Actors and boundaries:

- Oracle signer set: trusted only to attest specific default conditions.
- Relayer: untrusted transport; can submit stale/duplicated signals.
- Admin: trusted governance/operator who can approve irreversible transitions.
- Contract: verifies cryptographic authenticity, freshness, and replay protections.

Design objective: restrict oracle authority to *bounded attestations* and preserve admin override controls.

## Recommended Integration Pattern (Phased)

### Phase 1: Oracle-Assisted Admin Default (Recommended First)

Add a signal verification path that informs admin action but does not fully automate defaulting:

1. `submit_default_signal(...)` verifies signed signal and stores a pending record.
2. Admin calls `default_credit_line` (or `default_credit_line_with_signal`) referencing that pending record.
3. Contract consumes the pending signal and applies state transition.

Benefits:

- Keeps human/governance checkpoint for high-impact state changes.
- Allows production hardening of signer rotation, replay, and expiry without immediate automation risk.

### Phase 2: Controlled Auto-Default Entry Point

After operational confidence:

- Add `default_credit_line_with_signal(...)` to perform verification + transition in one transaction.
- Optionally require both valid signal and authorized operator for dual control.

## Proposed Signal Schema

Canonical signal payload (hashed before signature verification):

- `borrower: Address`
- `reason_code: u32` (e.g., delinquency, covenant breach, fraud flag)
- `observed_at: u64` (unix seconds from oracle domain)
- `expires_at: u64` (hard expiry)
- `nonce: u64` (monotonic or unique per borrower)
- `chain_id` / `network_id`
- `contract_id`

Bounded checks:

- `now <= expires_at`
- `nonce` unused (replay protection)
- signer is in active signer registry
- payload domain matches target network + contract

## On-Chain Storage Additions (Conceptual)

- `OracleSignerSet` (instance): approved signer keys + version.
- `UsedSignalNonce(borrower, nonce)` (persistent): replay lock.
- `PendingDefaultSignal(borrower)` (persistent/temporary): verified signal metadata.

Storage TTL policy:

- Keep nonce records long enough to prevent practical replay windows.
- Expire pending signals aggressively to reduce stale-risk and state bloat.

## Security Controls

Core controls:

- Signature verification over domain-separated payload.
- Freshness window (`expires_at`) with strict reject on stale signals.
- Replay prevention via per-borrower nonce usage tracking.
- Optional signer-threshold model (M-of-N) for stronger oracle integrity.
- Rate limiting per borrower (optional) to prevent spammed state churn.

Failure handling:

- Invalid signature or stale signal: reject without state change.
- Duplicate nonce: reject without state change.
- Missing borrower line: reject without state change.

## Avoiding Unbounded External Trust

Do not:

- Depend on unbounded off-chain API calls at execution time.
- Accept opaque "oracle says so" booleans without signed, scoped payloads.
- Permit signer-less relayer submissions to trigger defaults.

Do:

- Verify bounded, auditable payloads fully on-chain.
- Constrain authority by explicit signer registry + expiry + nonce checks.
- Keep admin recovery controls (`reinstate_credit_line`, `suspend_credit_line`) documented and operational.

## Admin Runbook Linkage

Suggested runbook sequence:

1. Oracle signal received and submitted on-chain.
2. Operator review of borrower context and signal reason.
3. `default_credit_line` execution.
4. If dispute/false positive:
   - `suspend_credit_line` for containment, and/or
   - `reinstate_credit_line` after adjudication.

## Testing Strategy (When Implemented)

Required test categories:

- valid signal acceptance
- invalid signature rejection
- stale expiry rejection
- replay nonce rejection
- wrong network/contract domain rejection
- signer rotation and old-signer invalidation
- admin workflow compatibility (`default`, `suspend`, `reinstate`)

## Open Questions

- Should signer updates be immediate or timelocked?
- Is single signer sufficient initially, or threshold required from day one?
- Should default reasons be enumerable on-chain or free-form off-chain metadata hash?
- What TTL and nonce-retention windows balance safety vs storage cost?
