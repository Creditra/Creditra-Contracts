# Creditra Credit Contract — Deployment & Invocation Playbook

> Covers WASM build, Testnet deployment, initialization, liquidity configuration,
> and all core method invocations for the `creditra-credit` Soroban contract.

---

## Prerequisites

```bash
# Stellar CLI v22+ (includes soroban subcommands)
cargo install --locked stellar-cli --features opt

stellar --version   # stellar 22.x.x

# WASM compile target
rustup target add wasm32-unknown-unknown
```

---

## 1. Identity Setup

**Never use raw secret keys on the command line.** The `--source` flag accepts a
named identity stored in the Stellar CLI keystore. All examples below use named
identities exclusively.

```bash
# Create the admin identity (key stored in ~/.config/stellar/identity/)
stellar keys generate --global admin --network testnet

# Create a borrower identity for testing
stellar keys generate --global borrower --network testnet

# Fund both accounts via Testnet Friendbot
stellar keys fund admin --network testnet
stellar keys fund borrower --network testnet

# Inspect an address without exposing the key
stellar keys address admin
stellar keys address borrower
```

On CI, inject the secret as an environment variable and reference it by name:

```bash
# In CI: set ADMIN_SECRET_KEY in vault/secrets manager, then:
stellar keys add admin --secret-key   # reads from stdin or $STELLAR_SECRET_KEY
```

> The `--source <identity-name>` flag used throughout this guide resolves the
> named key from the local keystore. It never prints or logs the raw secret.

---

## 2. Build

```bash
cargo build \
  --package creditra-credit \
  --target wasm32-unknown-unknown \
  --release
```

Output: `target/wasm32-unknown-unknown/release/creditra_credit.wasm`

Optimize before deploying (reduces on-chain storage fees):

```bash
stellar contract optimize \
  --wasm target/wasm32-unknown-unknown/release/creditra_credit.wasm
```

---

## 3. Deploy

```bash
stellar contract deploy \
  --wasm target/wasm32-unknown-unknown/release/creditra_credit.wasm \
  --source admin \
  --network testnet
```

Save the returned contract ID:

```bash
export CONTRACT_ID=<returned-contract-id>
```

---

## 4. Initialize

Must be called exactly once before any other function. Sets the admin address
and defaults the liquidity source to the contract itself.

```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --source admin \
  --network testnet \
  -- init \
  --admin $(stellar keys address admin)
```

---

## 5. Liquidity Configuration

### Set the liquidity token

The SAC (Stellar Asset Contract) address for the token used in draw transfers
and reserve balance checks. Admin-only.

```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --source admin \
  --network testnet \
  -- set_liquidity_token \
  --token_address <SAC-contract-address>
```

### Set the liquidity source

Defaults to the contract address. Override to use an external reserve. Admin-only.

```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --source admin \
  --network testnet \
  -- set_liquidity_source \
  --reserve_address <reserve-address>
```

### Configure rate-change limits (optional)

Enforces a maximum BPS delta and minimum interval between consecutive rate
changes on `update_risk_parameters`. Admin-only.

```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --source admin \
  --network testnet \
  -- set_rate_change_limits \
  --max_rate_change_bps 500 \
  --rate_change_min_interval 86400
```

When not set, no rate-change restrictions are enforced (backward-compatible).

---

## 6. Credit Line Lifecycle

### Open a credit line

```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --source admin \
  --network testnet \
  -- open_credit_line \
  --borrower $(stellar keys address borrower) \
  --credit_limit 10000 \
  --interest_rate_bps 300 \
  --risk_score 70
```

| Parameter | Type | Constraints |
|---|---|---|
| `credit_limit` | `i128` | > 0 |
| `interest_rate_bps` | `u32` | 0–10000 (100 bps = 1%) |
| `risk_score` | `u32` | 0–100 |

### Draw credit

Called by the borrower. `--source borrower` signs the transaction with the
borrower's key — the contract enforces `borrower.require_auth()`.

```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --source borrower \
  --network testnet \
  -- draw_credit \
  --borrower $(stellar keys address borrower) \
  --amount 2500
```

### Repay credit

Called by the borrower. Reduces `utilized_amount` (saturates at zero).

```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --source borrower \
  --network testnet \
  -- repay_credit \
  --borrower $(stellar keys address borrower) \
  --amount 1000
```

### Update risk parameters

Admin-only. New `credit_limit` must be ≥ current `utilized_amount`.

```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --source admin \
  --network testnet \
  -- update_risk_parameters \
  --borrower $(stellar keys address borrower) \
  --credit_limit 15000 \
  --interest_rate_bps 250 \
  --risk_score 65
```

