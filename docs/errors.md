# `ContractError` Reference — Canonical

**Source of truth:** [`ContractError`](../contracts/credit/src/types.rs) in
`contracts/credit/src/types.rs`.
**Verify the tables against it:** `python3 scripts/list_contract_errors.py --check`

This is the **single canonical error reference** for the `creditra-credit`
Soroban contract. Every other error document in this repository is a redirect
to this page. Integrators (TypeScript SDK, Rust SDK, indexers) match on the
integer codes published here.

If you find a code here that the contract does not emit, or an emitted code that
is missing, it is a bug in this page — fix it here, not in a copy.

---

## Scope

This page covers the credit contract's `ContractError` only. Errors from other
crates have their own canonical pages:

| Enum | Canonical page |
|------|----------------|
| `ContractError` (credit) | this page |
| `CollateralError` (collateral) | [`docs/errors/collateral.md`](./errors/collateral.md) |
| `FreezeError` (freeze) | [`docs/errors/freeze.md`](./errors/freeze.md) |

The V1 → V2 client-side error-encoding migration of the CosmWasm
`creditra-credit` package is a separate concern with its own log:
[`docs/ERROR_MIGRATION.md`](./ERROR_MIGRATION.md).

---

## Stability Guarantee

Discriminants are **permanent and immutable** once assigned. The contract is
deployed on Stellar Soroban and cannot be upgraded without a migration. Changing
or reordering a discriminant would silently break all SDK clients that match on
integer codes.

Rules asserted by `contracts/credit/tests/error_discriminants.rs`:

- Every variant has an explicit `= N` assignment in `types.rs`.
- No two variants share the same integer.
- New variants are always appended at the end with the next available integer.
- The assertion test file must be updated alongside any enum change.
- This page's two tables match the enum exactly.

`scripts/list_contract_errors.py --check` re-asserts the last item without a
Rust toolchain, and runs in CI. Run it before opening a PR that touches
`types.rs` or this file.

**13** (`LimitDecreaseRequiresRepayment`) was removed and **56** was reserved and
never assigned. Both discriminants are permanently retired: they must never be
reissued, so a stale client that still maps `13` fails closed instead of
decoding into a live variant.

`ContractError` is declared with `#[soroban_sdk::contracterror(export = false)]`:
it has grown past the 50-case `SCSpecUdtUnionV0.cases<50>` limit of the
Soroban contract spec. Errors still reach clients with their pinned numeric
discriminants — only the generated spec entry is skipped.

---

## Error Code Table

61 variants. `Category` is the value returned by
[`ContractError::category()`](../contracts/credit/src/types.rs).

