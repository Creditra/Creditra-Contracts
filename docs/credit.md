  # Credit Contract Documentation

**Version: 2026-03-26**

The `Credit` contract implements on-chain credit lines for the Creditra protocol on Stellar Soroban. It manages the full lifecycle of a borrower's credit line — from opening to closing or defaulting — and emits events at each stage.

For indexer-specific event ingestion and decoding guidance, see `docs/indexer-integration.md`.

---

## Data Model

### `CreditLineData`

Stored in persistent storage keyed by the borrower's address.

| Field                | Type     | Description |
|----------------------|----------|-----------|
| `borrower`           | `Address` | The borrower's Stellar address |
| `credit_limit`       | `i128`   | Maximum amount the borrower can draw |
| `utilized_amount`    | `i128`   | Amount currently drawn |
| `interest_rate_bps`  | `u32`    | Annual interest rate in basis points (e.g. 300 = 3%) |
| `risk_score`         | `u32`    | Risk score assigned by the risk engine (0–100) |
| `status`             | `CreditStatus` | Current status of the credit line |
| `last_rate_update_ts`| `u64`    | Ledger timestamp of the last interest-rate change (0 = never updated) |

### `RateChangeConfig`
Stored in instance storage under the `"rate_cfg"` key. Optional — when absent, no rate-change limits are enforced.

| Field                     | Type  | Description |
|---------------------------|-------|-----------|
| `max_rate_change_bps`     | `u32` | Maximum absolute change in `interest_rate_bps` allowed per update |
| `rate_change_min_interval`| `u64` | Minimum elapsed seconds between consecutive rate changes |

### `CreditStatus`

| Variant    | Value | Description |
|------------|-------|-----------|
| `Active`   | 0     | Credit line is open and available |
| `Suspended`| 1     | Credit line is temporarily suspended |
| `Defaulted`| 2     | Borrower has defaulted; draw disabled, repay allowed |
| `Closed`   | 3     | Credit line has been permanently closed |

### Status transitions

| From       | To         | Trigger |
|------------|------------|---------|
| Active     | Defaulted  | Admin calls `default_credit_line` |
| Suspended  | Defaulted  | Admin calls `default_credit_line` |
| Defaulted  | Active     | Admin calls `reinstate_credit_line` |
| Defaulted  | Suspended  | Admin calls `suspend_credit_line` |
| Defaulted  | Closed     | Admin or borrower (when `utilized_amount == 0`) calls `close_credit_line` |

When status is **Defaulted**: `draw_credit` is disabled; `repay_credit` is still allowed.

---

## Methods

### `init(env, admin)`
Initializes the contract with an admin address. Must be called exactly once.

### `set_liquidity_token(env, token_address)`
Sets the Stellar Asset Contract token used for draws and repayments (admin only).

### `set_liquidity_source(env, reserve_address)`
Sets the address that holds liquidity for draws and receives repayments (defaults to contract address).

### `open_credit_line(env, borrower, credit_limit, interest_rate_bps, risk_score)`
Opens a new credit line for a borrower. Called by the backend/risk engine.

Emits: `("credit", "opened")` event.

### `draw_credit(env, borrower, amount)`
Draw funds from an **Active** credit line. Caller must be the borrower.

- Reverts if line is Closed, Suspended, Defaulted, or does not exist.
- Reverts if draw would exceed `credit_limit`.
- Transfers tokens from liquidity source → borrower.

Emits: `("credit", "drawn")` event.

### `repay_credit(env, borrower, amount)`
Repay outstanding drawn funds.

**Allowed on**: Active, Suspended, or Defaulted credit lines.  
**Not allowed on**: Closed credit lines.

- The borrower must have approved the contract to pull tokens via `transfer_from`.
- Effective repayment = `min(amount, utilized_amount)` (over-payments are safe).
- Tokens are transferred **before** state is updated. If the transfer fails, the call reverts with no state change.
- Works even when no liquidity token is configured (state-only update).

