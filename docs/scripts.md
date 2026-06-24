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
prints the resulting wasm file paths on completion.

> **Prerequisite:** `rustup target add wasm32-unknown-unknown`.

## `scripts/check_workspace.sh`

Thin wrapper around `cargo check --workspace`. Extra arguments are forwarded:

```bash
scripts/check_workspace.sh --all-targets
scripts/check_workspace.sh --release -p creditra-credit
```

Useful as a hook target so contributors and CI invoke the same command.

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
scripts/list_contract_errors.py            # text table
scripts/list_contract_errors.py --json     # machine-readable
```

The JSON output is convenient for keeping SDK / indexer error tables in
sync with the contract source of truth.

## Conventions

- Shell scripts target `bash` and use `set -euo pipefail`.
- Python scripts target Python 3.9+ with the standard library only.
- All scripts are runnable from any cwd; they `cd` to the repo root first.
- Scripts must remain side-effect free against `target/` (they never run
  destructive `cargo clean`, never touch `Cargo.lock`).
