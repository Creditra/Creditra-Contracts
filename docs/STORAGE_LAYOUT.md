# Storage Layout — Creditra Contracts

**Issue:** #1306  
**Source of truth:** `contracts/credit/src/storage.rs`, `contracts/credit/src/oracles.rs`,
`contracts/credit/src/lifecycle.rs`

This document is the single authoritative reference for every storage key used
across the Creditra contract suite. It replaces the stale `storage-layout.md`
and `storage-tiers.md` files (both deleted with this PR).

---

## Storage Tier Primer

Soroban exposes three storage tiers. Creditra uses **instance** and
**persistent** only.

| Tier | Soroban API | TTL scope | Eviction |
|------|-------------|-----------|----------|
| **Instance** | `env.storage().instance()` | Single TTL shared across all instance keys for this contract | Never while the instance TTL is live |
| **Persistent** | `env.storage().persistent()` | Per-key independent TTL | Yes — archived when TTL expires |
| **Temporary** | `env.storage().temporary()` | Short, auto-expiring | Yes — automatic; not used in the credit contract |

---

## TTL Constants (`contracts/credit/src/storage.rs`)

```rust
LEDGER_BUMP_THRESHOLD  = 1_555_200 ledgers  // ~3 months at 5 s/ledger
LEDGER_BUMP_AMOUNT     = 3_110_400 ledgers  // ~6 months at 5 s/ledger

INSTANCE_BUMP_THRESHOLD = LEDGER_BUMP_THRESHOLD
INSTANCE_BUMP_AMOUNT    = LEDGER_BUMP_AMOUNT
```

### Bump helper call chain

```
bump_instance_ttl(env)
  └─ env.storage().instance()
         .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT)

bump_persistent_ttl(env, key)
  ├─ bump_instance_ttl(env)              // side-effect: keeps instance alive
  └─ env.storage().persistent()
         .extend_ttl(key, LEDGER_BUMP_THRESHOLD, LEDGER_BUMP_AMOUNT)

bump_credit_line_ttl(env, borrower)
  └─ bump_persistent_ttl(env, borrower)  // key = borrower Address directly
```

`extend_ttl` is a no-op when the remaining TTL already exceeds the threshold,
so helpers can be called on every read/write path without wasted ledger writes.

---

## 1. Credit Contract — Symbol-keyed Instance Entries

These entries live in **instance** storage under `Symbol` keys (not `DataKey`
enum variants). They share the instance TTL and are bumped by
`bump_instance_ttl`, which is called transitively from every
`bump_persistent_ttl` invocation.

| Symbol key | Helper | Value type | Writer entrypoints |
|------------|--------|------------|--------------------|
| `"admin"` | `admin_key` | `Address` | `init`, `accept_admin` |
| `"proposed_admin"` | `proposed_admin_key` | `Address` | `propose_admin` |
| `"proposed_at"` | `proposed_at_key` | `u64` | `propose_admin` |
| `"reentrancy"` | `reentrancy_key` | `bool` | `draw_credit`, `repay_credit` (set/clear guard) |
| `"rate_cfg"` | `rate_cfg_key` | `RateConfig` | `update_risk_parameters`, `set_rate_change_limits` |
| `"rate_form"` | `rate_formula_key` | `RateFormulaConfig` | `set_rate_formula_config`, `clear_rate_formula_config` |
| `"paused"` | `paused_key` | `bool` | `pause_protocol`, `unpause_protocol` |
| `"grace_cfg"` | `grace_period_key` | `GracePeriodConfig` | `set_grace_period_config` |

### symbol_short keys (risk admin cooldown)

| Key (`symbol_short!`) | Value type | Writer entrypoints |
|-----------------------|------------|--------------------|
| `"rad_cool"` | `u64` (seconds) | `set_risk_admin_cooldown_seconds` |
| `"rad_last"` | `u64` (timestamp) | `set_last_risk_admin_action_ts` — called by all risk mutation entrypoints |

---

## 2. Credit Contract — `DataKey` Enum

`DataKey` is defined in `contracts/credit/src/storage.rs` with
`#[contracttype(export = false)]`. It is an internal type and never crosses the
contract ABI. Variants are distributed across the two long-lived storage tiers
as described below.

