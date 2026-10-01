# Helper scripts

This document explains the helper scripts under `scripts/`. None of these
are required to build or run the contract; they exist to keep common chores
reproducible across machines.

## `scripts/build_wasm.sh`

Builds the Soroban contracts to `wasm32-unknown-unknown` release artifacts.

```bash
scripts/build_wasm.sh            # both contracts
scripts/build_wasm.sh credit     # creditra-credit only
scripts/build_wasm.sh auction    # gateway-auction only
```

Output lives at `target/wasm32-unknown-unknown/release/*.wasm`. The script
prints the resulting wasm file paths on completion. Before building it
asserts the release overflow policy via `scripts/check-overflow-checks.sh`,
so a release build cannot silently disable overflow checks.

> **Prerequisite:** `rustup target add wasm32-unknown-unknown`.

## `scripts/check_workspace.sh`

Thin wrapper around `cargo check --workspace`. It runs
`scripts/check-overflow-checks.sh` first, so the release overflow policy is
asserted before anything compiles. Extra arguments are forwarded:

```bash
scripts/check_workspace.sh --all-targets
scripts/check_workspace.sh --release -p creditra-credit
```

Useful as a hook target so contributors and CI invoke the same command.

## `scripts/check-overflow-checks.sh`

Fails when `overflow-checks` is missing or not `true` in a release profile.
The release WASM must revert on arithmetic overflow instead of wrapping, and
`README.md` states that guarantee, so this guard keeps the statement honest:

```bash
scripts/check-overflow-checks.sh                  # every release profile
scripts/check-overflow-checks.sh --manifest Cargo.toml
```

Checked profiles are the root workspace `Cargo.toml` and the standalone
`contracts/creditra-credit/Cargo.toml`. The guard is also invoked by
`scripts/check_workspace.sh` and `scripts/build_wasm.sh`.

## `scripts/check-wasm-size.sh`

Builds every workspace contract WASM (via `build_wasm.sh`) and fails if **any**
release artifact exceeds the size budget. The default limit is **100 KiB**
(`THRESHOLD_BYTES=102400`), enforced in CI by `.github/workflows/wasm-size.yml`.

```bash
scripts/check-wasm-size.sh              # build + verify all *.wasm
scripts/check-wasm-size.sh --check-only # verify artifacts already in target/
THRESHOLD_BYTES=102400 scripts/check-wasm-size.sh
```

Scans `target/wasm32-unknown-unknown/release/*.wasm` (override with `WASM_DIR`).

Companion self-test (no contract build):

```bash
scripts/test_check_wasm_size.sh
```

## `scripts/clean_profraw.sh`

Removes stray `*.profraw` coverage files that pile up outside `target/`
when running `cargo llvm-cov` interrupted mid-run. The script never touches
files under `target/`.

```bash
scripts/clean_profraw.sh             # delete
scripts/clean_profraw.sh --dry-run   # report-only
```

## `scripts/list_contract_errors.py`

Parses `contracts/credit/src/types.rs` and prints every `ContractError`
variant with its discriminant. Has no third-party dependencies and runs on
Python 3.9+.

```bash
scripts/list_contract_errors.py                # text table
scripts/list_contract_errors.py --json         # machine-readable
scripts/list_contract_errors.py --categories   # grouped by category
scripts/list_contract_errors.py --check        # verify docs/errors.md
```

`--check` is the guard for the canonical error reference: it fails if
[`docs/errors.md`](./errors.md) publishes a code the contract cannot emit, or
omits / misnames / miscategorizes one that it can. `tests/error_discriminants.rs`
asserts the same invariant, so the published table cannot drift from the enum.

## `scripts/gas-regression.sh`

Runs the budget regression tests for the `credit` contract, comparing observed resource usage (CPU/memory) against the pinned baselines in `contracts/.gas-baseline.json`. It fails if any measurement drifts beyond the configured tolerance.

```bash
scripts/gas-regression.sh             # run tests (used in CI)
scripts/gas-regression.sh --regen     # regenerate baselines, then test
scripts/gas-regression.sh --regen-only  # regenerate baselines only
```

To regenerate the baseline, run `scripts/gas-regression.sh --regen-only`. This overwrites `contracts/.gas-baseline.json` with fresh numbers. Review the diff and commit the updated baseline.

## Conventions

- Shell scripts target `bash` and use `set -euo pipefail`.
- Python scripts target Python 3.9+ with the standard library only.
- All scripts are runnable from any cwd; they `cd` to the repo root first.
- Scripts must remain side-effect free against `target/` (they never run
  destructive `cargo clean`, never touch `Cargo.lock`).
