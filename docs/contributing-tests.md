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