| Code | Variant | Category | When it occurs | SDK recovery |
|------|---------|----------|----------------|--------------|
| `1`  | `Unauthorized` | Auth | Caller is not the authorized party for the operation (e.g. non-borrower calling `draw_credit`). | Reconnect a wallet holding the required role. |
| `2`  | `NotAdmin` | Auth | Caller does not hold the admin role stored in instance storage. | Use the admin keypair, or rotate admin via `propose_admin` / `accept_admin`. |
| `3`  | `CreditLineNotFound` | Misc | No credit line exists in persistent storage for the given borrower. | Call `open_credit_line` first, or fix the borrower address. |
| `4`  | `CreditLineClosed` | Lifecycle | The line's status is `Closed`; draws and state transitions are blocked. | Terminal — open a new credit line. |
| `5`  | `InvalidAmount` | Numeric | Amount is zero, negative, or outside the accepted range. | Pass a strictly positive `i128`. |
| `6`  | `OverLimit` | Limit | The draw would push `utilized_amount` above `credit_limit`. | Reduce the draw to the available headroom. |
| `7`  | `NegativeLimit` | Numeric | A negative credit limit was supplied to a credit-line configuration flow. | Supply a non-negative `i128` credit limit. |
| `8`  | `RateTooHigh` | Risk | The proposed rate exceeds `10 000` bps (100 %) or the configured `max_rate_change_bps` delta. | Clamp the change inside `RateChangeConfig` bounds. |
| `9`  | `ScoreTooHigh` | Risk | `risk_score` exceeds `100`. | Normalize the score to `[0, 100]`. |
| `10` | `UtilizationNotZero` | Limit | An operation requiring zero utilization ran while debt was outstanding. | Repay the full balance, then retry. |
| `11` | `Reentrancy` | Reentrancy | A reentrant call was detected by the guard on `draw_credit` / `repay_credit`. | Do **not** retry blindly. Inspect the token contract for callbacks. |
| `12` | `Overflow` | Numeric | A `checked_*` operation would exceed `i128` range. | Reduce the amount; values near `i128::MAX` are out of scope. |
| `14` | `AlreadyInitialized` | Lifecycle | `init` was called on a contract that already has an admin stored. | No action — `init` is one-time. |
| `15` | `AdminAcceptTooEarly` | Misc | `accept_admin` ran before the `delay_seconds` window from `propose_admin` elapsed. | Wait until `ledger().timestamp() >= accept_after`. |
| `16` | `BorrowerBlocked` | Block | The borrower is on the admin-managed block list; draws are disabled. | Contact the admin; repayments are still accepted. |
| `17` | `DrawExceedsMaxAmount` | Limit | The draw exceeds the per-transaction cap set by `set_max_draw_amount`. | Split the draw into smaller transactions. |
| `18` | `Paused` | Risk | The emergency circuit breaker is active; guarded operations are blocked. | Retry after `set_protocol_paused(false)`. `repay_credit` stays live. |
| `19` | `DrawsFrozen` | Block | Draws are globally frozen for liquidity-reserve operations. | Wait for `unfreeze_draws`. Repayments remain available. |
| `20` | `CreditLineSuspended` | Lifecycle | The line's status is `Suspended` (admin or self-suspension). | Await reinstatement. Repayments are still allowed. |
| `21` | `CreditLineDefaulted` | Lifecycle | The line's status is `Defaulted`. | Cure by repayment or proceed through the liquidation path. |
| `22` | `MissingLiquidityToken` | Liquidity | `draw_credit` / `repay_credit` needs a liquidity token, but none is configured. | Admin must call `set_liquidity_token`. |
| `23` | `MissingLiquiditySource` | Liquidity | A liquidity source is required but none is configured. | Admin must call `set_liquidity_source`. |
| `24` | `InsufficientLiquidityReserve` | Liquidity | The reserve token balance is below the requested draw. | Fund the reserve or reduce the draw. |
| `25` | `LiquidityTokenCallFailed` | Liquidity | A token interaction failed on a path the contract can observe. | Inspect the token contract before retrying. |
| `26` | `InsufficientRepaymentAllowance` | Liquidity | Borrower allowance is below the effective repayment amount. | Approve at least the effective repayment for this contract. |
| `27` | `InsufficientRepaymentBalance` | Liquidity | Borrower token balance is below the effective repayment amount. | Fund the borrower, then retry. |
| `28` | `RepayExceedsMaxAmount` | Limit | The repay exceeds the per-transaction cap. | Split into smaller transactions. |
| `29` | `DrawCooldownActive` | Risk | The borrower drew again before `draw_min_interval_seconds` elapsed. | Wait for the cooldown window. |
| `30` | `TreasuryNotSet` | Liquidity | A treasury withdrawal was attempted with no treasury configured. | Admin must call `set_treasury`. |
| `31` | `ExposureCapExceeded` | Liquidity | The draw would push total utilization past the global exposure cap. | Reduce the draw or wait for repayments elsewhere. |
| `32` | `AdminNotInitialized` | Auth | An admin-gated entrypoint ran before `init` set the admin. | Deployer must call `init()` with a valid admin address. |
| `33` | `TimestampRegression` | Numeric | A write carried a timestamp not strictly greater than the stored one. | Re-sync the ledger view and retry. |
| `34` | `LimitOutOfBounds` | Numeric | A credit limit fell outside the configured `[min_limit, max_limit]`. | Read the bounds and adjust the proposed limit. |
| `35` | `CollateralRatioBelowMinimum` | Collateral | A withdrawal (or draw) would leave the ratio below `MinCollateralRatioBps`. | Reduce the withdrawal or add collateral. |
| `36` | `OraclePriceInvalid` | Oracle | The oracle price is zero, negative, or malformed. | Await a valid positive price. |
| `37` | `OraclePriceStale` | Oracle | The oracle price is older than `max_age_seconds`. | Await a price update. |
| `38` | `OraclePriceDeviation` | Oracle | Price deviation from the prior value exceeds `max_deviation_bps`. | Await a new price; do not retry with the same one. |
| `39` | `InsufficientCollateralBalance` | Collateral | The withdrawal amount exceeds the borrower's collateral balance. | Query the balance and reduce the amount. |
| `40` | `BorrowerFrozen` | Block | The borrower's draws are frozen until a stored expiry timestamp. | Wait for expiry or ask the admin to lift the freeze. |
| `41` | `BountyNotSet` | Liquidity | A bounty withdrawal was attempted with no bounty pool configured. | Admin must call `set_bounty`. |
| `42` | `NoPendingTreasuryWithdrawal` | Misc | A treasury withdrawal was executed with no pending proposal. | Create a proposal via `propose_treasury_withdrawal`. |
| `43` | `TreasuryTimelockActive` | Misc | The 24-hour treasury timelock has not elapsed since the proposal. | Wait for the timelock. |
| `44` | `TreasuryProposalExists` | Misc | A treasury withdrawal proposal already exists. | Execute or cancel the existing proposal first. |
| `45` | `CloseFactorAboveMax` | Limit | The supplied `close_factor_bps` exceeds the protocol maximum. | Reduce `close_factor_bps` to the configured max. |
| `46` | `CreditLineFrozen` | Block | An admin freeze with a structured `FreezeReason` is set on the line. | Admin calls `unfreeze_credit_line`. Repayments stay available. |
| `47` | `DrawReversalWindowExpired` | Limit | `reverse_draw` ran after `DRAW_REVERSAL_WINDOW_SECS`. | No reversal is possible; the window has closed. |
| `48` | `OriginalDrawNotFound` | Misc | No draw audit record matches the reversal request. | No reversal is possible without the original record. |
| `49` | `AttestationBatchNotFound` | Misc | Attestation verification ran with no committed batch. | Admin must commit a batch first. |
| `50` | `OracleQuorumNotMet` | Oracle | Fewer than `min_quorum_k` feeds agreed within the deviation bound. | Submit prices from more independent feeds. |
| `51` | `AlreadySettled` | Lifecycle | Liquidation for this `(borrower, settlement_id)` pair was already processed. | No action — replay protection. Use a fresh `settlement_id` per event. |
| `52` | `InvalidRiskWeight` | Numeric | A collateral risk weight above `10 000` bps was supplied. | Use a weight in `0..=10_000` bps. |
| `53` | `InvalidAttestation` | Misc | The attestation proof failed verification, or no batch is committed. | Commit a valid batch and resubmit a valid proof. |
| `54` | `RiskAdminCooldownActive` | Risk | A risk-admin mutation ran before `risk_admin_cooldown_seconds` elapsed since the last one. | Wait for the cooldown, or set it to `0` to disable. |
| `55` | `OracleNotFound` | Oracle | The oracle address is not present in the oracle registry. | Register the oracle before managing it. |
| `57` | `FreezeCooldownActive` | Block | A freeze action ran before `freeze_cooldown_seconds` elapsed since the last freeze. | Wait for the cooldown, or clear the configured interval. |
| `58` | `AdminCollateralCooldownActive` | Collateral | A critical collateral admin action ran before its cool-off window elapsed. | Wait for the window, or set the interval to `0`. |
| `59` | `LiquidationGraceActive` | Lifecycle | Defaulting ran while the per-borrower suspension grace window was still open. | Wait for the grace window to expire, then retry. |
| `60` | `StaleStateTransition` | Lifecycle | The line is already in the requested target state (stale or duplicate transition). | No action — re-read state; the transition already applied. |
| `61` | `IncompatibleVersion` | Handshake | The auction contract's protocol version does not match the credit contract's. | Upgrade one side to a compatible version, then retry the settlement. |
| `62` | `AuctionCallFailed` | Handshake | The auction CPI call failed or returned an unexpected value. | Fix the `recovered_amount` / auction issue, then retry. No credit state was mutated. |
| `63` | `AuctionActive` | Lifecycle | A fee-configuration change was attempted while a liquidation auction was in flight. | Wait until the last active auction leaves the `Defaulted` pipeline. |

