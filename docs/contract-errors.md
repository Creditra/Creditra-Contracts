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

Each variant also belongs to a stable [`ContractErrorCategory`](../contracts/credit/src/types.rs)
accessible via [`ContractError::category()`](../contracts/credit/src/types.rs).
See the [category enum reference](#contracterrorcategory) below.

## Codes

| Code | Variant | Category | Meaning |
| ---: | ------- | -------- | ------- |
| 1  | `Unauthorized` | Auth | Caller is not authorized. |
| 2  | `NotAdmin` | Auth | Caller lacks admin privileges. |
| 3  | `CreditLineNotFound` | Misc | Credit line does not exist. |
| 4  | `CreditLineClosed` | Lifecycle | Credit line is permanently closed. |
| 5  | `InvalidAmount` | Numeric | Amount is zero, negative, or otherwise invalid. |
| 6  | `OverLimit` | Limit | Draw would exceed the credit limit. |
| 7  | `NegativeLimit` | Numeric | Credit limit cannot be negative. |
| 8  | `RateTooHigh` | Risk | Interest rate exceeds maximum allowed. |
| 9  | `ScoreTooHigh` | Risk | Risk score exceeds maximum (100). |
| 10 | `UtilizationNotZero` | Limit | Operation requires zero utilization. |
| 11 | `Reentrancy` | Reentrancy | Reentrancy detected. |
| 12 | `Overflow` | Numeric | Arithmetic overflow. |
| 13 | `LimitDecreaseRequiresRepayment` | Limit | Limit decrease below utilized amount. |
| 14 | `AlreadyInitialized` | Lifecycle | Contract already initialized. |
| 15 | `AdminAcceptTooEarly` | Misc | Admin acceptance attempted before delay elapsed. |
| 16 | `BorrowerBlocked` | Block | Borrower is blocked from drawing. |
| 17 | `DrawExceedsMaxAmount` | Limit | Draw exceeds per-tx cap. |
| 18 | `Paused` | Risk | Protocol is paused (circuit breaker). |
| 19 | `DrawsFrozen` | Block | Draws are globally frozen. |
| 20 | `CreditLineSuspended` | Lifecycle | Credit line is suspended. |
| 21 | `CreditLineDefaulted` | Lifecycle | Credit line is defaulted. |
| 22 | `MissingLiquidityToken` | Liquidity | Liquidity token is not configured. |
| 23 | `MissingLiquiditySource` | Liquidity | Liquidity source is not configured. |
| 24 | `InsufficientLiquidityReserve` | Liquidity | Reserve balance below draw amount. |
| 25 | `LiquidityTokenCallFailed` | Liquidity | Liquidity token call failed where observable. |
| 26 | `InsufficientRepaymentAllowance` | Liquidity | Borrower allowance below repayment. |
| 27 | `InsufficientRepaymentBalance` | Liquidity | Borrower balance below repayment. |
| 28 | `RepayExceedsMaxAmount` | Limit | Repay exceeds per-tx cap. |
| 29 | `DrawCooldownActive` | Risk | Draw attempted within cooldown window. |
| 30 | `TreasuryNotSet` | Liquidity | Treasury address not configured. |
| 31 | `ExposureCapExceeded` | Liquidity | Draw would exceed global exposure cap. |
| 32 | `AdminNotInitialized` | Auth | Admin address not initialized. |
| 33 | `TimestampRegression` | Numeric | Timestamp not strictly greater than stored value. |
| 34 | `LimitOutOfBounds` | Numeric | Credit limit outside configured min/max. |
| 35 | `CollateralRatioBelowMinimum` | Collateral | Collateral ratio below minimum. |
| 36 | `OraclePriceInvalid` | Oracle | Oracle price is zero, negative, or malformed. |
| 37 | `OraclePriceStale` | Oracle | Oracle price exceeds `max_age_seconds`. |
| 38 | `OraclePriceDeviation` | Oracle | Oracle price deviation exceeds configured maximum. |
| 39 | `InsufficientCollateralBalance` | Collateral | Borrower collateral balance below withdrawal amount. |
| 40 | `BorrowerFrozen` | Block | Borrower's draws are temporarily frozen until expiry. |

## `ContractErrorCategory`

[`ContractErrorCategory`](../contracts/credit/src/types.rs) is a stable
`#[repr(u32)]` enum that groups `ContractError` variants into 11 named
categories. Access it at runtime via [`ContractError::category()`](../contracts/credit/src/types.rs).

| Code | Category | Variants |
| ---: | -------- | -------- |
| 1  | Auth | `Unauthorized`, `NotAdmin`, `AdminNotInitialized` |
| 2  | Lifecycle | `CreditLineClosed`, `AlreadyInitialized`, `CreditLineSuspended`, `CreditLineDefaulted` |
| 3  | Numeric | `InvalidAmount`, `NegativeLimit`, `Overflow`, `TimestampRegression`, `LimitOutOfBounds` |
| 4  | Limit | `OverLimit`, `UtilizationNotZero`, `LimitDecreaseRequiresRepayment`, `DrawExceedsMaxAmount`, `RepayExceedsMaxAmount` |
| 5  | Liquidity | `MissingLiquidityToken`, `MissingLiquiditySource`, `InsufficientLiquidityReserve`, `LiquidityTokenCallFailed`, `InsufficientRepaymentAllowance`, `InsufficientRepaymentBalance`, `TreasuryNotSet`, `ExposureCapExceeded` |
| 6  | Risk | `RateTooHigh`, `ScoreTooHigh`, `Paused`, `DrawCooldownActive` |
| 7  | Oracle | `OraclePriceInvalid`, `OraclePriceStale`, `OraclePriceDeviation` |
| 8  | Collateral | `CollateralRatioBelowMinimum`, `InsufficientCollateralBalance` |
| 9  | Block | `BorrowerBlocked`, `DrawsFrozen`, `BorrowerFrozen` |
| 10 | Reentrancy | `Reentrancy` |
| 11 | Misc | `CreditLineNotFound`, `AdminAcceptTooEarly` |

## Taxonomy

See [`docs/error-taxonomy.md`](./error-taxonomy.md) for the authoritative
grouping of all 40 variants into **named categories** (Auth, Lifecycle,
Numeric, Limit, Liquidity, Risk, Oracle, Collateral, Block, Reentrancy, Misc)
with **SDK-side recovery actions** per category.
