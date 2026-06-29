# feat(credit): reinstate_credit_line — Defaulted → Active / Suspended

## Summary

Implements `reinstate_credit_line` as a public contract entry point, allowing an admin to transition a credit line out of the `Defaulted` state back to either `Active` or `Suspended`, per the documented state machine. Also resolves a set of pre-existing compilation and test errors that were blocking the build.

---

## What Changed

### Core Feature — `reinstate_credit_line`

**`contracts/credit/src/lifecycle.rs`**

Updated the existing `reinstate_credit_line` function to accept a `target_status: CreditStatus` parameter instead of hardcoding `Active`. The function now:

- Validates the credit line exists (panics with `"Credit line not found"` otherwise)
- Validates the current status is `Defaulted` (panics with `"credit line is not defaulted"` otherwise)
- Validates `target_status` is either `Active` or `Suspended` (panics with `"target_status must be Active or Suspended"` for any other value)
- Persists the new status
- Emits a `("credit", "reinstate")` `CreditLineEvent` with the new status

**`contracts/credit/src/lib.rs`**

Exposed `reinstate_credit_line` as a public `#[contractimpl]` function on the `Credit` struct, with full doc comments covering parameters, panics, events, and post-reinstatement invariants.

### State Machine

```
Active ──────────────────────────────────────────► Closed
  │                                                   ▲
  ▼                                                   │
Suspended ──────────────────────────────────────────► │
  │                                                   │
  ▼                                                   │
Defaulted ──── reinstate_credit_line ──► Active ─────►│
                                     └─► Suspended ──►│
```

### Invariants After Reinstatement

- `utilized_amount` is preserved unchanged (outstanding debt is not forgiven)
- `credit_limit`, `interest_rate_bps`, and `risk_score` are unchanged
- Draws are re-enabled when target is `Active`; remain disabled when target is `Suspended`
- Only admin can call this function

---

## Tests Added (`contracts/credit/src/test.rs`)

12 new explicit transition tests:

| Test | What it covers |
|---|---|
| `test_reinstate_to_active_enables_draws` | Defaulted → Active; draw succeeds after |
| `test_reinstate_to_suspended_status` | Defaulted → Suspended; status is Suspended |
| `test_reinstate_to_suspended_blocks_draws` | Defaulted → Suspended; draw still panics |
| `test_reinstate_preserves_utilized_amount` | All fields unchanged after → Active |
| `test_reinstate_to_suspended_preserves_utilized_amount` | All fields unchanged after → Suspended |
| `test_reinstate_invalid_target_status_closed_reverts` | Closed as target panics |
| `test_reinstate_invalid_target_status_defaulted_reverts` | Defaulted as target panics |
| `test_reinstate_to_active_emits_event_with_active_status` | Event has correct status + borrower |
| `test_reinstate_to_suspended_emits_event_with_suspended_status` | Event has correct status + borrower |
| `test_reinstate_to_active_then_suspend_again` | Full round-trip: Defaulted → Active → Suspended |
| `test_reinstate_to_suspended_then_admin_close` | Defaulted → Suspended → Closed |
| `test_reinstate_to_suspended_unauthorized` | Non-admin call panics |

All existing reinstate call sites updated to pass `&CreditStatus::Active` as the target.

---

## Pre-existing Errors Fixed

The codebase had 97 compilation errors and several failing tests before this PR. The following were resolved as part of this work:

### Compilation Errors (lib.rs)

| Error | Root Cause | Fix |
|---|---|---|
| `ContractError` undeclared | `use types::{}` was missing `ContractError` | Added to import |
| `config::set_liquidity_token` / `set_liquidity_source` | `mod config` was never declared in lib.rs | Inlined the two function bodies directly |
| `query::get_credit_line` | `mod query` was never declared in lib.rs | Inlined `env.storage().persistent().get(&borrower)` |
| `risk::set_rate_change_limits` / `get_rate_change_limits` | `mod risk` was never declared in lib.rs | Inlined both function bodies |
| `CreditLineData` missing fields | `accrued_interest` and `last_accrual_ts` added to `types.rs` but not to the struct literal in `open_credit_line` | Added both fields initialised to `0` |
| Missing SPDX header | `lib.rs` first line was `#![no_std]` | Added `// SPDX-License-Identifier: MIT` as line 1 |

### Test Errors (lib.rs test modules)

| Test | Problem | Fix |
|---|---|---|
| All repay tests in `mod test` | `setup()` and `approve()` helpers called but not defined in that module | Added both helpers to `mod test` |
| Event tests in `mod test` | `TryFromVal` / `TryIntoVal` not in scope | Added to `use` statement |
| `test_suspend_nonexistent_credit_line` | Body opened a line with invalid rate (10001) instead of suspending a nonexistent borrower | Rewrote to call `suspend_credit_line` on an address with no line |
| `suspend_defaulted_line_reverts` | Body was testing draw/balance assertions, never called `default_credit_line` or `suspend_credit_line` | Rewrote to: default → suspend → expect panic |
| `test_draw_credit_updates_utilized` | Called `update_risk_parameters` with `risk_score = 101` (exceeds max of 100) | Changed to `70` |
| `test_multiple_borrowers` (smoke) | Called `suspend_credit_line` after `default_credit_line` — invalid transition that panics | Rewrote to open two borrowers and assert independent state |
| `test_event_reinstate_credit_line` (coverage gaps) | Called `setup_contract_with_credit_line` which is not in scope in that module | Switched to `base_setup` which is defined in the same module |

### Integration Test Error (`tests/duplicate_open_policy.rs`)

| Error | Root Cause | Fix |
|---|---|---|
| Non-exhaustive `match` on `CreditStatus` | `Restricted` variant added to enum but not covered in match | Added `CreditStatus::Restricted => {}` arm |

---

## Test Results

```
test result: ok. 66 passed; 0 failed  (lib)
test result: ok. 28 passed; 0 failed  (integration)
test result: ok. 3 passed;  0 failed  (spdx_header_bug_exploration)
test result: ok. 6 passed;  0 failed  (spdx_preservation)
test result: ok. 7 passed;  0 failed  (duplicate_open_policy)
```

---

## Security Notes

- `reinstate_credit_line` is admin-only. No borrower-initiated reinstatement path exists.
- Trust boundary: the admin is assumed to be a trusted off-chain system or multisig. No on-chain oracle or automated trigger is wired to this function.
- `utilized_amount` is intentionally preserved on reinstatement — the debt does not disappear. Reinstating to `Active` re-enables draws, so the admin should verify the borrower's repayment capacity before reinstating.
- Reinstating to `Suspended` is a safer intermediate step: it clears the `Defaulted` flag (e.g. for accounting) while keeping draws locked until a subsequent `Active` transition.
- Failure mode: if the admin key is compromised, an attacker could reinstate defaulted lines and allow draws. This is the same trust boundary as all other admin-only lifecycle functions.

---

Closes issue #115