Emits: `("credit", "repay")` event with `RepaymentEvent` payload containing the effective amount transferred and new `utilized_amount`.

### `update_risk_parameters(env, borrower, credit_limit, interest_rate_bps, risk_score)`
Update credit limit, interest rate, and risk score (admin only).

When `RateChangeConfig` is set, rate changes are subject to:
- Maximum delta ≤ `max_rate_change_bps`
- Minimum time interval ≥ `rate_change_min_interval`

Emits: `("credit", "risk_updated")` event.

### `set_rate_change_limits(env, max_rate_change_bps, rate_change_min_interval)`
Configure rate-change limits (admin only).

### `get_rate_change_limits(env) -> Option<RateChangeConfig>`
Returns the current rate-change configuration (or `None` if not set).
### `update_risk_parameters(env, borrower, credit_limit, interest_rate_bps, risk_score)`

Update the risk parameters for an existing credit line. Admin-only.

| Parameter           | Type      | Description                                            |
| ------------------- | --------- | ------------------------------------------------------ |
| `borrower`          | `Address` | Borrower whose credit line to update                   |
| `credit_limit`      | `i128`    | New credit limit (must be ≥ current `utilized_amount`) |
| `interest_rate_bps` | `u32`     | New interest rate in basis points (0–10000)            |
| `risk_score`        | `u32`     | New risk score (0–100)                                 |

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

| Condition                        | Panic message                                          |
| -------------------------------- | ------------------------------------------------------ |
| Caller is not admin              | Auth error                                             |
| Credit line not found            | `ContractError::CreditLineNotFound`                    |
| `credit_limit < utilized_amount` | `ContractError::OverLimit`                             |
| `credit_limit < 0`               | `ContractError::NegativeLimit`                         |
| `interest_rate_bps > 10000`      | `ContractError::RateTooHigh`                           |
| `risk_score > 100`               | `ContractError::ScoreTooHigh`                          |
| Rate delta exceeds max           | `"rate change exceeds maximum allowed delta"`          |
| Too soon since last change       | `"rate change too soon: minimum interval not elapsed"` |

Emits: `RiskParametersUpdatedEvent` with borrower, new credit limit, new rate, new score.

#### Security notes

- Rate-change config is optional and stored in instance storage.
- Absence of config means **no limits** — fully backward-compatible.
- `last_rate_update_ts = 0` (never updated) always bypasses the interval check,
  so the first rate change is never blocked by the time window.
- The delta check uses `abs_diff` which is symmetric and overflow-safe.

#### Ledger timestamp trust assumptions
- The cooldown window relies on `env.ledger().timestamp()` from the Soroban host.
- Production deployments therefore trust the network-provided ledger timestamp to be monotonic enough for coarse cooldown enforcement.
- This mechanism is suitable for protocol-level spacing of administrative rate changes, not for sub-second precision or wall-clock guarantees.
- Test coverage should explicitly exercise:
  - first update with `last_rate_update_ts == 0`
  - exactly-at-boundary acceptance
  - just-before-boundary rejection
  - `rate_change_min_interval == 0` disabling the timing gate entirely

### `suspend_credit_line(env, borrower)`
Suspend an Active credit line (admin only).

Emits: `("credit", "suspend")` event.

### `close_credit_line(env, borrower, closer)`
Close a credit line.

- Admin can close any time.
- Borrower can close only when `utilized_amount == 0`.

Emits: `("credit", "closed")` event.

### `default_credit_line(env, borrower)`
Mark credit line as Defaulted (admin only).

Emits: `("credit", "default")` event.

### `reinstate_credit_line(env, borrower)`
Reinstate a Defaulted credit line to Active (admin only).

Emits: `("credit", "reinstate")` event.

### `get_credit_line(env, borrower) -> Option<CreditLineData>`
View function — returns credit line data or `None`.

