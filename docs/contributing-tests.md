# Contributing Tests

This guide covers test-only helpers used in `contracts/credit/src/lib.rs` for
draw/repay integration scenarios.

## Liquidity Test Helpers

The main contract test module keeps liquidity setup lightweight with helper
functions around the real Soroban token client rather than a separate fake
token implementation.

Use these helpers in `contracts/credit/src/lib.rs` when a test needs to model
balance changes across multiple calls:
- `setup(...)` to deploy the contract, configure the liquidity token, and seed
	the initial reserve;
- `mint_liquidity(...)` to top up the reserve or borrower between calls;
- `liquidity_balance(...)` to assert reserve depletion and repayment effects;
- `approve(...)` for repay-path allowance setup.

## When To Use It

- Draw scenarios that need explicit reserve funding checks.
- Repay scenarios that need borrower balance/allowance fixtures.
- Any new integration-style test that currently duplicates token setup code.

## Reserve Depletion Sequences

Reserve-sensitive draw regressions should snapshot both state and events around
the failing call:
- perform one successful draw to consume part of the reserve;
- record `utilized_amount`, `last_accrual_ts`, and event counts;
- attempt a second draw that exceeds the remaining reserve;
- assert the panic message, unchanged reserve balance, unchanged stored credit
	line fields, and no additional `drawn` or `accrue` events.

Cover both a single borrower issuing sequential draws and multiple borrowers
sharing the same reserve so shared-liquidity regressions are caught.

## Reentrancy guard lifecycle (`token_failure_rollback.rs`)

Integration tests in `contracts/credit/tests/token_failure_rollback.rs` assert
that `draw_credit` / `repay_credit` clear the reentrancy guard after both
pre-transfer validation failures and mid-transfer CPI failures:

```bash
cargo test -p creditra-credit --test token_failure_rollback rollback
```

- **Pre-transfer failures** use the real Stellar asset contract (insufficient
  reserve / allowance) with `catch_unwind` to continue the same test after panic.
- **Mid-transfer failures** use the in-test `FailingTokenContract` mock (internal
  balances, configurable `set_fail_transfer` / `set_fail_transfer_from`) for
  draw-fail-then-draw and repay-fail-then-repay sequencing.

## Scope Boundary

`MockLiquidityToken` is test-only (`#[cfg(test)]`) and must not be imported
into contract runtime logic.

---

## Snapshot fuzzing: `prorate_interest`

`math_utils::prorate_interest` is the single rounding-floor primitive used by
every interest accrual in the protocol. A dedicated snapshot harness pins 4 096
deterministic `(principal, rate_bps, seconds)` inputs together with their
expected floor-rounded outputs, so any rounding-direction regression is caught
at PR time.

### Files

| File | Role |
|---|---|
| `contracts/credit/test_snapshots/prorate_interest.json` | Checked-in pinned snapshot (4 096 entries) |
| `contracts/credit/fuzz/fuzz_targets/prorate_interest_snapshot.rs` | libFuzzer target that loads and verifies the snapshot |
| `contracts/credit/tests/snapshot_prorate_interest.rs` | `cargo test` integration test (verify + regenerate) |

### Running the snapshot test (CI / default)

```bash
cargo test -p creditra-credit --test snapshot_prorate_interest
```

This loads the checked-in JSON and re-runs `prorate_interest` for every entry.
It fails immediately if any output diverges from its pinned value.

### Running the fuzz target

```bash
cargo fuzz run prorate_interest_snapshot -- -max_total_time=60
```

libFuzzer ignores the mutated corpus bytes; every invocation executes the full
4 096-entry snapshot verification sweep.

### Regenerating the snapshot

Run regeneration **only after an intentional change** to `prorate_interest`
(e.g., a constant update or a deliberate rounding-direction change).

```bash
# 1. Regenerate and self-check
cargo test -p creditra-credit --test snapshot_prorate_interest \
    -- --nocapture regenerate

# 2. Verify the freshly written file
cargo test -p creditra-credit --test snapshot_prorate_interest

# 3. Commit the updated snapshot alongside the implementation change
git add contracts/credit/test_snapshots/prorate_interest.json
git commit -m "test: regenerate prorate_interest snapshot after <describe change>"
```

### Input generation design

Inputs are produced by a seeded 64-bit LCG (Knuth/MMIX parameters,
seed `0xDEADBEEFCAFE1234`) with no external crate dependency, ensuring full
reproducibility across platforms and Rust versions.

The corpus begins with 15 hand-chosen anchors covering:
- zero-input short-circuit paths (`principal=0`, `rate_bps=0`, `seconds=0`)
- exact Julian-year, half-year, and quarter-year time boundaries
- maximum rate (10 000 bps = 100 %)
- minimum and maximum principal values
- the exact-divisibility boundary where `floor == ceil`

The remaining entries are LCG-generated with:
- `principal` ∈ [0, 10^24]  (safe ceiling: `10^24 × 10_000 × u32::MAX < u128::MAX`)
- `rate_bps`  ∈ [0, 10_000]
- `seconds`   ∈ [0, u32::MAX]

### Properties verified per entry

1. **Exact match** — `live_result == pinned_output` (primary regression gate).
2. **Zero-input short-circuit** — any zero input produces zero output.
3. **Floor ≤ ceil** — `prorate_interest(..., Floor) ≤ prorate_interest(..., Ceil)`.
4. **Ceil − floor ∈ {0, 1}** — rounding never moves by more than 1 ulp.