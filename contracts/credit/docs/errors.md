# Contract Error Reference

This document provides detailed information about all error variants in the Creditra credit contract.

## Error Discriminants

All errors are represented as `ContractError` enum variants with stable discriminant values. These discriminants are **permanent** and must never be reordered or renumbered.

---

## Error Catalog

### 1. Unauthorized (Code: 1)
**Description:** Caller is not authorized to perform this action.

**Trigger Conditions:**
- Attempting to perform an action without proper authorization
- Missing required signature

**Recovery:** Ensure the correct account signs the transaction.

---

### 2. NotAdmin (Code: 2)
**Description:** Caller does not have admin privileges.

**Trigger Conditions:**
- Non-admin account attempting admin-only operations

**Recovery:** Use the admin account or request admin to perform the operation.

---

### 3. CreditLineNotFound (Code: 3)
**Description:** The specified credit line was not found.

**Trigger Conditions:**
- Attempting to operate on a non-existent credit line
- Borrower address has never had a credit line opened

**Recovery:** Open a credit line first using `open_credit_line()`.

---

### 4. CreditLineClosed (Code: 4)
**Description:** Action cannot be performed because the credit line is closed.

**Trigger Conditions:**
- Attempting to draw or repay on a closed credit line
- Attempting to modify a closed credit line

**Recovery:** Credit lines cannot be reopened. Open a new credit line if needed.

---

### 5. InvalidAmount (Code: 5)
**Description:** The requested amount is invalid (e.g., zero or negative where positive is expected).

**Trigger Conditions:**
- Passing zero or negative amounts to draw/repay functions
- Setting invalid configuration values

**Recovery:** Provide a valid positive amount.

---

### 6. OverLimit (Code: 6)
**Description:** The requested draw exceeds the available credit limit.

**Trigger Conditions:**
- Drawing more than `credit_limit - utilized_amount`
- Attempting to draw when already at limit

**Recovery:** Reduce draw amount or repay existing balance first.

---

### 7. NegativeLimit (Code: 7)
**Description:** The credit limit cannot be negative.

**Trigger Conditions:**
- Setting a negative credit limit in `update_risk_parameters()`

**Recovery:** Provide a non-negative credit limit (>= 0).

---

### 8. RateTooHigh (Code: 8)
**Description:** The interest rate change exceeds the maximum allowed delta.

**Trigger Conditions:**
- Setting interest rate > 10,000 bps (100%)
- Rate change exceeds configured `max_rate_change_bps`
- Rate change attempted before `rate_change_min_interval` elapsed

**Recovery:** Use a lower rate or wait for the interval to elapse.

---

### 9. ScoreTooHigh (Code: 9)
**Description:** The risk score is above the acceptable maximum threshold.

**Trigger Conditions:**
- Setting risk score > 100

**Recovery:** Provide a risk score between 0 and 100.

---

### 10. UtilizationNotZero (Code: 10)
**Description:** Action cannot be performed because the credit line utilization is not zero.

**Trigger Conditions:**
- Attempting operations that require zero balance
- Borrower trying to close line with outstanding balance

**Recovery:** Repay all outstanding balance first.

---

### 11. Reentrancy (Code: 11)
**Description:** Reentrancy detected during cross-contract calls.

**Trigger Conditions:**
- Contract is re-entered while a guarded operation is in progress
- Malicious token contract attempting reentrancy

**Recovery:** This is a security protection. Do not attempt to bypass.

---

### 12. Overflow (Code: 12)
**Description:** Math overflow occurred during calculation.

**Trigger Conditions:**
- Arithmetic operation would exceed `i128::MAX`
- Utilization calculation overflow
- Interest accrual overflow

**Recovery:** Use smaller amounts or report to admin for investigation.

---

### 13. LimitDecreaseRequiresRepayment (Code: 13)
**Description:** Credit limit decrease requires immediate repayment of excess amount.

**Trigger Conditions:**
- Attempting to decrease limit below current `utilized_amount`

**Recovery:** Repay excess amount first, or accept `Restricted` status.

---

### 14. AlreadyInitialized (Code: 14)
**Description:** Contract has already been initialized; `init` may only be called once.

**Trigger Conditions:**
- Calling `init()` more than once
- Attempting to open Active credit line that already exists

