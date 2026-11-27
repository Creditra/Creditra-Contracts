# Upgrade Policy: Native WASM Upgrade Path

## Overview

The Creditra credit contract implements an admin-gated upgrade path using Soroban's
native `env.deployer().update_current_contract_wasm()` mechanism. This allows the
protocol to ship bug fixes and feature additions without migrating borrower state.

Upgrading a live lending contract is a high-stakes operation: a failed upgrade can
leave every credit line inaccessible until a rollback is executed. Follow this
runbook in full for every upgrade, including testnet rehearsals.

---

## 1. Upgrade Mechanism

### Implementation

The `upgrade` entrypoint (`contracts/credit/src/lib.rs`, line ~2808) is the sole
path to replace the contract WASM. Its execution sequence is:

1. **Pause check** — `assert_not_paused()` ensures upgrades cannot happen during
   a live circuit-breaker event. If the protocol is paused,
   `ContractError::Paused` (code 18) is returned and no state is mutated.

2. **Admin authentication** — `require_admin_auth()` requires a valid signature
   from the address stored under the `"admin"` instance-storage key. Any other
   caller reverts with `ContractError::NotAdmin` (code 2).

3. **Schema version bump** — reads `DataKey::SchemaVersion` from instance
   storage (see §2 below), increments by one via `saturating_add(1)`, and
   writes the new value back before the WASM swap occurs.

4. **Atomic WASM swap** — `env.deployer().update_current_contract_wasm(new_wasm_hash)`
   replaces the executing code. The swap is atomic: either the entire transaction
   commits (new WASM active, version bumped, event emitted) or it rolls back
   entirely.

5. **Audit event** — `ContractUpgradedEvent { old_wasm_hash, new_wasm_hash }` is
   published with topic `("credit", "upgraded")` so off-chain indexers can track
   the full upgrade history.

### Source reference

```
contracts/credit/src/lib.rs       — fn upgrade()
contracts/credit/src/storage.rs   — get_schema_version / set_schema_version
contracts/credit/src/events.rs    — ContractUpgradedEvent / publish_contract_upgraded_event
```

---

## 2. Schema Version Semantics

`SchemaVersion` is a `u32` stored in **instance storage** under `DataKey::SchemaVersion`.
It is the contract's on-chain monotonic counter of WASM replacements.

### Lifecycle

| Event | Effect on SchemaVersion |
|---|---|
| First `init` call | Written as `SCHEMA_VERSION` constant (currently `1`) |
| Each `upgrade` call | Incremented by 1 via `saturating_add` |
| Rollback (re-upgrade to old WASM) | Still incremented — version reflects upgrade count, not code lineage |

### What it does and does not guarantee

**Does guarantee:**
- The version is strictly non-decreasing.
- A higher version means at least one `upgrade` call has executed after the
  previous version was written.
- Post-upgrade verification can use `get_schema_version` to confirm the
  transaction committed.

**Does not guarantee:**
- That the new WASM is forward-compatible with existing storage.
- The specific code version deployed (for that, read the `new_wasm_hash` from
  the `ContractUpgradedEvent`).
- Any mapping between version numbers and semantic releases — that mapping is
  maintained in the project CHANGELOG.

### Querying the current version

```bash
soroban contract invoke \
  --id <contract-address> \
  --network <network> \
  -- \
  get_schema_version
```

Returns `Option<u32>`. A `None` result from a contract that has not been upgraded
since the last `init` is possible only on very old deployments; all current
deployments return `Some(u32)`.

---

## 3. Zero `old_wasm_hash` — Why the Event Field Is All Zeros

The `ContractUpgradedEvent` struct is:

```rust
pub struct ContractUpgradedEvent {
    pub old_wasm_hash: BytesN<32>,
    pub new_wasm_hash: BytesN<32>,
}
```

The Soroban SDK version used by this contract does not expose a
`get_current_contract_wasm` or equivalent API for reading the executing
contract's own WASM hash at runtime. As a result, `old_wasm_hash` is always
`BytesN::from_array(&env, &[0u8; 32])` — a 32-byte zero sentinel value.

**Practical consequence for operators:**

- The `new_wasm_hash` field is authoritative and correct for every upgrade.
- To reconstruct upgrade lineage, index the `new_wasm_hash` fields across
  successive `ContractUpgradedEvent` emissions rather than relying on
  `old_wasm_hash`.
