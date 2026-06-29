# ContractError Codes

> **Source of truth:** [`ContractError`](../contracts/credit/src/types.rs) enum in
> `contracts/credit/src/types.rs`.
>
> **CI guard:** `tests/error_discriminants.rs` pins every discriminant.

---

## Error Categories

| Code | Category | Description                          |
|------|----------|--------------------------------------|
| 1    | `Auth`   | Authentication / authorization failures |
| 2    | `Math`   | Arithmetic / computation failures    |
| 3    | `State`  | Contract or entity state prevents the operation |
| 4    | `Oracle` | Oracle price-feed failures           |

Each `ContractError` variant maps to exactly one category via
[`ContractError::category()`](../contracts/credit/src/types.rs).

---

## Auth (Category 1)

Authentication or authorization failures — the caller does not have the
required privileges.

| Code | Variant                | Meaning                                      | Returned When |
|------|------------------------|----------------------------------------------|---------------|
| 1    | `Unauthorized`         | Caller is not authorized for this action      | `accept_admin` if caller is not the pending admin |
| 2    | `NotAdmin`             | Caller does not have admin privileges         | Reserved; admin checks use `AdminNotInitialized` |
| 11   | `Reentrancy`           | Reentrancy detected during cross-contract call | Reentrant call to a state-changing entrypoint |
| 15   | `AdminAcceptTooEarly`  | Admin acceptance before timelock elapsed      | `accept_admin` before delay expires |
| 32   | `AdminNotInitialized`  | Admin address not yet set in storage          | Any admin-gated entrypoint before `init` |

### Compatibility

- Codes 1, 2, 11, 15, 32 unchanged from previous releases.

---

## Math (Category 2)

Arithmetic or numeric computation failures — inputs or calculations fall
outside acceptable ranges.

| Code | Variant              | Meaning                                  | Returned When |
|------|----------------------|------------------------------------------|---------------|
| 5    | `InvalidAmount`      | Amount is zero, negative, or invalid     | `draw_credit`, `repay_credit`, collateral operations, config setters |
| 7    | `NegativeLimit`      | Credit limit cannot be negative          | `open_credit_line`, `update_risk_parameters` |
| 12   | `Overflow`           | Arithmetic overflow during calculation   | `draw_credit` utilization add, collateral math, interest accrual |
| 33   | `TimestampRegression`| Timestamp regression detected            | Storage guard `assert_ts_monotonic`, risk update |

### Compatibility

- Codes 5, 7, 12, 33 unchanged from previous releases.

---

## State (Category 3)

The contract configuration, credit-line lifecycle, or entity state prevents
the requested operation. This is the largest category and includes
lifecycle, liquidity, limit, risk, collateral, and system-configuration
errors.

