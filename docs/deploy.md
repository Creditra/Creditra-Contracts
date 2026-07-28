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

## Step 3 — Call `set_liquidity_token` (recommended)

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

## Step 4 — Call `set_liquidity_source` (optional)

By default the contract itself is the liquidity reserve. To use an external
reserve (e.g. a multisig treasury):

```bash
soroban contract invoke \
  --id $CONTRACT_ID \
  --source $ADMIN_KEY \
  -- set_liquidity_source \
  --reserve_address $RESERVE_ADDRESS
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

## Related files

| File | Role |
|------|------|
| `contracts/credit/src/config.rs` | `init`, `set_liquidity_token`, `set_liquidity_source` |
| `contracts/credit/src/storage.rs` | `admin_key`, `DataKey` |
| `contracts/credit/src/types.rs` | `ContractError::AlreadyInitialized` |
| `contracts/credit/tests/init_idempotency.rs` | Tests for init guard |
