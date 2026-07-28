# fix: borrow TTL bump — per-borrower persistent storage key TTL hygiene

## Overview

Closes #824

This PR ensures every per-borrower persistent storage entry accessed during borrow operations (`draw_credit`, `repay_credit`) has its Time-To-Live (TTL) automatically extended on read and write paths. Without these bumps, per-borrower keys such as `LastDrawTs`, `UtilizationCapBps`, `MaxBorrowerExposure`, and `FrozenBorrower` can be **silently archived** by the Soroban network independently of the credit-line entry, causing borrow-specific state to appear as absent / default values.

### Problem

The credit contract stores per-borrower auxiliary state in persistent storage under separate `DataKey` variants:

| Key | Purpose | Read during |
|---|---|---|
| `LastDrawTs(Address)` | Draw cooldown enforcement | Every `draw_credit` |
| `UtilizationCapBps(Address)` | Per-borrower utilization cap | Every `draw_credit` |
| `MaxBorrowerExposure(Address)` | Hard cap on concentration risk | Every `draw_credit` |
| `FrozenBorrower(Address)` | Temporary draw freeze | Every `draw_credit` |
| `BlockedBorrower(Address)` | Admin blocklist | Views / enumeration |
| `RateFloorBps(Address)` | Minimum interest rate | Risk updates |
| `RateCeilingBps(Address)` | Maximum interest rate | Risk updates |
| `BorrowerExposureCap(Address)` | Admin-set per-borrower cap | Config reads |
| `CreditLineIdByBorrower(Address)` | Stable enumeration ID | Enumeration |
| `CreditLineBorrowerById(u32)` | Reverse enumeration lookup | Enumeration |

While `CreditLineData` (the main credit-line entry) and `CollateralBalance` already had TTL bumps on every read/write path, the auxiliary keys above were being read without bumping, meaning an active borrower's draw-cooldown timestamp, utilization cap, exposure cap, and freeze state could evaporate while the credit line itself remained live.

### Solution

Added `bump_persistent_ttl(env, &key)` calls (extending TTL to `LEDGER_BUMP_AMOUNT ≈ 6 months` when remaining TTL drops below `LEDGER_BUMP_THRESHOLD ≈ 3 months`) to **every per-borrower persistent getter and setter** that was missing one. This follows the exact same pattern already used by:

- `get_collateral_balance` / `set_collateral_balance`
- `get_repayment_schedule` / `set_repayment_schedule`
- `get_per_borrower_liquidation_grace` / `set_per_borrower_liquidation_grace`

## Changes

### `contracts/credit/src/storage.rs` (+175 / −46)

#### Getters — now bump TTL on read (10 functions)

