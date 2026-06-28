# Storage layout reference

Authoritative reference for which `DataKey` variants live in which Soroban
storage tier. Source of truth: `contracts/credit/src/storage.rs`.

> **Full TTL audit:** For the complete per-variant TTL bump matrix — including
> bump function names, cadence, and touching entrypoints for all 30+ variants —
> see [`docs/storage-tiers.md`](storage-tiers.md).

## Why this matters

Soroban exposes three storage tiers — `temporary`, `instance`, and
`persistent`. The Creditra credit contract uses only `instance` and
`persistent`.

| Tier | Lifetime | TTL behavior |
| ---- | -------- | ------------ |
| `instance` | Bound to the contract instance, shared across all instance keys. | One TTL window per contract; `bump_instance_ttl` extends it. |
| `persistent` | Per-key. Survives instance archival. | Each key keeps its own TTL; bumped through `bump_credit_line_ttl`. |

## Instance storage

Used for global configuration and counters that the contract reads on
nearly every entrypoint. Sharing a single TTL is acceptable because the
working set is bounded and frequently touched.

| `DataKey` variant | Holds |
| ----------------- | ----- |
| `LiquidityToken` | Address of the SAC/token contract used for draws & repayments. |
| `LiquiditySource` | Reserve address that funds draws. |
| `DrawsFrozen` | Emergency switch for `draw_credit`. |
| `SchemaVersion` | Migration marker. |
| `CreditLineCount` | Monotonic count of indexed borrowers. |
| `TotalUtilized` | Aggregate outstanding principal across all lines. |
| `MaxDrawAmount` / `MaxRepayAmount` | Per-tx caps. |
| `DrawMinIntervalSeconds` | Global draw cooldown. |
| `MinCreditLimit` / `MaxCreditLimit` | Configurable limit bounds. |
| `PenaltySurchargeBps` | Delinquency surcharge. |
| `AuctionContract` | Default-liquidation hook target. |
| `MaxTotalExposure` | Protocol-level exposure cap. |
| `ProtocolFeeBps` | Fee taken on interest portion of repayments. |
| `TreasuryAddress` / `TreasuryBalance` | Fee sink. |
| `MinCollateralRatioBps` | Collateral floor for withdrawals. |
| `OracleConfig` / `OracleLastPrice` / `OracleLastPriceTs` | Oracle circuit breaker. |

Symbol-keyed instance entries (admin, proposed_admin, proposed_at,
reentrancy, rate_cfg, rate_form, paused, grace_cfg) live in the same tier.

## Persistent storage

Used for state that is unbounded in count (one entry per borrower or per
draw) and whose individual TTLs need to be tracked.

| `DataKey` variant | Holds |
| ----------------- | ----- |
| `CreditLineIdByBorrower(Address)` | Borrower → stable id. |
| `CreditLineBorrowerById(u32)` | Stable id → borrower. |
| `LastDrawTs(Address)` | Per-borrower cooldown clock. |
| `BlockedBorrower(Address)` | Per-borrower block list. |
| `UtilizationCapBps(Address)` | Per-borrower utilization ceiling. |
| `RateFloorBps(Address)` | Per-borrower interest floor. |
| `RateCeilingBps(Address)` | Per-borrower interest ceiling. |
| `RepaymentSchedule(Address)` | Installment schedule. |
| `CollateralBalance(Address)` | Per-borrower collateral. |
| `DrawAudit(Address, u64)` | Audit trail entry. |
| `DrawReversedAmount(Address, u64)` | Reversal accumulator. |

The `CreditLineData` struct itself is stored against the borrower address
key directly (not via a `DataKey` variant) for backward compatibility.

## TTL bump strategy

```text
LEDGER_BUMP_THRESHOLD = 1_555_200 ledgers (~3 months)
LEDGER_BUMP_AMOUNT    = 3_110_400 ledgers (~6 months)
```

Both instance and persistent storage use the same constants. `extend_ttl`
is a no-op if the remaining TTL is already above the threshold, so callers
can invoke the helpers liberally without worrying about wasted writes.

`bump_persistent_ttl` opportunistically bumps the instance TTL too, so any
function touching a per-borrower record also keeps global config alive.