- When planning a rollback, retrieve the desired target hash from the
  `new_wasm_hash` of the event that corresponds to the last known-good upgrade
  (see §8, Rollback Procedure).
- The zero value is intentional and documented, not a bug. If a future SDK
  version exposes the current WASM hash, this field will be populated and the
  sentinel behavior removed.

---

## 4. Storage Compatibility: Checking Against `types.rs` Structs

Soroban uses XDR-encoded contract types. Any `#[contracttype]`-tagged struct or
enum stored in persistent or instance storage must remain layout-compatible
across upgrades — the new WASM must be able to deserialize values written by
the old WASM.

### Structs that live in persistent storage

These are read and written on every credit-line access. A field addition,
removal, or reordering is a breaking change for live accounts.

| Type | Storage key | File |
|---|---|---|
| `CreditLineData` | `DataKey::CreditLineIdByBorrower(Address)` (via `Address` directly) | `contracts/credit/src/types.rs:404` |
| `RepaymentSchedule` | `DataKey::RepaymentSchedule(Address)` | `contracts/credit/src/types.rs` |

### Structs that live in instance storage

These are read on every transaction execution. A layout change takes effect
globally and immediately.

| Type | Storage key | File |
|---|---|---|
| `RateFormulaConfig` | `DataKey::RateFormulaConfig` | `contracts/credit/src/types.rs` |
| `RateChangeConfig` | `DataKey::RateChangeConfig` | `contracts/credit/src/types.rs` |
| `GracePeriodConfig` | `DataKey::GracePeriodConfig` | `contracts/credit/src/types.rs` |
| `OracleConfig` | `DataKey::OracleConfig` | `contracts/credit/src/types.rs` |

### Safe vs. unsafe struct changes

| Change type | Safe? | Notes |
|---|---|---|
| Adding a new field at the end of a struct | ✅ with care | New WASM must supply a default when deserializing old values that lack the field; confirm with a simulated read on testnet |
| Removing a field | ❌ | Old values will fail to deserialize |
| Reordering fields | ❌ | XDR encoding is positional — reordering silently corrupts values |
| Changing a field's type | ❌ | Type mismatch causes deserialization panic |
| Adding a new enum variant at the end | ✅ | Discriminants must be appended; existing values remain decodable |
| Renumbering or reordering enum variants | ❌ | Stored discriminants become invalid |

### Safe vs. unsafe enum changes (`CreditStatus`, `ContractError`)

`CreditStatus` variants are stored inside `CreditLineData`. `ContractError`
discriminants are part of the public ABI. Both follow the same append-only rule:

- New variants must receive the next available integer and be placed at the end.
- Existing discriminants are pinned and must never change.

### Pre-upgrade compatibility checklist

Before submitting an upgrade transaction, verify every item below:

**Diff review:**

```bash
git diff <previous-tag> HEAD -- contracts/credit/src/types.rs
```

Check for:
- [ ] No struct fields removed or reordered in `CreditLineData`, `RepaymentSchedule`,
      or any other `#[contracttype]` struct
- [ ] No enum variants reordered or renumbered in `CreditStatus` or `ContractError`
- [ ] New struct fields have a fallback default (or the new WASM handles `None`
      on deserialization)
- [ ] New `DataKey` variants are appended at the end

**Stability test suite:**

```bash
# Pin error discriminants — must pass with zero failures
cargo test -p creditra-credit error_discriminants_are_stable

# Pin event topics — must pass with zero failures
cargo test -p creditra-credit test_event_topics_stability
```

These tests live in:
- `contracts/credit/tests/error_discriminants.rs` — asserts every `ContractError`
  discriminant against its hardcoded integer. Any reordering fails immediately.
- `contracts/credit/tests/event_topic_stability.rs` — asserts that all 25+ event
  topic strings are unchanged.

**Full upgrade test suite:**

```bash
cargo test -p creditra-credit upgrade
```

This runs all tests in `contracts/credit/tests/upgrade.rs`, covering:
- Happy path: admin successfully upgrades
- Unauthorized caller rejected
- Event emission: correct topic and hashes
- Schema version bump after upgrade
- State preservation: credit lines survive upgrade
- Pause enforcement: upgrade blocked when paused
- Multiple sequential upgrades
- Credit-line operations usable immediately after upgrade