### 2a. Instance Storage Variants

Instance storage entries share a single TTL window with the contract.
`bump_instance_ttl` is called as a side-effect of every `bump_persistent_ttl`
invocation, so any entrypoint that touches a per-borrower record also refreshes
instance storage.

| `DataKey` variant | Value type | Writer entrypoints |
|-------------------|------------|--------------------|
| `LiquidityToken` | `Address` | `set_liquidity_token` |
| `LiquiditySource` | `Address` | `set_liquidity_source` |
| `DrawsFrozen` | `bool` | `freeze_draws`, `unfreeze_draws` |
| `SchemaVersion` | `u32` | Internal migration only |
| `CreditLineCount` | `u32` | `open_credit_line` (via `ensure_credit_line_id`) |
| `ActiveLineCount` | `u32` | `persist_credit_line` (auto-maintained on status transitions) |
| `PendingAuctionCount` | `u32` | `default_credit_line` (increment), `settle_default_liquidation` / `reinstate_credit_line` / `close_credit_line` (decrement) |
| `TotalUtilized` | `i128` | `persist_credit_line` (via `adjust_total_utilized`) — every draw/repay/open/close/settle |
| `MaxDrawAmount` | `i128` | `set_max_draw_amount` |
| `MaxRepayAmount` | `i128` | `set_max_repay_amount` |
| `DrawMinIntervalSeconds` | `u64` | `set_draw_min_interval` |
| `BorrowAdminCooldownSeconds` | `u64` | `set_borrow_admin_cooldown` |
| `AccrualAdminCooldownSeconds` | `u64` | `set_accrual_admin_cooldown` |
| `MinCreditLimit` | `i128` | `set_credit_limit_bounds` |
| `MaxCreditLimit` | `i128` | `set_credit_limit_bounds` |
| `CloseFactorBps` | `u32` | `set_close_factor_bps` |
| `PenaltySurchargeBps` | `u32` | `set_penalty_surcharge_bps` |
| `LateFeeFlat` | `i128` | `set_late_fee_flat` |
| `LateFeeConfig` | `LateFeeConfig` | `set_late_fee_config` |
| `AuctionContract` | `Address` | `set_auction_contract` |
| `MaxTotalExposure` | `i128` | `set_max_total_exposure` |
| `ProtocolFeeBps` | `u32` | `set_protocol_fee_bps` |
| `TreasuryFeeShareBps` | `u32` | `set_treasury_fee_share_bps` |
| `TreasuryAddress` | `Address` | `set_treasury` |
| `TreasuryBalance` | `i128` | `add_treasury_balance` (← `repay_credit`), `clear_treasury_balance` (← `withdraw_treasury`) |
| `BountyAddress` | `Address` | `set_bounty_address` |
| `BountyBalance` | `i128` | `add_bounty_balance`, `clear_bounty_balance` |
| `MinCollateralRatioBps` | `u32` | `set_min_collateral_ratio_bps` |
| `AdminCollateralCooldownSeconds` | `u64` | `set_admin_collateral_cooldown_seconds` |
| `LastColAdminActionTs` | `u64` | `set_last_admin_collateral_critical_action_ts` |
| `CollateralRiskWeightBps(Address)` | `u32` | `set_collateral_risk_weight_bps` |
| `OracleConfig` | `OracleConfig` | `set_oracle_config` |
| `OracleLastPrice` | `i128` | `set_oracle_last_price` (atomic pair) |
| `OracleLastPriceTs` | `u64` | `set_oracle_last_price` (atomic pair) |
| `OracleQuorumConfig` | `OracleQuorumConfig` | `set_oracle_quorum_config` |
| `OracleQuorumPrice` | `i128` | `set_oracle_quorum_price` (atomic pair) |
| `OracleQuorumPriceTs` | `u64` | `set_oracle_quorum_price` (atomic pair) |
| `TotalCollateral` | `i128` | `set_collateral_balance` / `set_collateral_balance_for_token` (via `adjust_total_collateral`) |
| `PendingTreasuryWithdrawal` | `TreasuryWithdrawalProposal` | `set_pending_treasury_withdrawal`, cleared by `clear_pending_treasury_withdrawal` |
| `PauseReason` | `PauseReason` | `set_pause_reason` (← `pause_protocol`), `clear_pause_reason` (← `unpause_protocol`) |
| `FreezeCooldownSeconds` | `u64` | `set_freeze_cooldown_seconds` |
| `LastFreezeTimestamp` | `u64` | `record_freeze_timestamp_if_cooldown` |
| `CollateralTokenAllowlist` | `Vec<Address>` | `set_collateral_token_allowlist` |

