# ContractError Codes — Categorized Reference

> **Source of truth:** [`ContractError`](../contracts/credit/src/types.rs) enum in
> `contracts/credit/src/types.rs`.
>
> **CI guard:** `tests/error_discriminants.rs` pins every discriminant and category
> mapping. `ContractErrorCategory` discriminants are also pinned.
>
> **Runtime API:** [`ContractError::category()`](../contracts/credit/src/types.rs)
> returns the [`ContractErrorCategory`](../contracts/credit/src/types.rs) enum.

---

## Categories

| Category      | Discriminant | Count | Description |
|---------------|:------------:|:-----:|-------------|
| Auth          | 1            | 4     | Authentication / authorization failures |
| Lifecycle     | 2            | 5     | Credit-line lifecycle state violations |
| Numeric       | 3            | 6     | Numeric computation failures |
| Limit         | 4            | 8     | Credit limit / draw / repay cap violations |
| Liquidity     | 5            | 12    | Liquidity configuration or reserve failures |
| Risk          | 6            | 5     | Risk-parameter violations |
| Oracle        | 7            | 5     | Oracle price-feed failures |
| Collateral    | 8            | 2     | Collateral ratio or balance violations |
| Block         | 9            | 4     | Draw-block conditions (blocked, frozen) |
| Reentrancy    | 10           | 1     | Reentrancy guard violations |
| Misc          | 11           | 7     | Miscellaneous errors |
| **Total**     | 1–11         | **59** | — |

---

## 1. Auth (codes 1, 2, 32, 55)

Authentication or authorization failures — the caller does not have the required privileges.

| Code | Variant | Meaning | Returned When |
|:----:|---------|---------|---------------|
| 1    | `Unauthorized` | Caller is not authorized for this action | `accept_admin` if caller is not the pending admin; borrower auth mismatch |
| 2    | `NotAdmin` | Caller does not have admin privileges | Admin-gated entrypoints when caller is not the stored admin |
| 32   | `AdminNotInitialized` | Admin address not yet set in storage | Any admin-gated entrypoint before `init` is called |
| 55   | `BorrowerMismatch` | Stored borrower does not match the load key | Data integrity check on credit line load |

**SDK recovery:** Prompt the caller to re-connect a wallet with the correct role. For `AdminNotInitialized`, advise the deployer to call `init()` with a valid admin address.

---

## 2. Lifecycle (codes 4, 14, 20, 21, 51)

The credit-line is in a state that prevents the requested operation.

| Code | Variant | Meaning | Returned When |
|:----:|---------|---------|---------------|
| 4    | `CreditLineClosed` | Credit line is permanently closed | Draw or repay on a closed line |
| 14   | `AlreadyInitialized` | `init()` called more than once | Second call to `init` |
| 20   | `CreditLineSuspended` | Credit line is suspended | Draw or state transition on a suspended line |
| 21   | `CreditLineDefaulted` | Credit line is defaulted | Draw or state transition on a defaulted line |
| 51   | `AlreadySettled` | Liquidation settlement already processed | Replay of the same `(borrower, settlement_id)` pair |

**SDK recovery:**
- `CreditLineClosed`: No recovery — the line is terminal. Create a new credit line.
- `AlreadyInitialized`: No action needed (contract is already live).
- `AlreadySettled`: No action needed — the settlement has already been processed.
- `CreditLineSuspended`: Wait for admin reinstatement or self-reinstatement; repayments are still allowed.
- `CreditLineDefaulted`: The line is in default; the borrower must either cure via repayment or the position must be liquidated.

---

## 3. Numeric (codes 5, 7, 12, 33, 34, 52)

Arithmetic or numeric computation failures — inputs or calculations fall outside acceptable ranges.

| Code | Variant | Meaning | Returned When |
|:----:|---------|---------|---------------|
| 5    | `InvalidAmount` | Amount is zero, negative, or malformed | `draw_credit`, `repay_credit`, collateral operations, config setters |
| 7    | `NegativeLimit` | Credit limit cannot be negative | `open_credit_line`, `update_risk_parameters` |
| 12   | `Overflow` | Arithmetic overflow during calculation | `draw_credit` utilization add, collateral math, interest accrual |
| 33   | `TimestampRegression` | Timestamp regression detected | Storage guard `assert_ts_monotonic`, risk update |
| 34   | `LimitOutOfBounds` | Credit limit outside configured min/max | `open_credit_line`, `update_risk_parameters` |
| 52   | `InvalidRiskWeight` | Collateral risk weight exceeds 10 000 bps | `set_collateral_risk_weight` |