### Suspend a credit line

```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --source admin \
  --network testnet \
  -- suspend_credit_line \
  --borrower $(stellar keys address borrower)
```

### Default a credit line

Disables draws; repayment remains allowed.

```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --source admin \
  --network testnet \
  -- default_credit_line \
  --borrower $(stellar keys address borrower)
```

### Reinstate a defaulted credit line

Transitions Defaulted → Active. Admin-only.

```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --source admin \
  --network testnet \
  -- reinstate_credit_line \
  --borrower $(stellar keys address borrower)
```

### Close a credit line

Admin can force-close regardless of utilization. Borrower can only close when
`utilized_amount == 0`. The `--source` must match the `--closer` argument.

```bash
# Admin force-close (works at any utilization)
stellar contract invoke \
  --id $CONTRACT_ID \
  --source admin \
  --network testnet \
  -- close_credit_line \
  --borrower $(stellar keys address borrower) \
  --closer $(stellar keys address admin)

# Borrower self-close (only when utilized_amount == 0)
stellar contract invoke \
  --id $CONTRACT_ID \
  --source borrower \
  --network testnet \
  -- close_credit_line \
  --borrower $(stellar keys address borrower) \
  --closer $(stellar keys address borrower)
```

---

## 7. Read-Only Queries

These do not require `--source` and do not submit a transaction.

```bash
# Get full credit line state
stellar contract invoke \
  --id $CONTRACT_ID \
  --network testnet \
  -- get_credit_line \
  --borrower $(stellar keys address borrower)

# Get rate-change config (returns null if not set)
stellar contract invoke \
  --id $CONTRACT_ID \
  --network testnet \
  -- get_rate_change_limits
```

---

## 8. Status Transitions

| From | To | Method | Caller |
|---|---|---|---|
| Active | Suspended | `suspend_credit_line` | Admin |
| Active | Defaulted | `default_credit_line` | Admin |
| Suspended | Defaulted | `default_credit_line` | Admin |
| Defaulted | Active | `reinstate_credit_line` | Admin |
| Defaulted | Suspended | `suspend_credit_line` | Admin |
| Any (non-Closed) | Closed | `close_credit_line` | Admin (any utilization) or Borrower (utilized=0) |

`close_credit_line` is idempotent — calling it on an already-Closed line is a no-op.

---

## 9. Trust Boundaries

This table defines who is permitted to call each function. Any call that
violates these boundaries will be rejected by the contract's auth checks.

| Function | Permitted Caller | Auth Mechanism |
|---|---|---|
| `init` | Deployer (once) | None — first caller wins |
| `set_liquidity_token` | Admin | `admin.require_auth()` |
| `set_liquidity_source` | Admin | `admin.require_auth()` |
| `set_rate_change_limits` | Admin | `admin.require_auth()` |
| `open_credit_line` | Admin / risk engine | `admin.require_auth()` |
| `update_risk_parameters` | Admin / risk engine | `admin.require_auth()` |
| `suspend_credit_line` | Admin | `admin.require_auth()` |
| `default_credit_line` | Admin | `admin.require_auth()` |
| `reinstate_credit_line` | Admin | `admin.require_auth()` |
| `close_credit_line` | Admin (force) or Borrower (zero utilization) | `closer.require_auth()` |
| `draw_credit` | Borrower | `borrower.require_auth()` |
| `repay_credit` | Borrower | `borrower.require_auth()` |
| `get_credit_line` | Anyone | None (read-only) |
| `get_rate_change_limits` | Anyone | None (read-only) |

**Admin key compromise** is the highest-severity risk. A compromised admin can:
- Force-close all credit lines
- Drain the liquidity reserve by reconfiguring `set_liquidity_source`
- Set arbitrary risk parameters on any borrower

Use a multisig or hardware-backed key for the admin identity in production.

**`init` is unguarded.** If `init` is not called immediately after deployment,
any address can call it and claim admin. Deploy and initialize in the same
transaction batch or script.

---

## 10. Failure Modes & Recovery

