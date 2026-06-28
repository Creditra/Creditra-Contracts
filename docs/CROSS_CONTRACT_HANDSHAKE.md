# Cross-Contract Handshake: Events ↔ Profile

> **Spec**: [boundless-platform-contract-prd.md](https://github.com/boundlessfi/boundless-contract/blob/main/boundless-platform-contract-prd.md) Section 4  
> **Last updated**: 2026-06-28

This document describes the cross-contract calls between the **boundless-events** and **boundless-profile** Soroban contracts, covering call/return shape, error mapping, reentrancy guarantees, and replay protection.

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                    boundless-events contract                     │
│                                                                  │
│  apply_to_bounty ──────────────┐                                │
│  withdraw_application ─────────┼──► profile_client::client()    │
│  select_winners ───────────────┼─► ProfileClient                │
│  claim_milestone ──────────────┘                                │
│                                                                  │
│  Uses: ProfileInterface trait + generated ProfileContractClient  │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                   boundless-profile contract                     │
│                                                                  │
│  Verifies: caller == events_contract (via require_events_contract)│
│  Stores:   DataKey::EventsContract                               │
│                                                                  │
│  Exposes: bootstrap, spend_credits, earn_credits, refund_credits │
│           bump_reputation, slash_reputation, register_earnings   │
└─────────────────────────────────────────────────────────────────┘
```

Both contracts are independently deployable and upgradeable. The events contract holds a configurable binding to the profile contract address.

---

## Call/Return Shape

### ProfileInterface Trait (defined in `contracts/events/src/profile_client.rs`)

```rust
#[contractclient(name = "ProfileClient")]
pub trait ProfileInterface {
    fn bootstrap(env: Env, user: Address, op_id: BytesN<32>);
    fn spend_credits(env: Env, user: Address, amount: u32, reason: Symbol, op_id: BytesN<32>);
    fn earn_credits(env: Env, user: Address, amount: u32, reason: Symbol, op_id: BytesN<32>);
    fn refund_credits(env: Env, user: Address, amount: u32, reason: Symbol, op_id: BytesN<32>);
    fn bump_reputation(env: Env, user: Address, delta: u32, reason: Symbol, op_id: BytesN<32>);
    fn slash_reputation(env: Env, user: Address, delta: u32, reason: Symbol, op_id: BytesN<32>);
    fn register_earnings(env: Env, user: Address, token: Address, amount: i128, op_id: BytesN<32>);
}
```

### Cross-Contract Call Sites

| Events Entry Point | Profile Method(s) Called | Reason Symbol | Child op_id Tag |
|--------------------|--------------------------|---------------|-----------------|
| `bounty::apply()` | `bootstrap()` + `spend_credits()` | `"apply"` | `BOOTSTRAP`, `SPEND_CREDITS` |
| `bounty::withdraw_application()` | `refund_credits()` | `"wd_refund"` | `REFUND_CREDITS` |
| `event_ops::select_winners()` | `bootstrap()` + `earn_credits()` + `bump_reputation()` + `register_earnings()` | `"win"` | `BOOTSTRAP`, `EARN_CREDITS`, `BUMP_REP`, `REGISTER_EARNINGS` |
| `grant::claim_milestone()` | `bootstrap()` + `earn_credits()` + `bump_reputation()` + `register_earnings()` | `"milestone"` | `BOOTSTRAP`, `EARN_CREDITS`, `BUMP_REP`, `REGISTER_EARNINGS` |

### Return Shape

All profile methods return `Result<(), Error>`:
- **Ok(())** on success
- **Err(Error::InsufficientCredits)** - not enough credits to spend
- **Err(Error::ProfileNotFound)** - user has no profile (bootstrap required first)
- **Err(Error::AlreadyInitialized)** - bootstrap already called
- **Err(Error::Paused)** - profile contract is paused
- **Err(Error::OpAlreadySeen)** - op_id already used (idempotency)

The events contract propagates these errors directly via the `?` operator.

### Client Creation Pattern

```rust
// In events contract, get typed client pointing at registered profile address
let profile = profile_client::client(env);

// Each cross-contract call uses a unique derived child op_id
let child_op = idempotency::derive_child(env, &parent_op_id, tag::BOOTSTRAP);
profile.bootstrap(&recipient, &child_op);
```

The `profile_client::client()` helper reads `DataKey::EventsContract` from events contract storage and returns a `ProfileContractClient`.

---

## Authorization Pattern

### Profile Contract Gate

The profile contract uses **two-step authorization**:

1. **First deployment** (single-step):
   ```rust
   pub fn set_events_contract(env: &Env, new_addr: Address) -> Result<(), Error> {
       // Allowed if events_contract is None (first-set only)
       if storage::get_events_contract(env).is_some() {
           return Err(Error::EventsContractAlreadyConfigured);
       }
       // ...
   }
   ```

2. **Subsequent rotations** (two-step with timelock):
   ```rust
   // Step 1: propose
   pub fn propose_events_contract(env: &Env, new_addr: Address) -> Result<(), Error>
   
   // Step 2: accept (after EVENTS_CONTRACT_TIMELOCK_LEDGERS = 17,280 ledgers ~1 day)
   pub fn accept_events_contract(env: &Env) -> Result<(), Error>
   ```

3. **Authorization check in each mutation**:
   ```rust
   pub fn require_events_contract(env: &Env) -> Result<(), Error> {
       let events = storage::get_events_contract(env)
           .ok_or(Error::EventsContractNotConfigured)?;
       events.require_auth();
       Ok(())
   }
   ```

### Caller Identity Verification

```rust
// In spend_credits, earn_credits, bump_reputation, etc.
pub fn spend(...) -> Result<(), Error> {
    require_not_paused(env)?;
    require_events_contract(env)?;  // Verifies caller == registered events contract
    // ...
}
```

**Note**: `bootstrap_self()` allows any user to bootstrap their own profile without events_contract dependency (onboarding path).

---

## Error Mapping

### Propagation Chain

```
Events Contract Call                    Profile Contract Return
─────────────────────────               ───────────────────────
apply_to_bounty()                       Error::InsufficientCredits
  └─► profile.spend_credits()           Error::ProfileNotFound
                                          Error::Paused

withdraw_application()                  Error::InsufficientCredits
  └─► profile.refund_credits()          Error::ProfileNotFound

select_winners()                        Error::InsufficientCredits
  ├─► profile.earn_credits()            Error::ProfileNotFound
  ├─► profile.bump_reputation()         Error::Paused
  └─► profile.register_earnings()       Error::ProfileNotFound

claim_milestone()                       Error::InsufficientCredits
  ├─► profile.earn_credits()            Error::ProfileNotFound
  ├─► profile.bump_reputation()         Error::Paused
  └─► profile.register_earnings()       Error::ProfileNotFound
```

### Error Enums

**Events Error** (`contracts/events/src/errors.rs`):
- `EventNotFound`, `EventNotActive`, `InvalidPillar`, `InvalidReleaseKind`
- `ApplicantNotApplied`, `SubmissionAlreadyExists`, `DeadlinePassed`
- `WinnersAlreadySelected`, `InvalidWinnerPosition`, `DuplicateWinnerPosition`
- `InsufficientEscrow`, `DistributionMismatch`, `InvalidDistribution`
- `Paused`, `NotAdmin`, `AlreadyInitialized`

**Profile Error** (`contracts/profile/src/errors.rs`):
- `InsufficientCredits`, `ProfileNotFound`, `AlreadyInitialized`
- `OpAlreadySeen`, `PendingAdminMismatch`, `PendingAdminExpired`
- `EventsContractNotConfigured`, `EventsContractAlreadyConfigured`
- `PendingEventsContractMismatch`, `PendingEventsContractExpired`, `PendingEventsContractTimelock`
- `Paused`, `NotAdmin`, `NotInitialized`

### Error Semantics

| Profile Error | When Returned | Events Impact |
|---------------|---------------|---------------|
| `InsufficientCredits` | User has < required credits | `apply_to_bounty()` reverts; user must earn more |
| `ProfileNotFound` | User never bootstrapped | Preceded by `bootstrap()` which is idempotent |
| `OpAlreadySeen` | Child op_id already used | Indicates replay or bug in child op derivation |
| `Paused` | Profile contract admin-paused | All cross-contract calls fail |
| `EventsContractNotConfigured` | Profile doesn't know events contract | Admin must call `set_events_contract()` |

---

## Reentrancy Guarantees

### Design Decision: No Explicit Reentrancy Guard

Unlike traditional reentrancy locks, the cross-contract handshake relies on:

1. **Idempotency markers** for replay protection
2. **Distinct child op_ids** for each cross-contract call
3. **Auth-gated mutations** so only authorized callers can invoke profile methods

### Why No Reentrancy Lock?

The handshake is **safe** because:

1. **No state shared between calls**: Each cross-contract call modifies distinct profile state (user-specific credits, reputation, earnings)
2. **No callback into events**: Profile contract has no reference back to events contract
3. **Deterministic child op_ids**: Even if events calls profile multiple times in one transaction, each call gets a unique derived op_id
4. **Events contract idempotency**: The main entry point (`apply_to_bounty`, `select_winners`, etc.) checks `OpSeen` before any cross-contract call

### OpId Derivation Pattern

```rust
// Each cross-contract call gets a unique child op_id
// parent_op_id is the original operation identifier
// tag is a short symbol identifying which sub-call this is

let bootstrap_op = idempotency::derive_child(env, &op_id, tag::BOOTSTRAP);
profile.bootstrap(&recipient, &bootstrap_op);

let earn_op = idempotency::derive_child(env, &op_id, tag::EARN_CREDITS);
profile.earn_credits(&recipient, &credit_earn, &reason, &earn_op);
```

### Panic Behavior

If a cross-contract call **panics** (reverts):
- The entire transaction reverts (Soroban atomic model)
- No state changes persist in either contract
- The parent's `OpSeen` marker is **not** set (since the tx reverted before `mark_seen`)
- Orchestrator interprets the revert as failure and can retry with same op_id

If the events contract **panics after** making cross-contract calls:
- All profile mutations have already completed
- Parent `mark_seen` never called
- **Result**: profile state is updated but events side is not (inconsistent)
- **Mitigation**: Profile's own idempotency prevents duplicate profile mutations with same child op_id

---

## Replay Protection

### Two-Layer Idempotency

**Layer 1: Events Contract Main Entry Point**
```rust
// In every state-mutating entry point
pub fn apply(...) -> Result<(), Error> {
    idempotency::require_unseen(env, &op_id)?;  // Check OpSeen
    // ... do work ...
    idempotency::mark_seen(env, &op_id);        // Mark after success
}
```

**Layer 2: Profile Contract Child Operations**
```rust
// In each profile method
pub fn earn_credits(...) -> Result<(), Error> {
    require_events_contract(env)?;
    idempotency::require_unseen(env, &op_id)?;  // Child op_id
    // ... do work ...
    idempotency::mark_seen(env, &op_id);
}
```

### Child OpId Format

```
parent_op_id: BytesN<32>  (e.g., sha256("event:123:apply:v1"))
    │
    ├── derive_child(parent, "bootstrap") ─► unique child op_id
    ├── derive_child(parent, "spend_credits") ─► unique child op_id
    └── derive_child(parent, "earn_credits") ─► unique child op_id
```

The derivation uses `env.ledger().sequence()` and the tag to create distinct op_ids per sub-call.

### Marker TTL

Both contracts use **temporary storage** for `OpSeen` markers:
- Auto-expires per Soroban TTL (typically ~14 days)
- Orchestrator caps reconciliation window to stay inside TTL
- Prevents unbounded storage growth

### Multi-Winner Atomicity

When `select_winners()` processes multiple winners:
```rust
// For each winner at index `idx`:
let earn_op = idempotency::derive_child_indexed(env, &op_id, tag::EARN_CREDITS, idx as u8);
profile.earn_credits(&spec.recipient, &spec.credit_earn, &reason_win, &earn_op);
```

Each winner's profile mutation uses a **distinct indexed child op_id**:
- Winner 0: `derive_child_indexed(op_id, EARN_CREDITS, 0)`
- Winner 1: `derive_child_indexed(op_id, EARN_CREDITS, 1)`

This allows independent replay protection per winner while maintaining atomicity within the single transaction.

---

## API Changes

### No Breaking Changes

The cross-contract interface is **stable** as of v0.1.0 (events) / v0.2.0 (profile). The ProfileInterface trait has not changed since initial deployment.

### Future Rotation Support

When the events contract is upgraded or replaced:

1. Admin calls `propose_events_contract(new_addr)` on profile
2. Waits for timelock (`EVENTS_CONTRACT_TIMELOCK_LEDGERS` ≈ 1 day)
3. Admin calls `accept_events_contract()` on profile
4. Events contract reads new profile address from storage

This two-step rotation prevents accidental or malicious profile binding changes without audit trail visibility.

---

## Test Coverage

Cross-contract interaction tests live in:
- `contracts/events/src/tests/cross_contract.rs` — integration tests wiring real events + profile contracts

### Covered Scenarios

| Test | What It Verifies |
|------|------------------|
| `select_winners_pays_recipient_and_bumps_profile` | Full flow: token payment + credits + reputation + earnings |
| `select_winners_requires_position_in_distribution` | Error propagation: InvalidWinnerPosition |
| `select_winners_rejects_duplicate_position` | Error propagation: DuplicateWinnerPosition |
| `select_winners_replayed_reverts` | Replay protection at events level |
| `select_winners_rejects_second_call_winners_already_selected` | One-shot enforcement |
| `cancel_refunds_remaining_escrow_to_owner` | No profile calls on cancel |
| `cancel_after_select_winners_refunds_only_remaining` | Partial payout scenario |
| `claim_milestone_pays_per_milestone_amount` | Per-milestone profile updates |
| `claim_milestone_idempotent_per_recipient_and_milestone` | Replay protection at profile level |
| `claim_milestone_invalid_milestone_index_reverts` | Error propagation: InvalidMilestone |
| `claim_milestone_rejects_non_grant_events` | Pillar validation |
| `claim_milestone_final_milestone_marks_event_completed` | State transition |
| `grant_last_milestone_sweeps_rounding_residue` | Math precision |
| `select_winners_pays_against_remaining_escrow_including_top_ups` | M1: partner top-ups flow through |
| `bounty_apply_requires_prior_application` | Bounty-specific flow |
| `bounty_submit_succeeds_after_apply` | Submit after apply |
| `resubmit_preserves_original_submitted_at_and_updates_uri` | Idempotency edge case |

---

## Security Considerations

### H5: Events Contract Rotation Timelock

**Finding**: Single-step rotation of events_contract binding was a soft point.

**Mitigation**: Two-step rotation with `propose_events_contract` + `accept_events_contract` with ~1 day timelock. Off-chain monitors can react before the binding takes effect.

### Authorization Model

- Only the **registered events contract** can call profile mutations
- Admin direct mutations (`admin_grant_credits`, `admin_slash_reputation`) require admin auth only
- `bootstrap_self()` allows user self-service without events contract dependency

### Pause Behavior

When either contract is paused:
- Events contract: all state-mutating ops blocked except admin ops
- Profile contract: all mutations (including events contract calls) blocked
- **Cross-contract calls fail atomically** — no partial state updates

---

## Related Documentation

- [ARCHITECTURE.md](./ARCHITECTURE.md) — Why two contracts and the split line
- [audit-2026-06-stellar-skill.md](./audit-2026-06-stellar-skill.md) — Audit findings including H5
- [mainnet-deploy-runbook.md](./mainnet-deploy-runbook.md) — Deployment procedures