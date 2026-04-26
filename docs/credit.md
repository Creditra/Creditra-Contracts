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
| `Suspended`| 1     | Credit line is temporarily suspended; draws are blocked and repayments remain allowed |
| `Defaulted`| 2     | Borrower has defaulted; draw disabled, repay allowed |
| `Closed`   | 3     | Credit line has been permanently closed |
| `Restricted` | 4   | Limit is below utilization; additional draws are blocked until cured |

### Status transitions

| From       | To         | Trigger |
|------------|------------|---------|
| Active     | Suspended  | Admin calls `suspend_credit_line` |
| Active     | Suspended  | Borrower calls `self_suspend_credit_line` |
| Active     | Defaulted  | Admin calls `default_credit_line` |
| Suspended  | Defaulted  | Admin calls `default_credit_line` |
| Suspended  | Active     | Admin-approved `open_credit_line` reopen workflow |
| Defaulted  | Active     | Admin calls `reinstate_credit_line` |
| Defaulted  | Closed     | Admin or borrower (when `utilized_amount == 0`) calls `close_credit_line` |
| Active     | Closed     | Admin or borrower (when `utilized_amount == 0`) calls `close_credit_line` |
| Suspended  | Closed     | Admin or borrower (when `utilized_amount == 0`) calls `close_credit_line` |

When status is **Suspended** or **Defaulted**: `draw_credit` is disabled; `repay_credit` is still allowed.

---

## Methods

### `init(env, admin)`
Initializes the contract with an admin address. Must be called exactly once.

- Stores `admin` in instance storage under the `"admin"` key.
- Sets `LiquiditySource` to the contract's own address as a deterministic default.
- Sets `DataKey::SchemaVersion` to `1` in instance storage.
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
- Admin rotation is two-step (`propose_admin` then `accept_admin`) with an optional delay.
- `LiquiditySource` defaults to the contract address and can be updated post-init via `set_liquidity_source` (admin only).

### `propose_admin(env, new_admin, delay_seconds)`
Creates or overwrites a pending admin proposal (admin only).

- Stores `new_admin` under `"proposed_admin"` and acceptance timestamp under `"proposed_at"`.
- `delay_seconds = 0` allows immediate acceptance.
- A second proposal **overwrites** the previous pending proposal and its delay window.
- Emits `("credit", "admin_prop")` with `AdminRotationProposedEvent`.

### `accept_admin(env)`
Accepts a pending admin proposal (proposed admin only).

- Caller must be exactly the currently proposed admin.
- Reverts with `ContractError::AdminAcceptTooEarly` (15) if called before `"proposed_at"`.
- On success, updates `"admin"` and clears `"proposed_admin"`/`"proposed_at"`.
- Emits `("credit", "admin_acc")` with `AdminRotationAcceptedEvent`.

### `set_liquidity_token(env, token_address)`
Sets the Stellar Asset Contract token used for draws and repayments (admin only).

- Writes the token contract address to instance storage under `DataKey::LiquidityToken`.
- Only the configured admin may update this value; unauthorized callers fail auth before storage is mutated.
- Covered by unit tests in `contracts/credit/src/lib.rs` for both successful admin updates and rejected non-admin calls.

### `set_liquidity_source(env, reserve_address)`
Sets the address that holds liquidity for draws and receives repayments (defaults to contract address).

### `open_credit_line(env, borrower, credit_limit, interest_rate_bps, risk_score)`
Opens a new credit line for a borrower. Called by the backend or risk engine.

- Creating a brand-new line preserves the existing backend/risk-engine trust boundary.
- Re-opening any existing non-`Active` line now requires admin auth so a borrower cannot self-suspend and then reactivate themselves on-chain.

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
Draw funds from an **Active** credit line. Only the borrower is authorized to call this function.

- Reverts with `ContractError::Unauthorized` (1) if caller is not the borrower.
- Reverts with `ContractError::CreditLineNotFound` (3) if no line exists.
- Reverts with `ContractError::CreditLineSuspended` (18), `ContractError::CreditLineDefaulted` (19), or `ContractError::CreditLineClosed` (4) based on status.
- Reverts with `ContractError::InvalidAmount` (5) if `amount <= 0`.
- Reverts with `ContractError::Overflow` (12) on arithmetic overflow.
- Reverts with `ContractError::DrawCooldownActive` (20) when a borrower attempts to draw again before the configured cooldown interval has elapsed.
- Reverts with `ContractError::OverLimit` (6) if draw exceeds `credit_limit`.
- Transfers tokens from liquidity source → borrower.

Emits: `("credit", "drawn")` event.

### `reverse_draw(env, borrower, amount, original_ts, reason_code)`
Admin-only bounded reversal for erroneous draws.

- Reversal is allowed only when `ledger_timestamp - original_ts <= 3600` seconds.
- Reversal is validated against borrower-scoped draw audit data keyed by `(borrower, original_ts)`.
- Supports partial reversal; total reversed amount cannot exceed the original drawn amount at that timestamp.
- **Accounting-only behavior**: this call updates debt accounting (`utilized_amount`) and emits an audit event, but does not move tokens from borrower back to reserve.

Emits: `("credit", "draw_rev")` event with `DrawReversedEvent` payload containing borrower, amount, original draw timestamp, reason code, actor, and post-reversal utilization.