### 2b. Persistent Storage Variants

Each persistent entry carries its own TTL. `bump_persistent_ttl` extends it to
`LEDGER_BUMP_AMOUNT` (~6 months) when the remaining TTL drops below
`LEDGER_BUMP_THRESHOLD` (~3 months). Callers that do not go through the typed
helpers in `storage.rs` may not bump automatically — see the TTL hygiene note
below.

| `DataKey` variant | Value type | Explicit TTL bump? | Writer entrypoints |
|-------------------|------------|--------------------|--------------------|
| `CreditLineIdByBorrower(Address)` | `u32` | ✅ `bump_persistent_ttl` via `ensure_credit_line_id` | `open_credit_line` (first call per borrower) |
| `CreditLineBorrowerById(u32)` | `Address` | ✅ `bump_persistent_ttl` via `ensure_credit_line_id` | `open_credit_line` (first call per borrower) |
| `LastDrawTs(Address)` | `u64` | ✅ `bump_persistent_ttl` on read & write | `draw_credit` |
| `BlockedBorrower(Address)` | `bool` | ✅ `bump_persistent_ttl` on read & write | `block_borrower`, `unblock_borrower`, `bulk_block_borrowers` |
| `FrozenBorrower(Address)` | `u64` (expiry timestamp) | ✅ `bump_borrower_frozen_ttl` on read & write | `freeze_borrower` / admin freeze entrypoints |
| `CreditLineFreeze(Address)` | `FreezeInfo` | ✅ `bump_credit_line_freeze_ttl` on write | Admin freeze entrypoints |
| `UtilizationCapBps(Address)` | `u32` | ✅ `bump_persistent_ttl` on read & write | `set_utilization_cap` |
| `RateFloorBps(Address)` | `u32` | ✅ `bump_persistent_ttl` on read & write | `set_borrower_rate_floor` |
| `RateCeilingBps(Address)` | `u32` | ✅ `bump_persistent_ttl` on read & write | `set_borrower_rate_ceiling` |
| `RepaymentSchedule(Address)` | `RepaymentSchedule` | ✅ `bump_persistent_ttl` (implicit via `storage::set_repayment_schedule`) | `set_repayment_schedule`, `advance_repayment_schedule_after_repay` |
| `VrfCommitment(Address)` | `BytesN<32>` | ⚠️ No explicit bump | VRF-related admin entrypoints |
| `CollateralBalance(Address)` | `i128` | ✅ `bump_persistent_ttl` on read & write | `deposit_collateral`, `withdraw_collateral`, `partial_release_collateral` |
| `CollateralBalanceV2(Address, Address)` | `i128` (per-token) | ✅ `bump_persistent_ttl` on read & write | Multi-collateral deposit/withdraw entrypoints |
| `DrawAudit(DrawAuditKey)` | `i128` | ⚠️ No explicit bump — relies on co-bump via credit-line entry | `draw_credit` (write), `reverse_draw` (read) |
| `DrawReversedAmount(DrawAuditKey)` | `i128` | ⚠️ No explicit bump — relies on co-bump via credit-line entry | `reverse_draw` |
| `MaxBorrowerExposure(Address)` | `i128` | ⚠️ No explicit bump | `set_max_borrower_exposure` |
| `BorrowAdminCooldownSeconds` variant used as persistent | See note | — | See note |
| `LastBorrowAdminActionTs(Address)` | `u64` | ✅ `bump_persistent_ttl` on write | Admin borrow-action entrypoints |
| `LastAccrualAdminActionTs(Address)` | `u64` | ✅ `bump_persistent_ttl` on write | Admin accrual-action entrypoints |
| `LiquidationGracePeriod(Address)` | `u64` (seconds) | ✅ `bump_persistent_ttl` on write | `set_per_borrower_liquidation_grace` |
| `BorrowerExposureCap(Address)` | `i128` | ⚠️ No explicit bump | `set_borrower_exposure_cap` |
| `AttestationBatch(Address)` | Attestation batch | ⚠️ No explicit bump | Attestation admin entrypoints |

