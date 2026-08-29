# Freeze contract (v7)

Per-entrypoint authentication boundary coverage for every freeze-related
entrypoint on `creditra-credit` (issue #835 / buffer2 #15), plus a
read-only `capabilities()` bitmap view (issue #871).

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

| Entrypoint | Returns |
| --- | --- |
| `is_draws_frozen` | `bool` |
| `get_draws_freeze_reason` | `Option<FreezeReason>` |
| `is_credit_line_frozen` | `bool` |
| `get_credit_line_freeze_reason` | `Option<FreezeReason>` |
| `is_borrower_frozen` | `bool` |
| `get_borrower_frozen_until` | `Option<u64>` |
| `freeze_capabilities` | `u64` bitmap (v7) |

## Capabilities bitmap (v7)

`freeze_capabilities(&env)` returns a `u64` bitmask. Each bit corresponds
to a named constant exported from `creditra_freeze::views`:

| Constant | Bit | Hex | Feature |
| --- | --- | --- | --- |
| `CAPABILITY_FREEZE_DRAWS` | 0 | `0x01` | `freeze_draws` / `unfreeze_draws` |
| `CAPABILITY_FREEZE_CREDIT_LINE` | 1 | `0x02` | `freeze_credit_line` / `unfreeze_credit_line` |
| `CAPABILITY_FREEZE_BORROWER` | 2 | `0x04` | `freeze_borrower_until` / `unfreeze_borrower` |
| `CAPABILITY_FREEZE_REASON` | 3 | `0x08` | Structured `FreezeReason` + reason queries |
| `CAPABILITY_BORROWER_EXPIRY` | 4 | `0x10` | `get_borrower_frozen_until` expiry query |
| `CAPABILITY_FREEZE_COOLDOWN` | 5 | `0x20` | Admin cool-off guard on freeze ops |

The current aggregate value `ALL_FREEZE_CAPABILITIES = 0x3F` (all 6 bits set).

## Run

```bash
# Auth boundary tests
cargo test -p creditra-freeze --test auth_boundary

# Auth snapshot tests
cargo test -p creditra-freeze --test auth_snap

# Capabilities view tests
cargo test -p creditra-freeze --test views_capabilities

# Full freeze test suite
cargo test -p creditra-freeze
```

Implementation under test: [`contracts/credit/src/freeze.rs`](../credit/src/freeze.rs)
and the freeze entrypoints in [`contracts/credit/src/lib.rs`](../credit/src/lib.rs).
