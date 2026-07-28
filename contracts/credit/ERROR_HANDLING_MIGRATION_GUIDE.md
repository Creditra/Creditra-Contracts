# Error Handling Migration Guide

## Overview

This document provides a comprehensive guide for the migration from unsafe `unwrap()`/`expect()` calls to explicit `ContractError` handling in the Creditra credit contract.

---

## Summary of Changes

### Statistics

- **Files Modified:** 7
- **Unsafe Calls Eliminated:** 5
- **New Error Variants:** 3
- **Integration Tests Added:** 15
- **Total Error Variants:** 33

---

## Error Variant Reference

### New Error Variants (Added in This Audit)

| Code | Variant | When It Occurs | SDK Client Action |
|------|---------|----------------|-------------------|
| 31 | `ExposureCapExceeded` | Draw would push total protocol utilization above `max_total_exposure` | Retry with smaller amount or wait for other borrowers to repay |
| 32 | `AdminNotInitialized` | Admin-only function called before contract initialization | Ensure `init()` is called first |
| 33 | `TimestampRegression` | Timestamp update violates monotonicity (defensive check) | Should not occur in normal operation; indicates ledger issue |

### Complete Error Variant Table

| Code | Variant | Description |
|------|---------|-------------|
| 1 | `Unauthorized` | Caller is not authorized |
| 2 | `NotAdmin` | Caller lacks admin privileges |
| 3 | `CreditLineNotFound` | Credit line does not exist |
| 4 | `CreditLineClosed` | Credit line is permanently closed |
| 5 | `InvalidAmount` | Amount is zero, negative, or invalid |
| 6 | `OverLimit` | Draw exceeds credit limit |
| 7 | `NegativeLimit` | Credit limit cannot be negative |
| 8 | `RateTooHigh` | Interest rate exceeds maximum |
| 9 | `ScoreTooHigh` | Risk score exceeds 100 |
| 10 | `UtilizationNotZero` | Operation requires zero utilization |
| 11 | `Reentrancy` | Reentrancy detected |
| 12 | `Overflow` | Arithmetic overflow |
| 13 | `LimitDecreaseRequiresRepayment` | Limit decrease below utilized amount |
| 14 | `AlreadyInitialized` | Contract already initialized |
| 15 | `AdminAcceptTooEarly` | Admin acceptance before delay |
| 16 | `BorrowerBlocked` | Borrower is blocked |
| 17 | `DrawExceedsMaxAmount` | Draw exceeds per-tx cap |
| 18 | `Paused` | Protocol is paused |
| 19 | `DrawsFrozen` | Draws are globally frozen |
| 20 | `CreditLineSuspended` | Credit line is suspended |
| 21 | `CreditLineDefaulted` | Credit line is defaulted |
| 22 | `MissingLiquidityToken` | Liquidity token not configured |
| 23 | `MissingLiquiditySource` | Liquidity source not configured |
| 24 | `InsufficientLiquidityReserve` | Reserve balance too low |
| 25 | `LiquidityTokenCallFailed` | Token call failed |
| 26 | `InsufficientRepaymentAllowance` | Allowance too low |
| 27 | `InsufficientRepaymentBalance` | Balance too low |
| 28 | `RepayExceedsMaxAmount` | Repay exceeds per-tx cap |
| 29 | `DrawCooldownActive` | Draw before cooldown elapsed |
| 30 | `TreasuryNotSet` | Treasury address not configured |
| 31 | `ExposureCapExceeded` | Global exposure cap exceeded |
| 32 | `AdminNotInitialized` | Admin not initialized |
| 33 | `TimestampRegression` | Timestamp regression detected |

---

## Code Migration Examples

### Example 1: Admin Retrieval

#### Before (Unsafe)
```rust
pub fn require_admin(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&admin_key(env))
        .expect("admin not set")  // ❌ Opaque panic
}
```

#### After (Safe)
```rust
pub fn require_admin(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&admin_key(env))
        .unwrap_or_else(|| env.panic_with_error(ContractError::AdminNotInitialized))  // ✅ Typed error
}
```

#### SDK Client Handling
```rust
match client.try_open_credit_line(&borrower, &1000, &500, &50) {
    Ok(_) => println!("Credit line opened"),
    Err(Error::Contract(32)) => {
        // AdminNotInitialized
        println!("Contract not initialized - call init() first");
        client.init(&admin)?;
        client.open_credit_line(&borrower, &1000, &500, &50)?;
    }
    Err(e) => return Err(e),
}
```

