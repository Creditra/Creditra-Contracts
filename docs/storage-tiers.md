# Storage Tiers — Complete TTL Bump Audit

**Source of truth:** `contracts/credit/src/storage.rs`  
**Cross-reference:** [`docs/storage-layout.md`](storage-layout.md)

This document is the authoritative reference for every `DataKey` variant in
the credit contract, mapping each to its storage tier, TTL bump function,
bump cadence, and the entrypoints that read or write it.

---

## TTL Constants

```
LEDGER_BUMP_THRESHOLD  = 1_555_200 ledgers  (~3 months at 5 s/ledger)
LEDGER_BUMP_AMOUNT     = 3_110_400 ledgers  (~6 months at 5 s/ledger)
INSTANCE_BUMP_THRESHOLD = LEDGER_BUMP_THRESHOLD
INSTANCE_BUMP_AMOUNT    = LEDGER_BUMP_AMOUNT
```

### Bump helper call chain

```
bump_instance_ttl(env)
  └─ env.storage().instance()
         .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT)

bump_persistent_ttl(env, key)            ← private helper
  ├─ bump_instance_ttl(env)              ← also keeps instance alive
  └─ env.storage().persistent()
         .extend_ttl(key, LEDGER_BUMP_THRESHOLD, LEDGER_BUMP_AMOUNT)

bump_credit_line_ttl(env, borrower)
  └─ bump_persistent_ttl(env, borrower) ← key = borrower Address directly
```

> `extend_ttl` is a no-op when the remaining TTL is already above the
> threshold, so helpers can be called on every read/write without wasted
> ledger writes.

---

## Symbol-Keyed Instance Entries

These entries live in instance storage under `Symbol` keys (not `DataKey`
enum variants). They share the instance TTL and are bumped by
`bump_instance_ttl`.

| Symbol key | Helper that defines key | Bump function | Touching entrypoints |
|---|---|---|---|
| `"admin"` | `admin_key` | `bump_instance_ttl` (via any entrypoint) | `init`, `propose_admin`, `accept_admin`, `require_admin_auth` (all admin entrypoints) |
| `"proposed_admin"` | `proposed_admin_key` | `bump_instance_ttl` | `propose_admin`, `accept_admin` |
| `"proposed_at"` | `proposed_at_key` | `bump_instance_ttl` | `propose_admin`, `accept_admin` |
| `"reentrancy"` | `reentrancy_key` | `bump_instance_ttl` (no dedicated bump; transient flag) | `set_reentrancy_guard`, `clear_reentrancy_guard` — called by `draw_credit`, `repay_credit` |
| `"rate_cfg"` | `rate_cfg_key` | `bump_instance_ttl` | `update_risk_parameters`, `set_rate_change_limits`, interest accrual reads |
| `"rate_form"` | `rate_formula_key` | `bump_instance_ttl` | `set_rate_formula_config`, `clear_rate_formula_config`, risk scoring reads |
| `"paused"` | `paused_key` | `bump_instance_ttl` | `set_protocol_paused`, `assert_not_paused` (every mutating entrypoint) |
| `"grace_cfg"` | `grace_period_key` | `bump_instance_ttl` | `set_grace_period_config`, delinquency checks |

---

## Complete DataKey Variant Table

### Instance Storage Variants

Instance storage entries share a single TTL window with the contract.
They are bumped by `bump_instance_ttl`, which is called by
`bump_persistent_ttl` on every persistent read/write (so touching any
borrower record also refreshes instance TTL).