Each getter now checks `has()` before bumping (so absent keys don't trigger an unnecessary `extend_ttl` write), then calls `bump_persistent_ttl(env, &key)` before `get()`:

| Getter | DataKey bumped |
|---|---|
| `get_last_draw_ts` | `LastDrawTs(borrower)` |
| `get_utilization_cap_bps` | `UtilizationCapBps(borrower)` |
| `get_max_borrower_exposure` | `MaxBorrowerExposure(borrower)` |
| `is_borrower_frozen` | `FrozenBorrower(borrower)` |
| `is_borrower_blocked` | `BlockedBorrower(borrower)` |
| `get_borrower_rate_floor` | `RateFloorBps(borrower)` |
| `get_borrower_rate_ceiling` | `RateCeilingBps(borrower)` |
| `get_borrower_exposure_cap` | `BorrowerExposureCap(borrower)` |
| `get_credit_line_id` | `CreditLineIdByBorrower(borrower)` |
| `get_borrower_by_credit_line_id` | `CreditLineBorrowerById(id)` |

Example — before:

```rust
pub fn get_last_draw_ts(env: &Env, borrower: &Address) -> Option<u64> {
    env.storage()
        .persistent()
        .get(&DataKey::LastDrawTs(borrower.clone()))
}
```

After:

```rust
pub fn get_last_draw_ts(env: &Env, borrower: &Address) -> Option<u64> {
    let key = DataKey::LastDrawTs(borrower.clone());
    if env.storage().persistent().has(&key) {
        bump_persistent_ttl(env, &key);
    }
    env.storage().persistent().get(&key)
}
```

#### Setters — now bump TTL on write (7 functions)

Each setter now calls `bump_persistent_ttl(env, &key)` after `set()` to extend the entry's TTL on every write. The `key` variable is extracted before the set/remove branch to avoid unnecessary `borrower.clone()`:

| Setter | DataKey bumped |
|---|---|
| `set_last_draw_ts` | `LastDrawTs(borrower)` |
| `set_utilization_cap_bps` | `UtilizationCapBps(borrower)` |
| `set_max_borrower_exposure` | `MaxBorrowerExposure(borrower)` |
| `set_borrower_blocked` | `BlockedBorrower(borrower)` |
| `set_borrower_exposure_cap` | `BorrowerExposureCap(borrower)` |
| `set_borrower_rate_floor` | `RateFloorBps(borrower)` |
| `set_borrower_rate_ceiling` | `RateCeilingBps(borrower)` |

### `contracts/credit/tests/storage_ttl.rs` (+293 / −0)

Added a new `advance_past_ttl_threshold` helper and **11 focused regression tests** covering read-path and write-path TTL bumps:

#### Read-path bump tests (7 tests)

| Test | Key exercised |
|---|---|
| `get_max_borrower_exposure_bumps_persistent_ttl_on_read` | `MaxBorrowerExposure` |
| `get_borrower_rate_floor_bumps_persistent_ttl_on_read` | `RateFloorBps` |
| `get_borrower_rate_ceiling_bumps_persistent_ttl_on_read` | `RateCeilingBps` |
| `is_borrower_blocked_bumps_persistent_ttl_on_read` | `BlockedBorrower` |
| `is_borrower_frozen_bumps_persistent_ttl_on_read` | `FrozenBorrower` |
| `get_credit_line_id_bumps_persistent_ttl_on_read` | `CreditLineIdByBorrower` |
| `get_borrower_exposure_cap_bumps_persistent_ttl_on_read` | `BorrowerExposureCap` |

#### Write-path bump tests (4 tests)

| Test | Key exercised |
|---|---|
| `set_max_borrower_exposure_bumps_persistent_ttl_on_write` | `MaxBorrowerExposure` |
| `set_borrower_rate_floor_bumps_persistent_ttl_on_write` | `RateFloorBps` |
| `set_borrower_rate_ceiling_bumps_persistent_ttl_on_write` | `RateCeilingBps` |
| `set_borrower_blocked_bumps_persistent_ttl_on_write` | `BlockedBorrower` |

Each test:
1. Creates a credit line for a borrower
2. Writes the per-borrower key (via admin setter or direct storage)
3. Advances the ledger to drop the remaining TTL just below `LEDGER_BUMP_THRESHOLD`
4. Calls the read/set path
5. Asserts `remaining TTL >= LEDGER_BUMP_AMOUNT`

The existing test `utilization_cap_and_last_draw_keys_bump_persistent_ttl` was already present and now passes with the new bumps (it previously tested behavior that wasn't yet implemented).

## Security & Design Notes

### TTL policy

All bumps use `LEDGER_BUMP_THRESHOLD = 1_555_200` (~3 months at 5 s/ledger) and `LEDGER_BUMP_AMOUNT = 3_110_400` (~6 months), the same 2:1 extend-to:threshold ratio used by credit-line and collateral entries. `extend_ttl(key, threshold, extend_to)` only writes a ledger entry when the remaining TTL is below the threshold, so adding these calls to hot paths (e.g., `draw_credit`) adds negligible overhead — the bump write fires at most once per ~3 months per key.

### `has()` gating on read

All getter bumps are gated on `env.storage().persistent().has(&key)` before calling `bump_persistent_ttl`. This avoids an unnecessary `extend_ttl` write when the key doesn't exist (a read for a non-existent key would otherwise write a ledger entry with 0 TTL just to be told "nothing to extend"). This pattern is identical to `get_collateral_balance` and `get_repayment_schedule`.

### Bump after `remove()` in setters

Setters with a remove branch (e.g., `set_borrower_blocked(env, borrower, false)` removes the key) still call `bump_persistent_ttl(env, &key)` unconditionally after the if/else. `extend_ttl` on a deleted key is a harmless no-op, and the `bump_instance_ttl()` call within `bump_persistent_ttl` extends instance storage TTL, which is always beneficial for contract health.

### No new entrypoints or API surface changes

This PR is a purely internal storage-hygiene change. No entrypoints are added, removed, or modified. No event schemas change. No types change. The changes are confined to the `storage` module and its tests.

## Test Coverage

- **11 new tests** added for read-path and write-path TTL bumps
- **2 existing tests** (`utilization_cap_and_last_draw_keys_bump_persistent_ttl`, `set_repayment_schedule_bumps_schedule_and_credit_line_ttl`) now correctly exercise the new bump paths
- `require_auth` is enforced on all state-changing entrypoints (no changes to auth logic)
- No `unwrap()` in production paths — all getters use `unwrap_or()` or pattern-match `has()` before access

## Suggested Review Order

1. `contracts/credit/src/storage.rs` — scan the getter and setter modifications (each is a small, self-similar change)
2. `contracts/credit/tests/storage_ttl.rs` — verify the new tests cover the intent
3. Run `cargo test -p creditra-credit --test storage_ttl` to validate

## Example commit message

```
fix: borrow TTL bump

Add bump_persistent_ttl calls to all per-borrower persistent storage
getters and setters so that borrow hot-keys (LastDrawTs,
UtilizationCapBps, MaxBorrowerExposure, FrozenBorrower, etc.) are not
silently archived by the network independently of the credit line.

- 10 getters now bump TTL on read (has-gated)
- 7 setters now bump TTL on write
- 11 new regression tests in storage_ttl.rs

Closes #824
```
