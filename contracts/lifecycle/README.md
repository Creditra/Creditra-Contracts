# Lifecycle capabilities view (v7)

Read-only bitmap reporting which lifecycle transitions are currently
permitted for a borrower's credit line, so off-chain clients can pre-flight
a transition without simulating a reverting call.

## Entrypoint

| Entrypoint | Returns | Notes |
| --- | --- | --- |
| `lifecycle_capabilities(borrower)` | [`LifecycleCapabilities`](../credit/src/types.rs) | Read-only, no auth required. |

## Fields

Every field is derived purely from the credit line's current `CreditStatus`,
`utilized_amount`, and the protocol pause flag — no token CPIs, no auth
checks, no mutation. All fields are `false` when no credit line exists for
the borrower, or when the protocol is paused.

| Field | `true` when |
| --- | --- |
| `can_suspend` | Status is `Active` (mirrors `suspend_credit_line`, admin path) |
| `can_self_suspend` | Status is `Active` (mirrors `self_suspend_credit_line`, borrower path) |
| `can_close_admin` | Status is not `Closed` (mirrors `close_credit_line`'s unconditional admin force-close) |
| `can_close_borrower` | `can_close_admin` **and** `utilized_amount == 0` (mirrors `close_credit_line`'s borrower self-close path) |
| `can_default` | Status is `Active`, `Restricted`, or `Suspended` (mirrors `default_credit_line`) |
| `can_reinstate` | Status is `Defaulted` (mirrors `reinstate_credit_line`) |

## Implementation

Logic lives in [`src/views.rs`](./src/views.rs) (compiled into `creditra-credit`
via a `#[path]` module, the same pattern used by
[`contracts/collateral/src/admin.rs`](../collateral/src/admin.rs)). The
public entrypoint is `Credit::lifecycle_capabilities` in
`contracts/credit/src/lib.rs`.

See [`tests/capabilities.rs`](./tests/capabilities.rs) for focused coverage
across every `CreditStatus` value plus the "no credit line" and "protocol
paused" edge cases.
