# Credit Contract Documentation

The `Credit` contract implements on-chain credit lines for the Creditra protocol on Stellar Soroban. It manages the full lifecycle of a borrower's credit line — from opening to closing or defaulting — and emits events at each stage.

---

## Data Model

### `CreditLineData`
Stored in persistent storage keyed by the borrower's address.

| Field | Type | Description |
|---|---|---|
| `borrower` | `Address` | The borrower's Stellar address |
| `credit_limit` | `i128` | Maximum amount the borrower can draw |
| `utilized_amount` | `i128` | Amount currently drawn |
| `interest_rate_bps` | `u32` | Annual interest rate in basis points (e.g. 300 = 3%) |
| `risk_score` | `u32` | Risk score assigned by the risk engine (0–100) |
| `status` | `CreditStatus` | Current status of the credit line |
| `last_rate_update_ts` | `u64` | Ledger timestamp of the last interest-rate change (0 = never updated) |

### `RateChangeConfig`
Stored in instance storage under the `"rate_cfg"` key. Optional — when absent, no rate-change limits are enforced (backward-compatible).

| Field | Type | Description |
|---|---|---|
| `max_rate_change_bps` | `u32` | Maximum absolute change in `interest_rate_bps` allowed per update |
| `rate_change_min_interval` | `u64` | Minimum elapsed seconds between consecutive rate changes |

### `CreditStatus`

| Variant | Value | Description |
|---|---|---|
| `Active` | 0 | Credit line is open and available |
| `Suspended` | 1 | Credit line is temporarily suspended |
| `Defaulted` | 2 | Borrower has defaulted; draw disabled, repay allowed |
| `Closed` | 3 | Credit line has been closed |

### Status transitions

| From | To | Trigger |
|------|-----|--------|
| Active | Defaulted | Admin calls `default_credit_line` (e.g. after past-due or oracle signal). |
| Suspended | Defaulted | Admin calls `default_credit_line`. |
| Defaulted | Active | Admin calls `reinstate_credit_line`. |
| Defaulted | Suspended | Admin calls `suspend_credit_line`. |
| Defaulted | Closed | Admin or borrower (when `utilized_amount == 0`) calls `close_credit_line`. |

When status is **Defaulted**: `draw_credit` is disabled; `repay_credit` is allowed.

### `CreditLineEvent`
Emitted on every lifecycle state change.

| Field | Type | Description |
|---|---|---|
| `event_type` | `Symbol` | Short symbol identifying the event |
| `borrower` | `Address` | The affected borrower |
| `status` | `CreditStatus` | New status after the event |
| `credit_limit` | `i128` | Credit limit at time of event |
| `interest_rate_bps` | `u32` | Interest rate at time of event |
| `risk_score` | `u32` | Risk score at time of event |

---

## Methods

### `init(env, admin)`
Initializes the contract with an admin address. Must be called once before any other function.

| Parameter | Type | Description |
|---|---|---|
| `admin` | `Address` | Address authorized for admin operations |

---

### `open_credit_line(env, borrower, credit_limit, interest_rate_bps, risk_score)`
Opens a new credit line for a borrower. Called by the backend or risk engine.

| Parameter | Type | Description |
|---|---|---|
| `borrower` | `Address` | Borrower's address |
| `credit_limit` | `i128` | Maximum drawable amount |
| `interest_rate_bps` | `u32` | Interest rate in basis points |
| `risk_score` | `u32` | Risk score from the risk engine |

Emits: `("credit", "opened")` event.

---

### `draw_credit(env, borrower, amount)`
Draw funds from an active credit line. Requires status **Active**; reverts if status is Suspended, Defaulted, or Closed.

| Parameter | Type | Description |
|---|---|---|
| `borrower` | `Address` | Borrower's address |
| `amount` | `i128` | Amount to draw |

Emits: `("credit", "draw")` and drawn event. Transfers protocol token from reserve to borrower.
Draw funds from an active credit line. Verifies limit, updates utilized amount, and transfers the protocol token from the contract reserve to the borrower. Caller must be the borrower and must authorize.

---

### `repay_credit(env, borrower, amount)`
Repay drawn funds. Allowed when status is **Active**, **Suspended**, or **Defaulted**. Reverts if credit line does not exist, is Closed, or borrower has not authorized.

| Parameter | Type | Description |
|---|---|---|
| `borrower` | `Address` | Borrower's address |
| `amount` | `i128` | Amount to repay |

Emits: `("credit", "repay")` event. Reduces `utilized_amount` (capped at zero).
Repay drawn funds. The borrower must transfer the repayment amount from their account to the contract reserve via the Stellar token contract. The transfer is executed before any state change; if the transfer fails (e.g. insufficient balance or missing authorization), the call reverts and `utilized_amount` is unchanged. The amount applied is capped at the current utilized amount.

| Parameter | Type | Description |
|---|---|---|
| `borrower` | `Address` | Borrower (must authorize the call and token transfer) |
| `amount` | `i128` | Nominal repayment; effective transfer is min(amount, utilized_amount) |

Emits: `("credit", "repay")` with `RepaymentEvent` (borrower, amount actually transferred, new utilized amount, timestamp).

---

### `update_risk_parameters(env, borrower, interest_rate_bps, risk_score)`
Update the risk parameters for an existing credit line. Called by admin or risk engine.

When a `RateChangeConfig` is set:
- The absolute delta between the current and new `interest_rate_bps` must be ≤ `max_rate_change_bps`.
- If `last_rate_update_ts > 0`, the elapsed time since the last change must be ≥ `rate_change_min_interval`.
- If the rate is unchanged, both checks are skipped.