| `DataKey` variant | Value type | Bump function | Bump cadence | Touching entrypoints |
|---|---|---|---|---|
| `LiquidityToken` | `Address` | `bump_instance_ttl` | On every persistent r/w (via `bump_persistent_ttl`) | `set_liquidity_token`, `draw_credit` (read), `repay_credit` (read), `get_collateral_token` |
| `LiquiditySource` | `Address` | `bump_instance_ttl` | On every persistent r/w | `set_liquidity_source`, `draw_credit` (read) |
| `DrawsFrozen` | `bool` | `bump_instance_ttl` | On every persistent r/w | `freeze_draws`, `unfreeze_draws`, `is_draws_frozen`, `draw_credit` (guard) |
| `SchemaVersion` | `u32` | `bump_instance_ttl` | On every persistent r/w | `get_schema_version`, `set_schema_version` (internal migration only) |
| `CreditLineCount` | `u32` | `bump_instance_ttl` | On every persistent r/w; written by `ensure_credit_line_id` | `get_credit_line_count`, `open_credit_line`, `enumerate_credit_lines` |
| `TotalUtilized` | `i128` | `bump_instance_ttl` | On every `persist_credit_line` call | `adjust_total_utilized` ← `persist_credit_line` ← `draw_credit`, `repay_credit`, `open_credit_line`, `close_credit_line`, `forgive_debt`, `settle_default_liquidation`, `default_credit_line`, `reinstate_credit_line`, `suspend_credit_line`, `self_suspend_credit_line` |
| `MaxDrawAmount` | `i128` | `bump_instance_ttl` | On every persistent r/w | `set_max_draw_amount`, `draw_credit` (cap check) |
| `MaxRepayAmount` | `i128` | `bump_instance_ttl` | On every persistent r/w | `set_max_repay_amount`, `repay_credit` (cap check) |
| `DrawMinIntervalSeconds` | `u64` | `bump_instance_ttl` | On every persistent r/w | `set_draw_min_interval`, `draw_credit` (cooldown check) |
| `MinCreditLimit` | `i128` | `bump_instance_ttl` | On every persistent r/w | `set_min_credit_limit`, `open_credit_line` (validation) |
| `MaxCreditLimit` | `i128` | `bump_instance_ttl` | On every persistent r/w | `set_max_credit_limit`, `open_credit_line` (validation) |
| `PenaltySurchargeBps` | `u32` | `bump_instance_ttl` | On every persistent r/w | `get_penalty_surcharge_bps`, `set_penalty_surcharge_bps`, interest accrual (rate computation) |
| `AuctionContract` | `Address` | `bump_instance_ttl` | On every persistent r/w | `set_auction_contract`, `settle_default_liquidation` (cross-contract hook) |
| `MaxTotalExposure` | `i128` | `bump_instance_ttl` | On every persistent r/w | `set_max_total_exposure`, `draw_credit` (exposure guard) |
| `ProtocolFeeBps` | `u32` | `bump_instance_ttl` | On every persistent r/w | `set_protocol_fee_bps`, `repay_credit` (fee split) |
| `TreasuryAddress` | `Address` | `bump_instance_ttl` | On every persistent r/w | `set_treasury_address`, `withdraw_treasury_fees` |
| `TreasuryBalance` | `i128` | `bump_instance_ttl` | On every persistent r/w | `add_treasury_balance` ← `repay_credit`, `clear_treasury_balance` ← `withdraw_treasury_fees` |
| `MinCollateralRatioBps` | `u32` | `bump_instance_ttl` | On every persistent r/w | `set_min_collateral_ratio_bps`, collateral withdrawal check |
| `OracleConfig` | `OracleConfig` struct | `bump_instance_ttl` | On every persistent r/w | `set_oracle_config`, `settle_default_liquidation` (price validation) |
| `OracleLastPrice` | `i128` | `bump_instance_ttl` | On every persistent r/w; written atomically with `OracleLastPriceTs` | `set_oracle_last_price`, `get_oracle_last_price` ← `settle_default_liquidation` |
| `OracleLastPriceTs` | `u64` | `bump_instance_ttl` | On every persistent r/w; written atomically with `OracleLastPrice` | `set_oracle_last_price`, `get_oracle_last_price_ts` ← `settle_default_liquidation` |
| `TotalCollateral` | `i128` | `bump_instance_ttl` | On every persistent r/w | `adjust_total_collateral` ← `set_collateral_balance`, called by collateral deposit/withdraw entrypoints |

---

### Persistent Storage Variants

Each persistent entry carries its own TTL, extended when remaining TTL drops
below `LEDGER_BUMP_THRESHOLD` (~3 months) to `LEDGER_BUMP_AMOUNT` (~6 months).
`bump_persistent_ttl` also calls `bump_instance_ttl` as a side-effect.