**SDK recovery:**
- `InvalidAmount`: Re-validate inputs client-side.
- `NegativeLimit`: Clamp or reject the limit value before sending.
- `Overflow`: Retry with a smaller amount or under different rate/accrual conditions.
- `TimestampRegression`: Re-sync the caller's ledger view and retry.
- `LimitOutOfBounds`: Adjust the proposed limit to `[min_limit, max_limit]`.
- `InvalidRiskWeight`: Ensure risk weight is in `0..=10_000` bps.

---

## 4. Limit (codes 6, 10, 13, 17, 28, 45, 47, 59)

Draw, repay, or limit operations that violate numeric caps or boundary conditions.

| Code | Variant | Meaning | Returned When |
|:----:|---------|---------|---------------|
| 6    | `OverLimit` | Draw would exceed the credit limit | `draw_credit` when utilized + amount > limit |
| 10   | `UtilizationNotZero` | Operation requires zero utilization | `close_credit_line` with outstanding debt |
| 13   | `LimitDecreaseRequiresRepayment` | Limit decrease below utilized | Limit-decrease enforcement |
| 17   | `DrawExceedsMaxAmount` | Draw exceeds per-transaction cap | `draw_credit` when amount > `MaxDrawAmount` |
| 28   | `RepayExceedsMaxAmount` | Repay exceeds per-transaction cap | `repay_credit` when amount > `MaxRepayAmount` |
| 45   | `CloseFactorAboveMax` | Close factor exceeds protocol maximum | `settle_default_liquidation` validation |
| 47   | `DrawReversalWindowExpired` | Draw reversal window has expired | `reverse_draw` after `DRAW_REVERSAL_WINDOW_SECS` |
| 59   | `UtilizedNotZero` | Utilization must be zero | Borrower self-close with outstanding debt |

**SDK recovery:**
- `OverLimit`: Reduce draw amount to ≤ `credit_limit - utilized`.
- `UtilizationNotZero`: Repay outstanding balance first.
- `LimitDecreaseRequiresRepayment`: Repay excess before decreasing.
- `DrawExceedsMaxAmount` / `RepayExceedsMaxAmount`: Split into smaller chunks.
- `CloseFactorAboveMax`: Reduce close factor to ≤ configured max.
- `DrawReversalWindowExpired`: Reversal is no longer possible.

---

## 5. Liquidity (codes 22, 23, 24, 25, 26, 27, 30, 31, 41, 56, 57, 58)

Liquidity configuration is missing or a reserve/allowance/balance check fails.

| Code | Variant | Meaning | Returned When |
|:----:|---------|---------|---------------|
| 22   | `MissingLiquidityToken` | Liquidity token not configured | `draw_credit`, collateral ops before token set |
| 23   | `MissingLiquiditySource` | Liquidity source not configured | `draw_credit` before source set |
| 24   | `InsufficientLiquidityReserve` | Reserve cannot cover draw | `draw_credit` when reserve balance < amount |
| 25   | `LiquidityTokenCallFailed` | Token call failed (observable) | Token CPI failure |
| 26   | `InsufficientRepaymentAllowance` | Allowance below repayment | Allowance check |
| 27   | `InsufficientRepaymentBalance` | Balance below repayment | Balance check |
| 30   | `TreasuryNotSet` | Treasury not configured | `propose_treasury_withdrawal` without treasury |
| 31   | `ExposureCapExceeded` | Global exposure cap exceeded | `draw_credit` when total_utilized + amount > cap |
| 41   | `BountyNotSet` | Bounty address not configured | `withdraw_bounty` without bounty set |
| 56   | `InsufficientReserve` | Reserve balance below draw amount | `draw_credit` when token reserve < amount |
| 57   | `InsufficientAllowance` | Borrower token allowance insufficient | `repay_credit` when allowance < repayment |
| 58   | `InsufficientBalance` | Borrower token balance insufficient | `repay_credit` when balance < repayment |