---

## Categories

`ContractError::category()` returns a stable `#[repr(u32)]`
`ContractErrorCategory`. Use it for client-side grouping instead of
hard-coding code ranges.

| Code | Category | Count | Variants |
|------|----------|------:|----------|
| `1`  | Auth | 3 | `Unauthorized`, `NotAdmin`, `AdminNotInitialized` |
| `2`  | Lifecycle | 8 | `CreditLineClosed`, `AlreadyInitialized`, `CreditLineSuspended`, `CreditLineDefaulted`, `AlreadySettled`, `LiquidationGraceActive`, `StaleStateTransition`, `AuctionActive` |
| `3`  | Numeric | 6 | `InvalidAmount`, `NegativeLimit`, `Overflow`, `TimestampRegression`, `LimitOutOfBounds`, `InvalidRiskWeight` |
| `4`  | Limit | 6 | `OverLimit`, `UtilizationNotZero`, `DrawExceedsMaxAmount`, `RepayExceedsMaxAmount`, `CloseFactorAboveMax`, `DrawReversalWindowExpired` |
| `5`  | Liquidity | 9 | `MissingLiquidityToken`, `MissingLiquiditySource`, `InsufficientLiquidityReserve`, `LiquidityTokenCallFailed`, `InsufficientRepaymentAllowance`, `InsufficientRepaymentBalance`, `TreasuryNotSet`, `ExposureCapExceeded`, `BountyNotSet` |
| `6`  | Risk | 5 | `RateTooHigh`, `ScoreTooHigh`, `Paused`, `DrawCooldownActive`, `RiskAdminCooldownActive` |
| `7`  | Oracle | 5 | `OraclePriceInvalid`, `OraclePriceStale`, `OraclePriceDeviation`, `OracleQuorumNotMet`, `OracleNotFound` |
| `8`  | Collateral | 3 | `CollateralRatioBelowMinimum`, `InsufficientCollateralBalance`, `AdminCollateralCooldownActive` |
| `9`  | Block | 5 | `BorrowerBlocked`, `DrawsFrozen`, `BorrowerFrozen`, `CreditLineFrozen`, `FreezeCooldownActive` |
| `10` | Reentrancy | 1 | `Reentrancy` |
| `11` | Misc | 8 | `CreditLineNotFound`, `AdminAcceptTooEarly`, `NoPendingTreasuryWithdrawal`, `TreasuryTimelockActive`, `TreasuryProposalExists`, `OriginalDrawNotFound`, `AttestationBatchNotFound`, `InvalidAttestation` |
| `12` | Handshake | 2 | `IncompatibleVersion`, `AuctionCallFailed` |
| | **Total** | **61** | |

