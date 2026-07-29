# Coverage Guide

## Overview

The Creditra workspace enforces a **minimum 95 % line coverage** hard gate via
`cargo-llvm-cov` in CI.  A push or PR whose line coverage drops below 95 %
will have its CI check fail with a non-zero exit code.  The gate cannot be
bypassed by workflow configuration — all coverage jobs run without
`continue-on-error` on the enforcement step.

## CI Enforcement (PR #798)

Three workflows enforce coverage.  All three use the same command for the
**hard gate** step:

```bash
cargo llvm-cov --workspace --all-targets --fail-under-lines 95
```

| Workflow | Trigger | Hard gate step | Soft steps |
|---|---|---|---|
| `coverage.yml` | Push/PR to `main`, `master` | `Enforce 95 % line coverage (hard gate)` | Codecov upload, LCOV artifact |
| `ci.yml` (coverage job) | Push/PR to `main`, `master`, `develop`, `feature/**` | `Enforce line coverage ≥ 95 %` | LCOV artifact upload |
| `pr-coverage.yml` | PR to `main`, `master`, `develop` | `Enforce minimum 95 % line coverage` | LCOV artifact, step summary |

The gate step on every workflow has **no** `continue-on-error`.  Only the
optional Codecov upload and LCOV artifact upload steps carry
`continue-on-error: true` so that transient network issues or a missing
`CODECOV_TOKEN` secret do not accidentally block the gate.

### History

Before PR #798, all three workflows set `continue-on-error: true` on every
step, including the enforcement step.  This meant that a coverage drop below
95 % produced a yellow warning in the GitHub Actions UI but **never** failed
the check and never blocked a merge.  PR #798 removed `continue-on-error`
from the hard gate steps, making the 95 % threshold a real blocker.

## Running Locally

```bash
# Install the tool (one-time)
cargo install cargo-llvm-cov

# Run coverage across the workspace
cargo llvm-cov --workspace --all-targets

# Enforce threshold — exits 1 if coverage < 95 %
cargo llvm-cov --workspace --all-targets --fail-under-lines 95

# Generate HTML report (open target/llvm-cov/html/index.html)
cargo llvm-cov --workspace --all-targets --html

# Generate LCOV (for IDE plugins or external tools)
cargo llvm-cov --workspace --all-targets --lcov --output-path lcov.info
```

## Adding Coverage for New Code

1. Write unit tests alongside the implementation (`#[cfg(test)] mod tests`).
2. Run `cargo llvm-cov` to verify untested lines are covered.
3. For Soroban entrypoints that require `Env`, write integration tests in
   `contracts/credit/tests/`.
4. Run the full workspace suite before pushing:
   `cargo llvm-cov --workspace --all-targets --fail-under-lines 95`

## Coverage Gate Test Artifact

`contracts/credit/tests/coverage_gate.rs` (added in PR #798) documents the
95 % threshold as a first-class Rust constant:

```rust
/// The minimum line coverage threshold enforced by CI.
const CI_COVERAGE_THRESHOLD_PCT: u32 = 95;
```

This file also exercises edge cases in `math_utils` that were identified as
the remaining ≈ 1 % gap, and contains property assertions that the gate
arithmetic is correct.

## Excluding Code from Coverage

Use conditional compilation for coverage-only annotations:

```rust
// When a branch cannot be hit in practice, mark it explicitly.
#[cfg(not(coverage))]
```

The workspace recognises `cfg(coverage)` and `cfg(coverage_nightly)` lint
keys.

## Current Coverage

| Metric | Value |
|---|---|
| Lines | **98.94 %** |
| Regions | **99.51 %** |
| Gate threshold | **95.00 %** |

See the `coverage/` directory for the latest HTML report or the CI artifact
for the most recent LCOV file.

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `error: no 'cargo-llvm-cov' found` | Tool not installed | `cargo install cargo-llvm-cov` |
| Stale `*.profraw` files | Previous run artifacts | `scripts/clean_profraw.sh` |
| Coverage below 95 % | Untested new code | Add tests for uncovered lines |
| LCOV upload fails | Missing `CODECOV_TOKEN` secret | Set in GitHub repo → Settings → Secrets |
| CI check fails (red) | Coverage < 95 % | Investigate LCOV artifact; add tests |
| CI check yellow | Coverage ≥ 95 % but Codecov upload failed | Transient; set token if persistent |