**SDK recovery:**
- `MissingLiquidityToken` / `MissingLiquiditySource`: Admin must complete liquidity configuration.
- `InsufficientLiquidityReserve` / `InsufficientReserve`: Wait for reserve replenishment.
- `LiquidityTokenCallFailed`: Retry; if persistent, admin must replace token.
- `InsufficientRepaymentAllowance` / `InsufficientAllowance`: Guide borrower to increase allowance.
- `InsufficientRepaymentBalance` / `InsufficientBalance`: Guide borrower to deposit more tokens.
- `TreasuryNotSet`: Admin must call `set_treasury`.
- `ExposureCapExceeded`: Reduce draw amount or wait for repayments.
- `BountyNotSet`: Admin must call `set_bounty`.

---

## 6. Risk (codes 8, 9, 18, 29, 53)

Risk-parameter or protocol-state violations.

| Code | Variant | Meaning | Returned When |
|:----:|---------|---------|---------------|
| 8    | `RateTooHigh` | Rate exceeds maximum allowed | `open_credit_line`, `set_borrower_rate_ceiling` |
| 9    | `ScoreTooHigh` | Risk score exceeds max (100) | `open_credit_line` with score > 100 |
| 18   | `Paused` | Protocol is paused | State-changing operations while paused |
| 29   | `DrawCooldownActive` | Draw within cooldown window | `draw_credit` before cooldown elapses |
| 53   | `AdminQueryCooldownActive` | Admin query-critical action within cooldown | `set_oracle_config`, `set_rate_formula_config`, etc. before cooldown elapses |

**SDK recovery:**
- `RateTooHigh`: Clamp rate change within `RateChangeConfig` bounds.
- `ScoreTooHigh`: Normalize risk score to `[0, 100]`.
- `Paused`: Inform user; repayments are still accepted. Retry when unpaused.
- `DrawCooldownActive`: Wait `draw_min_interval_seconds` before retrying.
- `AdminQueryCooldownActive`: Wait for the configured admin query cooldown to elapse.

---

## 7. Oracle (codes 36, 37, 38, 50, 54)

Oracle price-feed failures — the price data cannot be trusted.

| Code | Variant | Meaning | Returned When |
|:----:|---------|---------|---------------|
| 36   | `OraclePriceInvalid` | Price is zero, negative, or malformed | `settle_default_liquidation` oracle validation |
| 37   | `OraclePriceStale` | Price exceeds `max_age_seconds` | `settle_default_liquidation` staleness check |
| 38   | `OraclePriceDeviation` | Price deviation exceeds max allowed | `settle_default_liquidation` deviation check |
| 50   | `OracleQuorumNotMet` | Quorum of K agreeing feeds not met | `submit_oracle_prices` quorum resolution |
| 54   | `OracleNotFound` | Oracle address not in the registry | `remove_oracle` when oracle not registered |

**SDK recovery:**
- `OraclePriceInvalid`: Ensure oracle returns a valid positive price.
- `OraclePriceStale`: Wait for oracle price update.
- `OraclePriceDeviation`: Circuit-breaker tripped; await a new price within bound.
- `OracleQuorumNotMet`: Submit prices from more independent feeds.
- `OracleNotFound`: Verify the oracle address is registered before removal.

---

## 8. Collateral (codes 35, 39)

Collateral ratio or balance violations.

| Code | Variant | Meaning | Returned When |
|:----:|---------|---------|---------------|
| 35   | `CollateralRatioBelowMinimum` | Collateral ratio below minimum | `draw_credit`, collateral withdrawal |
| 39   | `InsufficientCollateralBalance` | Collateral balance too low | Collateral withdrawal |

**SDK recovery:**
- Code 35: Reduce withdrawal amount so post-withdrawal HF ≥ minimum.
- Code 39: Query `get_collateral_balance` and ensure requested amount ≤ balance.

---

## 9. Block (codes 16, 19, 40, 46)

Draw-block conditions — the borrower, line, or protocol prevents draws.

| Code | Variant | Meaning | Returned When |
|:----:|---------|---------|---------------|
| 16   | `BorrowerBlocked` | Borrower is on the blocked list | `draw_credit` when borrower is blocked |
| 19   | `DrawsFrozen` | Draws globally frozen | `draw_credit` when `DrawsFrozen` is set |
| 40   | `BorrowerFrozen` | Borrower draws frozen until expiry | `draw_credit` when per-borrower freeze active |
| 46   | `CreditLineFrozen` | Credit line frozen by admin | `draw_credit` when per-line freeze active |