### `repay_credit(env, borrower, amount)`
Repay outstanding drawn funds.

**Allowed on**: Active, Suspended, or Defaulted credit lines.  
**Not allowed on**: Closed credit lines.

**Repayment allocation policy** (applied after pending interest accrual):
1. **Accrue pending interest** — `apply_pending_accrual` capitalizes any elapsed interest into `utilized_amount` and `accrued_interest` before repayment is applied. This prevents interest evasion through frequent repayments.
2. **Cap overpayment** — `effective_repay = min(amount, utilized_amount)`. Overpayments beyond total owed are ignored (no refund).
3. **Interest first** — `interest_repaid = min(effective_repay, accrued_interest)`.
4. **Principal second** — `principal_repaid = effective_repay - interest_repaid`.
5. **Update state** — `accrued_interest` and `utilized_amount` are reduced accordingly.

- The borrower must have approved the contract to pull tokens via `transfer_from`.
- Tokens are transferred **before** state is updated. If the transfer fails, the call reverts with no state change.
- Repayment failures due to insufficient allowance or balance do not alter `utilized_amount`, `accrued_interest`, or the credit line record.
- Works even when no liquidity token is configured (state-only update).

Emits: `("credit", "repay")` event with `RepaymentEvent` payload containing:
- `amount` — effective amount repaid (capped at total owed)
- `interest_repaid` — portion applied to accrued interest
- `principal_repaid` — portion applied to principal
- `new_utilized_amount` — total outstanding debt after repayment
- `new_accrued_interest` — remaining interest debt after repayment

Integrators can reconcile balances using:
- `principal_owed = new_utilized_amount - new_accrued_interest`
- `total_owed = new_utilized_amount`

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

### `get_schema_version(env) -> Option<u32>`
Returns the stored storage schema version from instance storage.

- After successful `init`, this returns `Some(1)`.
- Before initialization, this returns `None`.

### Storage schema versioning and migrations

The credit contract stores an explicit schema marker under `DataKey::SchemaVersion`.

- Current schema version: `1`
- Existing key/value layouts are unchanged; the version key is additive metadata.
- For immutable deployments, the version still gives off-chain tooling a deterministic way to detect schema expectations.
- For future contract deployments, bump the schema version when storage semantics change and document migration requirements in release notes and deployment playbooks.

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

### `self_suspend_credit_line(env, borrower)`
Allow a borrower to suspend their own Active credit line as a safety control.

- Requires borrower auth.
- Reverts if the line does not exist.
- Reverts unless the current status is `Active`.
- Blocks future draws but continues to allow `repay_credit`.
- Does not give the borrower any reinstatement path; reactivation still requires an admin-controlled workflow.

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

Emits:
- `("credit", "default")` lifecycle event.
- `("credit", "liq_req")` liquidation request event for auction orchestration.

### `settle_default_liquidation(env, borrower, recovered_amount, settlement_id)`
Apply auction liquidation proceeds to a defaulted line (admin only).

- Accounting-only operation (no token transfer in this method).
- Requires `status == Defaulted`.
- Requires positive `recovered_amount` and `recovered_amount <= utilized_amount`.
- Enforces one-time settlement per `(borrower, settlement_id)` to prevent replay.
- If remaining `utilized_amount == 0`, status transitions to `Closed`.

Emits: `("credit", "liq_setl")` event. When fully settled, also emits `("credit", "closed")`.

### `reinstate_credit_line(env, borrower)`
Reinstate a Defaulted credit line to Active. Admin only.

- Requires `status == Defaulted`.
- Self-suspended lines are not borrower-reinstatable. Any return to `Active` after borrower self-suspension must come from an admin-approved reopen workflow.

Emits: `("credit", "reinstate")` event.

### `get_credit_line(env, borrower) -> Option<CreditLineData>`
View function — returns the full [`CreditLineData`] for `borrower`, or `None` if no credit line exists.

#### Authentication
No authentication required. Any caller — indexer, client SDK, or another contract — may call this freely.

#### Stable serialization
The returned struct is stable for integrators. Fields are serialized in declaration order (see `types.rs`). New fields will only ever be appended; existing field positions will not change.

#### Accrual note
Interest accrual is lazy. `accrued_interest` and `utilized_amount` reflect the last mutating call (draw, repay, suspend, etc.). Pending interest since the last checkpoint is **not** applied by this query. To get the current accrued value, trigger a mutating call first or compute it off-chain using `last_accrual_ts` and `interest_rate_bps`.

#### Key fields for indexers

| Field | Description |
|---|---|
| `last_rate_update_ts` | Ledger timestamp of the last rate change; `0` means the rate has never been updated |
| `last_accrual_ts` | Ledger timestamp of the last interest checkpoint; `0` means no accrual has run yet |
| `accrued_interest` | Capitalized interest included in `utilized_amount` |
| `status` | Current lifecycle state (`Active`, `Suspended`, `Defaulted`, `Closed`, `Restricted`) |

#### Security notes
- Pure read — no storage is mutated, no auth is checked, no events are emitted.
- Safe to call from untrusted contexts; the worst outcome is a stale accrual snapshot (see accrual note above).
- Returns `None` for addresses that have never had a credit line; callers must handle this case.

