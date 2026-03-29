# Upgrade Policy: Immutability and Migration

## Contract Immutability

Creditra smart contracts are deployed as **immutable WASM** on the Stellar network.
Once a contract is deployed, its logic cannot be patched or upgraded in place.
This is a deliberate design choice aligned with Stellar's Soroban security model.

> **Stellar ecosystem reference:**
> [Soroban Smart Contracts — Deployment & Lifecycle](https://developers.stellar.org/docs/smart-contracts/getting-started/deploy-to-testnet)
> Soroban does not support in-place WASM upgrades; a new contract instance must
> be deployed for any logic change.

## Migration Policy

When a bug fix, feature addition, or breaking change is required, the following
process applies:

### 1. Deploy a New Contract

- Compile the updated contract:
```bash
  cargo build --release --target wasm32-unknown-unknown -p creditra-credit
```
- Deploy the new WASM to Stellar:
```bash
  soroban contract deploy \
    --wasm target/wasm32-unknown-unknown/release/creditra_credit.wasm \
    --source <identity> \
    --network <network>
```
- The new contract receives a **new contract address**.

### 2. Operational Data Migration

Because contract storage is bound to a specific contract address, all on-chain
state must be migrated manually:

1. **Export** all `CreditLineData` records from the old contract using
   `get_credit_line` for each known borrower address.
2. **Re-initialize** the new contract via `init` with the same admin, liquidity
   token, and liquidity source configuration.
3. **Re-import** each credit line by calling `open_credit_line` on the new
   contract with the exported parameters (`borrower`, `credit_limit`,
   `interest_rate_bps`, `risk_score`).
4. **Replay utilization** — restore `utilized_amount` via `draw_credit` calls
   (or a dedicated migration entry-point if added in the new contract).
5. **Restore status** — apply `suspend_credit_line`, `default_credit_line`, or
   `close_credit_line` as needed to match the old contract's state.

> ⚠️ Migration must be performed by the **admin** account. Ensure admin key
> continuity across deployments.

### 3. Backend Synchronization Tasks

The off-chain backend must be updated in coordination with any contract migration:

| Task | Description |
|------|-------------|
| **Update contract address** | Replace the old contract address with the new one in all backend configuration files and environment variables. |
| **Re-index events** | Re-index contract events from the new contract's deployment ledger to rebuild the backend's credit-line cache. |
| **Pause API writes** | Halt any write operations (draw, repay, open) during the migration window to prevent state divergence. |
| **Verify state parity** | After migration, compare backend-held state against on-chain `get_credit_line` responses for all borrowers. |
| **Update webhook/notification config** | Point event listeners and webhook subscribers to the new contract address. |
| **Communicate to integrators** | Notify downstream integrators (wallets, dApps) of the new contract address with a transition deadline for the old address. |

## Assumptions and Trust Boundaries

- Only the **admin** key may execute migration steps. Compromise of the admin key
  invalidates the migration's integrity.
- The migration window is a **trust-sensitive period**: no user-facing transactions
  should be processed until state parity is verified.
- The old contract remains on-chain indefinitely but should be treated as
  **deprecated** — the backend must enforce routing exclusively to the new address.

## Failure Modes

| Scenario | Impact | Mitigation |
|----------|--------|------------|
| Partial data migration | Some borrowers missing in new contract | Complete export/import before routing traffic |
| Admin key lost | Migration cannot be authorized | Use a multisig or key recovery policy |
| Backend not updated | Writes go to old contract | Automated config validation on deploy CI |
| State divergence after migration | Incorrect credit limits or utilization | Post-migration parity check script (compare all borrowers) |

## Running Tests Before Migration

Always run the full test suite before deploying a new contract version:
```bash
cargo test -p creditra-credit
```

For coverage validation (minimum 95% line coverage required):
```bash
cargo llvm-cov --workspace --all-targets --fail-under-lines 95
```
```