---

## 5. Testnet Rehearsal

Every upgrade must be rehearsed on Stellar testnet before being applied to
mainnet. The rehearsal is a full dry-run of the production sequence.

### Step 1 — Build a reproducible artifact

```bash
# Verify the toolchain pin is active
./scripts/check-toolchain.sh --verify-active

# Build size-optimized WASM (locked dependencies)
cargo build --release --target wasm32-unknown-unknown -p creditra-credit --locked

# Confirm WASM is under the 50 KB budget
ls -lh target/wasm32-unknown-unknown/release/creditra_credit.wasm
```

### Step 2 — Record the pre-upgrade state

```bash
export CONTRACT=<testnet-contract-address>
export NETWORK=testnet

# Record current schema version
soroban contract invoke --id $CONTRACT --network $NETWORK \
  -- get_schema_version
# Save this output as PRE_VERSION

# Record a sample credit line to verify post-upgrade preservation
soroban contract invoke --id $CONTRACT --network $NETWORK \
  -- get_credit_line --borrower <test-borrower-address>
# Save this output
```

### Step 3 — Upload the new WASM

```bash
soroban contract install \
  --wasm target/wasm32-unknown-unknown/release/creditra_credit.wasm \
  --source <admin-identity> \
  --network $NETWORK
# Output: <NEW_WASM_HASH>
export NEW_WASM_HASH=<output from above>
```

### Step 4 — Execute the upgrade

```bash
soroban contract invoke \
  --id $CONTRACT \
  --source <admin-identity> \
  --network $NETWORK \
  -- \
  upgrade \
  --new_wasm_hash $NEW_WASM_HASH
```

### Step 5 — Post-upgrade verification (see §6 for full details)

```bash
# Confirm schema version incremented
soroban contract invoke --id $CONTRACT --network $NETWORK \
  -- get_schema_version
# Must equal PRE_VERSION + 1

# Confirm upgrade event was emitted
soroban events --id $CONTRACT --start-ledger <upgrade-ledger>
# Locate ContractUpgradedEvent; confirm new_wasm_hash matches NEW_WASM_HASH

# Smoke test existing credit line data
soroban contract invoke --id $CONTRACT --network $NETWORK \
  -- get_credit_line --borrower <test-borrower-address>
# Must match pre-upgrade output
```

### Step 6 — Run critical-path operations

Test at minimum:
- `draw_credit` on an existing credit line
- `repay_credit` on the same line
- An admin operation (e.g., `update_risk_parameters`)

### Step 7 — Gate mainnet upgrade on rehearsal success

Do not proceed to mainnet if any testnet step fails. Diagnose the root cause,
fix in code, re-run the full test suite, and start the rehearsal over.

---

## 6. Post-Upgrade Verification

After each upgrade (testnet or mainnet), confirm the following before considering
the upgrade complete.

### 6.1 Confirm schema version incremented

```bash
soroban contract invoke \
  --id <contract-address> \
  --network <network> \
  -- \
  get_schema_version
```

Expected result: previous version + 1. Any other result indicates the upgrade
transaction did not commit or the version storage is corrupted.

### 6.2 Confirm upgrade event emission

```bash
soroban events \
  --id <contract-address> \
  --start-ledger <ledger-of-upgrade-tx>
```

Look for an event with:
- Topic[0]: `"credit"`
- Topic[1]: `"upgraded"`
- Data: `{ old_wasm_hash: "0000...0000", new_wasm_hash: "<expected-hash>" }`

Confirm `new_wasm_hash` matches the hash printed by `soroban contract install`
in step 3 of the rehearsal. The `old_wasm_hash` will always be all zeros (see §3).

### 6.3 Confirm contract version constant

```bash
soroban contract invoke \
  --id <contract-address> \
  --network <network> \
  -- \
  get_contract_version
```

Returns the `CONTRACT_API_VERSION` tuple `(major, minor, patch)` compiled into
the new WASM. Confirm it matches the release notes for this upgrade.

### 6.4 Smoke test critical paths