**Recovery:** Contract is already initialized. Proceed with normal operations.

---

### 15. AdminAcceptTooEarly (Code: 15)
**Description:** Admin acceptance attempted before the delay window has elapsed.

**Trigger Conditions:**
- Calling `accept_admin()` before `accept_after` timestamp

**Recovery:** Wait for the configured delay period to elapse.

---

### 16. BorrowerBlocked (Code: 16)
**Description:** Borrower is blocked from drawing credit.

**Trigger Conditions:**
- Borrower is on the admin-maintained block list
- Attempting to draw while blocked

**Recovery:** Contact admin to be unblocked.

---

### 17. DrawExceedsMaxAmount (Code: 17)
**Description:** The requested draw exceeds the configured per-transaction maximum.

**Trigger Conditions:**
- Draw amount > configured `MaxDrawAmount`

**Recovery:** Reduce draw amount or make multiple smaller draws.

---

### 18. Paused (Code: 18)
**Description:** Protocol is paused by the emergency circuit breaker.

**Trigger Conditions:**
- Attempting operations while protocol is paused
- Emergency pause activated by admin

**Recovery:** Wait for admin to unpause the protocol.

---

### 19. DrawsFrozen (Code: 19)
**Description:** All draws are globally frozen by admin for liquidity reserve operations.

**Trigger Conditions:**
- Attempting to draw while draws are frozen
- Liquidity reserve maintenance in progress

**Recovery:** Wait for admin to unfreeze draws. Repayments still allowed.

---

### 20. CreditLineSuspended (Code: 20)
**Description:** Action cannot be performed because the credit line is suspended.

**Trigger Conditions:**
- Attempting to draw on a suspended credit line
- Attempting to suspend an already suspended line

**Recovery:** Contact admin for reinstatement or wait for automatic reinstatement.

---

### 21. CreditLineDefaulted (Code: 21)
**Description:** Action cannot be performed because the credit line is defaulted.

**Trigger Conditions:**
- Attempting to draw on a defaulted credit line
- Invalid state transition from non-defaulted status

**Recovery:** Contact admin for reinstatement after resolving default.

---

### 22. MissingLiquidityToken (Code: 22)
**Description:** Liquidity token has not been configured.

**Trigger Conditions:**
- Attempting to draw before `set_liquidity_token()` is called
- Token configuration was removed

**Recovery:** Admin must call `set_liquidity_token()` first.

---

### 23. MissingLiquiditySource (Code: 23)
**Description:** Liquidity source has not been configured.

**Trigger Conditions:**
- Attempting to draw before `set_liquidity_source()` is called
- Source configuration was removed

**Recovery:** Admin must call `set_liquidity_source()` first.

---

### 24. InsufficientLiquidityReserve (Code: 24)
**Description:** Liquidity reserve balance is below the requested draw amount.

**Trigger Conditions:**
- Reserve balance < draw amount
- Reserve has been depleted

**Recovery:** Wait for reserve replenishment or reduce draw amount.

---

### 25. LiquidityTokenCallFailed (Code: 25)
**Description:** Liquidity token call failed where the contract can observe it.

**Trigger Conditions:**
- Token transfer failed
- Token contract reverted

**Recovery:** Check token contract state and balances.

---

### 26. InsufficientRepaymentAllowance (Code: 26)
**Description:** Borrower's token allowance is below the effective repayment amount.

**Trigger Conditions:**
- Allowance < repayment amount
- Allowance not set or expired

**Recovery:** Increase token allowance for the contract.

---

### 27. InsufficientRepaymentBalance (Code: 27)
**Description:** Borrower's token balance is below the effective repayment amount.

**Trigger Conditions:**
- Balance < repayment amount
- Insufficient funds

**Recovery:** Acquire more tokens before repaying.

---

### 28. RepayExceedsMaxAmount (Code: 28)
**Description:** The requested repay exceeds the configured per-transaction maximum.

**Trigger Conditions:**
- Repay amount > configured `MaxRepayAmount`

**Recovery:** Reduce repay amount or make multiple smaller repayments.

---

### 29. DrawCooldownActive (Code: 29)
**Description:** Borrower attempted to draw again before the cooldown interval elapsed.

**Trigger Conditions:**
- Time since last draw < configured `DrawMinIntervalSeconds`