| `DataKey` variant | Value type | Bump function | Bump cadence | Touching entrypoints |
|---|---|---|---|---|
| `CreditLineIdByBorrower(Address)` | `u32` | `bump_persistent_ttl` (via `ensure_credit_line_id` → `persist_credit_line`) | On every `persist_credit_line` for a new borrower | `ensure_credit_line_id` ← `open_credit_line`, `draw_credit`, `repay_credit` and any entrypoint calling `persist_credit_line` |
| `CreditLineBorrowerById(u32)` | `Address` | `bump_persistent_ttl` (via `ensure_credit_line_id`) | On every `persist_credit_line` for a new borrower | `ensure_credit_line_id` ← same as above; `get_borrower_by_credit_line_id` ← `enumerate_credit_lines` |
| `LastDrawTs(Address)` | `u64` | none (direct `persistent().set`) | Written on every successful draw; **no explicit TTL bump** — relies on credit-line entry being bumped by `persist_credit_line` in the same call | `set_last_draw_ts` ← `draw_credit`; `get_last_draw_ts` ← `draw_credit` (cooldown enforcement) |
| `BlockedBorrower(Address)` | `bool` | none (direct `persistent().set`) | Written on block/unblock; **no explicit TTL bump** | `set_borrower_blocked` ← `block_borrower`, `unblock_borrower`, `bulk_block_borrowers`; `is_borrower_blocked` ← `draw_credit` |
| `UtilizationCapBps(Address)` | `u32` | none (direct `persistent().set`) | Written on cap set/clear; **no explicit TTL bump** | `set_utilization_cap_bps` ← `set_utilization_cap`; `get_utilization_cap_bps` ← `draw_credit` |
| `RateFloorBps(Address)` | `u32` | none (direct `persistent().set`) | Written on floor set/clear; **no explicit TTL bump** | `set_borrower_rate_floor` ← `set_borrower_rate_floor` entrypoint; `get_borrower_rate_floor` ← interest rate computation |
| `RateCeilingBps(Address)` | `u32` | none (direct `persistent().set`) | Written on ceiling set/clear; **no explicit TTL bump** | `set_borrower_rate_ceiling` ← `set_borrower_rate_ceiling` entrypoint; `get_borrower_rate_ceiling` ← interest rate computation |
| `RepaymentSchedule(Address)` | `RepaymentSchedule` struct | `bump_persistent_ttl` | Bumped on **every read and write** via `storage::get_repayment_schedule` / `storage::set_repayment_schedule`; `set_repayment_schedule` also bumps the credit-line entry | `set_repayment_schedule` ← `set_repayment_schedule` entrypoint; `get_repayment_schedule` ← getter + delinquency checks; `clear_repayment_schedule` ← `close_credit_line` |
| `CollateralBalance(Address)` | `i128` | none (direct `persistent().set`) | Written on deposit/withdraw; **no explicit TTL bump** | `set_collateral_balance` ← collateral deposit/withdraw entrypoints; `get_collateral_balance` ← collateral ratio checks, `settle_default_liquidation` |
| `DrawAudit(Address, u64)` | `i128` | none (direct `persistent().set`) | Written on draw; **no explicit TTL bump** | Written in `reverse_draw` (read at line 1393 of `lib.rs`); `get` ← `reverse_draw` |
| `DrawReversedAmount(Address, u64)` | `i128` | none (direct `persistent().set`) | Accumulated on each partial reversal; **no explicit TTL bump** | `persistent().set` ← `reverse_draw` (line 1413 of `lib.rs`); `get` ← `reverse_draw` (line 1398) |

> **⚠ TTL hygiene note — unbumped persistent keys**
>
> The ten persistent-tier variants marked "no explicit TTL bump" above rely on
> co-location with the borrower's `CreditLineData` entry (stored directly
> under the borrower address) which *is* bumped by `bump_credit_line_ttl` on
> every `persist_credit_line` call. For an **active** borrower this is
> sufficient because every `draw_credit` / `repay_credit` will refresh the
> credit-line entry.
>
> However, entries written in isolation (e.g. an admin blocks a borrower who
> has never drawn, or a `DrawAudit` entry for a borrower whose credit line
> has since been closed) will age independently and may be archived if their
> TTL expires before the next `persist_credit_line` for that borrower. Callers
> that need these entries to survive beyond the ~6-month window without a draw
> or repay should call `bump_persistent_ttl` explicitly on the relevant key.

---

## Summary — Bump Coverage by Variant

