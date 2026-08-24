# Query capabilities view (v7)

Read-only bitmap reporting which borrower-scoped query results are currently
meaningful, so off-chain clients and keepers can batch availability checks
without issuing multiple separate reads.

## Entrypoint

| Entrypoint | Returns | Notes |
| --- | --- | --- |
| `query_capabilities(borrower)` | [`QueryCapabilities`](../credit/src/types.rs) | Read-only, no auth required. |

The implementation lives in [`src/views.rs`](./src/views.rs) as
`capabilities()` (compiled into `creditra-credit` via a `#[path]` module,
same pattern as [`contracts/lifecycle/src/views.rs`](../lifecycle/src/views.rs)).

## Fields

| Field | `true` when |
| --- | --- |
| `has_credit_line` | A credit line record exists for the borrower |
| `has_repayment_schedule` | A repayment schedule is configured |
| `health_factor_applicable` | `utilized_amount > 0` (otherwise health factor is `u32::MAX`) |
| `delinquency_applicable` | Open line + utilization + schedule (mirrors `is_delinquent` gates) |
| `is_delinquent` | Current delinquency status; always `false` when not applicable |

## Run tests

```bash
cargo test -p creditra-query --test capabilities
```

See [`tests/capabilities.rs`](./tests/capabilities.rs).