| Operation | Command snippet | Expected result |
|---|---|---|
| Read a credit line | `get_credit_line --borrower <addr>` | Returns same data as pre-upgrade |
| Draw on an existing line | `draw_credit --borrower <addr> --amount 1` | Succeeds, emits `Drawn` event |
| Repay | `repay_credit --borrower <addr> --amount 1` | Succeeds, emits `Repayment` event |
| Admin read | `get_contract_version` | Returns version tuple |

### 6.5 Monitor for anomalies

For the 24 hours following a mainnet upgrade:
- Watch for unexpected `ContractError` codes in event logs, particularly
  `Overflow` (12), `TimestampRegression` (33), or `AuctionCallFailed` (62).
- Compare gas consumption on `draw_credit` and `repay_credit` against
  pre-upgrade baselines.
- Alert if any credit line transitions to `Defaulted` within the monitoring
  window without a corresponding admin action — this can indicate a storage
  deserialization issue that silently zeroed a field.

---

## 7. State Preservation Guarantees

The native upgrade mechanism preserves:

- ✅ All persistent storage (credit lines, borrower data, repayment schedules,
  collateral balances)
- ✅ All instance storage (admin, liquidity token, config structs, schema version)
- ✅ Contract address (unchanged across all upgrades)
- ✅ Storage TTLs (no reset or archival triggered by the upgrade itself)
- ✅ All `DataKey` entries for active and historical credit lines

The upgrade **does not** preserve:

- ❌ In-flight transactions (must be retried after upgrade commits)
- ❌ Reentrancy guard state (cleared; safe to proceed — no legitimate re-entrant
  call should span an upgrade boundary)

---

## 8. Rollback Procedure

A rollback is a second `upgrade` call that supplies the hash of the previously
active WASM. The schema version continues to increment (rollback is still an
upgrade).

### When to roll back

Roll back if post-upgrade verification (§6) reveals any of:
- Schema version did not increment (transaction failed — no rollback needed,
  just investigate the failure)
- A critical-path smoke test fails
- On-chain storage values are corrupted or unreadable after upgrade
- Unexpected error codes in event logs suggesting logic regression

### Step 1 — Locate the target rollback hash

The desired hash is the `new_wasm_hash` from the `ContractUpgradedEvent` of
the last known-good upgrade.

```bash
soroban events \
  --id <contract-address> \
  --start-ledger <block-range-of-previous-upgrade>
```

Find the event with Topic[1] = `"upgraded"` from the upgrade you want to
return to. Extract `new_wasm_hash` from the event data.

Alternatively, if you archived the WASM binary:

```bash
soroban contract install \
  --wasm <path-to-known-good-wasm> \
  --source <admin-identity> \
  --network <network>
# Returns the hash of the already-installed WASM (re-install is idempotent)
```

### Step 2 — Confirm the old WASM is available on-chain

`env.deployer().update_current_contract_wasm()` requires the target hash to
already exist on-chain. Re-upload if needed:

```bash
soroban contract install \
  --wasm <path-to-known-good-wasm> \
  --source <admin-identity> \
  --network <network>
export ROLLBACK_HASH=<hash>
```

### Step 3 — Execute the rollback

```bash
soroban contract invoke \
  --id <contract-address> \
  --source <admin-identity> \
  --network <network> \
  -- \
  upgrade \
  --new_wasm_hash $ROLLBACK_HASH
```

### Step 4 — Verify the rollback

Repeat the full post-upgrade verification from §6 against the rolled-back contract.
In particular:

```bash
# Confirm schema version incremented (now at pre-failed-upgrade version + 2)
soroban contract invoke --id <contract-address> --network <network> \
  -- get_schema_version

# Confirm contract version tuple reflects the rolled-back code
soroban contract invoke --id <contract-address> --network <network> \
  -- get_contract_version

# Confirm critical paths work
# ... (repeat §6.4 smoke tests)
```

### Rollback time window

There is no time limit on rollback. You may execute a rollback at any point
after a failed upgrade, provided:
1. The admin key is available and the protocol is not paused.
2. The target WASM hash exists on-chain (or is re-uploaded).

### Storage compatibility on rollback

If the failed upgrade wrote any new storage fields that the rolled-back WASM
does not know about, those fields will be ignored by the old WASM (Soroban
deserializes only known fields and ignores unknown XDR extras, for structs
encoded with forward-compatible XDR). However, if the failed upgrade removed
or reordered existing fields, the rollback itself cannot repair stored values
that were written in the broken format. This is another reason why storage
layout changes must be validated on testnet before any mainnet upgrade.

