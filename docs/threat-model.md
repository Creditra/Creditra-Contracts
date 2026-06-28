# Threat Model — Authorization Matrix

**Crate:** `creditra-credit`  
**Source:** `contracts/credit/src/lib.rs`, `contracts/credit/src/lifecycle.rs`

---

## Auth roles

| Role | How it is established |
|---|---|
| **Admin** | Address stored in instance storage under `DataKey::Admin` during `init`. Rotated via `propose_admin` + `accept_admin` with a time-lock. |
| **Borrower** | The address that owns a credit line. Must sign their own draw, repay, and self-suspend calls. |
| **Proposed admin** | Temporary role set by `propose_admin`; must call `accept_admin` within the time-lock window. |
| **Closer** | Passed explicitly to `close_credit_line`; must be either the admin or the borrower. |

---

## Function authorization matrix

| Function | Auth required | Auth call | Notes |
|---|---|---|---|
| `init` | None | — | One-shot; re-calling is a no-op after admin is set. |
| `propose_admin` | Admin | `require_admin_auth` | Writes proposed admin + accept-after timestamp. |
| `accept_admin` | Proposed admin | `proposed_admin.require_auth()` | Enforces time-lock before storage write. |
| `open_credit_line` | Admin | `require_admin_auth` | Auth checked before any storage mutation. |
| `set_liquidity_token` | Admin | `require_admin_auth` | Also checks `assert_not_paused`. |
| `set_liquidity_source` | Admin | `require_admin_auth` | Also checks `assert_not_paused`. |
| `set_max_draw_amount` | Admin | `require_admin_auth` | Also checks `assert_not_paused`. |
| `set_max_repay_amount` | Admin | `require_admin_auth` | Also checks `assert_not_paused`. |
| `set_draw_min_interval` | Admin | `require_admin_auth` | Also checks `assert_not_paused`. |
| `set_utilization_cap` | Admin | `require_admin_auth` | Auth is first call in function body. |
| `set_rate_change_limits` | Admin | `require_admin_auth` | Delegated to `risk::set_rate_change_limits`. |
| `set_rate_formula_config` | Admin | `require_admin_auth` | Delegated to `risk`. |
| `clear_rate_formula_config` | Admin | `require_admin_auth` | Auth before storage remove. |
| `set_grace_period_config` | Admin | `require_admin_auth` | Auth before validation and write. |
| `set_protocol_paused` | Admin | `require_admin_auth` | Circuit-breaker control. |
| `freeze_draws` | Admin | `require_admin_auth` | Emergency draw freeze with [`FreezeReason`]. |
| `unfreeze_draws` | Admin | `require_admin_auth` | Lifts emergency draw freeze. |
| `freeze_credit_line` | Admin | `require_admin_auth` | Per-line draw freeze with [`FreezeReason`]. |
| `unfreeze_credit_line` | Admin | `require_admin_auth` | Lifts per-line draw freeze. |
| `suspend_credit_line` | Admin | `require_admin_auth` | Auth before state read. |
| `self_suspend_credit_line` | Borrower | `borrower.require_auth()` | No admin path; borrower-only. |
| `default_credit_line` | Admin | `require_admin_auth` | Auth before state read. |
| `reinstate_credit_line` | Admin | `require_admin_auth` | Auth before target validation and state read. |
| `forgive_debt` | Admin | `require_admin_auth` | Also checks `assert_not_paused`. |
| `settle_default_liquidation` | Admin | `require_admin_auth` | Auth is first call in function body. |
| `close_credit_line` | Closer | `closer.require_auth()` | Closer must be admin or borrower (enforced by business logic). |
| `block_borrower` | Admin | `admin.require_auth()` + `require_admin_auth` | Double check: explicit param auth + role check. |
| `unblock_borrower` | Admin | `admin.require_auth()` + `require_admin_auth` | Same double check as `block_borrower`. |
| `bulk_block_borrowers` | Admin | `admin.require_auth()` + `require_admin_auth` | Same double check; batch capped at 50. |
| `draw_credit` | Borrower | `borrower.require_auth()` | Auth after reentrancy guard, before any state read. |
| `repay_credit` | Borrower | `borrower.require_auth()` | Auth after reentrancy guard, before any state read. |
| `get_credit_line` | None | — | Pure storage read; no side effects. |
| `get_liquidity_source` | None | — | Pure storage read. |
| `get_rate_change_limits` | None | — | Pure storage read. |
| `get_utilization_cap` | None | — | Pure storage read. |
| `get_grace_period_config` | None | — | Pure storage read. |
| `get_max_draw_amount` | None | — | Pure storage read. |
| `get_max_repay_amount` | None | — | Pure storage read. |
| `get_draw_min_interval` | None | — | Pure storage read. |
| `get_schema_version` | None | — | Pure storage read. |
| `get_total_utilized` | None | — | Pure storage read. |
| `get_credit_line_count` | None | — | Pure storage read. |
| `enumerate_credit_lines` | None | — | Pure storage read; capped iteration. |
| `get_rate_formula_config` | None | — | Pure storage read. |
| `get_protocol_config` | None | — | Aggregated read; no side effects. |
| `is_draws_frozen` | None | — | Pure storage read. |
| `is_borrower_blocked` | None | — | Pure storage read. |