---

## Error Codes

The `Credit` contract uses standard `u32` discriminants for standardized error handling across the Rust and TypeScript SDK clients. Integrator clients can match these error codes to understand failure reasons.

| Error Code | Variant              | Description                                                                 |
| ---------- | -------------------- | --------------------------------------------------------------------------- |
| `1`        | `Unauthorized`       | Caller is not authorized to perform this action.                            |
| `2`        | `NotAdmin`           | Caller does not have admin privileges.                                      |
| `3`        | `CreditLineNotFound` | The specified credit line was not found.                                    |
| `4`        | `CreditLineClosed`   | Action cannot be performed because the credit line is closed.               |
| `5`        | `InvalidAmount`      | The requested amount is invalid (e.g., zero or negative).                   |
| `6`        | `OverLimit`          | The requested draw exceeds the available credit limit.                      |
| `7`        | `NegativeLimit`      | The credit limit cannot be negative.                                        |
| `8`        | `RateTooHigh`        | The interest rate change exceeds the maximum allowed delta.                 |
| `9`        | `ScoreTooHigh`       | The risk score is above the acceptable maximum threshold.                   |
| `10`       | `UtilizationNotZero` | Action cannot be performed because the credit line utilization is not zero. |
| `11`       | `Reentrancy`         | Reentrancy detected during cross-contract calls.                            |
| `12`       | `Overflow`           | Math overflow occurred during calculation.                                  |

---

## Events

| Topic                      | Event Type | Emitted By                  | Description |
|----------------------------|------------|-----------------------------|-----------|
| `("credit", "opened")`     | `opened`   | `open_credit_line`          | New credit line created |
| `("credit", "drawn")`      | `drawn`    | `draw_credit`               | Funds drawn |
| `("credit", "repay")`      | `repay`    | `repay_credit`              | Repayment made |
| `("credit", "suspend")`    | `suspend`  | `suspend_credit_line`       | Line suspended |
| `("credit", "closed")`     | `closed`   | `close_credit_line`         | Line closed |
| `("credit", "default")`    | `default`  | `default_credit_line`       | Line defaulted |
| `("credit", "reinstate")`  | `reinstate`| `reinstate_credit_line`     | Line reinstated |
| `("credit", "risk_updated")`| `risk_updated` | `update_risk_parameters` | Risk parameters changed |

---

## Access Control

| Function                 | Caller                |
| ------------------------ | --------------------- |
| `init`                   | Deployer (once)       |
| `open_credit_line`       | Backend / risk engine |
| `draw_credit`            | Borrower              |
| `repay_credit`           | Borrower              |
| `update_risk_parameters` | Admin / risk engine   |
| `suspend_credit_line`    | Admin                 |
| `close_credit_line`      | Admin or borrower     |
| `default_credit_line`    | Admin                 |
| `reinstate_credit_line`  | Admin                 |
| `set_liquidity_token`    | Admin                 |
| `set_liquidity_source`   | Admin                 |
| `set_rate_change_limits` | Admin                 |
| `get_rate_change_limits` | Anyone (view)         |
| `get_credit_line`        | Anyone (view)         |

> Note: `open_credit_line` requires admin authorization (`require_auth`). The admin key is the backend/risk engine signer — borrowers cannot open their own credit lines.

---

## Admin Rotation Proposal

### Current risk

The current contract stores a single immutable admin address in instance storage. That keeps the access model simple, but it creates a high-impact operational risk:

- a deployment initialized with the wrong admin address is effectively unrecoverable
- an admin key compromise cannot be remediated on-chain
- key-rotation policies require redeployment instead of controlled handoff

### Recommended design

Use a **two-step admin rotation** instead of a one-call `transfer_admin`.

#### Proposed API

