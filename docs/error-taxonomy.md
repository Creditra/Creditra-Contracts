# `ContractError` Taxonomy — Recovery Actions by Category

Grouped reference for SDK clients, indexers, and front-end integrators.
Each category names its variants, explains when the contract raises them, and
prescribes an **SDK-side recovery action** the caller can take.

Categories are also available as a stable `#[repr(u32)]` enum —
[`ContractErrorCategory`](../contracts/credit/src/types.rs) — with
[`ContractError::category()`](../contracts/credit/src/types.rs) to map
any error to its category at runtime.

Source of truth: `contracts/credit/src/types.rs`.
Cross-reference: [`docs/contract-errors.md`](./contract-errors.md) (flat code
table).

---

## Auth (codes 1, 2, 32)

| Code | Variant | When raised |
| ---- | ------- | ----------- |
| 1    | `Unauthorized` | Caller is not the expected `Address` for the operation. |
| 2    | `NotAdmin` | Caller invoked an admin-only entrypoint without admin privileges. |
| 32   | `AdminNotInitialized` | Admin address has not been set in contract storage. |

**Recovery action:** Prompt the caller to re-connect a wallet with the correct
role. For `AdminNotInitialized`, advise the deployer to call `init()` with a
valid admin address before any admin-gated operation.

---

## Lifecycle (codes 4, 14, 20, 21, 45)

| Code | Variant | When raised |
| ---- | ------- | ----------- |
| 4    | `CreditLineClosed` | Action attempted on a permanently closed credit line. |
| 14   | `AlreadyInitialized` | `init()` called more than once. |
| 20   | `CreditLineSuspended` | Draw or admin action on a suspended credit line. |
| 21   | `CreditLineDefaulted` | Draw or admin action on a defaulted credit line. |
| 45   | `AlreadySettled` | The liquidation for this (borrower, settlement_id) pair has already been processed. |

**Recovery action:**
- `CreditLineClosed`: No recovery — the line is terminal. Create a new credit
  line.
- `AlreadyInitialized`: No action needed (contract is already live).
- `AlreadySettled`: No action needed — the settlement has already been processed.
- `CreditLineSuspended`: Wait for admin reinstatement or self-reinstatement
  (see [`lifecycle.rs`](../contracts/credit/src/lifecycle.rs)); repayments are
  still allowed.
- `CreditLineDefaulted`: The line is in default; the borrower must either cure
  via repayment or the position must be liquidated through the auction contract.

---

## Numeric (codes 5, 7, 12, 33, 34)

| Code | Variant | When raised |
| ---- | ------- | ----------- |
| 5    | `InvalidAmount` | Amount is zero, negative, or malformed where a positive value was required. |
| 7    | `NegativeLimit` | Attempt to set a credit limit below zero. |
| 12   | `Overflow` | Arithmetic overflow during checked math (e.g., interest proration, limit
calculation). |
| 33   | `TimestampRegression` | Provided or ledger timestamp is not strictly greater than the stored value. |
| 34   | `LimitOutOfBounds` | Credit limit falls outside the configured `[min_limit, max_limit]` range. |

**Recovery action:**
- `InvalidAmount`: Re-validate inputs client-side — ensure draw/repay amounts
  are positive integers within bounds.
- `NegativeLimit`: Clamp or reject the limit value before sending.
- `Overflow`: This indicates an arithmetic failure at the protocol level.
  Retry with a smaller amount or under different rate/accrual conditions; if
  persistent, file a bug report.
- `TimestampRegression`: Likely a client clock issue. Re-sync the caller's
  ledger view and retry.
- `LimitOutOfBounds`: Adjust the proposed limit to `[min_limit, max_limit]`
  using `get_protocol_config()`.

---

## Limit (codes 6, 10, 13, 17, 28)

| Code | Variant | When raised |
| ---- | ------- | ----------- |
| 6    | `OverLimit` | Draw would exceed the borrower's credit limit minus utilized amount. |
| 10   | `UtilizationNotZero` | Operation (e.g., limit decrease, closure) requires zero outstanding debt. |
| 13   | `LimitDecreaseRequiresRepayment` | Credit limit decrease below the currently utilized amount. |
| 17   | `DrawExceedsMaxAmount` | Draw amount exceeds the per-transaction `max_draw_amount`. |
| 28   | `RepayExceedsMaxAmount` | Repay amount exceeds the per-transaction `max_repay_amount`. |