### Category-level recovery

| Category | Dominant SDK recovery |
|----------|-----------------------|
| Auth | Reconnect the correct wallet; if `AdminNotInitialized`, ask the deployer to run `init()`. |
| Lifecycle | Wait for the admin action, or open a new credit line when the old one is terminal. |
| Numeric | Re-validate inputs client-side; re-sync the ledger view on `TimestampRegression`. |
| Limit | Reduce the amount to fit the headroom, or repay first. |
| Liquidity | Replenish allowance / balance, or wait for the reserve. |
| Risk | Clamp the input, or wait for the cooldown / unpause. |
| Oracle | Await a valid, fresh price from a quorum of feeds. |
| Collateral | Reduce the withdrawal or deposit more collateral. |
| Block | Contact the admin, or wait for the freeze to lift. |
| Reentrancy | Do not retry; inspect on-chain state and the calling token. |
| Misc | Create the missing entity or wait out the timelock. |
| Handshake | Safe to retry once the peer contract is upgraded or the call is corrected. |

---

## SDK Usage

### Rust

```rust
use creditra_credit::types::{ContractError, ContractErrorCategory};

match result {
    Err(e) if e == ContractError::OverLimit as u32 => {
        // handle over-limit
    }
    Err(e) if e == ContractError::CreditLineNotFound as u32 => {
        // handle not found
    }
    _ => {}
}
```

