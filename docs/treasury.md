# Treasury, Bounty Pool & Fee Lifecycle (end to end)

Normative reference for **how protocol value accrues, how it is split, and how
it leaves the contract**. This is the page to read before proposing any change
to `ProtocolFeeBps`, `TreasuryFeeShareBps`, the late-fee schedule, or the
treasury withdrawal flow.

| | |
|---|---|
| Audience | Governance, auditors, integrators, operators |
| Status | Normative. Describes `contracts/credit` as implemented at this commit. |
| Source of truth | `contracts/credit/src/fees.rs`, `contracts/credit/src/lib.rs`, `contracts/credit/src/lifecycle.rs`, `contracts/credit/src/math_utils.rs`, `contracts/credit/src/storage.rs` |
| Companion docs | [`PROTOCOL_SPEC.md`](./PROTOCOL_SPEC.md) §2.7, [`EVENT_CATALOG.md`](./EVENT_CATALOG.md), [`STORAGE_LAYOUT.md`](./STORAGE_LAYOUT.md), [`SECURITY.md`](./SECURITY.md) (T9) |

> **Terminology.** *Treasury* and *bounty pool* are two **internal
> accumulators** (two `i128` instance-storage counters), not two token
> accounts. Until a withdrawal is executed, all accrued fees sit in the credit
> contract's own token balance. The accumulators are the accounting that says
> how much of that balance each recipient is owed.

---

## 1. Where the money comes from

Three configuration keys influence how much value reaches the treasury and
bounty accumulators. Only **two** of them credit an accumulator directly.

| # | Accrual source | Trigger | Config key | Sink | Event |
|---|---|---|---|---|---|
| 1 | **Protocol fee on realized interest** | `repay_credit` | `ProtocolFeeBps` (× `TreasuryFeeShareBps` for the split) | `TreasuryBalance` **and** `BountyBalance` | `("credit","fee_accrd")` |
| 2 | **Flat late fee per overdue installment** | `repay_credit` → `lifecycle::advance_repayment_schedule_after_repay` | `LateFeeFlat` | `TreasuryBalance` **only** — bypasses the split | `("credit","late_fee")` |
| 3 | **Delinquency penalty surcharge** | any mutating entrypoint that runs `accrual::apply_accrual` | `PenaltySurchargeBps` | *indirect* — raises the interest rate, so the extra interest is taxed by source 1 | `("credit","pen_enter")` / `("credit","pen_exit")` |

### 1.1 Protocol fee on realized interest (the main path)

Executed inside `repay_credit`, after repayment has been allocated between
interest and principal by `lifecycle::allocate_repayment`:

```text
interest_repaid        = interest portion of this repayment
fee_bps                = ProtocolFeeBps (default 0 when unset)
fee                    = floor(interest_repaid × fee_bps / 10_000)     // math_utils::apply_bps, Rounding::Floor
transfer_from(borrower → contract, fee)                                 // fee stays in the contract; not sent to the reserve
fees::accrue_protocol_fee(env, borrower, fee)                           // ⇒ split, then credit accumulators
transfer_from(borrower → reserve, effective_repay − fee)
```

Properties that matter for governance:

- The fee is charged on **realized interest only**, never on principal. A
  principal-only repayment accrues zero fee (see
  `tests/protocol_fee_total_repayment.rs::fee_on_principal_only_repayment`).
- The fee is floored **once**, before the split, so a sub-unit fee
  (`interest_repaid × fee_bps < 10_000`) rounds to `0` and no event is emitted
  (`protocol_fee.rs::protocol_fee_rounding_floors_sub_bps_fee_to_zero`).
- `accrue_protocol_fee` is a no-op for `total_fee <= 0`, so a zero or negative
  fee cannot move the accumulators.
- The fee is collected by a **`transfer_from` of the borrower into the same
  contract**. `set_liquidity_source` does not affect it: the fee never reaches
  the reserve. The reserve receives `effective_repay − fee`.

### 1.2 Flat late fee per overdue installment

`lifecycle::advance_repayment_schedule_after_repay` runs at the end of
`repay_credit`, once per repayment that retires at least one full installment
of **principal**:

```text
principal_repaid    = effective_repay − interest_repaid
installments_paid   = floor(principal_repaid / schedule.amount_per_period)   // return early if 0
late_fee            = LateFeeFlat (default 0 = disabled)

for i in 0..installments_paid:
    due_ts = schedule.next_due_ts + i × schedule.period_seconds
    if now > due_ts:                          // strict; a payment exactly at due_ts is on time
        add_treasury_balance(late_fee)        // 100 % treasury — the split is NOT applied
        emit ("credit","late_fee") { borrower, fee: late_fee, installment_index: i + 1 }

schedule.next_due_ts += installments_paid × schedule.period_seconds
```