| Variant | Tier | Explicit bump? | Bump function |
|---|---|---|---|
| `LiquidityToken` | Instance | ✅ | `bump_instance_ttl` |
| `LiquiditySource` | Instance | ✅ | `bump_instance_ttl` |
| `DrawsFrozen` | Instance | ✅ | `bump_instance_ttl` |
| `SchemaVersion` | Instance | ✅ | `bump_instance_ttl` |
| `CreditLineCount` | Instance | ✅ | `bump_instance_ttl` |
| `TotalUtilized` | Instance | ✅ | `bump_instance_ttl` |
| `MaxDrawAmount` | Instance | ✅ | `bump_instance_ttl` |
| `MaxRepayAmount` | Instance | ✅ | `bump_instance_ttl` |
| `DrawMinIntervalSeconds` | Instance | ✅ | `bump_instance_ttl` |
| `MinCreditLimit` | Instance | ✅ | `bump_instance_ttl` |
| `MaxCreditLimit` | Instance | ✅ | `bump_instance_ttl` |
| `PenaltySurchargeBps` | Instance | ✅ | `bump_instance_ttl` |
| `AuctionContract` | Instance | ✅ | `bump_instance_ttl` |
| `MaxTotalExposure` | Instance | ✅ | `bump_instance_ttl` |
| `ProtocolFeeBps` | Instance | ✅ | `bump_instance_ttl` |
| `TreasuryAddress` | Instance | ✅ | `bump_instance_ttl` |
| `TreasuryBalance` | Instance | ✅ | `bump_instance_ttl` |
| `MinCollateralRatioBps` | Instance | ✅ | `bump_instance_ttl` |
| `OracleConfig` | Instance | ✅ | `bump_instance_ttl` |
| `OracleLastPrice` | Instance | ✅ | `bump_instance_ttl` |
| `OracleLastPriceTs` | Instance | ✅ | `bump_instance_ttl` |
| `TotalCollateral` | Instance | ✅ | `bump_instance_ttl` |
| `CreditLineIdByBorrower(Address)` | Persistent | ✅ | `bump_persistent_ttl` (via `ensure_credit_line_id`) |
| `CreditLineBorrowerById(u32)` | Persistent | ✅ | `bump_persistent_ttl` (via `ensure_credit_line_id`) |
| `LastDrawTs(Address)` | Persistent | ⚠️ indirect | Co-bumped by `bump_credit_line_ttl` on same `draw_credit` call |
| `BlockedBorrower(Address)` | Persistent | ⚠️ indirect | Co-bumped only if `persist_credit_line` runs for that borrower |
| `UtilizationCapBps(Address)` | Persistent | ⚠️ indirect | Co-bumped only if `persist_credit_line` runs for that borrower |
| `RateFloorBps(Address)` | Persistent | ⚠️ indirect | Co-bumped only if `persist_credit_line` runs for that borrower |
| `RateCeilingBps(Address)` | Persistent | ⚠️ indirect | Co-bumped only if `persist_credit_line` runs for that borrower |
| `RepaymentSchedule(Address)` | Persistent | ✅ | `bump_persistent_ttl` on every `storage::get_repayment_schedule` / `set_repayment_schedule` read/write |
| `CollateralBalance(Address)` | Persistent | ⚠️ indirect | Co-bumped only if `persist_credit_line` runs for that borrower |
| `DrawAudit(Address, u64)` | Persistent | ⚠️ indirect | Co-bumped only if `persist_credit_line` runs for same borrower |
| `DrawReversedAmount(Address, u64)` | Persistent | ⚠️ indirect | Co-bumped only if `persist_credit_line` runs for same borrower |

---

## Auction Contract Storage (gateway-contract)

For completeness, the auction contract's `storage.rs`
(`gateway-contract/contracts/auction_contract/src/storage.rs`) uses a
separate, shorter TTL policy:

```
PERSISTENT_BUMP_AMOUNT    = 518_400 ledgers  (~30 days)
PERSISTENT_LIFETIME_THRESHOLD = 120_960 ledgers  (~7 days)
```

Its reentrancy guard (`Symbol("reentrancy")`) is stored in instance storage
and is functionally transient (set/cleared within a single transaction).
See [`docs/threat-model.md`](threat-model.md) §"Soroban-Specific Reentrancy
via `__check_auth` Callbacks" for the full security analysis.