**Recovery:** Wait for cooldown period to elapse.

---

### 30. TreasuryNotSet (Code: 30)
**Description:** Treasury address is not configured when attempting a treasury withdrawal.

**Trigger Conditions:**
- Calling `withdraw_treasury()` before `set_treasury()` is called

**Recovery:** Admin must call `set_treasury()` first.

---

### 31. ExposureCapExceeded (Code: 31)
**Description:** Draw would exceed the global protocol exposure cap.

**Trigger Conditions:**
- `total_utilized + draw_amount > max_total_exposure`
- Protocol-wide utilization limit reached

**Recovery:** Wait for other borrowers to repay, or admin can increase cap.

---

### 32. AdminNotInitialized (Code: 32)
**Description:** Admin address has not been initialized in contract storage.

**Trigger Conditions:**
- Calling admin-only functions before `init()` is called
- Contract deployment incomplete

**Recovery:** Call `init()` with admin address first.

---

### 33. TimestampRegression (Code: 33)
**Description:** Timestamp regression detected (new timestamp is not greater than stored timestamp).

**Trigger Conditions:**
- Ledger timestamp moved backwards (should not occur in normal operation)
- Defensive check triggered

**Recovery:** This indicates a serious ledger issue. Contact support.

---

### 34. LimitOutOfBounds (Code: 34)
**Description:** Credit limit is outside the configured minimum/maximum bounds.

**Trigger Conditions:**
- Opening credit line with `credit_limit < min_credit_limit`
- Opening credit line with `credit_limit > max_credit_limit`
- Updating risk parameters to set limit outside bounds
- Setting `max_credit_limit < min_credit_limit` in bounds configuration

**Recovery:** 
- For credit line operations: Use a limit within the configured bounds
- For bounds configuration: Ensure `min >= 0` and `max >= min`
- Query current bounds using `get_credit_limit_bounds()` to see valid range

**Related Functions:**
- `set_credit_limit_bounds(min, max)` - Configure global bounds (admin only)
- `get_credit_limit_bounds()` - Query current bounds
- `open_credit_line()` - Validates limit against bounds
- `update_risk_parameters()` - Validates new limit against bounds

**Example:**
```rust
// Admin sets bounds
client.set_credit_limit_bounds(&10_000, &1_000_000);

// Valid: within bounds
client.open_credit_line(&borrower, &500_000, &500, &50); // ✅

// Invalid: below minimum
client.open_credit_line(&borrower, &5_000, &500, &50); // ❌ Error 34

// Invalid: above maximum
client.open_credit_line(&borrower, &2_000_000, &500, &50); // ❌ Error 34
```

**Security Rationale:**
This error protects the protocol from extreme concentration risk by enforcing admin-configurable minimum and maximum credit limits. It prevents:
- Malicious or erroneous admin actions that could create excessively large credit lines
- Credit lines too small to be economically viable
- Concentration of protocol risk in a small number of large borrowers

---

## Error Handling Best Practices

### For SDK Clients

```rust
use creditra_credit::types::ContractError;

match client.try_draw_credit(&borrower, &amount) {
    Ok(_) => println!("Draw successful"),
    Err(Error::Contract(code)) => {
        match code {
            3 => println!("Credit line not found - open one first"),
            6 => println!("Draw exceeds limit - reduce amount"),
            24 => println!("Insufficient reserve - try again later"),
            34 => println!("Limit outside configured bounds"),
            _ => println!("Error code: {}", code),
        }
    }
    Err(e) => println!("Other error: {:?}", e),
}
```

### For Contract Developers

- Always use `env.panic_with_error(ContractError::SpecificError)` instead of `panic!()` or `unwrap()`
- Never reorder or renumber existing error discriminants
- Add new errors at the end of the enum with the next available number
- Update this documentation when adding new errors
- Write integration tests that verify the correct error discriminant is returned

---

## Stability Guarantee

Error discriminants are **permanent** and form part of the contract's public API. Once assigned, a discriminant value must never be changed or reused. This ensures:

- SDK clients can reliably match on error codes
- Error handling logic remains stable across contract upgrades
- Integrators can build robust error recovery mechanisms

---

**Last Updated:** 2026-05-29  
**Contract Version:** 1.0.0  
**Total Error Variants:** 34
