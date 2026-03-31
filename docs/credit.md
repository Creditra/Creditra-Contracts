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
| `accrued_interest`   | `i128`   | Cumulative capitalized interest recorded on the line |
| `last_accrual_ts`    | `u64`    | Ledger timestamp of the last interest accrual checkpoint (0 = never accrued) |

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
| `Restricted` | 4   | Limit is below utilization; additional draws are blocked until cured |

### Status transitions

| From       | To         | Trigger |
|------------|------------|---------|
| Active     | Suspended  | Admin calls `suspend_credit_line` |
| Active     | Defaulted  | Admin calls `default_credit_line` |
| Suspended  | Defaulted  | Admin calls `default_credit_line` |
| Defaulted  | Active     | Admin calls `reinstate_credit_line` |
| Defaulted  | Closed     | Admin or borrower (when `utilized_amount == 0`) calls `close_credit_line` |
| Active     | Closed     | Admin or borrower (when `utilized_amount == 0`) calls `close_credit_line` |
| Suspended  | Closed     | Admin or borrower (when `utilized_amount == 0`) calls `close_credit_line` |

When status is **Defaulted**: `draw_credit` is disabled; `repay_credit` is still allowed.

---

## Methods

### `init(env, admin)`
Initializes the contract with an admin address. Must be called exactly once.

- Stores `admin` in instance storage under the `"admin"` key.
- Sets `LiquiditySource` to the contract's own address as a deterministic default.
- Reverts with `ContractError::AlreadyInitialized` (14) if called a second time, preventing admin takeover via re-initialization.

#### Parameters
| Parameter | Type | Description |
|---|---|---|
| `admin` | `Address` | Address that will hold admin authority over this contract |

#### Errors
| Condition | Error |
|---|---|
| Contract already initialized | `ContractError::AlreadyInitialized` (14) |

#### Security notes
- Must be called by the deployer immediately after deployment.
- The guard checks for the presence of the `"admin"` key before writing; no storage is mutated on a rejected second call.
- The admin address is immutable after initialization. See the Admin Rotation Proposal section for a safe rotation design.
- `LiquiditySource` defaults to the contract address and can be updated post-init via `set_liquidity_source` (admin only).

### `set_liquidity_token(env, token_address)`
Sets the Stellar Asset Contract token used for draws and repayments (admin only).

- Writes the token contract address to instance storage under `DataKey::LiquidityToken`.
- Only the configured admin may update this value; unauthorized callers fail auth before storage is mutated.
- Covered by unit tests in `contracts/credit/src/lib.rs` for both successful admin updates and rejected non-admin calls.

### `set_liquidity_source(env, reserve_address)`
Sets the address that holds liquidity for draws and receives repayments (defaults to contract address).

### `open_credit_line(env, borrower, credit_limit, interest_rate_bps, risk_score)`
Opens a new credit line for a borrower. Called by the backend or risk engine.

| Parameter | Type | Description |
|---|---|---|
| `borrower` | `Address` | Borrower's address |
| `credit_limit` | `i128` | Maximum drawable amount (must be > 0) |
| `interest_rate_bps` | `u32` | Annual interest rate in basis points (0–10000) |
| `risk_score` | `u32` | Risk score from the risk engine (0–100) |

`last_rate_update_ts`, `accrued_interest`, and `last_accrual_ts` are initialized to `0`.

#### Errors
| Condition | Error |
|---|---|
| `credit_limit <= 0` | `ContractError::InvalidAmount` |
| `interest_rate_bps > 10000` | `ContractError::RateTooHigh` |
| `risk_score > 100` | `ContractError::ScoreTooHigh` |
| Borrower already has an Active line | `ContractError::Unauthorized` |

Emits: `("credit", "opened")` event with a `CreditLineEvent` payload.

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

#### Credit Limit Decrease Behavior

When `credit_limit` is decreased below the current `utilized_amount`:

- The credit line status changes to **Restricted**
- `utilized_amount` remains unchanged (borrower must repay excess)
- `draw_credit` is disabled until excess is repaid or limit is increased
- A `("credit", "limit_dec")` event is emitted with details

When `credit_limit` is decreased but remains ≥ `utilized_amount`:

- The credit line remains **Active**
- Normal operation continues

When `credit_limit` is increased or unchanged:

- Normal behavior applies
- If currently Restricted, increasing limit above `utilized_amount` reactivates to **Active**

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

- Reverts if the line does not exist.
- Reverts unless the current status is `Active`.

Emits: `("credit", "suspend")` event.

### Interest accrual

Interest accrual fields exist in storage, but scheduled/lazy accrual logic is not yet active in the contract.

The intended implementation design is documented separately in [`docs/interest-accrual.md`](interest-accrual.md).

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

## Overflow Policy

Arithmetic paths that affect credit limit and utilization stay in integer-only arithmetic.

- `draw_credit`: utilization update uses `checked_add`; arithmetic overflow reverts with `ContractError::Overflow` (`12`).
- `repay_credit`: inputs must be positive integers; the contract computes `effective_repay = min(amount, utilized_amount)` and then applies an integer floor at zero with `saturating_sub`. This keeps utilization non-negative even for over-repayments.
- `update_risk_parameters`: limit/risk bounds are validated before state updates; rate delta uses `abs_diff` for overflow-safe unsigned distance checks.

### Integer arithmetic assumptions

- Amounts and limits are stored as whole-number `i128` values; there is no fractional accounting or rounding path inside the contract.
- `open_credit_line` requires a positive limit, and `draw_credit` / `repay_credit` both reject non-positive amounts at the contract boundary.
- Because `repay_credit` caps the applied amount to current utilization before subtraction, repayment paths preserve the invariant `0 <= utilized_amount`.
- While a line is `Active`, draw paths also preserve `utilized_amount <= credit_limit`; dedicated invariant tests cover repeated draw and repay sequences across status changes.