| Failure | Cause | Recovery |
|---|---|---|
| `Credit line not found` | Borrower address has no open line | Verify address; call `open_credit_line` first |
| `credit line is closed` | Line status is `Closed` | Re-open with `open_credit_line` (allowed after close) |
| `credit line is not defaulted` | `reinstate_credit_line` called on non-Defaulted line | Check status with `get_credit_line`; call `default_credit_line` first if needed |
| `exceeds credit limit` | Draw would push `utilized_amount` past `credit_limit` | Reduce draw amount or call `update_risk_parameters` to raise the limit |
| `cannot close: utilized amount not zero` | Borrower tried to self-close with outstanding balance | Repay in full first, then close |
| `unauthorized` | `close_credit_line` called by a third party (not admin or borrower) | Use the correct identity (`--source admin` or `--source borrower`) |
| `amount must be positive` | Draw or repay called with zero or negative amount | Pass a positive `i128` value |
| `Insufficient liquidity reserve` | Reserve balance < requested draw amount | Top up the reserve address with the liquidity token before drawing |
| `credit_limit cannot be less than utilized amount` | `update_risk_parameters` tried to lower limit below current debt | Repay first, or set `credit_limit` ≥ current `utilized_amount` |
| `interest_rate_bps exceeds maximum` | Rate > 10000 bps (100%) | Use a value in 0–10000 |
| `risk_score exceeds maximum` | Score > 100 | Use a value in 0–100 |
| `admin not set` | Contract called before `init` | Call `init` with the intended admin address |
| `reentrancy guard` | Re-entrant call into `draw_credit` or `repay_credit` | This is a contract bug in an integration; do not retry — investigate the calling contract |

**Transaction reverts are atomic.** If any panic occurs mid-execution, all state
changes in that transaction are rolled back. `utilized_amount`, `status`, and
storage are unchanged. It is always safe to retry after fixing the input.

**`repay_credit` does not yet transfer tokens.** The current implementation
updates `utilized_amount` in state only. Do not rely on it for token settlement
in production until the token transfer is implemented.

---

## 11. Error Reference

| Code | Variant | Description |
|---|---|---|
| 1 | `Unauthorized` | Caller not authorized |
| 2 | `NotAdmin` | Admin-only function called by non-admin |
| 3 | `CreditLineNotFound` | No credit line for borrower |
| 4 | `CreditLineClosed` | Operation on a closed line |
| 5 | `InvalidAmount` | Zero or negative amount |
| 6 | `OverLimit` | Draw exceeds available credit |
| 7 | `NegativeLimit` | Credit limit is negative |
| 8 | `RateTooHigh` | Rate change exceeds max delta |
| 9 | `ScoreTooHigh` | Risk score > 100 |
| 10 | `UtilizationNotZero` | Close attempted with outstanding balance |
| 11 | `Reentrancy` | Re-entrant call detected |
| 12 | `Overflow` | Arithmetic overflow |

---

## 12. Security Notes

**Key hygiene** — All `--source` flags in this guide use named identities from
the Stellar CLI keystore (`stellar keys generate`). Never pass a raw secret key
as a CLI argument or store it in a `.env` file committed to version control.

**Testnet vs Mainnet** — Replace `--network testnet` with `--network mainnet`
for production. Testnet state is periodically reset; do not use Testnet
addresses or contract IDs in production configs.

**Liquidity reserve authorization** — Before any draw can succeed, the reserve
address must have authorized the contract to transfer tokens on its behalf via
the SAC `approve` call. Failure to do this will cause every `draw_credit` to
panic with `Insufficient liquidity reserve`.

**Rate-change limits** — Call `set_rate_change_limits` before opening credit
lines in production. Without it, an admin (or compromised admin key) can change
interest rates by any amount in a single transaction.

---

## 13. Running Tests & Coverage

```bash
# Run all 80 unit tests
cargo test -p creditra-credit --lib

# Coverage check (requires cargo-llvm-cov)
cargo llvm-cov --workspace --all-targets --fail-under-lines 95
```

**Test summary (80 tests, 0 failed):**

| Category | Tests |
|---|---|
| `open_credit_line` | 6 (valid, duplicate, zero/negative limit, rate/score bounds, re-open after close) |
| `draw_credit` | 12 (single/multi draw, exact limit, zero/negative amount, closed/exceeded, suspended, defaulted, liquidity) |
| `repay_credit` | 12 (partial, full, overpayment, zero utilization, suspended, defaulted, closed, invalid amounts, nonexistent) |
| `update_risk_parameters` | 8 (success, unauthorized, nonexistent, below utilized, negative limit, rate/score max, boundaries, event) |
| `set/get_rate_change_limits` | 3 (set+get, unauthorized, zero interval) |
| Status transitions | 10 (suspend, default, reinstate, close variants, suspended→defaulted) |
| Reentrancy guard | 2 (cleared after draw, cleared after repay) |
| Liquidity integration | 5 (sufficient, insufficient, external source, token auth) |
| Misc | 2 (multiple borrowers, get_credit_line returns None) |