---

## Auth-before-mutation guarantee

Every mutating function calls its auth check as the first or second statement
(after `assert_not_paused` and/or the reentrancy guard where applicable).
No storage write or state change occurs before the auth check returns.

Key ordering for admin mutators:
```
assert_not_paused  (optional, where relevant)
require_admin_auth ← auth check
<validation>
<storage write>
```

Key ordering for borrower mutators (`draw_credit`, `repay_credit`):
```
set_reentrancy_guard
borrower.require_auth() ← auth check
<validation>
<storage write>
clear_reentrancy_guard
```

---

## Test coverage

Every privileged entrypoint has a corresponding negative test in
`contracts/credit/tests/unauthorized_matrix.rs`. Each test confirms that
calling the function without valid authorization panics (reverts).

| Test | Entrypoint covered |
|---|---|
| `set_liquidity_token_unauthorized` | `set_liquidity_token` |
| `set_liquidity_source_unauthorized` | `set_liquidity_source` |
| `set_max_draw_amount_unauthorized` | `set_max_draw_amount` |
| `set_max_repay_amount_unauthorized` | `set_max_repay_amount` |
| `set_draw_min_interval_unauthorized` | `set_draw_min_interval` |
| `freeze_draws_unauthorized` | `freeze_draws` |
| `unfreeze_draws_unauthorized` | `unfreeze_draws` |
| `propose_admin_unauthorized` | `propose_admin` |
| `accept_admin_wrong_signer` | `accept_admin` |
| `open_credit_line_unauthorized` | `open_credit_line` |
| `set_utilization_cap_unauthorized` | `set_utilization_cap` |
| `suspend_credit_line_unauthorized` | `suspend_credit_line` |
| `default_credit_line_unauthorized` | `default_credit_line` |
| `reinstate_credit_line_unauthorized` | `reinstate_credit_line` |
| `forgive_debt_unauthorized` | `forgive_debt` |
| `settle_default_liquidation_unauthorized` | `settle_default_liquidation` |
| `close_credit_line_stranger_unauthorized` | `close_credit_line` |
| `block_borrower_unauthorized` | `block_borrower` |
| `unblock_borrower_unauthorized` | `unblock_borrower` |
| `bulk_block_borrowers_unauthorized` | `bulk_block_borrowers` |
| `update_risk_parameters_unauthorized` | `update_risk_parameters` |
| `set_rate_change_limits_unauthorized` | `set_rate_change_limits` |
| `set_rate_formula_config_unauthorized` | `set_rate_formula_config` |
| `clear_rate_formula_config_unauthorized` | `clear_rate_formula_config` |
| `set_grace_period_config_unauthorized` | `set_grace_period_config` |
| `set_protocol_paused_unauthorized` | `set_protocol_paused` |
| `draw_credit_wrong_signer` | `draw_credit` |
| `repay_credit_wrong_signer` | `repay_credit` |
| `self_suspend_wrong_signer` | `self_suspend_credit_line` |
| `suspend_credit_line_non_admin_mock_auth` | `suspend_credit_line` (mock non-admin) |
| `default_credit_line_non_admin_mock_auth` | `default_credit_line` (mock non-admin) |
| `freeze_draws_non_admin_mock_auth` | `freeze_draws` (mock non-admin) |
| `update_risk_parameters_non_admin_mock_auth` | `update_risk_parameters` (mock non-admin) |
| `set_protocol_paused_non_admin_mock_auth` | `set_protocol_paused` (mock non-admin) |


---

## Soroban-Specific Reentrancy via `__check_auth` Callbacks

### Background

Traditional reentrancy exploits reenter a contract during an external token
transfer (the classic EVM pattern). Soroban introduces a second, less obvious
vector: the **`__check_auth` callback**.

When a contract calls `address.require_auth()`, the Soroban host invokes
`__check_auth` on the authorising account/contract. If the authorising
address is itself a smart contract (a "custom account"), that contract's
`__check_auth` implementation runs **inside the same transaction**, with the
ability to invoke any other contract — including the one that just called
`require_auth()`.

This means an attacker can deploy a malicious custom-account contract whose
`__check_auth` re-enters `place_bid` or `claim_auction` *before the outer
call has finished mutating state*.

---

### Attack Scenario — `place_bid` via `__check_auth`

**Pre-conditions**

- Auction is open with one existing bid from honest bidder `H`.
- Attacker controls a custom-account contract `M` whose `__check_auth`
  re-enters the auction contract.

**Step-by-step**

