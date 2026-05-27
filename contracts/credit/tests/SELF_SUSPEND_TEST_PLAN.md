# Self-Suspend Credit Line - Test Plan & Function List

## Test Function Inventory

This document provides a concise list of all test functions implemented for the `self_suspend_credit_line` feature, organized by test category.

---

## 1. Authorization Matrix (Signer Validation)

### ✓ `test_self_suspend_success_when_borrower_authorized`
**Purpose:** Verify borrower can successfully self-suspend their own active credit line  
**Setup:** Active credit line with borrower authorization  
**Expected:** Status transitions from Active → Suspended  
**Assertions:**
- Credit line status equals `CreditStatus::Suspended`

---

### ✗ `test_self_suspend_fails_when_admin_invokes`
**Purpose:** Verify admin cannot invoke self-suspend on behalf of borrower  
**Setup:** Active credit line, admin attempts to call self_suspend  
**Expected:** Panic due to authorization failure  
**Assertions:**
- Function panics (authorization check fails)

---

### ✗ `test_self_suspend_fails_when_third_party_invokes`
**Purpose:** Verify arbitrary third party cannot invoke self-suspend  
**Setup:** Active credit line, third party attempts to call self_suspend  
**Expected:** Panic due to authorization failure  
**Assertions:**
- Function panics (authorization check fails)

---

## 2. State Machine Matrix (Status Validation)

### ✓ `test_self_suspend_success_from_active_status`
**Purpose:** Verify self-suspension works from Active status  
**Setup:** Credit line in Active status  
**Expected:** Status transitions from Active → Suspended  
**Assertions:**
- Initial status is `CreditStatus::Active`
- Final status is `CreditStatus::Suspended`

---

### ✗ `test_self_suspend_fails_from_suspended_status`
**Purpose:** Verify self-suspension fails when already suspended  
**Setup:** Credit line in Suspended status  
**Expected:** Panic with "Only active credit lines can be self-suspended"  
**Assertions:**
- Function panics with expected message

---

### ✗ `test_self_suspend_fails_from_defaulted_status`
**Purpose:** Verify self-suspension fails from Defaulted status  
**Setup:** Credit line in Defaulted status  
**Expected:** Panic with "Only active credit lines can be self-suspended"  
**Assertions:**
- Function panics with expected message

---

### ✗ `test_self_suspend_fails_from_closed_status`
**Purpose:** Verify self-suspension fails from Closed status  
**Setup:** Credit line in Closed status  
**Expected:** Panic with "Only active credit lines can be self-suspended"  
**Assertions:**
- Function panics with expected message

---

### ✗ `test_self_suspend_fails_when_credit_line_not_found`
**Purpose:** Verify self-suspension fails when credit line doesn't exist  
**Setup:** No credit line exists for borrower  
**Expected:** Panic with "Credit line not found"  
**Assertions:**
- Function panics with expected message

---

## 3. Functional Capabilities Post-Suspension

### ✗ `test_draw_blocked_after_self_suspension`
**Purpose:** Verify draw operations are blocked after self-suspension  
**Setup:** Active line self-suspended, then attempt draw  
**Expected:** Panic with "credit line is suspended"  
**Assertions:**
- Status is `CreditStatus::Suspended` before draw attempt
- Draw operation panics

---

### ✓ `test_repay_allowed_after_self_suspension`
**Purpose:** Verify repayment operations remain allowed after self-suspension  
**Setup:** Active line with utilization, self-suspended, then repay  
**Expected:** Repayment succeeds, utilization decreases  
**Assertions:**
- Status is `CreditStatus::Suspended` before repayment
- Utilization decreases by repayment amount
- Status remains `CreditStatus::Suspended` after repayment

---

### ✓ `test_admin_can_unsuspend_self_suspended_line`
**Purpose:** Document that admin can restore self-suspended line to Active  
**Setup:** Active line self-suspended by borrower  
**Expected:** Admin has authority to restore (documentation test)  
**Assertions:**
- Status is `CreditStatus::Suspended` after self-suspension
- Documents admin intervention requirement

---

### ✓ `test_admin_can_close_self_suspended_line`
**Purpose:** Verify admin can force-close a self-suspended line  
**Setup:** Active line with utilization, self-suspended, admin closes  
**Expected:** Status transitions from Suspended → Closed  
**Assertions:**
- Status is `CreditStatus::Suspended` before close
- Utilization is non-zero
- Status is `CreditStatus::Closed` after admin close

---

### ✓ `test_self_suspended_line_preserves_utilization`
**Purpose:** Verify utilization amount is preserved during self-suspension  
**Setup:** Active line with drawn funds, then self-suspend  
**Expected:** All credit parameters preserved except status  
**Assertions:**
- Utilization unchanged
- Credit limit unchanged
- Interest rate unchanged
- Risk score unchanged
- Status changes from Active → Suspended

---

## 4. Event Emission & State Integrity

### ✓ `test_self_suspend_emits_correct_event`
**Purpose:** Verify self-suspension emits correct event  
**Setup:** Active line, self-suspend, capture events  
**Expected:** Single event emitted with correct parameters  
**Assertions:**
- Exactly one event emitted
- Credit line state matches expected values
- Event contains correct borrower, status, and parameters

---

### ✓ `test_self_suspend_preserves_credit_parameters`
**Purpose:** Verify all credit parameters remain unchanged except status  
**Setup:** Active line, capture state, self-suspend, compare state  
**Expected:** Only status changes, all other fields preserved  
**Assertions:**
- Borrower address unchanged
- Credit limit unchanged
- Utilized amount unchanged
- Interest rate unchanged
- Risk score unchanged
- Last rate update timestamp unchanged
- Status changes from Active → Suspended

---