---

## 9. Security Considerations

### Admin key protection

The admin key is the sole authorization gate for upgrades. Compromise of the
admin key allows arbitrary WASM replacement and therefore arbitrary code
execution over all protocol funds.

- **Use a multisig or hardware wallet** for the admin key on all production
  deployments. A single EOA is insufficient.
- Record the admin address and rotation history off-chain.
- Admin rotation uses a two-step propose/accept pattern
  (`propose_admin` / `accept_admin`) with a configurable delay. Do not rotate
  the admin and upgrade in the same maintenance window.

### Pause enforcement

Upgrades are blocked while `ContractError::Paused` is active. This prevents
a rushed upgrade from being pushed during an active incident. If an upgrade is
urgently needed during a paused state, the admin must explicitly call
`unpause_protocol` first — a deliberate two-step decision.

### Audit trail

Every upgrade emits a `ContractUpgradedEvent` (topic `("credit", "upgraded")`).
Off-chain indexers consuming this event can reconstruct the full history of WASM
hashes and the ledger at which each became active. The `SCHEMA_VERSION` stored
on-chain provides an independent monotonic counter that does not rely on event
log availability.

---

## 10. Failure Modes

| Scenario | Impact | Detection | Mitigation |
|---|---|---|---|
| Admin key unavailable | Upgrade cannot proceed | `ContractError::NotAdmin` (2) | Multisig recovery; key rotation before planned upgrade |
| Protocol is paused | Upgrade reverts with `ContractError::Paused` (18) | Transaction error | Unpause first (requires admin); confirm pause reason is resolved |
| New WASM hash not on-chain | Upgrade reverts | Transaction error | `soroban contract install` the WASM first |
| Storage layout break (field removed/reordered) | Deserialization panics on first affected read | Post-upgrade smoke test failures | Roll back immediately; fix layout before next attempt |
| Schema version overflow (`u32::MAX`) | Version saturates at `u32::MAX` (no panic) | Monitoring | Effectively never reachable in practice (~4 billion upgrades) |
| Rollback WASM unavailable on-chain | Rollback blocked until WASM is re-uploaded | Manual inspection | Archive all deployed WASM binaries off-chain; re-upload before rollback |
| Upgrade during active auction | No restriction — fee-config entrypoints are blocked during `AuctionActive`, but `upgrade` is not | Monitoring | Review auction state before upgrading; prefer upgrading outside active auction windows |

---

## 11. Testing

The upgrade functionality is covered by integration tests in
`contracts/credit/tests/upgrade.rs`:

| Test | What it asserts |
|---|---|
| `upgrade_happy_path_succeeds` | Admin successfully upgrades; `ContractUpgradedEvent` emitted |
| `upgrade_bumps_schema_version` | Version increments by exactly 1 |
| `upgrade_preserves_existing_state` | Credit lines readable with correct values after upgrade |
| `upgrade_event_contains_correct_hashes` | Event topic is `"upgraded"`; `new_wasm_hash` matches supplied hash |
| `upgrade_unauthorized_caller_rejected` | Non-admin call panics with `Auth` error |
| `upgrade_blocked_when_paused` | Paused contract rejects upgrade with `ContractError::Paused` (18) |
| `upgrade_can_be_called_multiple_times` | Version increments on each call |
| `upgrade_with_same_wasm_hash_succeeds` | Re-upgrade to same hash is idempotent |
| `upgrade_does_not_affect_credit_line_operations` | `draw_credit` works immediately after upgrade |
| `upgrade_event_topic_is_stable` | Topic string is exactly `"upgraded"` |
| `upgrade_admin_rotation_still_works_after_upgrade` | Admin rotation operational post-upgrade |

Run all upgrade tests:

```bash
cargo test -p creditra-credit upgrade
```

Run the stability guards (must pass before any upgrade):

```bash
# Guard: ContractError discriminants must not change
cargo test -p creditra-credit error_discriminants_are_stable

# Guard: event topic strings must not change
cargo test -p creditra-credit test_event_topics_stability
```

Run full suite with coverage:

```bash
cargo llvm-cov --workspace --all-targets --fail-under-lines 95
```

---

## 12. Operational Checklists

### Pre-upgrade