```rust
/// Propose a new admin. Callable only by the current admin.
pub fn propose_admin(env: Env, new_admin: Address);

/// Accept a pending admin role. Callable only by the pending admin.
pub fn accept_admin(env: Env);

/// Cancel a pending admin handoff. Callable only by the current admin.
pub fn cancel_admin_rotation(env: Env);

/// View the current pending admin, if any.
pub fn get_pending_admin(env: Env) -> Option<Address>;
```

#### Why two-step is preferred

A direct `transfer_admin(new_admin)` permanently changes authority in one call. That is efficient, but it increases wrong-address risk because:

- the current admin may submit the wrong destination address
- the destination may be a contract or wallet that cannot complete intended operations
- the protocol loses the ability to prove that the receiving operator actually controls the destination key

The two-step model lowers that risk because the recipient must explicitly accept the role.

### Storage additions

If implemented, add a new instance-storage slot:

| Key | Storage Type | Value |
|---|---|---|
| `"pending_admin"` | Instance | `Address` |

The `"admin"` slot remains authoritative until `accept_admin` succeeds.

### Threat model update

#### Assets protected

- admin authority over credit-line lifecycle operations
- admin authority over liquidity source/token configuration
- admin authority over risk-parameter changes

#### Trust boundaries

- the current `admin` is trusted to nominate a valid successor
- the `pending_admin` is trusted only after they successfully authenticate and accept
- observers and indexers may treat rotation events as security-relevant governance actions

#### Failure modes and mitigations

| Failure mode | Risk | Mitigation |
|---|---|---|
| Wrong address proposed | Permanent governance loss with one-step transfer | Two-step acceptance keeps current admin active until recipient confirms |
| Proposed admin never responds | Rotation stuck in pending state | `cancel_admin_rotation` allows admin to abort and retry |
| Current admin key compromise | Attacker can still propose a malicious admin | Not fully solvable on-chain; mitigated operationally by hardware wallets, monitoring, and fast cancellation if compromise is detected before acceptance |
| Malicious pending admin | Attempts to seize control without nomination | `accept_admin` must require `pending_admin.require_auth()` and exact match against stored pending admin |
| Event/indexing ambiguity | Off-chain systems misread control state | Emit explicit proposal / cancellation / acceptance events and document that only accepted admin is authoritative |

### Operational procedure

Recommended production workflow:

1. Current admin verifies the target address out of band.
2. Current admin calls `propose_admin(new_admin)`.
3. Off-chain monitoring confirms the pending-admin event and storage value.
4. Proposed admin verifies the contract ID and calls `accept_admin()`.
5. Monitoring confirms the old admin was replaced and `pending_admin` was cleared.
6. If the proposal was wrong or stale, current admin calls `cancel_admin_rotation()` before acceptance.

### Testing requirements for implementation

If/when implemented, the minimum invariant coverage should include:

- only current admin can call `propose_admin`
- only current admin can call `cancel_admin_rotation`
- only the exact pending admin can call `accept_admin`
- `admin` remains unchanged until acceptance
- `pending_admin` is cleared after acceptance or cancellation
- proposing the current admin should be rejected to avoid no-op ambiguity
- a missing pending admin should cause `accept_admin` to fail deterministically

### Implementation note

Given the sensitivity of governance handoff, a one-step `transfer_admin` should only be added if maintainers explicitly prefer operational simplicity over wrong-address protection. The safer default for this contract is the two-step rotation flow above.

---

## Interest Model

All sensitive functions enforce authorization via `require_auth()`.

---

## Storage

| Key                  | Type       | Value                     |
|----------------------|------------|---------------------------|
| `"admin"`            | Instance   | Admin `Address`           |
| `borrower: Address`  | Persistent | `CreditLineData`          |
| `"rate_cfg"`         | Instance   | `RateChangeConfig` (optional) |
| `"reentrancy"`       | Instance   | Reentrancy guard (internal) |

---

## Deployment and CLI Usage

(Examples unchanged — still valid)

---

## Running Tests

```bash
cargo test
```