| Code | Variant                           | Meaning                                    | Returned When |
|------|-----------------------------------|--------------------------------------------|---------------|
| 3    | `CreditLineNotFound`              | Credit line does not exist                 | Any operation on a borrower without an open line |
| 4    | `CreditLineClosed`                | Credit line is permanently closed          | Draw or repay on closed line |
| 6    | `OverLimit`                       | Draw exceeds credit limit                  | `draw_credit` when utilized + amount > limit |
| 8    | `RateTooHigh`                     | Rate exceeds maximum allowed               | `open_credit_line`, `set_borrower_rate_ceiling` |
| 9    | `ScoreTooHigh`                    | Risk score exceeds max (100)               | `open_credit_line` with score > 100 |
| 10   | `UtilizationNotZero`              | Operation requires zero utilization        | `close_credit_line` with outstanding debt |
| 13   | `LimitDecreaseRequiresRepayment`  | Limit decrease below utilized              | Reserved for limit-decrease enforcement |
| 14   | `AlreadyInitialized`              | Contract already initialized               | `init` called a second time |
| 16   | `BorrowerBlocked`                 | Borrower is on the blocked list            | `draw_credit` when borrower is blocked |
| 17   | `DrawExceedsMaxAmount`            | Draw exceeds per-transaction cap           | `draw_credit` when amount > `MaxDrawAmount` |
| 18   | `Paused`                          | Protocol is paused                         | `draw_credit`, admin actions while paused |
| 19   | `DrawsFrozen`                     | Draws globally frozen                      | `draw_credit` when `DrawsFrozen` is set |
| 20   | `CreditLineSuspended`             | Credit line is suspended                   | `draw_credit` when line status is Suspended |
| 21   | `CreditLineDefaulted`             | Credit line is defaulted                   | `draw_credit` when line status is Defaulted |
| 22   | `MissingLiquidityToken`           | Liquidity token not configured             | `draw_credit`, collateral ops before token set |
| 23   | `MissingLiquiditySource`          | Liquidity source not configured            | `draw_credit` before source set |
| 24   | `InsufficientLiquidityReserve`    | Reserve cannot cover draw                  | `draw_credit` when reserve balance < amount |
| 25   | `LiquidityTokenCallFailed`        | Token call failed (observable)             | Reserved for token CPI failure |
| 26   | `InsufficientRepaymentAllowance`  | Allowance below repayment                  | Reserved for allowance check |
| 27   | `InsufficientRepaymentBalance`    | Balance below repayment                    | Reserved for balance check |
| 28   | `RepayExceedsMaxAmount`           | Repay exceeds per-transaction cap          | `repay_credit` when amount > `MaxRepayAmount` |
| 29   | `DrawCooldownActive`              | Draw within cooldown window                | `draw_credit` before cooldown elapses |
| 30   | `TreasuryNotSet`                  | Treasury not configured                    | `propose_treasury_withdrawal` without treasury |
| 31   | `ExposureCapExceeded`             | Global exposure cap exceeded               | `draw_credit` when total_utilized + amount > cap |
| 34   | `LimitOutOfBounds`                | Limit outside min/max bounds               | `open_credit_line` with limit < min or > max |
| 35   | `CollateralRatioBelowMinimum`     | Collateral ratio too low                   | `draw_credit`, collateral withdrawal |
| 39   | `InsufficientCollateralBalance`   | Collateral balance too low                 | Collateral withdrawal |
| 40   | `BorrowerFrozen`                  | Borrower draws frozen until expiry         | `draw_credit` when per-borrower freeze active |
| 41   | `BountyNotSet`                    | Bounty address not configured              | `withdraw_bounty` without bounty set |
| 42   | `NoPendingTreasuryWithdrawal`     | No pending withdrawal proposal             | `execute_treasury_withdrawal` without proposal |
| 43   | `TreasuryTimelockActive`          | 24h timelock not elapsed                   | `execute_treasury_withdrawal` before timelock |
| 44   | `TreasuryProposalExists`          | Proposal already exists                    | `propose_treasury_withdrawal` while pending |
| 45   | `CreditLineFrozen`                | Credit line frozen by admin                | `draw_credit` when per-line freeze active |
| 46   | `AttestationBatchNotFound`        | No attestation batch committed             | Attestation verification without a batch |

### Compatibility

- **Codes 3, 4, 6, 8, 9, 10, 13, 14, 16–31, 34, 35, 39–44** unchanged.
- **Code 45 (`CreditLineFrozen`)** — new variant. Previously this was a gap
  or unreachable path; it now maps to a stable discriminant.
- **Code 46 (`AttestationBatchNotFound`)** — new variant. Previously this was
  handled by `unwrap_or_else` in `attestation.rs` without a stable code.

---

## Oracle (Category 4)

Oracle price-feed failures — the price data cannot be trusted.

| Code | Variant               | Meaning                                  | Returned When |
|------|-----------------------|------------------------------------------|---------------|
| 36   | `OraclePriceInvalid`  | Price is zero, negative, or malformed    | `settle_default_liquidation` oracle validation |
| 37   | `OraclePriceStale`    | Price exceeds max_age_seconds            | `settle_default_liquidation` staleness check |
| 38   | `OraclePriceDeviation`| Price deviation exceeds max allowed      | `settle_default_liquidation` deviation check |

### Compatibility

- Codes 36, 37, 38 unchanged from previous releases.

---

## Summary of Changes

| Aspect               | Change |
|----------------------|--------|
| **Categories**       | Reduced from 11 to 4: `Auth` (1), `Math` (2), `State` (3), `Oracle` (4) |
| **Variants**         | Added `CreditLineFrozen` (45), `AttestationBatchNotFound` (46) |
| **Codes preserved**  | All 44 existing codes (1–44) remain unchanged |
| **Behavior**         | No runtime behavior changes; all panics and error returns identical |