Governance-relevant consequences:

- **Late fees are not split.** They land entirely in `TreasuryBalance`
  regardless of `TreasuryFeeShareBps`. The bounty pool never receives late
  revenue.
- The late fee is **not transferred again** from the borrower: it was already
  collected as part of `effective_repay`. Crediting the accumulator is an
  accounting allocation of money the contract already holds.
- `add_treasury_balance` can therefore push the tracked `TreasuryBalance`
  above the movement implied by `repay_credit`'s own fee transfer. That is
  intentional but means `TreasuryBalance` is **not** simply "protocol fee skims
  minus withdrawals" — late fees must be added.
- The `installment_index` in the event is **1-based** (`i + 1`).

### 1.3 Delinquency penalty surcharge (indirect accrual)

`accrual::apply_accrual` raises the rate used to realize interest when the
borrower is delinquent:

```text
effective_rate_bps = is_delinquent && PenaltySurchargeBps > 0
    ? min(interest_rate_bps + PenaltySurchargeBps, 10_000)   // MAX_INTEREST_RATE_BPS
    : interest_rate_bps
```

The surcharge has **no accumulator of its own**. It increases accrued interest,
and that increase is subsequently taxed by the source-1 protocol fee when the
borrower repays — so surcharge revenue reaches treasury *and* bounty through the
normal split, on the normal `repay_credit` path.

The stored `line.interest_rate_bps` is **not** mutated by the surcharge; the
surcharge is re-derived on every accrual. `pen_enter` / `pen_exit` events
signal when the borrower's *effective* rate changed.

---

## 2. The two accumulators

| Storage key (`DataKey`) | Type | Tier | Meaning |
|---|---|---|---|
| `TreasuryBalance` | `i128` | Instance | Fees owed to `TreasuryAddress` |
| `BountyBalance` | `i128` | Instance | Fees owed to `BountyAddress` |
| `TreasuryAddress` | `Address` | Instance | Withdrawal recipient for the treasury |
| `BountyAddress` | `Address` | Instance | Withdrawal recipient for the bounty pool |
| `ProtocolFeeBps` | `u32` | Instance | Fee on realized interest (bps) |
| `TreasuryFeeShareBps` | `u32` | Instance | Treasury's share of that fee (bps) |
| `LateFeeFlat` | `i128` | Instance | Flat late fee per overdue installment |
| `PenaltySurchargeBps` | `u32` | Instance | Delinquency surcharge (bps) |
| `PendingTreasuryWithdrawal` | `TreasuryWithdrawalProposal` | Instance | In-flight 24 h proposal |

Both accumulators are cleared to `0` by their respective withdrawal paths
(`clear_treasury_balance`, `clear_bounty_balance`). Accumulation uses
`checked_add` and reverts with `Overflow` rather than wrapping.

---

## 3. Split math (normative)

### 3.1 `split_conserving` — the rounding rule

`fees::split_protocol_fee` delegates the actual apportionment to
`math_utils::split_conserving(total, &weights)`, which is the **authoritative
rounding rule** for fee splits. It is a largest-remainder (Hamilton)
apportionment:

```text
W          = Σ weights
q, r       = total / W, total % W              // integer division
for weight w_i:
    part_i      = q × w_i + (r × w_i) / W      // integer floor per bucket
    remainder_i = (r × w_i) % W

leftover   = total − Σ part_i                  // always 0 ≤ leftover < number of buckets
sort buckets by remainder DESCENDING, ties by ASCENDING index
give one extra unit to the first `leftover` buckets in that order
```

Guarantees:

- **Value-conserving:** `Σ parts == total`, always. No dust is created or lost.
  This holds by construction, including for dust inputs (`total == 1`) and for
  `u128`-magnitude totals.
- **Deterministic:** identical inputs always produce identical output. Ties in
  the fractional remainder are broken by **ascending bucket index**, and
  because the treasury bucket is always index `0`, a tie favours the treasury.
- **Overflow-safe:** `(total / W) × w_i ≤ total` and `(total % W) × w_i` stays
  well inside `u128` because each `w_i ≤ W`.
- **No allocated weight:** if every weight is `0` the whole amount collapses
  onto bucket 0 (still conserved). All-zero weights cannot occur for this
  caller, since `split_protocol_fee` short-circuits the `0` and `10_000`
  share cases before reaching it.
- **Asymmetric-safe:** `total ≤ 0` and non-divisible cases are handled before
  and inside the function; a zero or negative fee yields `(0, 0)`.