Group by category instead of enumerating codes — `category()` is itself stable
ABI, so a new variant inherits its bucket without an SDK change:

```rust
use creditra_credit::types::{ContractError, ContractErrorCategory};

/// Route access-control and draw-block failures to support; the caller
/// cannot make the call succeed by retrying.
fn needs_support(error: ContractError) -> bool {
    matches!(
        error.category(),
        ContractErrorCategory::Auth | ContractErrorCategory::Block
    )
}
```

### TypeScript (Soroban SDK)

```typescript
import { ContractError } from "@creditra/credit-sdk";

try {
  await client.drawCredit({ borrower, amount });
} catch (err) {
  if (err.code === 6 /* OverLimit */) {
    console.error("Draw exceeds credit limit");
  } else if (err.code === 3 /* CreditLineNotFound */) {
    console.error("No credit line found for borrower");
  }
}
```

---

## Security Notes

### Failure modes and trust boundaries

**Reentrancy (11)** — the guard on `draw_credit` and `repay_credit` is
defense-in-depth. Standard Stellar Asset Contracts do not invoke callbacks into
the caller, so this should never appear in production. If it does, the token
contract in use is non-standard and must be audited.

**Overflow (12)** — all arithmetic on `utilized_amount` uses `checked_add`;
overflow reverts with no state change. Amounts near `i128::MAX`
(~1.7 × 10³⁸) are outside the protocol's intended operating range.

**AlreadyInitialized (14)** — the `init` guard prevents admin takeover via
re-initialization. It reads instance storage before writing, so a rejected
second call leaves storage unchanged.

**AdminAcceptTooEarly (15)** — the two-step admin rotation
(`propose_admin` → `accept_admin`) includes an optional timelock enforced
against `env.ledger().timestamp()`. That clock is coarse and monotonic enough
for governance windows, but is not suitable for sub-second precision.

**Paused (18)** — the emergency circuit breaker blocks every guarded
state-mutating operation except `repay_credit`, which stays live so users can
still reduce debt exposure during an incident. Read-only views are never
blocked.

**Liquidity errors (22–27)** — liquidity-moving paths use typed
`ContractError` codes rather than ad-hoc panic strings, and check configuration,
reserve, allowance, and balance *before* any state mutation. Soroban token calls
that trap internally are not catchable; these variants cover the failures this
contract can observe.

**Handshake errors (61, 62)** — the auction settlement path clears its
reentrancy guard before raising either variant, so no partial credit-line state
is left behind and the settlement is safe to retry once the auction contract is
upgraded (61) or the call is corrected (62).

**AuctionActive (63)** — fee parameters stay frozen while any liquidation
auction is in flight, so an ongoing auction's settlement economics cannot change
mid-flight. The block lifts when the last active auction leaves the `Defaulted`
pipeline.

### Trust model

| Actor | Trusted for |
|-------|-------------|
| Admin | Lifecycle operations, risk parameters, block list, liquidity config |
| Borrower | Drawing and repaying their own credit line only |
| Liquidity token | Standard Stellar Asset Contract behavior (no callbacks) |
| Auction contract | Protocol version match and honest CPI return values |
| Ledger timestamp | Coarse monotonic ordering (governance delays, accrual intervals) |

`Unauthorized` (1) and `NotAdmin` (2) indicate an access-control violation and
should be treated as security-relevant events by monitoring systems.

---

## Related documents

- [`docs/ERROR_MIGRATION.md`](./ERROR_MIGRATION.md) — V1 → V2 client-side error
  encoding migration for the CosmWasm `creditra-credit` package.
- [`docs/errors/collateral.md`](./errors/collateral.md) — `CollateralError`.
- [`docs/errors/freeze.md`](./errors/freeze.md) — `FreezeError`.
- [`docs/PROTOCOL_SPEC.md`](./PROTOCOL_SPEC.md) — per-entrypoint validation
  order and which error each check raises.
- [`docs/indexer-integration.md`](./indexer-integration.md) — event decoding.
