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

## Categorization

Useful when surfacing errors to end users:

- **Auth & lifecycle**: 1, 2, 14, 15, 32.
- **State guards**: 4, 20, 21, 19, 18.
- **Numeric guards**: 5, 6, 7, 8, 9, 10, 12, 13, 17, 28, 31, 34.
- **Treasury / liquidity**: 22, 23, 24, 25, 26, 27, 30.
- **Cooldown / regression**: 29, 33.
- **Oracle circuit breaker**: 36, 37, 38.
- **Collateral**: 35.
- **Concurrency**: 11.
- **Risk / blocklist**: 16.
- **Indexing**: 3.