> This rule **replaced** the earlier "floor the treasury share, give the
> remainder to bounty" convention. The old convention was value-conserving but
> biased the rounding error toward the bounty pool for every fee-share
> configuration, and could hand the final base unit to the bucket with the
> *smaller* fractional claim. Any document, comment, or dashboard that still
> states "floor to treasury, remainder to bounty" is stale — see §3.3 for cases
> where the two rules disagree.

### 3.2 `split_protocol_fee` — special cases

```text
total_fee <= 0                 → (0, 0)                     // no-op; accumulators untouched
treasury_share_bps == 0        → (0, total_fee)             // all bounty
treasury_share_bps >= 10_000   → (total_fee, 0)             // all treasury
otherwise                      → split_conserving(total_fee, [share, 10_000 − share])
```

Note the `treasury_share_bps == 0` case is *not* "0 % of a split" only — it is
an explicit short-circuit, and likewise `10_000` short-circuits before
`split_conserving` is called.

### 3.3 Worked examples

Inputs are `(total_fee, treasury_share_bps)`. "Naive" is the retired
floor-to-treasury/remainder-to-bounty convention, shown to make the difference
concrete.

| `total_fee` | `TreasuryFeeShareBps` | **Largest-remainder (actual)** | Naive | Sum | Note |
|---|---|---|---|---|---|
| `100` | `10_000` (default) | treasury `100`, bounty `0` | same | `100` | Default config: 100 % treasury |
| `100` | `0` | treasury `0`, bounty `100` | same | `100` | Bounty-only config |
| `100` | `5_000` | treasury `50`, bounty `50` | same | `100` | Exact split |
| `101` | `5_000` | treasury `51`, bounty `50` | `50` / `51` | `101` | Tie on remainder → **ascending index wins → treasury** |
| `9` | `3_333` | treasury `3`, bounty `6` | `2` / `7` | `9` | Rounding error goes to the bucket with the larger fractional claim, not to bounty |
| `999` | `3_333` | treasury `333`, bounty `666` | `332` / `667` | `999` | Same divergence at scale |
| `1` | `5_000` | treasury `1`, bounty `0` | `0` / `1` | `1` | Dust is not lost; treasury wins the tie |
| `10` | `3_333` | treasury `3`, bounty `7` | same | `10` | Pinned by `tests/fee_split.rs` and `math_utils` unit tests |
| `0` or negative | any | treasury `0`, bounty `0` | same | `0` | Early return |

Step-by-step for the **`(9, 3_333)`** row — the case where the rules disagree:

```text
weights    = [3_333, 6_667]                W = 10_000
q, r       = 9 / 10_000 = 0,  9 % 10_000 = 9
bucket 0 (treasury): part = 0×3_333 + (9×3_333)/10_000 = 29_997/10_000 = 2   remainder = 29_997 % 10_000 = 9_997
bucket 1 (bounty)  : part = 0×6_667 + (9×6_667)/10_000 = 60_003/10_000 = 6   remainder = 60_003 % 10_000 = 3
allocated  = 8
leftover   = 9 − 8 = 1
order      = remainder DESC → [bucket 0 (9_997), bucket 1 (3)]
result     = treasury 3, bounty 6          Σ = 9 ✓
```

### 3.4 End-to-end fee example

`ProtocolFeeBps = 300`, `TreasuryFeeShareBps = 3_333`, borrower repays an
interest component of `300`:

```text
fee             = floor(300 × 300 / 10_000)              = 9
reserve_amount  = 300 − 9                                = 291
split(9, 3_333) = treasury 3, bounty 6                   (per §3.3)
```

Result: `TreasuryBalance += 3`, `BountyBalance += 6`, one
`("credit","fee_accrd")` event carrying all six fields, and `291` sent to the
liquidity reserve.

---

## 4. Configuration entrypoints

Every fee-related entrypoint, with its authority, bound, default, failure mode,
and whether it is frozen while a liquidation auction is active (§6).