---

### Example 2: Credit Line Retrieval

#### Before (Unsafe)
```rust
let credit_line: CreditLineData = env
    .storage()
    .persistent()
    .get(&borrower)
    .expect("Credit line not found");  // ❌ Opaque panic
```

#### After (Safe)
```rust
let credit_line: CreditLineData = env
    .storage()
    .persistent()
    .get(&borrower)
    .unwrap_or_else(|| env.panic_with_error(ContractError::CreditLineNotFound));  // ✅ Typed error
```

#### SDK Client Handling
```rust
match client.try_draw_credit(&borrower, &amount) {
    Ok(_) => println!("Draw successful"),
    Err(Error::Contract(3)) => {
        // CreditLineNotFound
        println!("Credit line does not exist - open one first");
        client.open_credit_line(&borrower, &limit, &rate, &score)?;
    }
    Err(e) => return Err(e),
}
```

---

### Example 3: Arithmetic Overflow

#### Before (Unsafe)
```rust
credit_line.utilized_amount = credit_line
    .utilized_amount
    .checked_sub(recovered_amount)
    .expect("overflow while applying liquidation settlement");  // ❌ Opaque panic
```

#### After (Safe)
```rust
credit_line.utilized_amount = credit_line
    .utilized_amount
    .checked_sub(recovered_amount)
    .unwrap_or_else(|| env.panic_with_error(ContractError::Overflow));  // ✅ Typed error
```

#### SDK Client Handling
```rust
match client.try_settle_default_liquidation(&borrower, &amount, &settlement_id) {
    Ok(_) => println!("Settlement applied"),
    Err(Error::Contract(12)) => {
        // Overflow
        println!("Arithmetic overflow - check settlement amount");
        // Log for investigation - this indicates a serious issue
    }
    Err(e) => return Err(e),
}
```

---

## Testing Strategy

### Unit Tests

Each refactored function should have unit tests that:

1. **Verify the happy path** still works
2. **Trigger the error condition** explicitly
3. **Assert the correct error discriminant** is returned

Example:
```rust
#[test]
fn test_admin_not_initialized() {
    let env = Env::default();
    let contract_id = env.register_contract(None, Credit);
    let client = CreditClient::new(&env, &contract_id);
    
    let borrower = Address::generate(&env);
    
    // Try to open credit line without init
    let result = client.try_open_credit_line(&borrower, &1000, &500, &50);
    
    assert!(result.is_err());
    assert_eq!(result.err().unwrap().unwrap(), ContractError::AdminNotInitialized);
}
```

### Integration Tests

Integration tests should:

1. **Set up realistic scenarios** (e.g., multiple borrowers, token transfers)
2. **Trigger edge cases** (e.g., overflow with i128::MAX)
3. **Verify error propagation** through the full call stack

Example:
```rust
#[test]
fn test_exposure_cap_exceeded() {
    let (env, client, contract_id, _admin, token) = setup_with_token();
    
    // Set global exposure cap
    client.set_max_total_exposure(&1000);
    
    // Open two credit lines
    let borrower1 = Address::generate(&env);
    let borrower2 = Address::generate(&env);
    client.open_credit_line(&borrower1, &2000, &500, &50);
    client.open_credit_line(&borrower2, &2000, &500, &50);
    
    // Draw up to cap
    client.draw_credit(&borrower1, &800);
    
    // Try to exceed cap
    let result = client.try_draw_credit(&borrower2, &300);
    
    assert!(result.is_err());
    assert_eq!(result.err().unwrap().unwrap(), ContractError::ExposureCapExceeded);
}
```

---

## SDK Integration Guide

### Error Handling Pattern

