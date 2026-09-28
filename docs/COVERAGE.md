# Coverage Guide

## Overview

The Creditra workspace enforces **minimum 95% line coverage** via `cargo-llvm-cov` in CI.

## CI Enforcement

One workflow enforces coverage:

| Workflow | Job | Trigger | Command |
|---|---|---|---|
| `ci.yml` | `coverage` | Push/PR to `main`, `master`, `develop` | `cargo llvm-cov --workspace --all-targets --locked --fail-under-lines 95 --html` |

The job uses `--fail-under-lines 95` to gate the pipeline. If coverage drops below 95%, the workflow exits non-zero and blocks the CI.

The job also runs `scripts/check-toolchain.sh --verify-active --lock Cargo.lock`, so the coverage run resolves the committed workspace lockfile on the toolchain pinned in `rust-toolchain.toml`.
`llvm-tools-preview` is declared in that same pin, which is what `cargo-llvm-cov` needs to link the LLVM profile runtime.

## Report Artefact

The HTML report is published as the `coverage-html` workflow artefact (`target/llvm-cov/html`) instead of being committed to the repository.
Download it from the run summary on the Actions page; the upload step runs with `if: always()` so the measurement that explains a failing gate is still available.

Because the report is generated per run, `coverage/` is git-ignored and no longer tracked.
The README badge is served by the Actions workflow status API for the `coverage` job, so it cannot drift from the last real measurement.

## Running Locally

```bash
# Install the tool (one-time)
cargo install cargo-llvm-cov

# Run coverage across the workspace
cargo llvm-cov --workspace --all-targets

# Enforce threshold
cargo llvm-cov --workspace --all-targets --fail-under-lines 95

# Generate HTML report
cargo llvm-cov --workspace --all-targets --html

# Generate LCOV (for IDE plugins or external tools)
cargo llvm-cov --workspace --all-targets --lcov --output-path lcov.info
```

Open `target/llvm-cov/html/index.html` in a browser for the interactive report.

## Adding Coverage for New Code

1. Write unit tests alongside the implementation (`#[cfg(test)] mod tests`).
2. Run `cargo llvm-cov` to verify untested lines are covered.
3. For Soroban entrypoints that require `Env`, write integration tests in `contracts/credit/tests/`.
4. Run the full workspace suite before pushing: `cargo llvm-cov --workspace --all-targets`.

## Excluding Code from Coverage

Use conditional compilation for coverage-only annotations:

```rust
// When a branch cannot be hit in practice, mark it explicitly.
// For blocks that should be excluded from coverage:
#[cfg(not(coverage))]
```

The workspace already recognizes `cfg(coverage)` and `cfg(coverage_nightly)` lint keys.

## Interpreting the Gate

The gate measures every target in the workspace, so a regression anywhere in the workspace, not only in the contract crate, can drop the total below the floor.
When a run fails, read the per-file line percentages in the uploaded `coverage-html` report before assuming the change you just made caused it.

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `error: no 'cargo-llvm-cov' found` | Tool not installed | `cargo install cargo-llvm-cov` |
| Stale `*.profraw` files | Previous run artifacts | `scripts/clean_profraw.sh` |
| Coverage below 95% | Untested new code | Add tests for uncovered lines |
| `error: failed to parse lock file` | Committed `Cargo.lock` is not a valid lockfile | Re-resolve with `cargo update --workspace`, or repair the offending entry |
| No report in the artefact list | The run failed before instrumentation, so no profile data was produced | Read the failing step; the coverage step reports the actual error |

