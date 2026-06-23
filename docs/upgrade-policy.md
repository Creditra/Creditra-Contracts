# Upgrade Policy: Native WASM Upgrade Path

## Overview

The Creditra credit contract implements an admin-gated upgrade path using Soroban's
native `env.deployer().update_current_contract_wasm()` mechanism. This allows the
protocol to ship bug fixes and feature additions without migrating borrower state.

## Upgrade Mechanism

### Implementation

The contract provides a public `upgrade` entrypoint that:

1. **Enforces security gates:**
   - Admin authentication via `require_admin_auth()`
   - Pause check via `assert_not_paused()` — upgrades are blocked during circuit breaker activation

2. **Updates state:**
   - Bumps `SCHEMA_VERSION` in instance storage to track upgrade history
   - Retrieves the current WASM hash before upgrade for event emission

3. **Performs atomic upgrade:**
   - Calls `env.deployer().update_current_contract_wasm(new_wasm_hash)`
   - This is an atomic operation that replaces the contract's WASM while preserving all storage

4. **Emits audit event:**
   - Publishes `ContractUpgradedEvent` with both old and new WASM hashes
   - Event topic: `("credit", "upgraded")`

### Usage

```rust
// 1. Deploy new WASM and get its hash
let new_wasm = include_bytes!("../target/wasm32-unknown-unknown/release/creditra_credit.wasm");
let new_wasm_hash = env.deployer().upload_contract_wasm(new_wasm.into());

// 2. Upgrade the contract (admin only)
client.upgrade(&new_wasm_hash);
```

### Command-Line Workflow

```bash
# 1. Build the new contract version
cargo build --release --target wasm32-unknown-unknown -p creditra-credit

# 2. Upload the new WASM to get its hash
soroban contract install \
  --wasm target/wasm32-unknown-unknown/release/creditra_credit.wasm \
  --source <admin-identity> \
  --network <network>

# Output: <new_wasm_hash>

# 3. Invoke the upgrade entrypoint
soroban contract invoke \
  --id <contract-address> \
  --source <admin-identity> \
  --network <network> \
  -- \
  upgrade \
  --new_wasm_hash <new_wasm_hash>
```

## Rollback Process

If an upgrade introduces a regression, the admin can roll back to the previous version:

1. **Retrieve the old WASM hash** from the `ContractUpgradedEvent` emitted during the upgrade
   - Event data contains: `{ old_wasm_hash, new_wasm_hash }`
   - Query the event log or use an off-chain indexer

2. **Re-upload the old WASM** (if not already available on-chain):
   ```bash
   soroban contract install \
     --wasm <path-to-old-wasm> \
     --source <admin-identity> \
     --network <network>
   ```

3. **Invoke upgrade with the old hash**:
   ```bash
   soroban contract invoke \
     --id <contract-address> \
     --source <admin-identity> \
     --network <network> \
     -- \
     upgrade \
     --new_wasm_hash <old_wasm_hash>
   ```

4. **Verify rollback**:
   - Check that the `ContractUpgradedEvent` was emitted with the old hash as `new_wasm_hash`
   - Verify contract behavior matches the previous version
   - Run integration tests against the rolled-back contract

### Rollback Time Window

- Rollback can be performed **at any time** after an upgrade
- No time-based restrictions (unlike admin rotation which has a delay)
- The old WASM must still be available on-chain or re-uploaded

## Review Process

### Pre-Upgrade Checklist

Before invoking the upgrade entrypoint, the admin must:

1. **Run the full test suite:**
   ```bash
   cargo test -p creditra-credit
   ```

2. **Verify test coverage** (minimum 95% line coverage):
   ```bash
   cargo llvm-cov --workspace --all-targets --fail-under-lines 95
   ```

3. **Review the diff** between current and new WASM:
   - Audit all changes to entrypoints, storage keys, and error codes
   - Verify no breaking changes to event schemas or public APIs
   - Confirm all new features have corresponding tests

4. **Test on testnet first:**
   - Deploy to Stellar testnet
   - Run integration tests against the testnet deployment
   - Verify all critical paths (draw, repay, admin operations)

5. **Prepare rollback plan:**
   - Document the current WASM hash before upgrade
   - Keep the old WASM binary accessible for quick rollback
   - Have the rollback command ready to execute

### Post-Upgrade Verification

After a successful upgrade:

1. **Verify the upgrade event:**
   ```bash
   soroban events --id <contract-address> --start-ledger <upgrade-ledger>
   ```
   - Confirm `ContractUpgradedEvent` was emitted
   - Verify `old_wasm_hash` and `new_wasm_hash` are correct

2. **Check schema version:**
   ```bash
   soroban contract invoke \
     --id <contract-address> \
     --network <network> \
     -- \
     get_schema_version
   ```
   - Confirm version was incremented

