# PR: Freeze Borrow Module Error Codes (Issue #807, GrantFox FWC26 Campaign)

## Summary

Added `contracts/borrow/tests/err_stab.rs` — a comprehensive error stability test suite that **freezes the numeric discriminants** of every `ContractError` variant produced by the borrow module (`draw_credit`, `repay_credit`, `repay_and_release_collateral`).

This is a **test-only change** that guards against silent client-facing breaking changes: if a future maintainer accidentally reorders or inserts error variants without explicitly pinning discriminants, this test will catch it at compile/test time rather than shipping a silent code renumbering to every off-chain integrator.

## Files Changed

- **Added:** `contracts/borrow/tests/err_stab.rs` — 3 test functions asserting error discriminants
- **Added:** `contracts/borrow/Cargo.toml` — minimal manifest for test discovery

## Frozen Error Codes (Borrow Module)

The borrow module currently produces or propagates these 11 error variants:

| Code | Variant                        | Context                                    |
|------|--------------------------------|--------------------------------------------|
| 3    | `CreditLineNotFound`           | Borrower has no credit line                |
| 4    | `CreditLineClosed`             | Cannot draw/repay on closed line           |
| 5    | `InvalidAmount`                | Amount ≤ 0 or malformed input              |
| 6    | `OverLimit`                    | Draw exceeds available credit limit        |
| 10   | `UtilizationNotZero`           | Edge case in repay/release logic           |
| 12   | `Overflow`                     | Arithmetic overflow in mul_div or apply_bps|
| 20   | `CreditLineSuspended`          | Draws blocked; repays allowed              |
| 21   | `CreditLineDefaulted`          | Draws blocked; repays allowed for cure     |
| 28   | `RepayExceedsMaxAmount`        | Repay exceeds per-transaction cap          |
| 29   | `DrawCooldownActive`           | Borrower draw cooldown still active        |
| 39   | `InsufficientCollateralBalance`| Collateral withdrawal exceeds balance      |

## Discriminant Safety Assessment

**All discriminants are EXPLICIT, not implicit.**

The `ContractError` enum in `contracts/credit/src/types.rs` uses:
- `#[repr(u32)]` — stable ABI representation
- **Explicit assignments:** `Unauthorized = 1`, `NotAdmin = 2`, ..., `LiquidationGraceActive = 55`

**Implication:** Discriminants are **already safe** against accidental implicit renumbering because they are explicitly pinned. A maintainer would have to *intentionally* change a value to break the ABI. However, this test provides an additional defense-in-depth safeguard by catching any such change at compile/test time.

**Recommendation (out of scope for this PR):** If future maintainers are concerned about the friction of maintaining 55 explicit discriminant assignments, a code generation approach (macro or build script) could auto-derive them — but this would be a refactor task separate from the test-only scope of this issue.

## Test Scope

### Test 1: `borrow_error_discriminants_are_stable()`
Asserts 11 hardcoded discriminant values (3, 4, 5, 6, 10, 12, 20, 21, 28, 29, 39).
- **Purpose:** Detect any change to the enum that shifts these codes.
- **Failure mode:** If a variant is reordered or its explicit assignment changes, this test fails loudly.

### Test 2: `borrow_errors_have_no_duplicate_discriminants()`
Collects all 11 error codes into a vector and verifies no collisions using a HashSet.
- **Purpose:** Catch even more severe bugs (two variants mapping to the same code).
- **Failure mode:** If somehow two variants ended up with the same code, this detects it.

### Test 3: `borrow_errors_known_variant_count()`
Verifies the borrow module error set has exactly 11 distinct codes.
- **Purpose:** Sanity check; if a new error is added or removed, maintainers must update this constant and the test lists.
- **Failure mode:** If the count changes, this test fails and guides the fix.

## Entrypoint Authorization & Overflow Safety

**require_auth on all state-changing borrow entrypoints:**
- ✅ `draw_credit` (line 25): `borrower.require_auth()`
- ✅ `repay_credit` (line 173): `borrower.require_auth()`
- ✅ `repay_and_release_collateral` (line 268): `borrower.require_auth()`

**Overflow-safe math:** The module uses `checked_add()`, `saturating_sub()`, and `mul_div()` in financial paths. ✓

**⚠️ Discovered concern (NOT fixed in this PR, flagged for follow-up):**

Several code paths use bare `panic!()` instead of `env.panic_with_error(ContractError::...)`:

- Line 61–65: `.unwrap_or_else(|| panic!("overflow"))`
- Line 69: `.unwrap_or_else(|| panic!("exceeds credit limit"))`
- Line 74: bare `panic!("Insufficient liquidity reserve...")`
- Line 196–200: bare `panic!("Insufficient allowance")` / `panic!("Insufficient balance")`
- Line 305–309: bare `panic!("Insufficient allowance")` / `panic!("Insufficient balance")`

**Why not fixed here:** This PR is test-only (issue #807 scope). Replacing bare panics with error codes would be a **separate, larger refactor** (likely 10–15 lines changed across 6 call sites) and should be tracked as a follow-up issue for consistency with other contract modules.

These bare panics currently result in **opaque string panics** rather than structured error codes, which degrades the client experience. Recommend a follow-up issue: *"Replace bare panic!() calls in borrow.rs with env.panic_with_error(ContractError::...)"* — this would make error handling consistent across the contract.

## How to Update This Test

If a new error variant is added to or removed from the borrow module:

1. Update the table above with the new code and context.
2. Add or remove the corresponding assertion in `borrow_error_discriminants_are_stable()`.
3. Add or remove the variant from the vectors in `borrow_errors_have_no_duplicate_discriminants()` and `borrow_errors_known_variant_count()`.
4. Update `BORROW_ERROR_COUNT` constant if the count changed.
5. Document the change in the PR description as a **breaking change for integrators**.

If a variant's discriminant value must change (rare, and only with a major version bump):

1. Update the hardcoded expected value in the test.
2. Clearly document in the PR: *"Breaking change: Error code `X` now maps to `Y` instead of `Z`."*
3. Notify off-chain integrators (SDKs, indexers, dashboards) of the ABI change.

## References

- **Issue:** GrantFox FWC26 campaign #807 — "Freeze the client-facing error code numbers for the borrow contract"
- **Related:** `contracts/credit/tests/error_discriminants.rs` — similar test for the main credit contract (54 variants)
- **Related:** `gateway-contract/contracts/auction_contract/tests/err_stab.rs` — similar pattern for the auction contract
- **Module doc:** `contracts/credit/src/borrow.rs`
- **Error enum:** `contracts/credit/src/types.rs` (`ContractError`, line 172)

## Verification

All assertions are manually verified against `contracts/credit/src/types.rs` lines 173–327:
- Each variant is explicitly assigned (e.g., `Unauthorized = 1`, `NotAdmin = 2`, ...)
- No two variants share the same discriminant.
- The 11 borrow-module errors are a proper subset of the 55 total contract errors.


