# CreditStatus State Machine

**Crate:** `creditra-credit`  
**Primary sources:** `contracts/credit/src/lifecycle.rs`, `contracts/credit/src/lib.rs`, `contracts/credit/src/types.rs`

---

## States

| Variant | Discriminant | Description |
|---|---|---|
| `Active` | 0 | Credit line is open and can draw or repay. |
| `Suspended` | 1 | Draws are blocked. Repayments remain allowed. |
| `Defaulted` | 2 | Draws are blocked. Repayments remain allowed. |
| `Closed` | 3 | Terminal state for the current line record. |

---

## Transition Table

| Source | Destination | Trigger | Caller | Allowed | Notes |
|---|---|---|---|---|---|
| *(none)* | `Active` | `open_credit_line` | Backend / risk engine | Yes | First issuance keeps the existing off-chain trust boundary. |
| `Active` | `Suspended` | `suspend_credit_line` | Admin | Yes | Admin-authenticated suspend. |
| `Active` | `Suspended` | `self_suspend_credit_line` | Borrower | Yes | Borrower-authenticated safety control. |
| `Active` | `Defaulted` | `default_credit_line` | Admin | Yes | Admin-authenticated default. |
| `Active` | `Closed` | `close_credit_line` | Admin | Yes | Admin can force-close regardless of utilization. |
| `Active` | `Closed` | `close_credit_line` | Borrower | Yes | Borrower may close only when `utilized_amount == 0`. |
| `Suspended` | `Defaulted` | `default_credit_line` | Admin | Yes | Escalation path from suspend to default. |
| `Suspended` | `Active` | `open_credit_line` reopen | Admin-approved workflow | Yes | Re-opening an existing non-Active line now requires admin auth. |
| `Suspended` | `Closed` | `close_credit_line` | Admin | Yes | Admin force-close remains available. |
| `Suspended` | `Closed` | `close_credit_line` | Borrower | Yes | Borrower may close only when `utilized_amount == 0`. |
| `Defaulted` | `Active` | `reinstate_credit_line` | Admin | Yes | `reinstate_credit_line` is only for `Defaulted -> Active`. |
| `Defaulted` | `Closed` | `close_credit_line` | Admin | Yes | Admin force-close remains available. |
| `Defaulted` | `Closed` | `close_credit_line` | Borrower | Yes | Borrower may close only when `utilized_amount == 0`. |
| `Suspended` | `Suspended` | `suspend_credit_line` / `self_suspend_credit_line` | Admin / Borrower | No | Suspend is Active-only. |
| `Defaulted` | `Suspended` | `suspend_credit_line` | Admin | No | Suspend is Active-only. |
| `Closed` | `Suspended` | `suspend_credit_line` / `self_suspend_credit_line` | Admin / Borrower | No | Closed lines cannot be suspended. |
| `Closed` | `Defaulted` | `default_credit_line` | Admin | No | Closed lines cannot be defaulted. |
| `Active` | `Active` | `reinstate_credit_line` | Admin | No | Reinstate rejects non-Defaulted lines. |
| `Suspended` | `Active` | `reinstate_credit_line` | Admin | No | Suspended lines are not directly reinstated by this entrypoint. |
| `Closed` | `Active` | `reinstate_credit_line` | Admin | No | Closed lines are not defaulted, so reinstate fails. |
| `Closed` | `Closed` | `close_credit_line` | Admin / Borrower | Idempotent | Returns early without mutating state. |

---

## Operational Rules

1. `draw_credit` is allowed only while the line is `Active`.
2. `repay_credit` remains allowed while the line is `Active`, `Suspended`, or `Defaulted`.
3. `self_suspend_credit_line` requires borrower auth and does not create any borrower-controlled reactivation path.
4. Returning a self-suspended line to `Active` requires an admin-approved reopen workflow.
5. Re-opening any existing non-`Active` line requires admin auth to prevent borrowers from bypassing self-suspend or other admin controls.

---

## Auth Matrix

| Function | Auth Requirement |
|---|---|
| `open_credit_line` | No auth on first issuance; admin auth required when reopening an existing non-Active line |
| `suspend_credit_line` | Admin auth |
| `self_suspend_credit_line` | Borrower auth |
| `default_credit_line` | Admin auth |
| `reinstate_credit_line` | Admin auth |
| `close_credit_line` | `closer.require_auth()` |
| `draw_credit` | Borrower auth |
| `repay_credit` | Borrower auth |

---

## Coverage

| Test | What it proves |
|---|---|
| `self_suspend_requires_only_borrower_auth` | Borrower can self-suspend without admin auth and transitions to `Suspended`. |
| `self_suspend_blocks_draws_but_allows_repayments` | Self-suspend blocks draws while leaving repayment available. |
| `self_suspended_line_cannot_be_reopened_without_admin_auth` | Borrower cannot bypass self-suspend by reopening without admin approval. |
| `suspend_only_valid_from_active` | Suspend remains Active-only. |
| `reinstate_only_valid_from_defaulted` | Reinstate remains limited to `Defaulted -> Active`. |
