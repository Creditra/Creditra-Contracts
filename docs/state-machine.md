# CreditStatus State Machine — Formal Audit Artifact

**Crate:** `creditra-credit`  
**Source:** `contracts/credit/src/lib.rs`, `contracts/credit/src/types.rs`  
**Audit date:** 2026-03-27  

---

## States

| Variant | Discriminant | Description |
|---|---|---|
| `Active` | 0 | Credit line is open and available for draw/repay. |
| `Suspended` | 1 | Temporarily frozen; repay allowed, draw allowed (see note). |
| `Defaulted` | 2 | Borrower has defaulted; repay allowed, draw allowed (see note). |
| `Closed` | 3 | Terminal state; no further operations permitted. |

---

## Transition Table

| Source State | Destination State | Trigger Function | Caller | Status | Test Coverage | Security Note |
|---|---|---|---|---|---|---|
| *(none)* | `Active` | `open_credit_line` | Backend / risk engine | **Allowed** | `test_init_and_open_credit_line` | No auth guard on caller; trust boundary is off-chain. Duplicate open on Active line is rejected. |
| `Active` | `Suspended` | `suspend_credit_line` | Admin | **Allowed** | `test_suspend_credit_line` | Admin `require_auth` enforced. No source-state guard — also suspends Defaulted (see below). |
| `Active` | `Defaulted` | `default_credit_line` | Admin | **Allowed** | `test_default_credit_line` | Admin `require_auth` enforced. No source-state guard — applies to any non-Closed status. |
| `Active` | `Closed` | `close_credit_line` (admin) | Admin | **Allowed** | `test_close_credit_line` | Admin can force-close regardless of `utilized_amount`. |
| `Active` | `Closed` | `close_credit_line` (borrower) | Borrower | **Allowed** | `test_close_credit_line_borrower_when_utilized_zero` | Borrower can only close when `utilized_amount == 0`. |
| `Suspended` | `Defaulted` | `default_credit_line` | Admin | **Allowed** | `test_transition_suspended_to_defaulted` | No source-state guard on `default_credit_line`; transition is unconditional. |
| `Suspended` | `Closed` | `close_credit_line` (admin) | Admin | **Allowed** | `test_transition_active_suspended_closed_path` | Admin force-close from any non-Closed state. |
| `Suspended` | `Closed` | `close_credit_line` (borrower) | Borrower | **Allowed** | `test_close_credit_line_borrower_when_utilized_zero` | Borrower close requires `utilized_amount == 0`. |
| `Defaulted` | `Active` | `reinstate_credit_line` | Admin | **Allowed** | `test_reinstate_credit_line` | Only allowed when status is exactly `Defaulted`; panics otherwise. |
| `Defaulted` | `Suspended` | `suspend_credit_line` | Admin | **Allowed** | `test_transition_defaulted_to_suspended` | No source-state guard on `suspend_credit_line`; applies to any status. |
| `Defaulted` | `Closed` | `close_credit_line` (admin) | Admin | **Allowed** | `test_close_credit_line_defaulted_admin_force_close` | Admin force-close preserves `utilized_amount`. |
| `Defaulted` | `Closed` | `close_credit_line` (borrower) | Borrower | **Allowed** | `test_close_credit_line_defaulted_borrower_when_zero_utilization` | Borrower close requires `utilized_amount == 0`. |
| `Closed` | `Active` | `reinstate_credit_line` | Admin | **Forbidden** | `test_transition_closed_to_active_forbidden` | Panics: `"credit line is not defaulted"`. `Closed` is terminal. |
| `Closed` | `Suspended` | `suspend_credit_line` | Admin | **Forbidden** | *(implicit — no source guard; would succeed silently)* | **Gap:** `suspend_credit_line` has no `Closed` guard. Calling it on a Closed line would overwrite status to `Suspended`. This is an undocumented escape from the terminal state. See Security Notes. |
| `Closed` | `Defaulted` | `default_credit_line` | Admin | **Forbidden** | *(implicit — no source guard; would succeed silently)* | **Gap:** Same issue as above for `default_credit_line`. No guard prevents re-opening a Closed line via these functions. |
| `Closed` | `Closed` | `close_credit_line` | Admin / Borrower | **Idempotent** | `test_transition_closed_to_closed_is_idempotent` | Returns early without emitting an event. Safe. |
| `Active` | `Active` | `reinstate_credit_line` | Admin | **Forbidden** | `test_reinstate_credit_line_not_defaulted` | Panics: `"credit line is not defaulted"`. |
| `Suspended` | `Active` | `reinstate_credit_line` | Admin | **Forbidden** | `test_reinstate_credit_line_not_defaulted` | Panics: `"credit line is not defaulted"`. |