> **TTL hygiene note — entries without explicit bumps**
>
> Variants marked ⚠️ rely on the borrower's `CreditLineData` entry (stored
> directly under the borrower `Address`) being bumped by `bump_credit_line_ttl`
> via `persist_credit_line`. For an **active** borrower this is sufficient
> because every `draw_credit` / `repay_credit` refreshes that entry.
>
> Entries written in isolation — e.g. `DrawAudit` written for a borrower whose
> credit line was subsequently closed, or `BlockedBorrower` set for a borrower
> who has never drawn — will age independently and may be archived if their TTL
> expires before the next `persist_credit_line` for that borrower. Callers
> needing these entries to survive beyond the ~6-month window without a draw
> should call `bump_persistent_ttl` explicitly on the relevant key.

---

## 3. Raw Address Key — `CreditLineData`

The `CreditLineData` struct is stored in **persistent** storage under the
borrower's `Address` directly — not under a `DataKey` variant. This was an
early design decision preserved for backward compatibility.

| Key | Value type | Tier | Bump function | Writer |
|-----|------------|------|---------------|--------|
| `borrower: Address` | `CreditLineData` | **Persistent** | `bump_credit_line_ttl` (called by `persist_credit_line`) | `persist_credit_line` — the sole write path; called by every entrypoint that mutates a credit line |

`persist_credit_line` also:
- Calls `ensure_credit_line_id` (writes `CreditLineIdByBorrower` /
  `CreditLineBorrowerById` on first call).
- Atomically adjusts `TotalUtilized` via `adjust_total_utilized`.
- Maintains `ActiveLineCount` on `Active ↔ non-Active` transitions.

---

## 4. Composite Key — Liquidation Settlement Markers

Settlement replay protection is stored in **persistent** storage under a
3-tuple key constructed in `lifecycle.rs`:

```rust
fn liquidation_settlement_key(
    borrower: &Address,
    settlement_id: &Symbol,
) -> (Symbol, Address, Symbol) {
    (symbol_short!("liq_seen"), borrower.clone(), settlement_id.clone())
}
```

| Key | Value type | Tier | TTL | Writer |
|-----|------------|------|-----|--------|
| `(Symbol("liq_seen"), borrower, settlement_id)` | `bool` | **Persistent** | Bumped to the credit-line window (~6 months) by `bump_settlement_marker_ttl` on write and on the replay-check read, so replay protection never lapses via archival | `settle_default_liquidation` |

The marker is checked via `env.storage().persistent().has(&settlement_key)`
before any state mutation. A second call with the same `(borrower, settlement_id)`
reverts with `ContractError::AlreadyInitialized (14)`.

---

## 5. Oracle Module — `OracleDataKey` Enum

`OracleDataKey` is defined in `contracts/credit/src/oracles.rs`. All five
variants live in **instance** storage and share the contract instance TTL.

| `OracleDataKey` variant | Value type | Tier | Writer entrypoints |
|-------------------------|------------|------|--------------------|
| `OracleList` | `Vec<Address>` | **Instance** | `add_oracle`, `remove_oracle` |
| `OracleWeight(Address)` | `u32` | **Instance** | `add_oracle`, `remove_oracle` |
| `OracleReport(Address)` | `OracleReportData` | **Instance** | `report_value` |
| `QuorumThreshold` | `u32` | **Instance** | `set_quorum_threshold` |
| `ReportingWindow` | `u64` (seconds) | **Instance** | `set_reporting_window` |

`OracleReportData` carries `{ value: u128, timestamp: u64 }`. Freshness is
evaluated at read time in `get_median_value` by comparing
`now - report.timestamp <= ReportingWindow`. Stale reports are silently excluded
from the quorum calculation.

---

## 6. Auction Contract — `DataKey` and `AuctionKey`