| Entrypoint | Auth | Valid range | Effective default | On violation | Frozen by `AuctionActive`? |
|---|---|---|---|---|---|
| `set_protocol_fee_bps(bps)` | admin | `0..=1_000` (`MAX_PROTOCOL_FEE_BPS`) | `0` (unset) — fee disabled | `Overflow` | **Yes** |
| `get_protocol_fee_bps()` | none | — | `None` when unset | — | — |
| `set_treasury_fee_share_bps(bps)` | admin | `0..=10_000` (`MAX_FEE_SHARE_BPS`) | `10_000` when unset → 100 % treasury | `Overflow` | **Yes** |
| `get_treasury_fee_share_bps()` | none | — | `None` when unset | — | — |
| `set_treasury(admin, treasury)` | admin (arg auth **+** `require_admin_auth`) | any address | unset → withdrawals revert `TreasuryNotSet` | — | No |
| `get_treasury()` | none | — | `None` when unset | — | — |
| `set_bounty(admin, bounty)` | admin (arg auth **+** `require_admin_auth`) | any address | unset → withdrawals revert `BountyNotSet` | — | No |
| `get_bounty()` | none | — | `None` when unset | — | — |
| `withdraw_treasury(admin)` | admin | — | no-op when balance is `0` | `TreasuryNotSet`, `MissingLiquidityToken` | No |
| `withdraw_bounty(admin)` | admin | — | no-op when balance is `0` | `BountyNotSet`, `MissingLiquidityToken` | No |
| `propose_treasury_withdrawal(admin)` | admin (arg auth **+** `require_admin_auth`) | — | snapshots `TreasuryBalance` | `TreasuryNotSet`, `TreasuryProposalExists` | No |
| `execute_treasury_withdrawal(admin)` | admin (arg auth **+** `require_admin_auth`) | — | — | `NoPendingTreasuryWithdrawal`, `TreasuryTimelockActive`, `MissingLiquidityToken` | No |
| `get_pending_treasury_withdrawal()` | none | — | `None` when idle | — | — |
| `set_late_fee_flat(fee)` | admin | `i128` (`0` disables) | `0` (unset) | — | **Yes** |
| `get_late_fee_flat()` | none | — | `0` (unset) | — | — |
| `set_late_fee_config(config)` | admin | `Flat{amount ≥ 0}` else `InvalidAmount`; `AprBased{surcharge_bps ≤ 10_000}` else `RateTooHigh`; `None` clears | `None` (unset) | `InvalidAmount`, `RateTooHigh` | **Yes** |
| `get_late_fee_config()` | none | — | `None` (unset) | — | — |
| `set_penalty_surcharge_bps(bps)` | admin | `u32` (applied saturating, clamped at `10_000`) | `0` (unset) | — | **Yes** |
| `get_penalty_surcharge_bps()` | none | — | `0` (unset) | — | — |

### 4.1 Two distinct "unset" semantics — do not conflate them

- `get_treasury_fee_share_bps()` returns `Option<u32>` and is `None` when the
  key is absent, **but the split actually applies `10_000`**. Read the value
  through the internal `fees::get_treasury_fee_share_bps`, which substitutes
  `DEFAULT_TREASURY_FEE_SHARE_BPS = 10_000`, if you need the effective share.
  A dashboard that renders `None` as "0 % treasury" is wrong and will invert
  the meaning of the default.
- `get_protocol_fee_bps()` returning `None` is equivalent to `0`: no fee is
  charged. Only a non-zero configured value moves money.
- `get_late_fee_flat()` cannot distinguish "never configured" from "explicitly
  `0`". Both mean the same thing (disabled).

---

## 5. Treasury vs bounty pool: what each receives

| Revenue | Treasury | Bounty pool |
|---|---|---|
| Protocol fee on realized interest | `share` bps (largest-remainder) | `10_000 − share` bps (largest-remainder) |
| Flat late fee per overdue installment | **100 %** | **nothing** |
| Delinquency penalty surcharge | via the interest tax of the split | via the interest tax of the split |

Consequences worth stating explicitly for governance:

- Configuring `TreasuryFeeShareBps` to `0` does **not** zero out treasury
  revenue. Late fees keep flowing to `TreasuryBalance`.
- There is **no** way to route a late fee to the bounty pool, and no
  per-repayment override of the split.
- Bounty revenue can only be produced by the split of realized interest, so it
  requires `ProtocolFeeBps > 0` **and** `TreasuryFeeShareBps < 10_000`.
- Both accumulators can be withdrawn in full at any time; there is no partial
  withdrawal parameter on any entrypoint (see §7).

---

## 6. The `AuctionActive` freeze on fee changes

**Rule.** While at least one liquidation auction is in flight, every
**fee-configuration** entrypoint reverts with
`ContractError::AuctionActive` (`63`) before writing anything. The stored
values are left untouched.

The guard is `storage::assert_no_active_auctions`:

```rust
pub fn assert_no_active_auctions(env: &Env) {
    if get_pending_auction_count(env) > 0 {
        env.panic_with_error(ContractError::AuctionActive);
    }
}
```

**Which entrypoints are frozen** (all five, and only these five):

| Entrypoint | Guard site |
|---|---|
| `set_protocol_fee_bps` | `lib.rs` |
| `set_treasury_fee_share_bps` | `lib.rs` |
| `set_late_fee_config` | `lib.rs` |
| `set_late_fee_flat` | `lifecycle.rs` |
| `set_penalty_surcharge_bps` | `risk.rs` |

