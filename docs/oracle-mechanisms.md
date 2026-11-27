# Oracle Mechanisms

## Scope

The `creditra-credit` contract (`contracts/credit`) contains **three distinct oracle
mechanisms**. They were added at different times, address different problems, and
store their state under different keys. Only two of them gate settlement.

| # | Mechanism | Entry-points | Gates `settle_default_liquidation`? |
|---|-----------|--------------|------------------------------------|
| 1 | Single-price circuit breaker | `set_oracle_config` | **Yes** (only when mechanism 2 is unset) |
| 2 | Quorum-of-K submission | `set_oracle_quorum_config`, `submit_oracle_prices` | **Yes** (takes precedence over 1) |
| 3 | Weighted-median registry | `add_oracle`, `remove_oracle`, `set_quorum_threshold`, `set_reporting_window`, `report_value`, `get_median_value` | **No** |

> **Read this first.** Mechanism 3 is **not** wired into settlement. Registering
> oracles, collecting reports, and computing a median have no effect on
> `settle_default_liquidation`. See
> [Mechanism 3](#mechanism-3--weighted-median-registry)
> and [What does not gate settlement](#what-does-not-gate-settlement) below.

**Source of truth.** Precedence, config keys, staleness, and failure codes in this
document are derived from [`contracts/credit/src/oracle_validation.rs`](../contracts/credit/src/oracle_validation.rs)
(function `validate_settlement_oracle_price`) and
[`contracts/credit/src/oracles.rs`](../contracts/credit/src/oracles.rs). Where this
document and the code disagree, the code wins.

**Related documents.** [`docs/default-oracle.md`](./default-oracle.md) covers the
*staged, unimplemented* default-signal attestation oracle — a different design with
different entry-points. [`docs/ORACLE_OUTAGE_SIMULATION.md`](./ORACLE_OUTAGE_SIMULATION.md)
covers quorum-mode outage scenarios and recovery routes.

---

## Precedence: which config keys gate settlement

`settle_default_liquidation` resolves its price through a fixed, total order. The
resolver is `validate_settlement_oracle_price` in `oracle_validation.rs:127`, reached
from `lifecycle.rs:1076` (Step 4 of settlement, *before* any state mutation).

### Decision table

| Priority | Condition | Storage key consulted | Mechanism | `oracle_price` argument | Result |
|---|---|---|---|---|---|
| **1** | `OracleQuorumConfig` is set | `DataKey::OracleQuorumConfig` (instance) | 2 — Quorum | **Ignored** | `QuorumMode(price)` |
| **2** | `OracleConfig` is set | `DataKey::OracleConfig` (instance) | 1 — Circuit breaker | **Used** | `SingleOracleMode(price)` |
| **3** | Neither config is set | — | None | **Ignored** | `NotConfigured` |

Priority 1 wins unconditionally. The resolver loads *both* configs first
(`oracle_validation.rs:134-135`), then short-circuits into the quorum branch
(`:138-140`) and returns before the single-oracle branch is ever reached
(`:143-146`). There is no configuration in which the caller-supplied price wins over
a configured quorum.

### Secondary keys, by branch

These are only read once the branch above has been selected.

| Branch | Keys read | Read when |
|---|---|---|
| Quorum | `DataKey::OracleQuorumPrice`, `DataKey::OracleQuorumPriceTs` (instance) | Always, once quorum config is set |
| Single-oracle | `DataKey::OracleLastPrice`, `DataKey::OracleLastPriceTs` (instance) | Only if `OracleLastPriceTs` is present; `OracleLastPrice` is read inside that guard |
| Registry | `OracleDataKey::OracleList`, `OracleWeight`, `OracleReport`, `QuorumThreshold`, `ReportingWindow` | **Never by settlement** |

`OracleLastPrice` and `OracleLastPriceTs` are always read as a pair. If
`OracleLastPriceTs` is absent, `OracleLastPrice` is not read at all and both the
staleness and deviation checks are skipped — see
[Mechanism 1](#mechanism-1--single-price-circuit-breaker).

### Keys never consulted by the resolver

- Every `OracleDataKey::*` registry key.
- `OracleDataKey::QuorumThreshold` and `OracleDataKey::ReportingWindow` are
  unrelated to `OracleQuorumConfig.min_quorum_k` / `max_age_seconds` despite the
  shared name. The registry's own parameters.

### Two validation layers, one precedence order

The settlement path enforces the circuit breaker **twice**, and both layers agree on
precedence:

1. **Inline block** — `lib.rs:1942-1979`, inside `settle_default_liquidation` itself.
   Guarded by `if get_oracle_quorum_config(&env).is_none()`, so it is skipped
   entirely in quorum mode. It writes `OracleLastPrice` / `OracleLastPriceTs` and
   emits the price-accepted event.
2. **Module resolver** — `oracle_validation::validate_settlement_oracle_price`,
   called from `lifecycle.rs:1076`. This is the authoritative resolver and the one
   documented above.

In quorum mode, only layer 2 runs. In single-oracle mode both run; because layer 1
writes the accepted price before layer 2 reads it, layer 2's deviation comparison
evaluates the price against itself and passes. Either way the effective outcome is
the one in the decision table.

### Cross-mechanism coupling

`submit_oracle_prices` writes **both** key sets (`lib.rs:2258-2259`):

```text
set_oracle_quorum_price(canonical_price, now)   -> OracleQuorumPrice / OracleQuorumPriceTs
set_oracle_last_price(canonical_price, now)     -> OracleLastPrice   / OracleLastPriceTs
```

The second write is deliberate: `OracleLastPrice` is the "last accepted price" the
deviation circuit breaker and collateral release read. Consequence — a quorum
submission advances the clock on the single-oracle circuit breaker too. If quorum
config is later removed, the single-oracle branch resumes with a `last_price` that
came from a quorum median.

---

## Mechanism 1 — Single-price circuit breaker

**Entry-points:** `set_oracle_config(max_deviation_bps, max_age_seconds)`,
`get_oracle_config()`.

A stateless per-settlement check: the caller supplies a price, and the contract
rejects it if it is non-positive, if the *previous* accepted price is too old, or if
it has moved more than `max_deviation_bps` from that previous price.

Storage (`contracts/credit/src/storage.rs`):

| Key | Type | Meaning |
|---|---|---|
| `DataKey::OracleConfig` | `OracleConfig { max_deviation_bps: u32, max_age_seconds: u64 }` | Admin policy. `None` disables the breaker. |
| `DataKey::OracleLastPrice` | `i128` | Last price that passed the breaker |
| `DataKey::OracleLastPriceTs` | `u64` | Ledger timestamp of that price |

### Configuration validation (`lib.rs:2133-2138`)

- `max_deviation_bps` must be in `1..=10_000`; otherwise `InvalidAmount` (5).
- `max_age_seconds` must be `> 0`; otherwise `InvalidAmount` (5).
- Requires `assert_not_paused` and `require_admin_auth`.

### Settlement checks (`oracle_validation.rs:181-220`), in order

1. `oracle_price` is `None` → `OraclePriceInvalid` (36).
2. `price <= 0` → `OraclePriceInvalid` (36).
3. If `OracleLastPriceTs` is present:
   - `now - OracleLastPriceTs > max_age_seconds` → `OraclePriceStale` (37).
   - If `OracleLastPrice` is also present, compute
     `compute_deviation_bps(price, last_price)`; `> max_deviation_bps` →
     `OraclePriceDeviation` (38).
4. Accepted; `record_accepted_oracle_price` stores the price and current timestamp
   after settlement commits.

**First-price exception.** When no prior price has been recorded, steps 3 and 4 are
skipped entirely: any strictly positive price is accepted, regardless of how far it
sits from any real market price. The breaker only constrains the *second* price
onward.

**Deviation is symmetric and relative.** `compute_deviation_bps(new, last)`
(`math_utils.rs:382`) returns `|new - last| * 10_000 / last`, capped at `u32::MAX`.
It returns `None` only when `last_price <= 0`, which the resolver converts to
`OraclePriceInvalid` (36). A +5% and a −5% move produce the same 500 bps.

---

## Mechanism 2 — Quorum-of-K submission

**Entry-points:** `set_oracle_quorum_config(min_quorum_k, max_deviation_bps, max_age_seconds)`,
`get_oracle_quorum_config()`, `submit_oracle_prices(prices)`.

An admin submits up to 20 independent prices. `resolve_quorum_price`
(`oracles.rs:520`) finds a K-wide window in the sorted array whose spread is within
tolerance, and stores the lower-median of that window as the canonical price.
Settlement then reads that stored price.

Storage:

| Key | Type | Meaning |
|---|---|---|
| `DataKey::OracleQuorumConfig` | `OracleQuorumConfig { min_quorum_k: u32, max_deviation_bps: u32, max_age_seconds: u64 }` | Admin policy. `Some(..)` activates quorum mode and disables mechanism 1. |
| `DataKey::OracleQuorumPrice` | `i128` | Last resolved canonical price |
| `DataKey::OracleQuorumPriceTs` | `u64` | Ledger timestamp of that price |

### Configuration validation (`lib.rs:2188-2196`)

- `min_quorum_k >= 2`; otherwise `InvalidAmount` (5).
- `max_deviation_bps <= 10_000`; otherwise `InvalidAmount` (5).
- `max_age_seconds > 0`; otherwise `InvalidAmount` (5).
- Requires `assert_not_paused` and `require_admin_auth`.

### Resolution algorithm (`oracles.rs:520-574`)

1. `n == 0 || n > MAX_ORACLE_FEEDS (20)` → `OraclePriceInvalid` (36).
2. `min_quorum_k < 2 || min_quorum_k > n` → `OracleQuorumNotMet` (50).
3. Any `price <= 0` → `OraclePriceInvalid` (36).
4. Selection-sort ascending into a fixed 20-element stack buffer. O(n²), bounded so
   gas is predictable.
5. Slide a K-wide window over the sorted array. A window qualifies when
   `compute_deviation_bps(hi, lo) <= max_deviation_bps`; when `lo` is not positive
   the deviation is treated as `u32::MAX` and the window never qualifies.
6. Return the **lower median** of the first qualifying window: index `i + (k - 1) / 2`.
7. No qualifying window → `OracleQuorumNotMet` (50).

The canonical price is stored with the current ledger timestamp.

### Submission-time checks (`lib.rs:2237-2261`)

- `assert_not_paused` and `require_admin_auth`.
- Quorum config unset → `OraclePriceInvalid` (36).
- `prices.len() > MAX_ORACLE_FEEDS` → `OraclePriceInvalid` (36) (redundant with
  step 1 above, checked first for a cheaper failure).
- Resolution failures surface as #36 / #50 per the algorithm above.

### Settlement checks (`oracle_validation.rs:157-173`)

Quorum mode checks **existence and staleness only**:

1. `OracleQuorumPrice` is `None` → `OracleQuorumNotMet` (50).
2. `OracleQuorumPriceTs` is `None` → `OracleQuorumNotMet` (50).
3. `now - OracleQuorumPriceTs > max_age_seconds` → `OraclePriceStale` (37).
4. Accepted as `QuorumMode(price)`.

**`max_deviation_bps` is a submission-time bound, not a settlement-time one.** The
tolerance is enforced once, inside `resolve_quorum_price`, when the price is
produced. `settle_default_liquidation` does not re-check the resolved price against
the previous one. Adding a second `submit_oracle_prices` call with a fresh,
internally-consistent but wildly different set of prices will be accepted as long
as it is not stale — the circuit-breaker behaviour that mechanism 1 provides is
**not** present in quorum mode.

---

## Mechanism 3 — Weighted-median registry

**Entry-points:** `add_oracle`, `remove_oracle`, `set_quorum_threshold`,
`set_reporting_window`, `report_value`, `get_median_value`.

A self-contained redundancy registry: approved oracles hold integer **weights**,
submit observed `u128` values, and `get_median_value` computes a **weighted
median** of the still-fresh reports subject to a weight quorum.

Implemented entirely in `oracles.rs:36-223` against its own `OracleDataKey` enum,
separate from both `DataKey::OracleConfig` and `DataKey::OracleQuorumConfig`.

| Key | Type | Meaning |
|---|---|---|
| `OracleDataKey::OracleList` | `Vec<Address>` | Registered oracles, in insertion order |
| `OracleDataKey::OracleWeight(Address)` | `u32` | Per-oracle weight |
| `OracleDataKey::OracleReport(Address)` | `OracleReportData { value: u128, timestamp: u64 }` | Latest report, overwritten on each call |
| `OracleDataKey::QuorumThreshold` | `u32` | Minimum total weight required. Defaults to `0` if unset. |
| `OracleDataKey::ReportingWindow` | `u64` | Freshness window in seconds. Defaults to `0` if unset. |

### Entry-point behavior

| Call | Auth | Notes |
|---|---|---|
| `add_oracle(oracle, weight)` | Admin | `weight == 0` → `InvalidAmount` (5). Upserts: re-adding an existing address only replaces the weight, it is not duplicated in the list. |
| `remove_oracle(oracle)` | Admin | Address not in the list → `OracleNotFound` (55). On success also removes the oracle's weight and report. |
| `set_quorum_threshold(threshold)` | Admin | No range validation. `0` means every weighted report qualifies. |
| `set_reporting_window(window_seconds)` | Admin | No validation. `0` makes every report instantly stale. |
| `report_value(oracle, value)` | The reporting oracle itself | Address not in `OracleList` → `Unauthorized` (1). No positivity check — `value` is `u128` and `0` is accepted. |
| `get_median_value()` | None | The only oracle function that **returns** `Result<u128, ContractError>` rather than panicking. |

### Median algorithm (`oracles.rs:135-223`)

1. For each oracle in `OracleList`, read its report. Skip if absent.
2. Freshness: keep the report when
   `now.saturating_sub(report.timestamp) <= window`.
3. Look up its weight; skip if absent or `0`. Weight accumulation uses
   `checked_add` → `Overflow` (12) on overflow.
4. `total_weight < quorum` → `OracleQuorumNotMet` (50).
5. Empty report set → `OracleQuorumNotMet` (50).
6. Sort ascending by value (insertion sort, O(n²), **not** bounded by
   `MAX_ORACLE_FEEDS`).
7. `target = total_weight.div_ceil(2)`. Walk the sorted reports accumulating weight;
   return the first `report.value` where `cumulative_weight >= target`.
   Accumulation uses `checked_add` → `Overflow` (12).
8. Note that the loop has no fallback: if it somehow falls through, the returned
   value is `0`.

### Registry staleness vs. the other two mechanisms

The freshness comparison is **inverted relative to mechanisms 1 and 2**, and the
failure mode differs:

| | Quorum / Single-oracle | Registry |
|---|---|---|
| Condition to reject | `age > max_age` | — |
| Condition to accept | — | `age <= window` |
| Stale data handling | Hard error (`OraclePriceStale`, #37) | **Silently dropped** from the candidate set |
| Unset window | — | Defaults to `0`, so every report is instantly stale |
| Signal that data went stale | Direct error code | Only `OracleQuorumNotMet` (50) if enough weight dropped out |

A registry oracle going offline does not produce a dedicated error. It reduces
`total_weight`, and the only symptom is that `get_median_value` starts returning
`OracleQuorumNotMet` once weight falls below the threshold.

---

## Staleness rules, side by side

| | Mechanism 1 (single) | Mechanism 2 (quorum) | Mechanism 3 (registry) |
|---|---|---|---|
| Timestamp source | `OracleLastPriceTs` | `OracleQuorumPriceTs` | Per-report `timestamp` |
| Bound | `OracleConfig.max_age_seconds` | `OracleQuorumConfig.max_age_seconds` | `OracleDataKey::ReportingWindow` |
| Test | `now - ts > max_age` → reject | `now - ts > max_age` → reject | `now - ts <= window` → keep, else drop |
| Boundary | `age == max_age` accepted | `age == max_age` accepted | `age == window` accepted |
| Checked when? | Only if a prior price + timestamp exist | Always (once quorum config is set) | Always, per report |
| On failure | `OraclePriceStale` (37) | `OraclePriceStale` (37) | Report dropped; `OracleQuorumNotMet` (50) if weight drops |
| Unset config | Breaker disabled entirely | Quorum mode not active | Window `0`; nothing is ever fresh |

### Shared edge-case behaviour

- **Boundary is inclusive.** An age of exactly `max_age_seconds` is accepted by
  mechanisms 1 and 2, and a report aged exactly `window` seconds is kept by
  mechanism 3. Exceeding the bound by one second fails.
- **`saturating_sub` never underflows.** A stored timestamp *ahead* of
  `env.ledger().timestamp()` yields `age == 0`, which is always fresh. Clock skew
  backwards is therefore not detected by any of the three.
- **No future-timestamp rejection.** None of the mechanisms reject a stored
  timestamp greater than `now`; a report or price "from the future" is treated as
  brand new and stays fresh until real time catches up.

---

## Failure codes

### Surfaced by `oracle_validation.rs`

These are the codes `validate_settlement_oracle_price` and its two helpers can
raise. All are raised via `env.panic_with_error`, before any state mutation.

| Code | Name | Category | Raised when |
|---|---|---|---|
| 36 | `OraclePriceInvalid` | Oracle | Single-oracle: `oracle_price` is `None`, or `price <= 0`, or `compute_deviation_bps` returns `None` (stored `last_price <= 0`). |
| 37 | `OraclePriceStale` | Oracle | Single-oracle: `now - OracleLastPriceTs > max_age_seconds`. Quorum: `now - OracleQuorumPriceTs > max_age_seconds`. |
| 38 | `OraclePriceDeviation` | Oracle | Single-oracle only: `compute_deviation_bps(price, last_price) > max_deviation_bps`. |
| 50 | `OracleQuorumNotMet` | Oracle | Quorum settlement only: `OracleQuorumPrice` or `OracleQuorumPriceTs` not yet written. |

`ResolvedOraclePrice::NotConfigured` is the fourth outcome, and it is not an error —
it is the no-gating path.

### Surfaces adjacent to `oracle_validation.rs`

Reached from the entry-points in the same settlement flow.

| Code | Name | Category | Raised by |
|---|---|---|---|
| 1 | `Unauthorized` | Auth | Non-admin call to any `set_oracle_*` / `add_oracle` / `remove_oracle` / `set_quorum_threshold` / `set_reporting_window` |
| 5 | `InvalidAmount` | Validation | `set_oracle_config` / `set_oracle_quorum_config` out-of-range parameters; `add_oracle` with `weight == 0` |
| 36 | `OraclePriceInvalid` | Oracle | `submit_oracle_prices` with quorum config unset, or `prices.len() > 20`, or any `price <= 0`; `resolve_quorum_price` with an empty list |
| 50 | `OracleQuorumNotMet` | Oracle | `resolve_quorum_price` when `k < 2`, `k > n`, or no K-wide window qualifies |
| 55 | `OracleNotFound` | Oracle | `remove_oracle` on an address not in the registry |
| 12 | `Overflow` | Validation | `get_median_value` weight accumulation overflows |

Numeric values are from `ContractError` in `contracts/credit/src/types.rs:218-240`.
Registry error codes are listed here for completeness only — see the note below on
why they cannot appear during settlement.

---

## What does not gate settlement

Restating the central point, because all three mechanisms are named "oracle" and
only two of them are load-bearing:

- **Mechanism 3 is not a settlement gate.** `oracle_validation.rs` never reads any
  `OracleDataKey::*`. There is no code path from `add_oracle`, `report_value`, or
  `get_median_value` into `settle_default_liquidation`. Configuring the registry,
  populating reports, and computing medians has **no effect** on which price
  settlement uses, and its errors can never surface from a settlement call.
- **Two consequences worth planning around.**
  1. A registry outage is invisible to settlement. Settlement keeps using the
     quorum price or the circuit-breaker price throughout, with no coupling to
     registry health.
  2. `get_median_value` is the only oracle function that returns rather than
     panics. To read a registry median, an off-chain caller must invoke it
     separately and inspect the `Result`; there is no on-chain consumer.
- **Mechanism 2's deviation bound is not re-checked at settlement.** Only existence
  and staleness are. See
  [Mechanism 2 — Settlement checks](#settlement-checks-oracle_validationrs157-173).
- **Registry and quorum share no state.** `OracleDataKey::QuorumThreshold` and
  `OracleDataKey::ReportingWindow` are unrelated to
  `OracleQuorumConfig.min_quorum_k` and `max_age_seconds`, despite the naming.
  Configuring one has no effect on the other.
- **The registry stores `u128`; mechanisms 1 and 2 store `i128`.** Even if the
  registry were wired in, the value types are not directly compatible with the
  `oracle_price: Option<i128>` argument to `settle_default_liquidation`.
- **Mechanism 1 is bypassed whenever mechanism 2 is configured**, even if the
  single-oracle config is still present and stricter. Removing
  `OracleQuorumConfig` is what re-enables it — and at that point
  `OracleLastPrice` holds the last quorum median, per
  [Cross-mechanism coupling](#cross-mechanism-coupling).

---

## Operator quick reference

| Goal | Do this |
|---|---|
| Bound per-settlement price movement | `set_oracle_config(max_deviation_bps, max_age_seconds)` |
| Aggregate several feeds into one canonical price | `set_oracle_quorum_config(k, max_deviation_bps, max_age_seconds)` |
| Keep a canonical price fresh | `submit_oracle_prices(prices)` on an interval below `max_age_seconds` |
| Drop back to the single-price breaker | Remove `OracleQuorumConfig` (no unset entry-point exists — see below) |
| Track several oracles with weights | `add_oracle` / `report_value` / `get_median_value` — **no effect on settlement** |

**There is no `clear_oracle_config` or `clear_oracle_quorum_config` entry-point.**
Mechanism 2 can only be disabled by a contract migration that removes the
`OracleQuorumConfig` instance key. Treat `set_oracle_quorum_config` as effectively
irreversible for the life of a deployed contract instance.
