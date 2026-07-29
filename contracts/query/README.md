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

## Auth snapshot (v7, issue #876)

Every entrypoint on the query surface is a pure read — none of them call
`require_auth`. [`tests/auth_snap.rs`](./tests/auth_snap.rs) pins that shape
per entrypoint so a future change that silently adds an authorization
requirement (breaking indexers/keepers that call these without a signer) is
caught immediately, even under `mock_all_auths`. See that file's module
doc for the full entrypoint table.

## Run tests

```bash
# Capabilities view tests
cargo test -p creditra-query --test capabilities

# Auth snapshot tests
cargo test -p creditra-query --test auth_snap

# Full query test suite
cargo test -p creditra-query
```

See [`tests/capabilities.rs`](./tests/capabilities.rs) and
[`tests/auth_snap.rs`](./tests/auth_snap.rs).
