# Deployment Guide

This document describes the required deployment sequence for the Creditra
Credit contract and the invariants that operators must maintain.

---

## Deployment sequence

The following steps must be performed **in order** immediately after deployment.
Skipping or reordering steps will leave the contract in an unusable or insecure
state.
---

## Step 1 — Deploy contract binary

Deploy the compiled WASM to the Stellar network using the Soroban CLI or SDK.
Note the resulting contract address.

---

## Step 2 — Call `init(admin)`

`init` is a **one-time** operation protected by an `AlreadyInitialized` guard:

```rust
pub fn init(env: Env, admin: Address)
```

### What it does

- Stores `admin` in instance storage under the `"admin"` key.
- Sets `LiquiditySource` to the contract's own address as the default reserve.

### What it does NOT do

- It does not emit an event.
- It does not set a liquidity token (that requires a separate call).

### Security guarantees

- A second call to `init` with any address reverts with
  `ContractError::AlreadyInitialized` (error code 14).
- The admin address is immutable after the first successful `init` call.
- No state is mutated on a failed re-init attempt.

### Example

```bash
soroban contract invoke \
  --id $CONTRACT_ID \
  --source $DEPLOYER_KEY \
  -- init \
  --admin $ADMIN_ADDRESS
```

---

## Step 3 — Call `set_liquidity_token` (required for drawing)

Without a liquidity token, draw operations transfer no tokens (state-only
accounting). Set the token before opening credit lines that will be drawn:

```bash
soroban contract invoke \
  --id $CONTRACT_ID \
  --source $ADMIN_KEY \
  -- set_liquidity_token \
  --token_address $TOKEN_ADDRESS
```

---

## Step 4 — Call `set_liquidity_source` (strongly recommended)

**WARNING:** By default, the contract itself is the liquidity reserve. This is an unsafe default for production unless the contract holds its own funds. To use an external reserve (e.g., a multisig treasury), you must set the liquidity source:

```bash
soroban contract invoke \
  --id $CONTRACT_ID \
  --source $ADMIN_KEY \
  -- set_liquidity_source \
  --reserve_address $RESERVE_ADDRESS
```

---

## Step 5 — Call `set_min_collateral_ratio_bps` (optional)

Set the minimum collateral ratio (in basis points) required for borrowers. For example, 15000 is 150%.

```bash
soroban contract invoke \
  --id $CONTRACT_ID \
  --source $ADMIN_KEY \
  -- set_min_collateral_ratio_bps \
  --ratio_bps 15000
```

---

## Step 6 — Auction Contract Wiring

To enable default liquidations, the credit and auction contracts must be wired together.

1. Set the auction contract on the credit contract:
```bash
soroban contract invoke \
  --id $CONTRACT_ID \
  --source $ADMIN_KEY \
  -- set_auction_contract \
  --auction_contract $AUCTION_CONTRACT_ID
```

2. Register the credit contract as a factory on the auction contract side:
```bash
soroban contract invoke \
  --id $AUCTION_CONTRACT_ID \
  --source $AUCTION_ADMIN_KEY \
  -- set_factory_contract \
  --factory $CONTRACT_ID
```

---

## Step 7 — Smoke Test Sequence

Verify the deployment by performing a basic open, draw, and repay sequence.

1. Open a credit line:
```bash
soroban contract invoke \
  --id $CONTRACT_ID \
  --source $BORROWER_KEY \
  -- open_credit_line \
  --borrower $BORROWER_ADDRESS
```

2. Draw credit:
```bash
soroban contract invoke \
  --id $CONTRACT_ID \
  --source $BORROWER_KEY \
  -- draw_credit \
  --borrower $BORROWER_ADDRESS \
  --amount 10000000
```

3. Repay credit:
```bash
soroban contract invoke \
  --id $CONTRACT_ID \
  --source $BORROWER_KEY \
  -- repay_credit \
  --borrower $BORROWER_ADDRESS \
  --amount 10000000
```

---

## AlreadyInitialized guard

The guard is implemented in `contracts/credit/src/config.rs`:

```rust
if env.storage().instance().has(&admin_key(&env)) {
    env.panic_with_error(ContractError::AlreadyInitialized);
}
```

This fires before any storage write, so a failed re-init leaves the contract
state completely unchanged.

### Error code

`ContractError::AlreadyInitialized = 14`

### Verification

```bash
# A second init call should return Error(Contract, #14)
soroban contract invoke \
  --id $CONTRACT_ID \
  --source $ANY_KEY \
  -- init \
  --admin $ANY_ADDRESS
# Expected: Error(Contract, #14)
```

---

## Admin rotation

The admin address is currently immutable after `init`. A safe rotation design
(propose + accept two-step pattern) is planned. Until then, protect the admin
key with a hardware wallet or multisig.

---

## Upgrading a deployed contract

Once the contract is live, WASM upgrades are applied through the admin-gated
`upgrade` entrypoint. Upgrades preserve all storage (credit lines, config,
borrower data) and keep the contract address unchanged.

Before upgrading, operators must:
1. Verify storage layout compatibility against `contracts/credit/src/types.rs`
   (no struct fields removed or reordered; no enum discriminants changed).
2. Pass the stability test guards:
   `cargo test -p creditra-credit error_discriminants_are_stable`
   `cargo test -p creditra-credit test_event_topics_stability`
3. Complete a full testnet rehearsal.
4. Confirm `get_schema_version` increments after the upgrade commits.

Full pre-upgrade checklist, schema version semantics, zero `old_wasm_hash`
explanation, testnet rehearsal steps, and rollback procedure:
**[`docs/upgrade-policy.md`](./upgrade-policy.md)**

---

## Related files

| File | Role |
|------|------|
| `contracts/credit/src/config.rs` | `init`, `set_liquidity_token`, `set_liquidity_source` |
| `contracts/credit/src/storage.rs` | `admin_key`, `DataKey` |
| `contracts/credit/src/types.rs` | `ContractError::AlreadyInitialized` |
| `contracts/credit/tests/init_idempotency.rs` | Tests for init guard |
| `docs/upgrade-policy.md` | WASM upgrade runbook: pre-checks, execution, verification, rollback |