**Recovery action:**
- `OverLimit`: Reduce the draw amount to ≤ `credit_limit - utilized`. Query
  `get_credit_line(borrower)` to compute the available headroom.
- `UtilizationNotZero`: Repay the outstanding balance first, then retry the
  operation.
- `LimitDecreaseRequiresRepayment`: Repay the excess above the new limit
  before decreasing.
- `DrawExceedsMaxAmount` / `RepayExceedsMaxAmount`: Split the transaction into
  smaller chunks or query `max_draw_amount` / `max_repay_amount` from
  protocol config and adjust the input.

---

## Liquidity (codes 22, 23, 24, 25, 26, 27, 30, 31)

| Code | Variant | When raised |
| ---- | ------- | ----------- |
| 22   | `MissingLiquidityToken` | Liquidity token address is not configured. |
| 23   | `MissingLiquiditySource` | Liquidity source contract is not configured. |
| 24   | `InsufficientLiquidityReserve` | The reserve pool cannot cover the requested draw. |
| 25   | `LiquidityTokenCallFailed` | An external call to the liquidity token reverted or returned an error
where the contract can observe it. |
| 26   | `InsufficientRepaymentAllowance` | Borrower's token allowance is below the effective repayment amount. |
| 27   | `InsufficientRepaymentBalance` | Borrower's token balance is below the effective repayment amount. |
| 30   | `TreasuryNotSet` | Treasury address is not configured when attempting a treasury withdrawal. |
| 31   | `ExposureCapExceeded` | Draw would push global `TotalUtilized` above `MaxTotalExposure`. |

**Recovery action:**
- `MissingLiquidityToken` / `MissingLiquiditySource`: Inform the admin to
  complete liquidity configuration; the protocol is not yet operational.
- `InsufficientLiquidityReserve`: Wait for the reserve to be replenished, or
  request admin to add liquidity.
- `LiquidityTokenCallFailed`: Retry the transaction. If the token contract is
  genuinely faulty, the admin must replace it.
- `InsufficientRepaymentAllowance`: Guide the borrower to increase the
  allowance for the contract address.
- `InsufficientRepaymentBalance`: Guide the borrower to deposit more tokens
  into their wallet.
- `TreasuryNotSet`: Admin must call `set_treasury` before withdrawing.
- `ExposureCapExceeded`: Reduce the draw amount or wait for other borrowers to
  repay so the global cap frees headroom. Query `MaxTotalExposure` and current
  `TotalUtilized` from protocol config.

---

## Risk (codes 8, 9, 18, 29)

| Code | Variant | When raised |
| ---- | ------- | ----------- |
| 8    | `RateTooHigh` | Interest rate change exceeds `max_rate_change_bps` or the absolute ceiling. |
| 9    | `ScoreTooHigh` | Risk score exceeds the maximum allowed (100). |
| 18   | `Paused` | Protocol is paused by the emergency circuit breaker. |
| 29   | `DrawCooldownActive` | Borrower attempted to draw before `draw_min_interval_seconds` elapsed. |

**Recovery action:**
- `RateTooHigh`: Clamp the rate change within `RateChangeConfig` bounds. Query
  `get_rate_change_config()` for `max_rate_change_bps`.
- `ScoreTooHigh`: Normalize the risk score to `[0, 100]` before submitting.
- `Paused`: Inform the user that the protocol is paused. Repayments are still
  accepted. Retry when the admin unpauses.
- `DrawCooldownActive`: Wait `draw_min_interval_seconds` from the last draw
  timestamp (available via `get_credit_line(borrower).last_accrual_ts`) before
  retrying.

---

## Oracle (codes 36, 37, 38)

| Code | Variant | When raised |
| ---- | ------- | ----------- |
| 36   | `OraclePriceInvalid` | Oracle price is zero, negative, or malformed. |
| 37   | `OraclePriceStale` | Oracle price exceeds `max_age_seconds` since last update. |
| 38   | `OraclePriceDeviation` | Oracle price deviation exceeds `max_deviation_bps` relative to prior. |

**Recovery action:**
- `OraclePriceInvalid`: Ensure the oracle is returning a valid positive price.
  May indicate a misconfigured oracle address.