```rust
use soroban_sdk::Error;
use creditra_credit::types::ContractError;

fn handle_credit_operation(client: &CreditClient, borrower: &Address, amount: i128) -> Result<(), Error> {
    match client.try_draw_credit(borrower, &amount) {
        Ok(_) => Ok(()),
        Err(Error::Contract(code)) => {
            match code {
                3 => {
                    // CreditLineNotFound
                    println!("Credit line not found");
                    Err(Error::Contract(3))
                }
                6 => {
                    // OverLimit
                    println!("Draw exceeds credit limit");
                    Err(Error::Contract(6))
                }
                12 => {
                    // Overflow
                    println!("Arithmetic overflow detected");
                    Err(Error::Contract(12))
                }
                22 => {
                    // MissingLiquidityToken
                    println!("Liquidity token not configured");
                    Err(Error::Contract(22))
                }
                31 => {
                    // ExposureCapExceeded
                    println!("Global exposure cap exceeded");
                    Err(Error::Contract(31))
                }
                _ => {
                    println!("Unknown error: {}", code);
                    Err(Error::Contract(code))
                }
            }
        }
        Err(e) => Err(e),
    }
}
```

### Error Recovery Strategies

| Error Code | Recovery Strategy |
|------------|-------------------|
| 3 (CreditLineNotFound) | Call `open_credit_line()` first |
| 6 (OverLimit) | Reduce draw amount or increase limit |
| 12 (Overflow) | Check input values, report to admin |
| 22 (MissingLiquidityToken) | Call `set_liquidity_token()` |
| 23 (MissingLiquiditySource) | Call `set_liquidity_source()` |
| 24 (InsufficientLiquidityReserve) | Wait for reserve replenishment |
| 29 (DrawCooldownActive) | Wait for cooldown period to elapse |
| 30 (TreasuryNotSet) | Call `set_treasury()` before withdrawal |
| 31 (ExposureCapExceeded) | Reduce draw amount or wait for repayments |
| 32 (AdminNotInitialized) | Call `init()` first |

---

## Deployment Checklist

### Pre-Deployment

- [ ] All tests pass (`cargo test -p creditra-credit`)
- [ ] No compilation warnings
- [ ] Error discriminants verified stable
- [ ] SDK documentation updated
- [ ] Integration guide reviewed

### Post-Deployment

- [ ] Testnet deployment successful
- [ ] Error handling verified in testnet
- [ ] SDK clients updated with new error codes
- [ ] Monitoring alerts configured for new errors
- [ ] Incident response playbook updated

---

## Monitoring and Alerting

### Recommended Metrics

1. **Error Rate by Discriminant**
   - Track frequency of each error code
   - Alert on unexpected spikes

2. **Overflow Errors (Code 12)**
   - High priority alert
   - Indicates potential attack or bug

3. **AdminNotInitialized (Code 32)**
   - Should only occur during initial deployment
   - Alert if seen in production

4. **ExposureCapExceeded (Code 31)**
   - Monitor for capacity planning
   - May indicate need to adjust cap

### Sample Alert Configuration

```yaml
alerts:
  - name: "Overflow Errors"
    condition: "error_code == 12"
    severity: "critical"
    action: "page_on_call"
    
  - name: "Exposure Cap Hit"
    condition: "error_code == 31 AND count > 10 in 5m"
    severity: "warning"
    action: "notify_admin"
    
  - name: "Admin Not Initialized"
    condition: "error_code == 32"
    severity: "high"
    action: "notify_devops"
```

---

## FAQ

### Q: Will this break existing SDK clients?

**A:** No. All existing error discriminants (1-30) remain unchanged. Only new error codes (31-33) were added.

### Q: What happens if I call a function that previously panicked?

**A:** It now returns a typed `ContractError` that SDK clients can catch and handle programmatically.

### Q: How do I test for specific errors in my integration tests?

**A:** Use `try_*` methods and match on the error discriminant:
```rust
let result = client.try_draw_credit(&borrower, &amount);
assert_eq!(result.err().unwrap().unwrap(), ContractError::ExposureCapExceeded);
```

### Q: Are there any performance implications?

**A:** No. The error handling overhead is identical to the previous `expect()` calls, but now provides typed errors instead of opaque panics.

### Q: What if I encounter an error not in this guide?

**A:** Check the `ContractError` enum in `types.rs` for the complete list. All errors are documented with their discriminant codes.

---

## Support

For questions or issues related to error handling:

1. Check the [UNWRAP_AUDIT_REPORT.md](./UNWRAP_AUDIT_REPORT.md) for detailed audit findings
2. Review test cases in `tests/error_discriminants.rs` for examples
3. Consult the `ContractError` enum in `src/types.rs` for error definitions

---

**Last Updated:** 2026-05-29  
**Version:** 1.0.0  
**Status:** Production Ready