Panics with `"rate change exceeds maximum allowed delta"` or `"rate change too soon: minimum interval not elapsed"` on violation.

---

### `suspend_credit_line(env, borrower)`
Suspends an active credit line. Called by admin.

Panics if the credit line does not exist.  
Emits: `("credit", "suspend")` event.

---

### `close_credit_line(env, borrower, closer)`
Closes a credit line. Can be called by admin (force-close) or by borrower when `utilized_amount` is 0. Allowed from Active, Suspended, or Defaulted.

Panics if the credit line does not exist.  
Emits: `("credit", "closed")` event.

---

### `default_credit_line(env, borrower)`
Marks a credit line as defaulted. Called by admin when the line is past due or when an oracle/off-chain signal indicates default. Transition: Active or Suspended → Defaulted. After this, `draw_credit` is disabled and `repay_credit` remains allowed.

Panics if the credit line does not exist.  
Emits: `("credit", "default")` event.

---

### `reinstate_credit_line(env, borrower)`
Reinstates a defaulted credit line to Active. Admin only. Allowed only when status is Defaulted. Transition: Defaulted → Active.

Panics if the credit line does not exist or status is not Defaulted.  
Emits: `("credit", "reinstate")` event.
### `set_rate_change_limits(env, max_rate_change_bps, rate_change_min_interval)`
Sets the global rate-change limits. Admin-only.

| Parameter | Type | Description |
|---|---|---|
| `max_rate_change_bps` | `u32` | Maximum BPS delta per update |
| `rate_change_min_interval` | `u64` | Minimum seconds between rate changes |

---

### `get_rate_change_limits(env) -> RateChangeConfig`
Returns the current `RateChangeConfig`. Panics if none is set.

---

### `get_credit_line(env, borrower) -> Option<CreditLineData>`
Returns the credit line data for a borrower, or `None` if not found. View function — does not modify state.

---

## Events

| Topic | Event Type Symbol | Emitted By | Description |
|---|---|---|---|
| `("credit", "opened")` | `opened` | `open_credit_line` | New credit line opened |
| `("credit", "repay")` | `repay` | `repay_credit` | Repayment (borrower, amount, new utilized, timestamp) |
| `("credit", "suspend")` | `suspend` | `suspend_credit_line` | Credit line suspended |
| `("credit", "closed")` | `closed` | `close_credit_line` | Credit line closed |
| `("credit", "default")` | `default` | `default_credit_line` | Credit line defaulted |
| `("credit", "reinstate")` | `reinstate` | `reinstate_credit_line` | Credit line reinstated to Active |

---

## Access Control

| Function | Caller |
|---|---|
| `init` | Deployer (once) |
| `open_credit_line` | Backend / risk engine |
| `draw_credit` | Borrower |
| `repay_credit` | Borrower |
| `update_risk_parameters` | Admin / risk engine |
| `suspend_credit_line` | Admin |
| `close_credit_line` | Admin or borrower |
| `default_credit_line` | Admin |
| `reinstate_credit_line` | Admin |
| `set_rate_change_limits` | Admin |
| `get_rate_change_limits` | Anyone (view) |
| `get_credit_line` | Anyone (view) |

> Note: On-chain authorization via `require_auth()` is not yet enforced in all functions. This is planned for a future release.

---

## Interest Model

Interest is expressed in basis points (`interest_rate_bps`). For example:
- `300` = 3% annual interest
- `500` = 5% annual interest

Interest accrual logic is not yet implemented (`repay_credit` is a placeholder). When implemented, interest will accrue on the `utilized_amount` over time.

---

## Storage

| Key | Storage Type | Value |
|---|---|---|
| `"admin"` | Instance | `Address` |
| `borrower: Address` | Persistent | `CreditLineData` |
| `"rate_cfg"` | Instance | `RateChangeConfig` |

---

## Deployment and CLI Usage

### Build
```bash
cargo build --target wasm32-unknown-unknown --release
```

### Deploy
```bash
soroban contract deploy \
  --wasm target/wasm32-unknown-unknown/release/credit.wasm \
  --source <your-keypair> \
  --network testnet
```

### Initialize
```bash
soroban contract invoke \
  --id <contract-id> \
  --source <admin-keypair> \
  --network testnet \
  -- init \
  --admin <admin-address>
```

### Open a Credit Line
```bash
soroban contract invoke \
  --id <contract-id> \
  --source <backend-keypair> \
  --network testnet \
  -- open_credit_line \
  --borrower <borrower-address> \
  --credit_limit 5000 \
  --interest_rate_bps 300 \
  --risk_score 75
```

### Get Credit Line
```bash
soroban contract invoke \
  --id <contract-id> \
  --network testnet \
  -- get_credit_line \
  --borrower <borrower-address>
```

### Suspend / Close / Default
```bash
soroban contract invoke --id <contract-id> --source <admin-keypair> --network testnet -- suspend_credit_line --borrower <borrower-address>
soroban contract invoke --id <contract-id> --source <admin-keypair> --network testnet -- close_credit_line --borrower <borrower-address>
soroban contract invoke --id <contract-id> --source <admin-keypair> --network testnet -- default_credit_line --borrower <borrower-address>
soroban contract invoke --id <contract-id> --source <admin-keypair> --network testnet -- reinstate_credit_line --borrower <borrower-address>
```

---

## Running Tests
```bash
cargo test
```