### `freeze_draws(env)`
Freeze all `draw_credit` calls contract-wide (admin only).

- Sets `DataKey::DrawsFrozen` to `true` in instance storage.
- Does **not** mutate any borrower's `CreditStatus`; lines remain Active, Defaulted, etc.
- Repayments are never blocked by this flag.
- Idempotent: calling when already frozen still emits the event.

Emits: `("credit", "drw_freeze")` with `DrawsFrozenEvent { frozen: true, timestamp, actor }`.

### `unfreeze_draws(env)`
Re-enable `draw_credit` after a global freeze (admin only).

- Sets `DataKey::DrawsFrozen` to `false` in instance storage.
- Idempotent: calling when already unfrozen still emits the event.

Emits: `("credit", "drw_freeze")` with `DrawsFrozenEvent { frozen: false, timestamp, actor }`.

### `set_draw_min_interval(env, seconds)`
Set the per-borrower draw cooldown interval in seconds (admin only).

- `seconds > 0` enforces a minimum interval between successful draws for every borrower.
- `seconds = 0` disables the per-borrower cooldown.
- This setting is optional and defaults to disabled when unset.
- It affects only `draw_credit`; `repay_credit` remains available regardless of the cooldown.

### `is_draws_frozen(env) -> bool`
Returns `true` when draws are globally frozen. Defaults to `false` when the key has never been set. No auth required.

---

## Overflow Policy

Arithmetic paths that affect credit limit and utilization stay in integer-only arithmetic.

- `draw_credit`: utilization update uses `checked_add`; arithmetic overflow reverts with `ContractError::Overflow` (`12`).
- `repay_credit`: inputs must be positive integers; the contract computes `effective_repay = min(amount, utilized_amount)` and then applies the allocation policy (interest first, then principal) using `saturating_sub` and `max(0)` to keep both `accrued_interest` and `utilized_amount` non-negative. Over-repayments are capped at total owed.
- `apply_pending_accrual`: interest calculation uses checked multiplication and division; overflow reverts with `ContractError::Overflow` (`12`).
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
| `15`       | `DrawsFrozen` | All draws are globally frozen by admin for liquidity reserve operations.    |

---

## Events

| Topic                      | Event Type | Emitted By                  | Description |
|----------------------------|------------|-----------------------------|-----------|
| `("credit", "opened")`     | `opened`   | `open_credit_line`          | New credit line created |
| `("credit", "drawn")`      | `drawn`    | `draw_credit`               | Funds drawn |
| `("credit", "draw_rev")`   | `draw_rev` | `reverse_draw`              | Admin accounting reversal for erroneous draw (audit trail with reason code) |
| `("credit", "repay")`      | `repay`    | `repay_credit`              | Repayment made (includes interest/principal allocation) |
| `("credit", "accrue")`     | `accrue`   | `apply_pending_accrual`     | Interest capitalized into debt |
| `("credit", "suspend")`    | `suspend`  | `suspend_credit_line`       | Line suspended |
| `("credit", "closed")`     | `closed`   | `close_credit_line`         | Line closed |
| `("credit", "default")`    | `default`  | `default_credit_line`       | Line defaulted |
| `("credit", "liq_req")`    | `liq_req`  | `default_credit_line`       | Default liquidation requested |
| `("credit", "liq_setl")`   | `liq_setl` | `settle_default_liquidation`| Auction settlement applied to debt accounting |
| `("credit", "reinstate")`  | `reinstate`| `reinstate_credit_line`     | Line reinstated |
| `("credit", "risk_updated")`| `risk_updated` | `update_risk_parameters` | Risk parameters changed |
| `("credit", "drw_freeze")` | `DrawsFrozenEvent` | `freeze_draws`, `unfreeze_draws` | Global draw freeze toggled |

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
| `reverse_draw`           | Admin                 |
| `repay_credit`           | Borrower              |
| `update_risk_parameters` | Admin / risk engine   |
| `suspend_credit_line`    | Admin                 |
| `self_suspend_credit_line` | Borrower            |
| `close_credit_line`      | Admin or borrower     |
| `default_credit_line`    | Admin                 |
| `settle_default_liquidation` | Admin             |
| `reinstate_credit_line`  | Admin                 |
| `set_liquidity_token`    | Admin                 |
| `set_liquidity_source`   | Admin                 |
| `set_rate_change_limits` | Admin                 |
| `get_rate_change_limits` | Anyone (view)         |
| `get_credit_line`        | Anyone (view)         |
| `freeze_draws`           | Admin                 |
| `unfreeze_draws`         | Admin                 |
| `is_draws_frozen`        | Anyone (view)         |

> Note: `open_credit_line` requires admin authorization (`require_auth`). The admin key is the backend/risk engine signer — borrowers cannot open their own credit lines.

### Related Admin Workflows

- Default lifecycle: `default_credit_line` → optional `suspend_credit_line` containment → `reinstate_credit_line` or `close_credit_line`.
- Default liquidation lifecycle: `default_credit_line` emits `liq_req` → auction flow executes off-chain/on-chain as configured → admin applies proceeds via `settle_default_liquidation`.
- Oracle-assisted default design: `docs/default-oracle.md`.
- Auction hook architecture: `docs/default-liquidation-auction-hook.md`.

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

