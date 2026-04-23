# feat(credit): optional per-borrower draw cooldown

## Summary

Introduces an optional, admin-configurable minimum interval between successive draws per borrower. When enabled, a borrower must wait at least `min_interval_seconds` after their last draw before drawing again. Disabled by default (zero). Repayments are never blocked.

---

## What Changed

### New types (`contracts/credit/src/types.rs`)

- `DrawCooldownConfig` — stores `min_interval_seconds: u64`
- `ContractError::DrawCooldown = 14` — returned when a draw is attempted within the cooldown window

### New storage key (`lib.rs`)

- `draw_cooldown_key` — instance storage key `"draw_cool"` for the global cooldown config

### New contract functions (`lib.rs`)

`set_draw_cooldown(env, min_interval_seconds)` — admin-only. Sets the global minimum interval between draws. Zero disables the feature (default).

`get_draw_cooldown(env)` — view function. Returns `Option<DrawCooldownConfig>`, `None` if not configured.

### `CreditLineData` — new field

`last_draw_ts: u64` — ledger timestamp of the last successful draw. Zero on open (no draw yet). Updated atomically with `utilized_amount` on every successful draw.

### `draw_credit` enforcement

After all status checks (auth, closed, suspended, defaulted), before the token transfer:

```
if cooldown configured && min_interval > 0 && last_draw_ts > 0 {
    elapsed = now - last_draw_ts
    if elapsed < min_interval → panic DrawCooldown
}
```

The first draw (last_draw_ts == 0) is always allowed regardless of cooldown setting.

---

## State machine / interaction diagram

```
Admin: set_draw_cooldown(3600)   ← sets global config
Borrower: draw_credit(100)       ← succeeds, last_draw_ts = T
Borrower: draw_credit(100)       ← T+3599: REVERTS (DrawCooldown)
Borrower: draw_credit(100)       ← T+3600: succeeds
Borrower: repay_credit(50)       ← always succeeds, cooldown irrelevant
Admin: set_draw_cooldown(0)      ← disables cooldown globally
Borrower: draw_credit(100)       ← succeeds immediately
```

---

## Tests (`mod test_draw_cooldown`)

12 tests with ledger time advancing:

| Test | What it covers |
|---|---|
| `no_cooldown_successive_draws_succeed` | Default (no config): back-to-back draws succeed |
| `zero_cooldown_successive_draws_succeed` | Explicit zero: back-to-back draws succeed |
| `draw_within_cooldown_reverts` | Draw before interval elapses → `DrawCooldown` error |
| `draw_at_exact_cooldown_boundary_succeeds` | Draw at exactly `min_interval_seconds` → succeeds |
| `draw_after_cooldown_succeeds` | Draw one second past boundary → succeeds |
| `first_draw_always_succeeds_regardless_of_cooldown` | `last_draw_ts == 0` bypasses cooldown |
| `last_draw_ts_updated_after_draw` | Field updated to ledger timestamp on each draw |
| `cooldown_does_not_block_repayments` | Repayments succeed within cooldown window |
| `admin_can_update_cooldown` | Admin can change the interval; `get_draw_cooldown` reflects it |
| `non_admin_cannot_set_cooldown` | Non-admin call panics |
| `get_draw_cooldown_returns_none_when_not_set` | Returns `None` before first configuration |
| `cooldown_is_independent_per_borrower` | Each borrower's `last_draw_ts` is tracked independently |

---

## Security Notes

**Trust model:**
- Only the admin can configure the cooldown. A compromised admin could set it to `u64::MAX`, effectively freezing all draws. The admin key should be a multisig or governance contract.
- Setting cooldown to zero re-enables draws immediately for all borrowers.

**Trust boundaries:**
- The cooldown is a global setting applied per-borrower based on their individual `last_draw_ts`. It does not prevent a borrower from drawing from multiple credit lines (if they had more than one).
- The first draw is always allowed (last_draw_ts == 0) to avoid blocking newly opened lines.

**Failure modes:**
- Draw within cooldown → `Error(Contract, #14)` (DrawCooldown)
- Admin sets cooldown to very large value → all borrowers with a prior draw are blocked until that interval elapses
- Repayments are never affected by the cooldown

**Operational use:**
- Set a short cooldown (e.g. 60s) during an incident to slow a rapid-drain attack while the team investigates
- Disable (set to 0) once the incident is resolved

---

## Test Results

```
test result: ok. 78 passed; 0 failed  (lib — includes 12 new cooldown tests)
test result: ok. 28 passed; 0 failed  (integration)
test result: ok. 3 passed;  0 failed  (spdx_header_bug_exploration)
test result: ok. 6 passed;  0 failed  (spdx_preservation)
test result: ok. 7 passed;  0 failed  (duplicate_open_policy)
```

---

Closes issue #255