**SDK recovery:**
- `BorrowerBlocked`: Permanent — borrower must contact admin.
- `DrawsFrozen`: Temporary — repayments remain open.
- `BorrowerFrozen`: Time-bounded — wait for expiry or contact admin.
- `CreditLineFrozen`: Admin hold — wait for `unfreeze_credit_line`.

---

## 10. Reentrancy (code 11)

| Code | Variant | Meaning | Returned When |
|:----:|---------|---------|---------------|
| 11   | `Reentrancy` | Reentrant call detected | Reentrant call to a state-changing entrypoint |

**SDK recovery:** Do **not** retry the same transaction. Wait for resolution and inspect on-chain state before submitting a new call. If integrating a token contract, ensure it does not re-enter the credit contract during `transfer` / `transfer_from`.

---

## 11. Misc (codes 3, 15, 42, 43, 44, 48, 49)

Errors that do not fit into other categories — entity-not-found, timelock, and treasury proposal conflicts.

| Code | Variant | Meaning | Returned When |
|:----:|---------|---------|---------------|
| 3    | `CreditLineNotFound` | Credit line does not exist | Any operation on a borrower without an open line |
| 15   | `AdminAcceptTooEarly` | Admin acceptance before timelock | `accept_admin` before delay expires |
| 42   | `NoPendingTreasuryWithdrawal` | No pending proposal | `execute_treasury_withdrawal` without proposal |
| 43   | `TreasuryTimelockActive` | 24h timelock not elapsed | `execute_treasury_withdrawal` before timelock |
| 44   | `TreasuryProposalExists` | Proposal already exists | `propose_treasury_withdrawal` while pending |
| 48   | `OriginalDrawNotFound` | Draw audit record not found | Draw reversal without matching record |
| 49   | `AttestationBatchNotFound` | No attestation batch committed | `verify_attestation_proof` without batch |
| 50   | `OracleQuorumNotMet` | Oracle quorum condition not satisfied | `submit_oracle_prices` quorum resolution |
| 51   | `AlreadySettled` | Liquidation settlement already processed | Replay of the same `(borrower, settlement_id)` pair |
| 52   | `InvalidRiskWeight` | Collateral risk weight exceeds 10 000 bps | `set_collateral_risk_weight` |
| 53   | `InvalidAttestation` | Attestation proof is invalid or no batch committed | `verify_attestation_proof` with an invalid proof or missing batch |
| 55   | `LiquidationGraceActive` | Per-borrower liquidation grace window active | `default_credit_line` called before grace period expiry |

**SDK recovery:**
- `CreditLineNotFound`: Create a credit line first via `open_credit_line`.
- `AdminAcceptTooEarly`: Wait for the full delay window to elapse.
- `NoPendingTreasuryWithdrawal`: Create a proposal first.
- `TreasuryTimelockActive`: Wait for 24h timelock.
- `TreasuryProposalExists`: Execute or cancel existing proposal first.
- `OriginalDrawNotFound`: No reversal possible — no matching draw record.
- `AttestationBatchNotFound`: Admin must commit a batch first.

---

## Summary

| Category      | Codes | Count | Dominant SDK Recovery |
|---------------|:-----:|:-----:|-----------------------|
| Auth          | 1, 2, 32, 55 | 4 | Reconnect wallet / re-deploy with admin init |
| Lifecycle     | 4, 14, 20, 21, 51 | 5 | Await admin action or create new line |
| Numeric       | 5, 7, 12, 33, 34, 52 | 6 | Validate inputs / re-sync ledger view |
| Limit         | 6, 10, 13, 17, 28, 45, 47, 59 | 8 | Reduce amount or repay first |
| Liquidity     | 22, 23, 24, 25, 26, 27, 30, 31, 41, 56, 57, 58 | 12 | Replenish allowance / wait for reserve |
| Risk          | 8, 9, 18, 29, 53 | 5 | Clamp inputs / wait for cooldown or unpause |
| Oracle        | 36, 37, 38, 50, 54 | 5 | Await valid price feed |
| Collateral    | 35, 39 | 2 | Reduce withdrawal amount |
| Block         | 16, 19, 40, 46 | 4 | Contact admin or wait for unfreeze / expiry |
| Reentrancy    | 11 | 1 | Do not retry; inspect on-chain state |
| Misc          | 3, 15, 42, 43, 44, 48, 49 | 7 | Create line first / wait for delay |
| **Total**     | 1–59 | **59** | — |
