# `ContractError` reference

Authoritative table of every error code the `creditra-credit` contract can
return. Source of truth: `contracts/credit/src/types.rs`.

Regenerate with:

```bash
scripts/list_contract_errors.py
```

## Stability

Discriminants are part of the contract ABI. Existing variants must never
be reordered or renumbered; new variants must be appended.

## Codes

| Code | Variant | Meaning |
| ---: | ------- | ------- |
| 1  | `Unauthorized` | Caller is not authorized. |
| 2  | `NotAdmin` | Caller lacks admin privileges. |
| 3  | `CreditLineNotFound` | Credit line does not exist. |
| 4  | `CreditLineClosed` | Credit line is permanently closed. |
| 5  | `InvalidAmount` | Amount is zero, negative, or otherwise invalid. |
| 6  | `OverLimit` | Draw would exceed the credit limit. |
| 7  | `NegativeLimit` | Credit limit cannot be negative. |
| 8  | `RateTooHigh` | Interest rate exceeds maximum allowed. |
| 9  | `ScoreTooHigh` | Risk score exceeds maximum (100). |
| 10 | `UtilizationNotZero` | Operation requires zero utilization. |
| 11 | `Reentrancy` | Reentrancy detected. |
| 12 | `Overflow` | Arithmetic overflow. |
| 13 | `LimitDecreaseRequiresRepayment` | Limit decrease below utilized amount. |
| 14 | `AlreadyInitialized` | Contract already initialized. |
| 15 | `AdminAcceptTooEarly` | Admin acceptance attempted before delay elapsed. |
| 16 | `BorrowerBlocked` | Borrower is blocked from drawing. |
| 17 | `DrawExceedsMaxAmount` | Draw exceeds per-tx cap. |
| 18 | `Paused` | Protocol is paused (circuit breaker). |
| 19 | `DrawsFrozen` | Draws are globally frozen. |
| 20 | `CreditLineSuspended` | Credit line is suspended. |
| 21 | `CreditLineDefaulted` | Credit line is defaulted. |
| 22 | `MissingLiquidityToken` | Liquidity token is not configured. |
| 23 | `MissingLiquiditySource` | Liquidity source is not configured. |
| 24 | `InsufficientLiquidityReserve` | Reserve balance below draw amount. |
| 25 | `LiquidityTokenCallFailed` | Liquidity token call failed where observable. |
| 26 | `InsufficientRepaymentAllowance` | Borrower allowance below repayment. |
| 27 | `InsufficientRepaymentBalance` | Borrower balance below repayment. |
| 28 | `RepayExceedsMaxAmount` | Repay exceeds per-tx cap. |
| 29 | `DrawCooldownActive` | Draw attempted within cooldown window. |
| 30 | `TreasuryNotSet` | Treasury address not configured. |
| 31 | `ExposureCapExceeded` | Draw would exceed global exposure cap. |
| 32 | `AdminNotInitialized` | Admin address not initialized. |
| 33 | `TimestampRegression` | Timestamp not strictly greater than stored value. |
| 34 | `LimitOutOfBounds` | Credit limit outside configured min/max. |
| 35 | `CollateralRatioBelowMinimum` | Collateral ratio below minimum. |
| 36 | `OraclePriceInvalid` | Oracle price is zero, negative, or malformed. |
| 37 | `OraclePriceStale` | Oracle price exceeds `max_age_seconds`. |
| 38 | `OraclePriceDeviation` | Oracle price deviation exceeds configured maximum. |

## Taxonomy

See [`docs/error-taxonomy.md`](./error-taxonomy.md) for the authoritative
grouping of all 38 variants into **named categories** (Auth, Lifecycle,
Numeric, Limit, Liquidity, Risk, Oracle, Collateral, Block, Reentrancy, Misc)
with **SDK-side recovery actions** per category.