This section provides step-by-step instructions to deploy the contract on Stellar testnet,
initialize it, configure liquidity, and invoke core methods.

### Prerequisites

- **Rust 1.75+** with `wasm32-unknown-unknown` target installed
- **Stellar Soroban CLI** v21.0.0+: [install guide](https://developers.stellar.org/docs/tools-and-sdks/cli/install-soroban-cli)
- **soroban-cli configured network**: add testnet or futurenet if not present
- **Account on testnet**: funded with XLM for gas and operations

### Step 1: Network and Identity Setup

#### Configure Stellar Testnet

```bash
soroban network add --name testnet --rpc-url https://soroban-testnet.stellar.org:443 --network-passphrase "Test SDF Network ; September 2015"
```

#### Create or Import an Identity

```bash
# Generate a new identity (stores keypair in ~/.config/soroban/keys/)
soroban keys generate admin --network testnet

# Or import an existing keypair
soroban keys generate admin --secret-key --network testnet
# Then paste your secret key (starts with S...)
```

Verify the identity was created:

```bash
soroban keys ls
```

Fund the identity's address on testnet:
1. Get the public key: `soroban keys show admin`
2. Visit [Stellar Testnet Friendbot](https://friendbot.stellar.org/) and fund the address
3. Wait for the transaction to confirm (~5 seconds)

### Step 2: Build the Contract

```bash
# Build release WASM (optimized for size and deployment)
rustup target add wasm32-unknown-unknown
cargo build --release --target wasm32-unknown-unknown -p creditra-credit
```

The compiled WASM is at: `target/wasm32-unknown-unknown/release/creditra_credit.wasm`

### Step 3: Deploy the Contract

```bash
# Deploy to testnet
CONTRACT_ID=$(soroban contract deploy \
  --wasm target/wasm32-unknown-unknown/release/creditra_credit.wasm \
  --source admin \
  --network testnet)

echo "Contract deployed at: $CONTRACT_ID"
```

Save the `CONTRACT_ID` in an environment variable for subsequent commands.

### Step 4: Initialize the Contract

```bash
# Get the admin identity's public key
ADMIN_PUBKEY=$(soroban keys show admin)

# Initialize with admin
soroban contract invoke \
  --id $CONTRACT_ID \
  --source admin \
  --network testnet \
  -- init --admin $ADMIN_PUBKEY
```

This sets the admin address and defaults the liquidity source to the contract address.

### Step 5: Configure Liquidity Token and Source

#### (Optional) Create a Test Liquidity Token

If deploying a mock token for testing:

```bash
# Deploy a Stellar Asset Contract for USDC (testnet)
USDC_CONTRACT=$(soroban contract deploy native \
  --network testnet \
  --source admin)

echo "USDC contract at: $USDC_CONTRACT"
```

#### Set Liquidity Token

```bash
soroban contract invoke \
  --id $CONTRACT_ID \
  --source admin \
  --network testnet \
  -- set_liquidity_token --token_address $USDC_CONTRACT
```

#### Set Liquidity Source (Reserve Account)

The liquidity source is where reserve tokens are held. It can be the contract address,
an external reserve account, or another contract.

```bash
# Option A: Keep contract as reserve (already set in init)
# No additional action needed

# Option B: Set a different reserve account
RESERVE_PUBKEY=$(soroban keys show reserve)
soroban contract invoke \
  --id $CONTRACT_ID \
  --source admin \
  --network testnet \
  -- set_liquidity_source --reserve_address $RESERVE_PUBKEY
```

### Step 6: Open a Credit Line

Create a credit line for a borrower. This is typically called by the backend/risk engine.

```bash
# Generate or use an existing borrower identity
soroban keys generate borrower --network testnet
BORROWER_PUBKEY=$(soroban keys show borrower)

# Open a credit line
# - borrower: the borrower address
# - credit_limit: 10000 (in smallest token unit, typically microunits)
# - interest_rate_bps: 300 (3% annual interest)
# - risk_score: 75 (out of 100)
soroban contract invoke \
  --id $CONTRACT_ID \
  --source admin \
  --network testnet \
  -- open_credit_line \
    --borrower $BORROWER_PUBKEY \
    --credit_limit 10000 \
    --interest_rate_bps 300 \
    --risk_score 75
```

Verify the credit line was created:

```bash
soroban contract invoke \
  --id $CONTRACT_ID \
  --source admin \
  --network testnet \
  -- get_credit_line --borrower $BORROWER_PUBKEY
```

### Step 7: Fund the Liquidity Reserve

If using a liquidity token, the reserve account must hold sufficient balance for draws.

```bash
# If USDC contract is the token, fund the reserve
# This example assumes the contract is the reserve
soroban contract invoke \
  --id $USDC_CONTRACT \
  --source admin \
  --network testnet \
  -- mint --to $CONTRACT_ID --amount 50000

# Verify reserve balance
soroban contract invoke \
  --id $USDC_CONTRACT \
  --source admin \
  --network testnet \
  -- balance --id $CONTRACT_ID
```

### Step 8: Draw Credit

A borrower draws against their credit line. This transfers tokens from the reserve to the borrower.

```bash
# Borrower draws 1000 units
soroban contract invoke \
  --id $CONTRACT_ID \
  --source borrower \
  --network testnet \
  -- draw_credit \
    --borrower $BORROWER_PUBKEY \
    --amount 1000
```

Verify the draw:

```bash
soroban contract invoke \
  --id $CONTRACT_ID \
  --source admin \
  --network testnet \
  -- get_credit_line --borrower $BORROWER_PUBKEY
```

Expected result: `utilized_amount` should now be 1000.

### Step 9: Repay Credit

Borrowers repay their drawn amount. The tokens are transferred back to the liquidity source.

#### Prerequisite: Approve Token Transfer

The borrower must approve the contract to transfer tokens on their behalf.

```bash
# Borrower approves the contract to transfer up to 2000 units
soroban contract invoke \
  --id $USDC_CONTRACT \
  --source borrower \
  --network testnet \
  -- approve \
    --from $BORROWER_PUBKEY \
    --spender $CONTRACT_ID \
    --amount 2000 \
    --expiration_ledger 1000000
```

#### Execute Repayment

```bash
# Borrower repays 500 units
soroban contract invoke \
  --id $CONTRACT_ID \
  --source borrower \
  --network testnet \
  -- repay_credit \
    --borrower $BORROWER_PUBKEY \
    --amount 500
```

Verify the repayment:

```bash
soroban contract invoke \
  --id $CONTRACT_ID \
  --source admin \
  --network testnet \
  -- get_credit_line --borrower $BORROWER_PUBKEY
```

Expected result: `utilized_amount` should now be 500.

### Step 10: Update Risk Parameters (Admin Only)

The admin can adjust credit limits, interest rates, and risk scores.

```bash
soroban contract invoke \
  --id $CONTRACT_ID \
  --source admin \
  --network testnet \
  -- update_risk_parameters \
    --borrower $BORROWER_PUBKEY \
    --credit_limit 20000 \
    --interest_rate_bps 400 \
    --risk_score 85
```

### Step 11: Manage Credit Line Status

#### Suspend a Credit Line

Prevent draws while allowing repayment.

```bash
soroban contract invoke \
  --id $CONTRACT_ID \
  --source admin \
  --network testnet \
  -- suspend_credit_line --borrower $BORROWER_PUBKEY
```

#### Default a Credit Line

Mark the borrower as in default (blocks draws, allows repayment).

```bash
soroban contract invoke \
  --id $CONTRACT_ID \
  --source admin \
  --network testnet \
  -- default_credit_line --borrower $BORROWER_PUBKEY
```

#### Close a Credit Line

- **Admin**: can force-close at any time
- **Borrower**: can only close when `utilized_amount` is 0

```bash
# Admin force-close
soroban contract invoke \
  --id $CONTRACT_ID \
  --source admin \
  --network testnet \
  -- close_credit_line \
    --borrower $BORROWER_PUBKEY \
    --closer $ADMIN_PUBKEY

# Or borrower self-close (only when fully repaid)
soroban contract invoke \
  --id $CONTRACT_ID \
  --source borrower \
  --network testnet \
  -- close_credit_line \
    --borrower $BORROWER_PUBKEY \
    --closer $BORROWER_PUBKEY
```

### Useful Quick Reference

**Export identities to variables for scripting:**

```bash
ADMIN=$(soroban keys show admin)
BORROWER=$(soroban keys show borrower)
RESERVE=$(soroban keys show reserve)
TOKEN=$USDC_CONTRACT
CONTRACT=$CONTRACT_ID
```

**Query contract state:**

```bash
# Check a specific credit line
soroban contract invoke --id $CONTRACT --source admin --network testnet -- get_credit_line --borrower $BORROWER

# Check token balance
soroban contract invoke --id $TOKEN --source admin --network testnet -- balance --id $CONTRACT
```

**Troubleshooting common errors:**

| Error | Cause | Fix |
|-------|-------|-----|
| `HostError: Error(Auth, InvalidAction)` | Identity not authorized | Ensure `--source` identity is loaded and has been funded |
| `HostError: Value(ContractError(1))` | Credit line not found | Verify credit line was opened with correct borrower address |
| `HostError: Error(Contract, InvalidContractData)` | Contract ID invalid or contract not deployed | Check `$CONTRACT_ID` and verify deployment succeeded |
| `Insufficient liquidity reserve` | Reserve balance too low | Fund the reserve with more tokens via `mint` or transfer |
| `Insufficient allowance` | Token approval too low | Increase borrower's approval via token `approve` |

---

## Running Tests

```bash
cargo test -p creditra-credit
```

---

## Appendix: Storage Key Audit

This appendix documents all storage keys used by the credit contract, their
storage types (instance, persistent, or temporary), TTL implications, and
security considerations.

### Storage Type Definitions

| Storage Type | TTL Behavior | Use Case |
|--------------|--------------|----------|
| **Instance** | Shared TTL across all instance keys. If the instance is archived, all instance keys are lost. | Global singleton configuration (admin, protocol settings) |
| **Persistent** | Independent TTL per key. Each borrower's data can be archived separately. | Per-borrower credit line data |
| **Temporary** | Lives only for the duration of a single invocation. | Short-lived state (not currently used) |

### Instance Storage

Keys that share the contract instance TTL. If the instance is archived, all
these keys are lost. Production deployments should call
`env.storage().instance().extend_ttl()` periodically to prevent archival.

| Key | Rust type | Value type | Written by | TTL Notes |
|-----|-----------|------------|------------|-----------|
| `Symbol("admin")` | `Symbol` | `Address` | `init` | Written exactly once; second `init` reverts with `AlreadyInitialized`. Critical for access control — loss of instance storage means loss of admin. |
| `Symbol("proposed_admin")` | `Symbol` | `Address` | `propose_admin` | Pending admin address during two-step rotation. Cleared on `accept_admin` or overwrite on new proposal. |
| `Symbol("proposed_at")` | `Symbol` | `u64` | `propose_admin` | Ledger timestamp after which the proposed admin can accept. Cleared on `accept_admin`. |
| `Symbol("reentrancy")` | `Symbol` | `bool` | `set_reentrancy_guard`, `clear_reentrancy_guard` | Defense-in-depth reentrancy guard. Set on entry to `draw_credit`/`repay_credit`, cleared on every exit path (success and failure). |
| `Symbol("rate_cfg")` | `Symbol` | `RateChangeConfig` | `set_rate_change_limits` | Optional rate-change governance config. Absent = no limits enforced. |
| `Symbol("rate_form")` | `Symbol` | `RateFormulaConfig` | Internal (risk module) | Optional piecewise-linear rate formula config. Absent = manual rate mode. |
| `Symbol("paused")` | `Symbol` | `bool` | `set_paused` | Circuit breaker pause flag. Absent = `false` (not paused). Blocks all mutating operations except `repay_credit`. |
| `Symbol("grace_period")` | `Symbol` | `GracePeriodConfig` | `set_grace_period_config` | Optional grace period policy for suspended lines. |
| `DataKey::LiquidityToken` | `DataKey` | `Option<Address>` | `set_liquidity_token` | Token contract address for draw/repay transfers. Optional — contract works without a token configured. |
| `DataKey::LiquiditySource` | `DataKey` | `Address` | `init`, `set_liquidity_source` | Reserve address holding liquidity. Defaults to contract address on `init`. |
| `DataKey::MaxDrawAmount` | `DataKey` | `i128` | `set_max_draw_amount` | Optional per-transaction draw cap. Absent = no limit. |
| `DataKey::DrawsFrozen` | `DataKey` | `bool` | `freeze_draws`, `unfreeze_draws` | Global emergency draw freeze. Absent = `false` (draws allowed). Does not affect repayments. |
| `DataKey::SchemaVersion` | `DataKey` | `u32` | `init` | Storage schema version marker. Current version: `1`. Used for migration detection. |
| `DataKey::BlockedBorrower(Address)` | `DataKey` | `bool` | `set_borrower_blocked` | Per-borrower block flag stored in **persistent** storage (note: uses `DataKey` enum but stored via `env.storage().persistent()`). |

**TTL Management Recommendations:**
- Call `env.storage().instance().extend_ttl(TTL_THRESHOLD, TTL_EXTEND_TO)` in frequently-called functions like `draw_credit`, `repay_credit`, or a dedicated `bump_instance_ttl()` admin function.
- Recommended thresholds: check/extend when TTL drops below 100 ledgers, extend to 10,000 ledgers.

### Persistent Storage

Per-borrower records with independent TTL per entry. These keys survive instance archival and have their own TTL lifecycle.

| Key | Rust type | Value type | Written by | TTL Notes |
|-----|-----------|------------|------------|-----------|
| `borrower: Address` | `Address` | `CreditLineData` | `open_credit_line`, `draw_credit`, `repay_credit`, `update_risk_parameters`, lifecycle transitions | Long-lived borrower credit line data. TTL should be extended on each access to prevent archival of active lines. |
| `DataKey::BlockedBorrower(Address)` | `DataKey` | `bool` | `set_borrower_blocked` | Per-borrower blocking flag. Independent TTL from credit line data. |
| `(Symbol("liq_seen"), borrower: Address, settlement_id: Symbol)` | Tuple | `bool` | `settle_default_liquidation` | One-time settlement marker to prevent replay of liquidation settlements. |

**Why Persistent?** Each borrower's credit line must survive beyond a single transaction and has an independent lifecycle. Persistent storage is correct because:
1. Borrower data outlives any single invocation
2. Each borrower's TTL is independent (one borrower's archival doesn't affect others)
3. Per-entity storage scales better than instance storage for large numbers of borrowers

**TTL Management Recommendations:**
- Extend TTL on credit line access: `env.storage().persistent().extend_ttl(&borrower, TTL_THRESHOLD, TTL_EXTEND_TO)`
- Consider a keeper service that periodically extends TTLs for active credit lines

### Temporary Storage

Not currently used in the contract. The reentrancy guard is stored in instance storage but is always cleared before the function returns, making it functionally equivalent to temporary storage.

**Future Consideration:** The reentrancy guard (`Symbol("reentrancy")`) could theoretically be moved to temporary storage (`env.storage().temporary()`) since it only needs to survive within a single invocation. However, Soroban's temporary storage has different cost characteristics and the current instance storage approach works correctly because the guard is always cleared.

### Audit Findings Summary

| Component | Storage Type | Correct? | Notes |
|-----------|--------------|----------|-------|
| Admin address | Instance | ✅ Yes | Single global value, correct for singleton pattern |
| Proposed admin / proposed_at | Instance | ✅ Yes | Temporary during rotation, shares instance TTL |
| LiquidityToken | Instance | ✅ Yes | Global configuration, one per contract |
| LiquiditySource | Instance | ✅ Yes | Global configuration, one per contract |
| Reentrancy flag | Instance | ✅ Yes* | *Cleared every call; could use temporary storage but instance works |
| Rate config (rate_cfg) | Instance | ✅ Yes | Global governance parameter |
| Rate formula config | Instance | ✅ Yes | Global formula configuration |
| Pause flag | Instance | ✅ Yes | Global circuit breaker |
| MaxDrawAmount | Instance | ✅ Yes | Global per-transaction limit |
| DrawsFrozen | Instance | ✅ Yes | Global emergency flag |
| SchemaVersion | Instance | ✅ Yes | Global schema marker |
| Borrower credit lines | Persistent | ✅ Yes | Per-entity data with independent lifecycle |
| BlockedBorrower | Persistent | ✅ Yes | Per-borrower flag, independent of credit line data |
| Liquidation settlement markers | Persistent | ✅ Yes | Per-(borrower, settlement_id) replay protection |

### Security Notes

1. **No borrower data on instance storage** — Verified. Per-borrower data correctly uses persistent storage, avoiding the shared TTL pitfall where one borrower's activity could affect another's data availability.

2. **Instance TTL is critical** — All global configuration shares one TTL. If the instance is archived, the contract loses admin, liquidity config, and all protocol settings. Production deployments must implement TTL extension.

3. **Reentrancy guard semantics** — While stored in instance storage, the guard is functionally temporary (set on entry, cleared on all exits). This is safe but relies on correct implementation at all exit paths.

4. **BlockedBorrower uses DataKey enum but persistent storage** — The `DataKey::BlockedBorrower(Address)` variant is stored via `env.storage().persistent()`, not instance storage. This is correct as it's per-borrower data.

5. **Trust boundaries** — Instance storage contains all admin-controlled configuration. Compromise of the admin key allows modification of all instance-stored values. Persistent storage contains borrower-specific data that is protected by different authorization rules (borrower auth for draws/repays, admin auth for lifecycle changes).

6. **Failure modes** — If instance TTL expires:
   - Admin cannot be retrieved → all admin operations fail
   - Liquidity config is lost → draws/repays may fail
   - Reentrancy guard defaults to `false` → no reentrancy protection
   - All protocol flags reset to defaults

   If persistent TTL expires for a borrower:
   - That borrower's credit line data is lost
   - Other borrowers are unaffected
   - The borrower would need to re-establish their credit line
| `DataKey::LiquidityToken` | `DataKey` | `Address` | `set_liquidity_token` | Token contract for reserve/draw transfers. |
| `DataKey::LiquiditySource` | `DataKey` | `Address` | `init`, `set_liquidity_source` | Reserve address. Defaults to contract address. |
| `DataKey::DrawMinIntervalSeconds` | `DataKey` | `u64` | `set_draw_min_interval` | Minimum per-borrower draw interval in seconds. Absent = disabled. |
| `Symbol("reentrancy")` | `Symbol` | `bool` | `set_reentrancy_guard`, `clear_reentrancy_guard` | Defense-in-depth flag. Cleared on every code path. |
| `Symbol("rate_cfg")` | `Symbol` | `RateChangeConfig` | `set_rate_change_limits` | Admin-configurable rate-change governance. |
| `DataKey::DrawsFrozen` | `DataKey` | `bool` | `freeze_draws`, `unfreeze_draws` | Global emergency draw freeze. Absent = `false` (draws allowed). |

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
8. **DrawsFrozen** — correctly on instance. Global singleton flag; absent key
   is treated as `false` (draws allowed). Shares instance TTL — extend alongside
   other instance keys.

You can also run all workspace tests from the repository root with `cargo test`.

---

## Error Reference

This section documents all contract errors and their exact error codes for consistent error handling across integrations.

### ContractError Enum

| Error Code | Variant | Description | Trigger |
|------------|---------|-------------|---------|
| 1 | `Unauthorized` | Caller is not authorized to perform this action | Various admin-only operations |
| 2 | `NotAdmin` | Caller does not have admin privileges | `require_admin_auth` checks |
| 3 | `CreditLineNotFound` | The specified credit line was not found | Operations on non-existent credit lines |
| 4 | `CreditLineClosed` | Action cannot be performed because the credit line is closed | Draw operations on closed lines |
| 5 | `InvalidAmount` | The requested amount is invalid (e.g., zero or negative) | Amount validation in draw/repay |
| 6 | `OverLimit` | The requested draw exceeds the available credit limit | Draw limit checks |
| 7 | `NegativeLimit` | The credit limit cannot be negative | Credit limit validation |
| 8 | `RateTooHigh` | The interest rate exceeds maximum allowed (10000 bps = 100%) | Rate bounds validation |
| 9 | `ScoreTooHigh` | The risk score exceeds maximum allowed (100) | Score bounds validation |
| 10 | `UtilizationNotZero` | Action cannot be performed because the credit line utilization is not zero | Certain admin operations |
| 11 | `Reentrancy` | Reentrancy detected during cross-contract calls | Reentrancy guard |
| 12 | `Overflow` | Math overflow occurred during calculation | Arithmetic operations |
| 13 | `LimitDecreaseRequiresRepayment` | Credit limit decrease requires immediate repayment of excess amount | Limit decrease validation |
| 14 | `AlreadyInitialized` | Contract has already been initialized; `init` may only be called once | Second `init` call |
| 15 | `DrawsFrozen` | All draws are globally frozen by admin for liquidity reserve operations | `draw_credit` when `DataKey::DrawsFrozen` is `true` |
| 16 | `DrawExceedsMaxAmount` | The requested draw exceeds the configured per-transaction maximum | `draw_credit` when `DataKey::MaxDrawAmount` is set |

### Rate and Score Validation

**Interest Rate Bounds:**
- Valid range: `0` to `10_000` basis points (0% to 100%)
- Error on violation: `ContractError::RateTooHigh` (code 8)
- Applied in: `open_credit_line`, `update_risk_parameters`

**Risk Score Bounds:**
- Valid range: `0` to `100`
- Error on violation: `ContractError::ScoreTooHigh` (code 9)  
- Applied in: `open_credit_line`, `update_risk_parameters`

### Boundary Test Coverage

The contract includes comprehensive table-driven tests that verify:

1. **Exact boundary acceptance**: Values at the exact limits (0, 10000 bps, 100 score) are accepted
2. **One-past boundary rejection**: Values one unit beyond limits (10001 bps, 101 score) are rejected
3. **Error mapping consistency**: Both `open_credit_line` and `update_risk_parameters` use the same error types
4. **Edge case validation**: Granular testing around boundary values (9999, 10000, 10001)

For detailed test implementation, see `boundary_tests.rs` in the source code.

### Error Handling Best Practices

1. **Always check error codes**: Use the numeric error codes for reliable error handling
2. **Handle RateTooHigh/ScoreTooHigh specifically**: These errors indicate input validation failures
3. **Distinguish between error types**: `RateTooHigh` (8) vs `ScoreTooHigh` (9) for precise validation feedback
4. **Test boundary conditions**: Include tests for exact bounds and one-past bounds in all integrations

---

## Borrower Blocklist

The borrower blocklist provides an emergency gating mechanism that allows the protocol admin to temporarily prevent specific borrowers from drawing credit without modifying their underlying `CreditStatus` or credit line data. This is useful during investigations, compliance reviews, or when suspicious activity is detected.

### Methods

#### `set_borrower_blocked(env, borrower, blocked)`
- **Access**: Admin only
- **Parameters**:
  - `borrower`: Address to block or unblock
  - `blocked`: `true` to block, `false` to unblock
- **Behavior**: Stores the blocked flag in persistent storage keyed by borrower. Emits a `BorrowerBlockedEvent` with topic `("credit", "blocked")` or `("credit", "unblocked")`.
- **Security**: Requires admin auth. Does not mutate `CreditLineData` or `CreditStatus`.

#### `is_borrower_blocked(env, borrower) -> bool`
- **Access**: View function (no auth required)
- **Returns**: `true` if the borrower is currently blocked, `false` otherwise (including if no record exists).

### Enforcement

The blocklist is enforced exclusively in `draw_credit`. If a blocked borrower attempts to draw:
- The transaction reverts with `ContractError::BorrowerBlocked` (code 15)
- The reentrancy guard is cleared before reverting
- Repayments via `repay_credit` remain fully operational regardless of block status

### Operational Use Cases

1. **Investigation Hold**: A borrower's account shows suspicious activity. Admin blocks draws while the investigation proceeds. The borrower's existing utilization and status remain unchanged, and they can still repay.
2. **Compliance Freeze**: Regulatory requirement to pause new draws for a specific address. Blocking avoids the need to suspend or default the line, preserving the borrower's credit history.
3. **Temporary Risk Mitigation**: Rapid response to an oracle or off-chain risk signal. The admin can block immediately and unblock once the signal resolves, without going through the `Suspended` -> `Active` state transition.

### State Machine Independence

The blocklist is intentionally decoupled from `CreditStatus`:

| Aspect | Blocklist | `CreditStatus` |
|---|---|---|
| Scope | Per-address flag | Per-credit-line enum |
| Admin action | `set_borrower_blocked` | `suspend_credit_line`, `default_credit_line`, etc. |
| Affects draws | Yes | Yes (for Suspended, Defaulted, Closed) |
| Affects repay | No | No (except Closed) |
| Event topic | `("credit", "blocked")` / `("credit", "unblocked")` | `("credit", "suspend")` / `("credit", "default")` etc. |
| Persistence | Persistent storage (`DataKey::BlockedBorrower`) | Persistent storage (`CreditLineData`) |

This separation ensures that blocking is a lightweight, reversible operational action that does not interfere with lifecycle transitions or interest accrual logic.

### Testing Requirements

- Block and unblock round-trip
- Blocked borrower cannot draw
- Unblocked borrower can draw after being unblocked
- Repayment remains allowed while blocked
- Non-admin cannot block or unblock
- Events emitted with correct topics and payloads