The auction contract (`gateway-contract/contracts/auction_contract/src/storage.rs`)
uses a separate, shorter TTL policy:

```
PERSISTENT_BUMP_AMOUNT        = 518_400 ledgers  // ~30 days
PERSISTENT_LIFETIME_THRESHOLD = 120_960 ledgers  // ~7 days
```

| Key | Value type | Tier | Notes |
|-----|------------|------|-------|
| `DataKey::Status` | `AuctionStatus` | **Instance** | Current auction state machine status |
| `DataKey::HighestBidder` | `Address` | **Instance** | Address of the leading bidder |
| `DataKey::HighestBid` | `i128` | **Instance** | Current highest bid amount |
| `DataKey::EndTime` | `u64` | **Instance** | Auction close timestamp |
| `DataKey::FactoryContract` | `Address` | **Instance** | Parent factory / credit contract address |
| `DataKey::LiquidationGraceWindow` | `u64` | **Instance** | Grace period before bidding opens |
| `AuctionKey::Closed(Symbol)` | `bool` | **Persistent** | Per-auction-id closed marker |
| `AuctionKey::LiquidationSettled(Symbol)` | `bool` | **Persistent** | Per-auction-id one-shot settlement replay guard |

The reentrancy guard (`Symbol("reentrancy")`) is stored in instance storage and
is functionally transient: it is set on entry to `place_bid` and cleared on
exit within the same transaction.

---

## 7. Decision Rules

1. **Instance Storage** — use for global configuration, protocol-wide
   counters, circuit-breaker flags, and oracle state. The total size should
   stay small (< 1 KB) to avoid elevated invocation fees. Every instance key
   is implicitly bumped alongside any persistent key bump.

2. **Persistent Storage** — use for per-borrower state, per-key audit trails,
   and any data that is unbounded in count. Always call `bump_persistent_ttl`
   (or one of its wrappers) on every read and write path.

3. **Never use Temporary Storage** for balances, loan state, or any value that
   must survive a network restore. Temporary entries auto-expire and cannot be
   restored without an archive proof.

---

## 8. Security Considerations

- **Persistent keys** that are not bumped will be evicted by the network. An
  archived credit line can be restored via an archive proof, but this adds
  operational friction and cost. Always bump on read for user-facing data.
- **Instance storage** is shared across all instance keys; keep it small to
  avoid high invocation fees.
- Access control must be enforced **before** any storage write — never write
  first and validate later.
- The `TotalUtilized` and `TotalCollateral` accumulators are conservation
  invariants. `persist_credit_line` and `set_collateral_balance` are the
  **only** approved write paths; direct `persistent().set` on the borrower
  address or collateral key bypasses the accumulator update and will corrupt
  global state.
- Settlement markers under `(Symbol("liq_seen"), borrower, settlement_id)` are
  the credit-side half of the two-sided replay barrier. The matching barrier on
  the auction side is `AuctionKey::LiquidationSettled(auction_id)`. Both must
  pass for a settlement to proceed.

---

## 9. Maintenance

When adding a new `DataKey` variant or Symbol key:

1. Add a row to the appropriate table above.
2. Document the tier, value type, TTL bump function, and writer entrypoints.
3. If the entry is persistent and not co-bumped by `persist_credit_line`,
   add an explicit `bump_persistent_ttl` call to every read and write helper.
4. Update `docs/PROTOCOL_SPEC.md` §3 if the key is exposed via a new
   entrypoint.

---

## References

- [Soroban State Archival](https://developers.stellar.org/docs/learn/smart-contract-internals/state-archival)
- [TTL and Ledger Entry Lifetime](https://developers.stellar.org/docs/learn/smart-contract-internals/state-archival#time-to-live-ttl)
- Issue [#1306 — Rewrite storage layout docs from DataKey](../../issues/1306)
- `contracts/credit/src/storage.rs` — `DataKey` enum and all storage helpers
- `contracts/credit/src/oracles.rs` — `OracleDataKey` enum
- `contracts/credit/src/lifecycle.rs` — `liquidation_settlement_key`
- `docs/PROTOCOL_SPEC.md` §3 — per-entrypoint storage key cross-reference
- `docs/state-machine.md` — authoritative credit line status transition table