### Large-number test coverage

The contract test suite includes explicit large-value coverage:

- `test_draw_credit_near_i128_max_succeeds_without_overflow`
- `test_draw_credit_overflow_reverts_with_defined_error`
- `test_draw_credit_large_values_exceed_limit_reverts_with_defined_error`
- `test_repay_credit_large_amount_caps_at_zero_without_underflow`
- `utilization_stays_bounded_across_active_scenarios`
- `utilization_never_goes_negative_after_repays_across_statuses`
- `test_update_risk_parameters_rejects_limit_below_utilized_near_i128_max`

These tests validate behavior near `i128::MAX` and confirm overflow handling remains deterministic.

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
| `13`       | `LimitDecreaseRequiresRepayment` | Credit limit decrease requires immediate repayment of excess amount. |
| `14`       | `AlreadyInitialized` | Contract has already been initialized; `init` may only be called once.      |

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

The contract also emits additive v2 event topics (for indexer analytics fields
like actor/source/timestamp identifiers) while keeping v1 payloads stable. See
[`docs/indexer-integration.md`](indexer-integration.md) for full topic mapping.

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

### Related Admin Workflows

- Default lifecycle: `default_credit_line` → optional `suspend_credit_line` containment → `reinstate_credit_line` or `close_credit_line`.
- Oracle-assisted default design: `docs/default-oracle.md`.

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
| `"admin"`            | Instance   | Admin `Address` (written once; re-init reverts) |
| `borrower: Address`  | Persistent | `CreditLineData`          |
| `"rate_cfg"`         | Instance   | `RateChangeConfig` (optional) |
| `"reentrancy"`       | Instance   | Reentrancy guard (internal) |
| `DataKey::LiquiditySource` | Instance | Reserve `Address` (defaults to contract address) |
| `DataKey::LiquidityToken`  | Instance | Token `Address` (optional) |

---

## Deployment Playbook

This section covers deploying the credit contract to Stellar testnet and invoking its core methods. All examples use the [Stellar CLI](https://developers.stellar.org/docs/tools/developer-tools/cli/stellar-cli) (`stellar`).

### Prerequisites

- Rust with `wasm32-unknown-unknown` target: `rustup target add wasm32-unknown-unknown`
- Stellar CLI installed: `cargo install --locked stellar-cli --features opt`
- A funded testnet identity (never commit private keys)

### 1. Identity setup

```bash
# Generate a new keypair and store it locally under an alias
stellar keys generate --global admin --network testnet

# Fund it via Friendbot
stellar keys fund admin --network testnet

# Confirm the address
stellar keys address admin
```

For the backend/risk-engine identity used to open credit lines:

(Examples unchanged — still valid)

---

## Running Tests

```bash
cargo test -p creditra-credit
```

---

## Appendix: Storage Key Audit

### Instance Storage

Keys that share the contract instance TTL. If the instance is archived, all
these keys are lost. Production deployments should call
`env.storage().instance().extend_ttl()` periodically.

| Key | Rust type | Value type | Written by | Notes |
|-----|-----------|------------|------------|-------|
| `Symbol("admin")` | `Symbol` | `Address` | `init` | Contract admin. Written exactly once; second write reverts with `AlreadyInitialized`. |
| `DataKey::LiquidityToken` | `DataKey` | `Address` | `set_liquidity_token` | Token contract for reserve/draw transfers. |
| `DataKey::LiquiditySource` | `DataKey` | `Address` | `init`, `set_liquidity_source` | Reserve address. Defaults to contract address. |
| `Symbol("reentrancy")` | `Symbol` | `bool` | `set_reentrancy_guard`, `clear_reentrancy_guard` | Defense-in-depth flag. Cleared on every code path. |
| `Symbol("rate_cfg")` | `Symbol` | `RateChangeConfig` | `set_rate_change_limits` | Admin-configurable rate-change governance. |

**Why instance?** These are global singleton configuration values. There is
exactly one admin, one liquidity token, one liquidity source, and one rate
config per contract deployment. Instance storage is correct.

### Persistent Storage

Per-borrower records with independent TTL per entry.

| Key | Rust type | Value type | Written by | Notes |
|-----|-----------|------------|------------|-------|
| Borrower `Address` | `Address` | `CreditLineData` | `open_credit_line`, `draw_credit`, `repay_credit`, `update_risk_parameters`, status transitions | Long-lived borrower data. Independent TTL. |

**Why persistent?** Each borrower's credit line must survive beyond a single
transaction and has an independent lifecycle. Persistent is correct. If a
borrower's entry TTL expires (archival), their credit line data is lost —
production deployments should bump TTL on access or via a keeper.

### Temporary Storage

Not currently used. Future candidate: the reentrancy guard could move to
temporary storage since it only needs to survive within a single invocation.
Instance storage works correctly today because it is always cleared.

### Audit Findings

1. **Admin** — correctly on instance. Single value, global.
2. **LiquidityToken / LiquiditySource** — correctly on instance. Global config.
3. **Reentrancy flag** — correctly on instance (cleared every call). Could
   optionally move to temporary storage for cleaner semantics.
4. **Rate config** — correctly on instance. Global governance parameter.
5. **Borrower records** — correctly on persistent. Per-entity, long-lived.
6. **No borrower data on instance** — verified. No volatile/instance keys are
   used for per-borrower data.
7. **TTL management** — not yet implemented. Recommend adding
   `extend_ttl()` calls on instance (in `init` or a dedicated `bump` endpoint)
   and on persistent (on credit line access) before production deployment.

You can also run all workspace tests from the repository root with `cargo test`.
