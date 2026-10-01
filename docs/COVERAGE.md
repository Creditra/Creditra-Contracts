# Coverage Guide

## Overview

Line coverage is **measured and enforced in CI**, not asserted in prose.

The `coverage` job in `.github/workflows/ci.yml` runs `cargo-llvm-cov` on
`contracts/creditra-credit` and exits non-zero when measured line coverage falls
below `MIN_LINE_COVERAGE`.
The floor is a single value in the workflow's `env` block, so there is exactly
one place where it is set and one place where it is enforced.

| Setting | Value | Where |
|---|---|---|
| `MIN_LINE_COVERAGE` (enforced floor) | `92` | `.github/workflows/ci.yml` `env` |
| `CARGO_LLVM_COV_VERSION` (pinned reporter) | `0.9.1` | `.github/workflows/ci.yml` `env` |
| Measured line coverage at the time of writing | `92.93%` (3510 / 3777 lines) | `CI` workflow run summary |

## Why 92 and not 95

Earlier revisions of this document claimed that three workflows enforced a
`--fail-under-lines 95` floor across the whole workspace.
That was never true: no workflow measured coverage at all, and the
`coverage/` HTML tree that was committed to git was a stale snapshot.

Two facts, both reproducible on `main`, determine the real number:

1. **The root Soroban workspace cannot be measured by any tool.**
   `cargo check --locked --workspace --all-targets --keep-going` fails on 62
   compilation units across all 7 member crates. The first wall is a
   merge-artifact duplicate `init` in `contracts/credit/src/lib.rs`, which makes
   `creditra-credit` itself fail to build; repairing only that exposes
   `contracts/accrual`, `borrow`, and `collateral` failing to parse, and roughly
   fifty `contracts/credit/tests/*` targets failing with unresolved imports and
   missing types. The root `Cargo.lock` does not even parse — `creditra-credit`
   appears twice, so `cargo` aborts before compiling anything.
   Until that is repaired, `cargo llvm-cov --workspace` cannot run, so no
   threshold expressed over `--workspace` can be enforced.

2. **`contracts/creditra-credit` — the only workspace CI builds and tests —
   measures 92.93% line coverage** (3510 of 3777 lines), not 98.94%.

The floor is therefore set at `92`, slightly below the measured value, so the
gate is real and green on the day it lands and any regression below the
measured level fails the build.
The gap to 95% is a test-coverage work item for `contract.rs` and `views.rs`,
not a CI work item.

**The floor is a ratchet.**
Raise it as tests land; never lower it to unblock a pull request.

## Scope: which crate is measured

Coverage is measured over `contracts/creditra-credit` only, because that is the
crate the `contract` job compiles, tests, and enforces the WASM size budget on.
Measuring a crate that nothing in CI builds would reintroduce exactly the
problem this gate exists to fix: a number that looks authoritative and is not.

`contracts/credit` and the other root-workspace members are Soroban contracts
that CI does not build today.
When they are restored to a compiling state, extend the floor to them in a
follow-up that also documents the new denominator.

## CI Enforcement

| Job | Workflow | Trigger | Command |
|---|---|---|---|
| `Line coverage floor` | `ci.yml` | Push/PR to `main`, `master`, `develop` | `cargo llvm-cov --all-targets --html --fail-under-lines $MIN_LINE_COVERAGE` |

The job is separate from the `contract` job on purpose: it is a distinct
required check, so a coverage regression is reported as a coverage failure
instead of hiding inside a general build failure.

The HTML report is uploaded as the `coverage-report` artifact and the measured
numbers are written to the job summary.
Both run under `if: always()`, so a run that breaches the floor still
publishes the report that explains the breach.
The artifact has a 14-day retention window and replaces the `coverage/` tree
that used to be committed to git (the directory is already ignored via
`/coverage` in `.gitignore`).

The job measures with the compiler pinned in `rust-toolchain.toml`, including
the `llvm-tools` component that `cargo-llvm-cov` shells out to, so the CI number
and a local number are computed the same way.

## Running Locally

```bash
# Install the tool (one-time). Pin the version CI uses so the numbers match.
cargo install cargo-llvm-cov --version 0.9.1 --locked

cd contracts/creditra-credit

# Run coverage
cargo llvm-cov --all-targets

# Reproduce the CI gate exactly
cargo llvm-cov --all-targets --html --fail-under-lines 92

# Open the report
open target/llvm-cov/html/index.html   # macOS; xdg-open on Linux
```

`rust-toolchain.toml` declares the `llvm-tools` component, so a `rustup`-managed
toolchain resolves it automatically from the repo root.

## Adding Coverage for New Code

1. Write unit tests alongside the implementation (`#[cfg(test)] mod tests`).
2. Run `cargo llvm-cov --all-targets` in `contracts/creditra-credit` and check
   which lines are reported as uncovered.
3. Run the full suite before pushing:
   `cargo llvm-cov --all-targets --fail-under-lines 92`.
4. If the measured percentage rose, raise `MIN_LINE_COVERAGE` in
   `.github/workflows/ci.yml` in the same pull request.

## Excluding Code from Coverage

`cargo-llvm-cov` is run with no `--ignore-filename-regex` or `--ignore-filename`
filters, so every line in the crate counts toward the denominator.
An exclusion is a deliberate, reviewable change to the floor, not a convenience.

The workspace already recognizes `cfg(coverage)` and `cfg(coverage_nightly)`
cfg keys.

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `error: no 'cargo-llvm-cov' found` | Tool not installed | `cargo install cargo-llvm-cov --version 0.9.1 --locked` |
| `error: Failed to find a suitable version of llvm-tools` | `llvm-tools` component missing for the active toolchain | `rustup component add llvm-tools --toolchain 1.97.1` |
| Stale `*.profraw` files | Previous run artifacts | `scripts/clean_profraw.sh` |
| `error: package X is specified twice in the lockfile` | A standalone nested workspace package is also a root member | See the root-workspace note above; the root `Cargo.lock` does not currently parse |
| Coverage below the floor | Untested new code | Add tests for the uncovered lines, then raise `MIN_LINE_COVERAGE` |
