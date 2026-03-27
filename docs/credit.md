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

| From | To | Trigger | Authorization |
|------|-----|--------|----------------|
| Active | Defaulted | Admin calls `default_credit_line` (e.g. after past-due or oracle signal). | Admin |
| Active | Suspended | Admin calls `suspend_credit_line`. | Admin |
| Active | Closed | Admin or borrower (when `utilized_amount == 0`) calls `close_credit_line`. | Admin or borrower (zero utilization) |
| Suspended | Defaulted | Admin calls `default_credit_line`. | Admin |
| Suspended | Active | Admin calls `reinstate_credit_line`. | Admin |
| Suspended | Closed | Admin or borrower (when `utilized_amount == 0`) calls `close_credit_line`. | Admin or borrower (zero utilization) |
| Defaulted | Active | Admin calls `reinstate_credit_line`. | Admin |
| Defaulted | Suspended | Admin calls `suspend_credit_line`. | Admin |
| Defaulted | Closed | Admin or borrower (when `utilized_amount == 0`) calls `close_credit_line`. | Admin or borrower (zero utilization) |

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

### `update_risk_parameters(env, borrower, credit_limit, interest_rate_bps, risk_score)`
Update the risk parameters for an existing credit line. Admin-only.

| Parameter | Type | Description |
|---|---|---|
| `borrower` | `Address` | Borrower whose credit line to update |
| `credit_limit` | `i128` | New credit limit (must be ≥ current `utilized_amount`) |
| `interest_rate_bps` | `u32` | New interest rate in basis points (0–10000) |
| `risk_score` | `u32` | New risk score (0–100) |

#### Rate-change limits (optional, backward-compatible)
When a `RateChangeConfig` has been set via `set_rate_change_limits`, the following
checks are enforced **only when the interest rate is actually changing**:

- The absolute delta `|new_rate - old_rate|` must be ≤ `max_rate_change_bps`.
- If `last_rate_update_ts > 0` and `rate_change_min_interval > 0`, the elapsed
  time since the last rate change must be ≥ `rate_change_min_interval`.
- If the rate is **unchanged**, both checks are skipped entirely.
- If **no config is set**, no limits are enforced (fully backward-compatible).

On a successful rate change, `last_rate_update_ts` is updated to the current
ledger timestamp.

#### Errors
| Condition | Panic message |
|---|---|
| Caller is not admin | Auth error |
| Credit line not found | `ContractError::CreditLineNotFound` |
| `credit_limit < utilized_amount` | `ContractError::OverLimit` |
| `credit_limit < 0` | `ContractError::NegativeLimit` |
| `interest_rate_bps > 10000` | `ContractError::RateTooHigh` |
| `risk_score > 100` | `ContractError::ScoreTooHigh` |
| Rate delta exceeds max | `"rate change exceeds maximum allowed delta"` |
| Too soon since last change | `"rate change too soon: minimum interval not elapsed"` |

Emits: `RiskParametersUpdatedEvent` with borrower, new credit limit, new rate, new score.

#### Security notes
- Rate-change config is optional and stored in instance storage.
- Absence of config means **no limits** — fully backward-compatible.
- `last_rate_update_ts = 0` (never updated) always bypasses the interval check,
  so the first rate change is never blocked by the time window.
- The delta check uses `abs_diff` which is symmetric and overflow-safe.

### `suspend_credit_line(env, borrower)`
Suspends an active credit line. Called by admin.

Panics if the credit line does not exist.  
Emits: `("credit", "suspend")` event.

---

### `close_credit_line(env, borrower, closer)`
Closes a credit line permanently. Can be called by admin (force-close) or by borrower when `utilized_amount` is 0. Allowed from Active, Suspended, or Defaulted states.

| Parameter | Type | Description |
|---|---|---|
| `borrower` | `Address` | Borrower whose credit line to close |
| `closer` | `Address` | Address authorized to close (admin or borrower) |

#### Authorization Rules
- **Admin**: Can close any credit line regardless of utilization amount
- **Borrower**: Can only close when `utilized_amount == 0`

#### State Transitions
- **Active → Closed**: Admin (any utilization) or borrower (zero utilization)
- **Suspended → Closed**: Admin (any utilization) or borrower (zero utilization)  
- **Defaulted → Closed**: Admin (any utilization) or borrower (zero utilization)
- **Closed → Closed**: Idempotent (no effect)

#### Behavior
- Sets credit line status to `Closed` permanently
- Preserves `utilized_amount` for admin closes (force-close with outstanding debt)
- Emits `("credit", "closed")` event with final credit line data
- No further draws allowed after closing
- Repayment operations are also blocked after closing

Panics if the credit line does not exist, if `closer` is not authorized, or if borrower attempts to close with non-zero utilization.

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

## Error Codes

The `Credit` contract uses standard `u32` discriminants for standardized error handling across the Rust and TypeScript SDK clients. Integrator clients can match these error codes to understand failure reasons.

| Error Code | Variant | Description |
|---|---|---|
| `1` | `Unauthorized` | Caller is not authorized to perform this action. |
| `2` | `NotAdmin` | Caller does not have admin privileges. |
| `3` | `CreditLineNotFound` | The specified credit line was not found. |
| `4` | `CreditLineClosed` | Action cannot be performed because the credit line is closed. |
| `5` | `InvalidAmount` | The requested amount is invalid (e.g., zero or negative). |
| `6` | `OverLimit` | The requested draw exceeds the available credit limit. |
| `7` | `NegativeLimit` | The credit limit cannot be negative. |
| `8` | `RateTooHigh` | The interest rate change exceeds the maximum allowed delta. |
| `9` | `ScoreTooHigh` | The risk score is above the acceptable maximum threshold. |
| `10` | `UtilizationNotZero` | Action cannot be performed because the credit line utilization is not zero. |
| `11` | `Reentrancy` | Reentrancy detected during cross-contract calls. |
| `12` | `Overflow` | Math overflow occurred during calculation. |

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