# Contributing Tests

## Required CI Gate — Build Hygiene

Every pull request must pass the **Build Hygiene** workflow
(`.github/workflows/build-hygiene.yml`) before it can be merged.  The workflow
is configured as a required branch-protection status check under two job names:

| Status check name | Command |
|---|---|
| `cargo check (workspace, all-targets)` | `cargo check --workspace --all-targets` |
| `cargo clippy (workspace, -D warnings)` | `cargo clippy --workspace --all-targets -- -D warnings` |

### Why this gate exists

Creditra-Contracts shipped to `main` twice this quarter with merge-artifact
duplicates (`self_suspend_credit_line` in `lifecycle.rs`, double `use` blocks in
`risk.rs`).  `cargo check --workspace --all-targets` catches duplicate symbol
definitions and broken module paths — including those inside `#[cfg(test)]`
blocks that a plain `cargo build` skips.  `cargo clippy -D warnings` catches the
softer class of problems (dead code, redundant patterns, unused imports) that
often accompany incomplete conflict resolutions.

### Running the checks locally

```bash
# Mirror exactly what CI runs:
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
```

Both commands use the toolchain pinned in `rust-toolchain.toml` (`stable`).
Run `rustup update stable` if your local toolchain is more than a few weeks
behind to keep diagnostic output in sync with CI.

### Making the check required (maintainers)

1. Go to **Repository Settings → Branches → Branch protection rules → `main`**.
2. Enable **Require status checks to pass before merging**.
3. Search for and add both status check names from the table above.
4. Enable **Require branches to be up to date before merging** to prevent a
   stale-base bypass.

---

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

## Installment schedule property test

`contracts/credit/tests/proptest_installment.rs` covers installment due-date
advancement with randomized repayment schedules.  The model mirrors the public
`repay_credit` behaviour: each requested repayment is capped to the remaining
outstanding debt, then `next_due_ts` advances by
`floor(effective_repay / amount_per_period) * period_seconds` using saturating
`u64` arithmetic.  The test also keeps deterministic edge cases for partial,
exact, multi-installment, and over-repayment scenarios.

## Oracle deviation snapshot test

`contracts/credit/tests/snap_deviation.rs` adds snapshot-fuzz coverage for
`math_utils::compute_deviation_bps` across realistic price pairs. Run it with:

```bash
cargo test -p creditra-credit --test snap_deviation
```

The test locks in expected outcomes for common oracle-feed moves, zero/negative
price edge cases, and a deterministic proptest over positive prices so the
oracle circuit-breaker math stays stable.

## `accrued_interest` snapshot-fuzz test (`contracts/creditra-credit`)

`contracts/creditra-credit/tests/snap_prorate.rs` pins the CosmWasm
`accrued_interest` function — the 365-day simple-interest accrual primitive
that mirrors the Soroban `prorate_interest`.

### Run (verify mode — CI default)

```bash
cargo test -p creditra-credit --test snap_prorate
```

### Regenerate after an intentional change

```bash
cargo test -p creditra-credit --test snap_prorate \
    -- --nocapture regenerate
```

Commit both the updated test and the regenerated
`contracts/creditra-credit/tests/snapshots/accrued_interest.json`.

### What is tested

| Layer | Coverage |
|---|---|
| Snapshot (4 096 entries) | Exact bit-for-bit match against the pinned JSON; fails CI on any silent arithmetic drift |
| Deterministic unit cases | Zero inputs, exact-year boundary, half-year, floor-to-zero, exact-division, overflow path, monotone time series |
| `proptest` (512 cases each) | Monotone in time / principal / rate; zero boundary; total / panic-free; overflow upward-closed; interest ≤ principal within one year; additive consistency across split periods |

### Key differences from the Soroban twin

| Property | Soroban `prorate_interest` | CosmWasm `accrued_interest` |
|---|---|---|
| Year length | 31 557 600 s (Julian 365.25-day) | 31 536 000 s (365-day) |
| Rounding | Caller-controlled `Floor`/`Ceil` | Always floor |
| Overflow | Panics | Returns `Err(ContractError::Overflow)` |
| Snapshot location | `contracts/credit/test_snapshots/prorate_interest.json` | `contracts/creditra-credit/tests/snapshots/accrued_interest.json` |

The snapshot file is committed to the repository.  Any modification to
`accrued_interest`'s arithmetic must be followed by a regeneration run and the
new JSON committed in the same PR so CI never sees a stale snapshot.