```
Attacker transaction
│
├─ 1. call place_bid(auction_id, amount=X)   ← outer call begins
│       bidder = M (malicious custom account)
│
│   Auction contract execution
│   ├─ set_reentrancy_guard()                ← GUARD SET (flag = true)
│   ├─ bidder.require_auth()                 ← triggers M.__check_auth
│   │
│   │   M.__check_auth() execution           ← REENTRANT CALL
│   │   └─ call place_bid(auction_id,        ← re-enters before outer
│   │            amount=X+1)                    call completes
│   │       Auction contract (inner)
│   │       ├─ set_reentrancy_guard()
│   │       │       current flag == true
│   │       │       → panic! AuctionError::Reentrancy   ✓ BLOCKED
│   │       └─ (inner call reverts)
│   │
│   ├─ <validation continues normally>
│   ├─ refund previous bidder H
│   ├─ record M as highest bidder
│   └─ clear_reentrancy_guard()              ← GUARD CLEARED (flag = false)
│
└─ outer call succeeds normally
```

Without the guard, the inner `place_bid` would run against **stale state**
(old highest bidder, old highest bid) and could manipulate the auction outcome
or drain funds via double-refund.

---

### Attack Scenario — `claim_auction` via `__check_auth`

```
Attacker transaction
│
├─ 1. call claim_auction(auction_id)         ← outer call begins
│       winner = M (malicious custom account)
│
│   Auction contract execution
│   ├─ set_reentrancy_guard()                ← GUARD SET
│   ├─ winner.require_auth()                 ← triggers M.__check_auth
│   │
│   │   M.__check_auth() execution
│   │   └─ call claim_auction(auction_id)    ← re-enters before
│   │       Auction contract (inner)            settlement flag is set
│   │       ├─ set_reentrancy_guard()
│   │       │       current flag == true
│   │       │       → panic! AuctionError::Reentrancy   ✓ BLOCKED
│   │       └─ (inner call reverts)
│   │
│   ├─ mark auction as claimed
│   │       AuctionKey::Claimed(id) = true
│   ├─ transfer asset to winner
│   └─ clear_reentrancy_guard()              ← GUARD CLEARED
│
└─ outer call succeeds; double-claim prevented
```

A successful double-`claim_auction` would let the attacker receive the
auctioned asset twice while paying only once.

---

### Mitigation — `set_reentrancy_guard` / `clear_reentrancy_guard`

**Location:**
`gateway-contract/contracts/auction_contract/src/storage.rs`
— functions `set_reentrancy_guard` and `clear_reentrancy_guard`.

**Mechanism**

| Step | What happens |
|---|---|
| Function entry | `set_reentrancy_guard(env)` reads the instance-storage key `Symbol("reentrancy")`. If already `true`, panics with `AuctionError::Reentrancy`. Otherwise writes `true`. |
| `require_auth()` call | Any `__check_auth` callback that tries to re-enter sees `flag == true` and is rejected immediately. |
| Function exit (success **or** panic) | `clear_reentrancy_guard(env)` writes `false`. Soroban's transactional execution means a panic rolls back all storage writes including the guard, so the flag is always consistent after the transaction settles. |

**Storage layout**

```
Instance storage
└─ key:   Symbol("reentrancy")   // defined in reentrancy_key()
   value: bool
           false  →  no call in progress (safe to enter)
           true   →  call in progress    (reject re-entry)
```

**CEI ordering enforced by the guard**

```
// place_bid / claim_auction call ordering
set_reentrancy_guard(env)          // Check  — reject if already locked
caller.require_auth()              // Effect — auth (may trigger __check_auth)
<read and validate state>          // Check  — business logic
<mutate state>                     // Effect — storage writes
<external token transfer>          // Interact — CPI to token contract
clear_reentrancy_guard(env)        // Release — unlock for next call
```

The guard enforces **CEI (Check-Effect-Interact)** ordering even when the
Soroban host's `__check_auth` mechanism tries to insert an interaction
between the Check and Effect phases.

---

### Why Instance Storage for the Guard

Instance storage lives in a single ledger entry and is loaded atomically at
the start of each contract invocation. Using it for the guard means:

- No extra persistent-storage round-trips.
- The flag is scoped to this contract instance — a different auction contract
  deployment has its own flag.
- Soroban rolls back instance storage on panic, so a failed inner call cannot
  leave the guard permanently set.

---

### Residual Risk and Mitigations

| Residual risk | Status |
|---|---|
| Guard not cleared on panic path | Mitigated — Soroban rolls back all storage on `panic_with_error`, including the `true` write. |
| Guard set but `require_auth` never called | Not exploitable — the flag just gets cleared at the end of the same call. |
| Multiple concurrent callers (parallel transactions) | Not applicable — each Soroban transaction executes serially against a snapshot; instance storage is per-invocation. |
| `__check_auth` calls a *different* entrypoint not guarded | Out of scope for this guard. All state-mutating entrypoints that perform token transfers (`place_bid`, `claim_auction`) are individually guarded. |

---

### Related Functions Protected by the Guard

| Entrypoint | File | Guard applied |
|---|---|---|
| `place_bid` | `gateway-contract/contracts/auction_contract/src/lib.rs` | `set_reentrancy_guard` / `clear_reentrancy_guard` |
| `claim_auction` | `gateway-contract/contracts/auction_contract/src/lib.rs` | `set_reentrancy_guard` / `clear_reentrancy_guard` |
| `draw_credit` | `contracts/credit/src/lib.rs` | Mirrors the same guard pattern |
| `repay_credit` | `contracts/credit/src/lib.rs` | Mirrors the same guard pattern |