**Which are *not* frozen:** `set_treasury`, `set_bounty`,
`propose_treasury_withdrawal`, `execute_treasury_withdrawal`,
`withdraw_treasury`, `withdraw_bounty`. Withdrawal and address-repointing stay
available; only the *economics* of the split are locked.

**Why.** Changing a fee parameter mid-auction would silently re-price an
in-flight liquidation (a bidder's headroom, the recovery arithmetic, and the
post-settlement accumulator balances are all derived from the fee keys). The
freeze makes the parameters an auction reads the same parameters the
pre-auction agreement was made under. See issue #1169.

**When the freeze lifts.** "Active" spans
`CreditStatus::Defaulted` until the line leaves the `Defaulted` pipeline
through one of four terminal paths — full settlement, `reinstate_credit_line`,
admin force-close, or admin reopen. A **partial** settlement leaves the line
`Defaulted`, so the auction stays active and the freeze stays engaged. With
several concurrent auctions, the freeze holds until the **last** one exits.
Pause (`paused`) is a separate, orthogonal guard; it is not applied to the
withdrawal entrypoints at all.

**Test of record:** `tests/fee_config_during_auction.rs` (issue #1169), which
also pins the "non-fee admin config still allowed while auction active" case.

---

## 7. Withdrawal paths

There are **two independent ways** for the treasury accumulator to leave the
contract. Governance must know both exist, because they have different
guarantees.

### 7.1 Immediate withdrawal (no timelock)

```text
withdraw_treasury(admin):
    require_admin_auth, admin.require_auth
    treasury_addr = TreasuryAddress       or revert TreasuryNotSet
    amount        = TreasuryBalance
    if amount == 0: return                // no event, no write beyond the guard
    LiquidityToken                        or revert MissingLiquidityToken
    token.transfer(contract → treasury_addr, amount)
    clear_treasury_balance()              // → 0
```

```text
withdraw_bounty(admin):
    require_admin_auth, admin.require_auth
    bounty_addr = BountyAddress           or revert BountyNotSet
    amount      = BountyBalance
    if amount == 0: return
    LiquidityToken                        or revert MissingLiquidityToken
    token.transfer(contract → bounty_addr, amount)
    clear_bounty_balance()                // → 0
```

- **Immediate.** There is no timelock, no proposal, and no delay on this path.
- **All-or-nothing.** The full accumulator is moved; there is no `amount`
  parameter. Rounding/partial-withdrawal requests are impossible by design.
- Neither path is gated by `paused` or by the `AuctionActive` freeze.
- Neither path emits a treasury-specific event: the token contract's own
  transfer event is the on-chain record. `TreasuryBalance` returning `0` is the
  observable state change.

### 7.2 Timelocked withdrawal (two-step, 24 h)

```text
propose_treasury_withdrawal(admin):
    require_admin_auth, admin.require_auth
    treasury = TreasuryAddress            or revert TreasuryNotSet
    if PendingTreasuryWithdrawal exists:  revert TreasuryProposalExists
    amount        = TreasuryBalance       // ← SNAPSHOT taken now
    proposed_at   = ledger.timestamp
    execute_after = proposed_at + 86_400  // 24 hours
    store proposal { recipient: treasury, amount, proposer: admin, proposed_at, execute_after }
    emit ("credit","tre_prop") TreasuryWithdrawalProposedEvent

execute_treasury_withdrawal(admin):
    require_admin_auth, admin.require_auth
    proposal = PendingTreasuryWithdrawal  or revert NoPendingTreasuryWithdrawal
    if ledger.timestamp < proposal.execute_after: revert TreasuryTimelockActive
    if proposal.amount > 0:
        LiquidityToken                    or revert MissingLiquidityToken
        token.transfer(contract → proposal.recipient, proposal.amount)
    clear_pending_treasury_withdrawal()
    clear_treasury_balance()              // ← clears the WHOLE accumulator
    emit ("credit","tre_exec") TreasuryWithdrawalExecutedEvent
```

Boundary semantics, all pinned by `tests/treasury_timelock.rs`:

- The timelock is **inclusive**: executing at exactly
  `proposed_at + 86_400` succeeds; one second earlier reverts.
- `amount == 0` proposals still execute and still clear state (no token call).
- Replay is impossible: execution removes the proposal, and a second
  `execute_treasury_withdrawal` reverts with `NoPendingTreasuryWithdrawal`.
- A new proposal is allowed immediately after a previous execution.
- Only **one** proposal can exist at a time; `propose_treasury_withdrawal`
  reverts with `TreasuryProposalExists` while one is pending.

### 7.3 Timelock state machine

```text
                 propose                     execute (now ≥ execute_after)
   IDLE  ───────────────────▶  PENDING  ───────────────────────────▶  IDLE
     ▲                            │                                    │
     │                            │ propose again                      │ propose again
     └── TreasuryProposalExists ◀─┘                                    │
     │                                                                 │
     └─────────────────────────────────────────────────────────────────┘
```

### 7.4 Timelock vs immediate path — the governance caveat

The 24 h timelock is a property of the **propose/execute path only**. The
immediate `withdraw_treasury` entrypoint drains the same accumulator with **no**
delay, and is available to the same admin. Therefore:

> **The timelock is not a cryptographic or structural guarantee that treasury
> changes are observable for 24 h before they happen.** It is an opt-in
> procedure. An admin that intends to move funds quickly uses
> `withdraw_treasury` and no timelock applies.

Governance that wants a *hard* delay must treat `withdraw_treasury` /
`withdraw_bounty` as an admin capability to be constrained **off-chain** (for
example by making the contract admin a multisig or a governance contract
whose proposal step is itself delayed), and/or by removing the immediate path
in a future revision. See §10.

### 7.5 Balance drift between propose and execute

`propose_treasury_withdrawal` snapshots `TreasuryBalance` at proposal time.
`execute_treasury_withdrawal` transfers **the snapshot**, but clears the
**entire** accumulator. Any fee accrued during the 24 h window (a repayment, or
a late fee) between the two calls is therefore:

- present in the contract's token balance, and
- **no longer tracked** in `TreasuryBalance` after execution.

The surplus is not lost — it remains in the contract and is still covered by
the contract's token balance — but it is no longer attributable to the treasury
accumulator, and no event records the discrepancy. Operators must reconcile
`token.balanceOf(contract)` against the reserve plus the accumulators
manually. This is a known gap; see §10.

---

## 8. End-to-end walkthrough

Configuration: `ProtocolFeeBps = 300`, `TreasuryFeeShareBps = 3_000`,
`LateFeeFlat = 50`, `TreasuryAddress = T`, `BountyAddress = B`.

| Step | Action | `TreasuryBalance` | `BountyBalance` | Contract token balance | Notes |
|---|---|---|---|---|---|
| 0 | Initial state | `0` | `0` | `R` (reserve funding) | — |
| 1 | Repay: interest component `1_000` | `21` | `9` | `R + 30` | `fee = floor(1_000×300/10_000) = 30`; `split(30, 3_000)` → `(21, 9)`; reserve receives `1_000 − 30` |
| 2 | Repay retiring 3 installments, all overdue | `171` | `9` | `R + 30` | late fees `3 × 50 = 150` → treasury only; no token movement (already collected) |
| 3 | `propose_treasury_withdrawal` at `t` | `171` | `9` | `R + 30` | proposal snapshots `amount = 171`, `execute_after = t + 86_400` |
| 4 | Repay: a further `100` fee accrued at `t + 1 h` | `271` | `9` | `R + 130` | balance moves during the window — see §7.5 |
| 5 | `execute_treasury_withdrawal` at `t + 86_400` | `0` | `9` | `R + 130 − 171 = R − 41` | transfers the **snapshot** `171`; clears the accumulator; `100` becomes untracked |
| 6 | `withdraw_bounty(admin)` | `0` | `0` | `R − 41 − 9` | transfers `9` to `B`, no timelock |

Step 5 is the drift case of §7.5, shown concretely.

---

## 9. Invariants

Intended invariants, and where each is enforced or tested:

| # | Invariant | Enforcement |
|---|---|---|
| I1 | `treasury_share + bounty_share == total_fee` for every split | `split_conserving`; `math_utils` unit tests; `tests/fee_split.rs` |
| I2 | A split is deterministic for identical inputs | largest-remainder with explicit tie-break; `split_conserving_deterministic_across_calls` |
| I3 | No fee is charged on principal | fee computed on `interest_repaid` only; `protocol_fee_total_repayment.rs` |
| I4 | A zero/negative fee never moves an accumulator | `accrue_protocol_fee` early return; `fees` unit tests |
| I5 | Fee config cannot change while an auction is active | `assert_no_active_auctions`; `fee_config_during_auction.rs` |
| I6 | At most one pending treasury proposal exists | `TreasuryProposalExists`; `treasury_timelock.rs::duplicate_proposal_is_rejected` |
| I7 | A timelocked execution cannot happen before `execute_after` | `TreasuryTimelockActive`; `treasury_timelock.rs` |
| I8 | An executed proposal cannot be replayed | proposal removed + balance cleared; `replay_execution_is_rejected` |
| I9 | Accumulation never wraps silently | `checked_add` → `Overflow`; `assert_no_active_auctions` is checked *before* any write |
| I10 | Withdrawals never touch reserve or borrower funds | `transfer` from the contract's accumulator only; `SECURITY.md` T9 |

**Not** invariant, despite being commonly assumed:

- `TreasuryBalance ≤ contract token balance` is not *tracked* as an invariant,
  but §7.5 can make the tracking drift low (untracked surplus).
- The timelock does **not** delay all treasury movement (§7.4).
- Configuring a structured `LateFeeConfig` does **not** change the late fee
  actually charged (§10).

---

## 10. Known gaps and follow-ups

Behaviour that is real, reachable, and not currently covered by a fix. Listed
so governance and auditors do not have to rediscover it.

1. **`withdraw_treasury` bypasses the 24 h timelock (§7.4).** Two paths drain
   the same accumulator; only one is delayed. Either remove the immediate path,
   or document an off-chain admin constraint as the actual guarantee.
2. **Accumulator drift on timelocked execution (§7.5).** Execution clears the
   whole balance but transfers only the proposal snapshot. Consider
   transferring the live balance, or re-snapshotting at execution, or emitting
   the delta.
3. **`set_late_fee_config` does not affect fees charged.** `penalties::compute_late_fee`
   — the function that interprets `LateFeeConfig` — is only called from its own
   unit tests (`penalties_tests.rs`). The accrual path in
   `lifecycle::advance_repayment_schedule_after_repay` reads the legacy
   `LateFeeFlat` scalar directly. Consequently
   `set_late_fee_config(Some(Flat { amount }))` changes storage and emits
   nothing observable, while `set_late_fee_flat(amount)` is what actually
   charges. `AprBased` is consumed only indirectly, via
   `PenaltySurchargeBps` in `accrual::apply_accrual`. Until this is wired up,
   the two late-fee keys can disagree and the structured one wins nothing.
4. **No `cancel_treasury_withdrawal` entrypoint.** `docs/errors.md`,
   `docs/error-taxonomy.md`, and `docs/contract-errors.md` advise "execute or
   cancel the existing proposal", but no cancel entrypoint exists. Only
   execution clears a pending proposal.
5. **No events on immediate withdrawals (§7.1).** `fee_accrd`, `late_fee`,
   `tre_prop`, and `tre_exec` are indexed; `withdraw_treasury` and
   `withdraw_bounty` are not, so indexers must infer them from token transfers.
6. **Late fees are unreachable revenue for the bounty pool (§5).** Probably
   intended, but the asymmetry is not stated anywhere else.
7. **Stale "floor to treasury, remainder to bounty" descriptions.** The
   largest-remainder rule (§3.1) is the implemented behaviour. `PROTOCOL_SPEC.md`
   §2.7, the `fees.rs` module header, and the `set_treasury_fee_share_bps`
   rustdoc previously restated the retired convention and were corrected
   alongside this page.

---

## 11. Error codes

| Code | Variant | Raised by | Meaning |
|---|---|---|---|
| `5` | `InvalidAmount` | `set_late_fee_config` (`Flat{amount < 0}`) | Negative flat late fee |
| `12` | `Overflow` | `set_protocol_fee_bps`, `set_treasury_fee_share_bps`, `add_*_balance` | Fee above its hard cap, or accumulator overflow |
| `8` | `RateTooHigh` | `set_late_fee_config` (`AprBased{surcharge_bps > 10_000}`) | Surcharge above `MAX_INTEREST_RATE_BPS` |
| `22` | `MissingLiquidityToken` | all withdrawal paths | `LiquidityToken` unset; nothing can be transferred |
| `30` | `TreasuryNotSet` | `withdraw_treasury`, `propose_treasury_withdrawal` | `TreasuryAddress` unset |
| `41` | `BountyNotSet` | `withdraw_bounty` | `BountyAddress` unset |
| `42` | `NoPendingTreasuryWithdrawal` | `execute_treasury_withdrawal` | Nothing pending |
| `43` | `TreasuryTimelockActive` | `execute_treasury_withdrawal` | `now < execute_after` |
| `44` | `TreasuryProposalExists` | `propose_treasury_withdrawal` | A proposal is already pending |
| `63` | `AuctionActive` | the five frozen fee setters (§6) | A liquidation auction is in flight |

---

## 12. Events

| Topic | Payload | Emitted by | Fields |
|---|---|---|---|
| `("credit","fee_accrd")` | `FeeAccruedEvent` | `fees::accrue_protocol_fee` | `borrower`, `fee_amount` (total, pre-split), `treasury_amount`, `bounty_amount`, `new_treasury_balance`, `new_bounty_balance` |
| `("credit","late_fee")` | `LateFeeChargedEvent` | `lifecycle::advance_repayment_schedule_after_repay` | `borrower`, `fee` (per installment), `installment_index` (1-based) |
| `("credit","pen_enter")` | `PenaltyRateEnteredEvent` | `accrual::apply_accrual` | `borrower`, `base_rate_bps`, `penalty_surcharge_bps`, `effective_rate_bps` |
| `("credit","pen_exit")` | `PenaltyRateExitedEvent` | `accrual::apply_accrual` | `borrower`, `previous_rate_bps`, `new_rate_bps` |
| `("credit","tre_prop")` | `TreasuryWithdrawalProposedEvent` | `propose_treasury_withdrawal` | `recipient`, `amount`, `proposer`, `proposed_at`, `execute_after` |
| `("credit","tre_exec")` | `TreasuryWithdrawalExecutedEvent` | `execute_treasury_withdrawal` | `recipient`, `amount`, `executor`, `executed_at` |

Note the accumulator balances in `fee_accrd` are the **post-credit** values, so
a consumer can track both accumulators from this event alone without extra
reads. `fee_amount` is the pre-split total, which lets an indexer verify
`treasury_amount + bounty_amount == fee_amount` (§9, I1). See
[`EVENT_CATALOG.md`](./EVENT_CATALOG.md) for topics and versioning policy.

There is **no** event for `withdraw_treasury` / `withdraw_bounty` (§10, item 5)
and **no** event when `propose_treasury_withdrawal` reverts or when a split is
skipped because the fee floors to `0`.

---

## 13. Tests and where each claim is pinned

| Claim | Test |
|---|---|
| Default split is 100 % treasury | `tests/fee_split.rs::fee_split_default_is_all_treasury` |
| Even split, `0`-share, `10_000`-share short-circuits | `tests/fee_split.rs::{fee_split_even_ratio_splits_fee_between_pools, fee_split_all_bounty_when_share_is_zero}` |
| Rounding allocation | `tests/fee_split.rs::fee_split_remainder_goes_to_bounty_on_rounding`, `fees::tests::*`, `math_utils::tests::split_conserving_*` |
| Share above `10_000` rejected | `tests/fee_split.rs::set_treasury_fee_share_bps_rejects_above_max` |
| `u128`-magnitude conservation | `math_utils::tests::split_conserving_sums_to_total_exact` |
| Fee on interest only, flooring to zero | `tests/protocol_fee.rs`, `tests/protocol_fee_total_repayment.rs` |
| Fee bounds (`0..=1_000`, min ≤ max, hard cap) | `tests/governance_fee.rs` |
| Timelock boundaries, replay, duplicate, zero-balance | `tests/treasury_timelock.rs` |
| `AuctionActive` freeze on all five setters | `tests/fee_config_during_auction.rs` |
| Bounty withdrawal and `BountyNotSet` | `tests/fee_split.rs::{withdraw_bounty_transfers_accumulated_balance, withdraw_bounty_without_address_reverts}` |
| Error discriminants stable | `tests/error_discriminants.rs` |

Run the fee-split suite (the command named in the issue's validation section):

```bash
cargo test -p creditra-credit --test fee_split
```

> **Note on running this today.** The workspace-root `Cargo.lock` is currently
> corrupt — it contains two identical `[[package]] name = "creditra-credit"`
> entries, so cargo refuses to parse it and every workspace-level `cargo`
> invocation fails before compiling anything. Separately, the Soroban
> `creditra-credit` lib does not currently compile (duplicate `init` /
> `__init` definitions). Both are pre-existing and tracked as follow-ups; the
> fee-share logic above is unchanged by them and is additionally covered by
> `math_utils`/`fees` unit tests, which pin the split arithmetic in isolation.

---

## 14. Related documents

- [`PROTOCOL_SPEC.md`](./PROTOCOL_SPEC.md) §2.7 — the terse entrypoint table.
- [`EVENT_CATALOG.md`](./EVENT_CATALOG.md) — authoritative event topics.
- [`STORAGE_LAYOUT.md`](./STORAGE_LAYOUT.md) / [`STORAGE_LAYOUT.md`](./STORAGE_LAYOUT.md) — tier reference.
- [`SECURITY.md`](./SECURITY.md) T9 — why treasury drain cannot reach reserve funds.
- [`contract-errors.md`](./contract-errors.md) / [`errors.md`](./errors.md) — full error tables.
- [`EXECUTION_QUALITY.md`](./EXECUTION_QUALITY.md) — test catalog and CI.