---

## State Diagram

```
                    open_credit_line
  (none) ─────────────────────────────► Active
                                          │
              suspend_credit_line         │  default_credit_line
         ┌────────────────────────────────┤──────────────────────────────┐
         ▼                                │                              ▼
     Suspended ──────────────────────────►│                          Defaulted
         │       default_credit_line      │                              │
         │                                │   reinstate_credit_line      │
         │                                │◄─────────────────────────────┘
         │                                │
         │  close_credit_line             │  close_credit_line
         └────────────────────────────────┴──────────────────────────────►  Closed
                                                                              (terminal)
```

---

## Security Notes

### Trust Boundaries

| Function | Auth Mechanism | Caller Trust |
|---|---|---|
| `open_credit_line` | None (no `require_auth`) | Off-chain backend / risk engine — trust is implicit |
| `suspend_credit_line` | `require_admin_auth` | On-chain admin address |
| `default_credit_line` | `require_admin_auth` | On-chain admin address |
| `reinstate_credit_line` | `require_admin_auth` | On-chain admin address |
| `close_credit_line` | `closer.require_auth()` | Admin (any util) or borrower (zero util only) |
| `draw_credit` | `borrower.require_auth()` | Borrower |
| `repay_credit` | `borrower.require_auth()` | Borrower |

### Assumptions

1. **`open_credit_line` has no on-chain auth guard.** The protocol assumes the calling backend is trusted. Any address can open a credit line for any borrower. This is an intentional design choice for the current version but represents a trust boundary that should be hardened before mainnet deployment.

2. **`suspend_credit_line` and `default_credit_line` have no source-state guard.** They unconditionally overwrite `status` regardless of the current state. This means:
   - A `Closed` line can be re-opened to `Suspended` or `Defaulted` by an admin. This is an **undocumented escape from the terminal state** and should be treated as a bug or explicitly documented as intentional.
   - Calling `suspend_credit_line` on an already-`Suspended` line is a no-op in effect but still emits an event and writes storage.

3. **`draw_credit` does not block on `Suspended` or `Defaulted` status.** Only `Closed` is explicitly blocked. The module-level doc comment states draw is disabled when Defaulted, but the implementation does not enforce this. This is a **discrepancy between documentation and code** that should be resolved.

4. **`close_credit_line` is idempotent for `Closed → Closed`.** It returns early without emitting an event, which is safe and intentional.

5. **`reinstate_credit_line` is the only function with a strict source-state guard** (`status != Defaulted` panics). All other transitions are either unconstrained or only guard against `Closed`.

### Documented Exceptions (Untestable in Current Environment)

- The `Closed → Suspended` and `Closed → Defaulted` paths via `suspend_credit_line` / `default_credit_line` are technically reachable but not explicitly tested because they represent unintended behaviour. Adding `#[should_panic]` tests for these would be misleading since the current code does **not** panic — it silently succeeds. These are flagged as gaps requiring a source-state guard fix.

---

## Coverage Mapping

All transitions marked **Allowed** in the table above are exercised by at least one test in the `#[cfg(test)] mod test` block of `contracts/credit/src/lib.rs`. The five new tests added in this branch specifically cover previously untested transitions:

| New Test | Transition Covered |
|---|---|
| `test_transition_suspended_to_defaulted` | `Suspended → Defaulted` |
| `test_transition_defaulted_to_suspended` | `Defaulted → Suspended` |
| `test_transition_closed_to_active_forbidden` | `Closed → Active` (forbidden) |
| `test_transition_closed_to_closed_is_idempotent` | `Closed → Closed` (idempotent) |
| `test_transition_active_suspended_closed_path` | `Active → Suspended → Closed` (multi-hop) |
