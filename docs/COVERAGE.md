# Coverage Guide

## Overview

The Creditra workspace enforces **minimum 95% line coverage** via `cargo-llvm-cov` in CI.

## CI Enforcement

Three workflows enforce coverage:

| Workflow | Trigger | Command |
|---|---|---|
| `ci.yml` (coverage job) | Push/PR to `main`, `master`, `develop`, `feature/**` | `cargo llvm-cov --workspace --all-targets --fail-under-lines 95` |
| `coverage.yml` | Push/PR to `main`, `master` | Same + LCOV upload to Codecov |
| `pr-coverage.yml` | PR to `main`, `master`, `develop` | Same + PR summary comment |

All three use `--fail-under-lines 95` to gate the pipeline. If coverage drops below 95%, the workflow exits non-zero and blocks the CI.

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

## Current Coverage

The current line coverage is approximately **97–99%** across the workspace (see `coverage/` directory for the latest HTML report).

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `error: no 'cargo-llvm-cov' found` | Tool not installed | `cargo install cargo-llvm-cov` |
| Stale `*.profraw` files | Previous run artifacts | `scripts/clean_profraw.sh` |
| Coverage below 95% | Untested new code | Add tests for uncovered lines |
| LCOV upload fails | Missing `CODECOOK_TOKEN` secret | Set in GitHub repo settings |