- [ ] Run full test suite: `cargo test -p creditra-credit`
- [ ] Confirm 95%+ line coverage: `cargo llvm-cov --workspace --all-targets --fail-under-lines 95`
- [ ] Run `error_discriminants_are_stable` — zero failures required
- [ ] Run `test_event_topics_stability` — zero failures required
- [ ] Diff `contracts/credit/src/types.rs` — no struct fields removed or reordered
- [ ] Diff `contracts/credit/src/storage.rs` — no `DataKey` variants removed or reordered
- [ ] Review `contracts/credit/src/lib.rs` entrypoint signatures — no removals or parameter type changes
- [ ] Verify toolchain pin is active: `./scripts/check-toolchain.sh --verify-active`
- [ ] Build WASM with `--locked`: `cargo build --release --target wasm32-unknown-unknown -p creditra-credit --locked`
- [ ] Confirm WASM size < 50 KB
- [ ] Complete testnet rehearsal (§5) — all steps passed
- [ ] Record pre-upgrade `get_schema_version` output
- [ ] Archive the current (pre-upgrade) WASM binary off-chain
- [ ] Prepare rollback command (§8) before starting mainnet upgrade
- [ ] Notify integrators of planned upgrade window

### During upgrade

- [ ] Confirm admin key is available and signing is working
- [ ] Confirm protocol is not paused: `pause_protocol` not active
- [ ] Upload new WASM: `soroban contract install` — record the hash
- [ ] Invoke `upgrade --new_wasm_hash <hash>`
- [ ] Wait for transaction confirmation (ledger sealed)

### Post-upgrade

- [ ] `get_schema_version` returns pre-upgrade version + 1
- [ ] `ContractUpgradedEvent` emitted with correct `new_wasm_hash`
- [ ] `get_contract_version` returns the expected API version tuple
- [ ] Smoke test: `get_credit_line` on a known borrower — values unchanged
- [ ] Smoke test: `draw_credit` succeeds
- [ ] Smoke test: `repay_credit` succeeds
- [ ] Smoke test: admin operation succeeds
- [ ] Monitor event logs for unexpected errors for 24 hours post-upgrade
- [ ] Update CHANGELOG with deployed WASM hash and ledger number
- [ ] Notify integrators that upgrade is complete

---

## 13. Comparison to Migration-Based Upgrades

### Native upgrade (current implementation)

**Pros:**
- No state migration required — all `CreditLineData` records survive unchanged
- Atomic operation — no partial state and no downtime
- Contract address unchanged — integrators need no reconfiguration
- Instant rollback capability — re-upgrade to old hash in one transaction

**Cons:**
- Requires admin key security — single point of authorization failure
- No multi-step approval (single admin call unlocks upgrade)
- Storage layout changes require backward-compatible encoding (append-only fields)

### Migration-based upgrade (alternative)

**Pros:**
- Can restructure storage layout arbitrarily (copy-and-transform on new contract)
- Can change contract address if needed

**Cons:**
- Requires manual state export and import — downtime guaranteed
- New contract address breaks all integrations
- Complex rollback (must re-migrate to original contract)
- Not atomic — migration failure leaves state split across two contracts

---

## 14. References

- [Soroban Contract Deployment](https://developers.stellar.org/docs/smart-contracts/getting-started/deploy-to-testnet)
- [Soroban Deployer Interface](https://docs.rs/soroban-sdk/latest/soroban_sdk/deploy/struct.Deployer.html)
- [Contract Upgrade Best Practices](https://developers.stellar.org/docs/smart-contracts/guides/upgrading-contracts)
- [`docs/deploy.md`](./deploy.md) — initial deployment sequence and `init` guard
- [`docs/PROTOCOL_SPEC.md`](./PROTOCOL_SPEC.md) — full entrypoint signatures and storage key catalog
- [`docs/SECURITY.md`](./SECURITY.md) — threat model and admin key protection guidance
- [`contracts/credit/tests/upgrade.rs`](../contracts/credit/tests/upgrade.rs) — upgrade integration tests
- [`contracts/credit/tests/error_discriminants.rs`](../contracts/credit/tests/error_discriminants.rs) — discriminant stability guard
- [`contracts/credit/tests/event_topic_stability.rs`](../contracts/credit/tests/event_topic_stability.rs) — event topic stability guard
