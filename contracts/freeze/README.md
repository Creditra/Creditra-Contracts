# Freeze auth boundary tests

Per-entrypoint authentication boundary coverage for every freeze-related
entrypoint on `creditra-credit` (issue #835 / buffer2 #15).

## State-changing (admin auth required)

| Entrypoint | Required auth |
| --- | --- |
| `freeze_draws` | admin |
| `unfreeze_draws` | admin |
| `freeze_credit_line` | admin |
| `unfreeze_credit_line` | admin |
| `freeze_borrower_until` | admin (explicit + role) |
| `unfreeze_borrower` | admin (explicit + role) |

## Read-only (no auth)

| Entrypoint |
| --- |
| `is_draws_frozen` |
| `get_draws_freeze_reason` |
| `is_credit_line_frozen` |
| `get_credit_line_freeze_reason` |
| `is_borrower_frozen` |
| `get_borrower_frozen_until` |

## Run

```bash
cargo test -p creditra-freeze --test auth_boundary
```

Implementation under test: [`contracts/credit/src/freeze.rs`](../credit/src/freeze.rs)
and the freeze entrypoints in [`contracts/credit/src/lib.rs`](../credit/src/lib.rs).