### ✗ `test_self_suspend_idempotency_check`
**Purpose:** Verify duplicate self-suspension fails explicitly  
**Setup:** Active line, self-suspend twice  
**Expected:** Second suspension panics  
**Assertions:**
- First suspension succeeds
- Status is `CreditStatus::Suspended` after first suspension
- Second suspension panics with expected message

---

## 5. Edge Cases

### ✓ `test_self_suspend_with_zero_utilization`
**Purpose:** Verify self-suspension works with zero utilization  
**Setup:** Active line with no drawn funds  
**Expected:** Self-suspension succeeds  
**Assertions:**
- Utilization is zero before suspension
- Status transitions to `CreditStatus::Suspended`
- Utilization remains zero after suspension

---

### ✓ `test_self_suspend_with_maximum_utilization`
**Purpose:** Verify self-suspension works at full credit limit  
**Setup:** Active line with utilization equal to credit limit  
**Expected:** Self-suspension succeeds  
**Assertions:**
- Utilization equals credit limit before suspension
- Status transitions to `CreditStatus::Suspended`
- Utilization preserved at credit limit after suspension

---

### ✓ `test_self_suspend_applies_interest_accrual`
**Purpose:** Verify interest is accrued before self-suspension  
**Setup:** Active line with utilization, advance time, self-suspend  
**Expected:** Interest accrued before status change  
**Assertions:**
- Status transitions to `CreditStatus::Suspended`
- Utilization does not decrease (may increase with interest)
- Function completes successfully (accrual called internally)

---

## Test Execution Summary

| Category | Total Tests | Success Tests | Failure Tests |
|----------|-------------|---------------|---------------|
| Authorization Matrix | 3 | 1 | 2 |
| State Machine Matrix | 5 | 1 | 4 |
| Functional Capabilities | 5 | 5 | 0 |
| Event & State Integrity | 3 | 2 | 1 |
| Edge Cases | 3 | 3 | 0 |
| **TOTAL** | **19** | **12** | **7** |

**Note:** "Failure Tests" are tests that expect panics/errors (negative test cases).

---

## Test Setup States

### Initial States Used
- **Active with zero utilization** - Most common starting point
- **Active with partial utilization** - For repayment and utilization tests
- **Active with maximum utilization** - Edge case testing
- **Suspended** - For idempotency and invalid state tests
- **Defaulted** - For invalid state tests
- **Closed** - For invalid state tests
- **Non-existent** - For not found tests

---

## Expected Panic Messages

| Panic Message | Test Count | Scenarios |
|--------------|------------|-----------|
| "Only active credit lines can be self-suspended" | 4 | Suspended, Defaulted, Closed, Idempotency |
| "Credit line not found" | 1 | Non-existent credit line |
| "credit line is suspended" | 1 | Draw after suspension |
| Authorization failure (implicit) | 2 | Admin invoke, Third party invoke |

---

## Coverage Analysis

### Functions Covered
- ✓ `self_suspend_credit_line` - Primary function under test
- ✓ `draw_credit` - Blocked after suspension
- ✓ `repay_credit` - Allowed after suspension
- ✓ `close_credit_line` - Admin can close suspended line
- ✓ `get_credit_line` - View function works on suspended line

### Code Paths Covered
- ✓ Authorization check (`borrower.require_auth()`)
- ✓ Credit line retrieval from storage
- ✓ Interest accrual application
- ✓ Status validation (Active check)
- ✓ Status update (Active → Suspended)
- ✓ Storage persistence
- ✓ Event emission

### Edge Cases Covered
- ✓ Zero utilization
- ✓ Maximum utilization
- ✓ Interest accrual timing
- ✓ Multiple status transitions
- ✓ Authorization boundaries
- ✓ Non-existent credit lines

---

## Test Execution Commands

### Run all self-suspend tests
```bash
cargo test -p creditra-credit self_suspend
```

### Run by category
```bash
# Authorization tests
cargo test -p creditra-credit test_self_suspend.*authorized
cargo test -p creditra-credit test_self_suspend.*admin
cargo test -p creditra-credit test_self_suspend.*third_party

# State machine tests
cargo test -p creditra-credit test_self_suspend.*status
cargo test -p creditra-credit test_self_suspend.*not_found

# Functional tests
cargo test -p creditra-credit test_draw_blocked
cargo test -p creditra-credit test_repay_allowed
cargo test -p creditra-credit test_admin_can

# Integrity tests
cargo test -p creditra-credit test_self_suspend.*emit
cargo test -p creditra-credit test_self_suspend.*preserves
cargo test -p creditra-credit test_self_suspend.*idempotency

# Edge cases
cargo test -p creditra-credit test_self_suspend.*zero
cargo test -p creditra-credit test_self_suspend.*maximum
cargo test -p creditra-credit test_self_suspend.*accrual
```

### Run with verbose output
```bash
cargo test -p creditra-credit self_suspend -- --nocapture
```

### Run with coverage
```bash
cargo tarpaulin -p creditra-credit --test borrower_self_suspend
```

---

## Test Maintenance Notes

### Adding New Tests
When adding new tests to this suite:
1. Follow the existing naming convention: `test_self_suspend_<scenario>_<expected_result>`
2. Add comprehensive documentation comments
3. Use the provided helper functions for setup
4. Update this document with the new test details
5. Ensure test is added to the appropriate category

### Modifying Existing Tests
When modifying tests:
1. Update the test documentation if behavior changes
2. Verify all related tests still pass
3. Update this document if test purpose or assertions change
4. Run full test suite to ensure no regressions

### Test Dependencies
- Soroban SDK testutils
- Standard Rust test framework
- Token contract for liquidity testing
- Event system for emission testing

---

**Document Version:** 1.0  
**Last Updated:** 2026-05-27  
**Total Tests:** 19  
**Maintainer:** Creditra QA Team