- `OraclePriceStale`: Wait for an oracle price update, or trigger one via the
  oracle's push mechanism. Query `max_age_seconds` from `OracleConfig`.
- `OraclePriceDeviation`: A market-moving event or oracle fault. The
  circuit-breaker has tripped; await a new price within the deviation bound.
  Do **not** retry with the same price.

---

## Collateral (codes 35, 39)

| Code | Variant | When raised |
| ---- | ------- | ----------- |
| 35   | `CollateralRatioBelowMinimum` | Collateral withdraw would leave the ratio below `MinCollateralRatioBps`. |
| 39   | `InsufficientCollateralBalance` | Withdrawal amount exceeds the borrower's deposited collateral balance. |

**Recovery action:** For code 35, reduce the withdrawal amount so that
`(post_collateral * MinCollateralRatioBps) / 10_000 >= utilized`. Query the
minimum collateral ratio via `get_protocol_config()` and compute the maximum
safe withdrawal client-side. For code 39, query `get_collateral_balance` and
ensure the requested amount does not exceed it.

---

## Block (codes 16, 19, 40)

| Code | Variant | When raised |
| ---- | ------- | ----------- |
| 16   | `BorrowerBlocked` | Borrower is on the admin-managed block list. |
| 19   | `DrawsFrozen` | Global draw freeze is active (admin action for liquidity reserve ops). |
| 40   | `BorrowerFrozen` | Borrower's draws are temporarily frozen until the specified expiry timestamp. |

**Recovery action:**
- `BorrowerBlocked`: The borrower is permanently blocked. No recovery from the
  SDK side; the borrower must contact the protocol admin.
- `DrawsFrozen`: Inform the user that draws are temporarily frozen. Repayments
  remain open. Retry when the admin unfreezes.
- `BorrowerFrozen`: Wait for the freeze to expire (or contact the admin to
  unfreeze early). The freeze is time-bounded and auto-expires; check
  `get_borrower_frozen_until()` for the remaining duration.

---

## Reentrancy (code 11)

| Code | Variant | When raised |
| ---- | ------- | ----------- |
| 11   | `Reentrancy` | Reentrant call detected during a cross-contract transfer. |

**Recovery action:** Do **not** retry the same transaction — the reentrancy
guard prevents the contract from being called again during an ongoing
operation. Wait for the current transaction to resolve and inspect the on-chain
state before submitting a new call. If you are integrating a token contract,
ensure it does not re-enter the credit contract during `transfer` /
`transfer_from`.

---

## Misc (codes 3, 15)

| Code | Variant | When raised |
| ---- | ------- | ----------- |
| 3    | `CreditLineNotFound` | The specified borrower has no credit line. |
| 15   | `AdminAcceptTooEarly` | Admin acceptance attempted before the `propose_admin` delay elapsed. |

**Recovery action:**
- `CreditLineNotFound`: Direct the caller to create a credit line first via
  `init_credit_line(borrower, limit, ...)`.
- `AdminAcceptTooEarly`: Wait for the full delay window to elapse. Query
  `get_pending_admin()` for the scheduled acceptance timestamp.

---

## Quick reference: category summary

| Category | Codes | Count | Dominant SDK recovery |
| -------- | ----- | ----- | --------------------- |
| Auth | 1, 2, 32 | 3 | Reconnect wallet / re-deploy with admin init |
| Lifecycle | 4, 14, 20, 21, 45 | 5 | Await admin action or create new line |
| Numeric | 5, 7, 12, 33, 34 | 5 | Validate inputs / re-sync ledger view |
| Limit | 6, 10, 13, 17, 28 | 5 | Reduce amount or repay first |
| Liquidity | 22, 23, 24, 25, 26, 27, 30, 31, 41 | 9 | Replenish allowance / wait for reserve |
| Risk | 8, 9, 18, 29 | 4 | Clamp inputs / wait for cooldown or unpause |
| Oracle | 36, 37, 38 | 3 | Await valid price feed |
| Collateral | 35, 39 | 2 | Reduce withdrawal amount |
| Block | 16, 19, 40 | 3 | Contact admin or wait for unfreeze / expiry |
| Reentrancy | 11 | 1 | Do not retry; inspect on-chain state |
| Misc | 3, 15, 42, 43, 44 | 5 | Create line first / wait for delay |
| **Total** | 1–45 | **45** | — |