3. **Smoke test critical operations:**
   - Query existing credit lines: `get_credit_line`
   - Test draw/repay on a test borrower
   - Verify admin operations still work

4. **Monitor for anomalies:**
   - Watch for unexpected errors in logs
   - Monitor gas consumption for regressions
   - Track event emission patterns

## State Preservation Guarantees

The native upgrade mechanism preserves:

- ✅ All persistent storage (credit lines, borrower data)
- ✅ All instance storage (admin, liquidity token, global config)
- ✅ Contract address (remains unchanged)
- ✅ Storage TTLs (no reset or archival)

The upgrade **does not** preserve:

- ❌ In-flight transactions (must be retried after upgrade)
- ❌ Reentrancy guard state (cleared on upgrade, safe to proceed)

## Security Considerations

### Admin Key Protection

- The admin key is the **only** authorization required for upgrades
- Compromise of the admin key allows arbitrary WASM replacement
- **Recommendation:** Use a multisig or hardware wallet for the admin key

### Pause Enforcement

- Upgrades are blocked when the protocol is paused (`ContractError::Paused`)
- This prevents upgrades during emergency situations
- To upgrade during a pause, the admin must first unpause the protocol

### Audit Trail

- Every upgrade emits a `ContractUpgradedEvent` with both WASM hashes
- Off-chain indexers can track the full upgrade history
- The `SCHEMA_VERSION` provides an on-chain monotonic upgrade counter

## Failure Modes

| Scenario | Impact | Mitigation |
|----------|--------|------------|
| Admin key lost | Upgrades permanently disabled | Use multisig or key recovery policy |
| Malicious upgrade | Arbitrary code execution | Admin key protection + code review process |
| Upgrade during pause | Upgrade reverts with `ContractError::Paused` | Unpause first, or wait for automatic unpause |
| Rollback WASM unavailable | Cannot roll back to previous version | Archive all WASM binaries off-chain |
| Schema version overflow | Version counter wraps (unlikely) | Monitor version and plan migration at high values |

## Testing

The upgrade functionality is covered by comprehensive integration tests in
`contracts/credit/tests/upgrade.rs`:

- ✅ Happy path: admin successfully upgrades
- ✅ Sad path: unauthorized caller rejected
- ✅ Event emission: correct old/new WASM hashes
- ✅ State preservation: credit lines survive upgrade
- ✅ Schema version: incremented after upgrade
- ✅ Pause enforcement: upgrades blocked when paused
- ✅ Multiple upgrades: can be called repeatedly
- ✅ Rollback: can revert to previous WASM

Run the upgrade tests:
```bash
cargo test -p creditra-credit upgrade
```

## Comparison to Migration-Based Upgrades

### Native Upgrade (Current Implementation)

**Pros:**
- ✅ No state migration required
- ✅ Atomic operation (no downtime)
- ✅ Contract address unchanged
- ✅ Instant rollback capability

**Cons:**
- ❌ Requires admin key security
- ❌ No multi-step approval process (single admin call)

### Migration-Based Upgrade (Legacy Approach)

**Pros:**
- ✅ Can change contract address
- ✅ Can restructure storage layout

**Cons:**
- ❌ Requires manual state export/import
- ❌ Downtime during migration
- ❌ New contract address breaks integrations
- ❌ Complex rollback (must re-migrate)

## Running Tests Before Upgrade

Always run the full test suite before deploying a new contract version:
```bash
cargo test -p creditra-credit
```

For coverage validation (minimum 95% line coverage required):
```bash
cargo llvm-cov --workspace --all-targets --fail-under-lines 95
```

## Operational Checklist

### Pre-Upgrade

- [ ] Run full test suite (`cargo test -p creditra-credit`)
- [ ] Verify 95%+ test coverage
- [ ] Review code diff and audit changes
- [ ] Test on Stellar testnet
- [ ] Document current WASM hash
- [ ] Prepare rollback command
- [ ] Notify integrators of planned upgrade

### During Upgrade

- [ ] Verify admin key is available
- [ ] Confirm protocol is not paused
- [ ] Upload new WASM and get hash
- [ ] Invoke `upgrade` entrypoint
- [ ] Wait for transaction confirmation

### Post-Upgrade

- [ ] Verify `ContractUpgradedEvent` emission
- [ ] Check schema version increment
- [ ] Smoke test critical operations
- [ ] Monitor for anomalies
- [ ] Update documentation with new version
- [ ] Notify integrators of successful upgrade

## References

- [Soroban Contract Deployment](https://developers.stellar.org/docs/smart-contracts/getting-started/deploy-to-testnet)
- [Soroban Deployer Interface](https://docs.rs/soroban-sdk/latest/soroban_sdk/deploy/struct.Deployer.html)
- [Contract Upgrade Best Practices](https://developers.stellar.org/docs/smart-contracts/guides/upgrading-contracts)


