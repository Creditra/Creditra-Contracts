# scripts/

Helper scripts for local development and CI of the Creditra Soroban
contracts. None of the files here are compiled into the contract WASM —
they are operator-facing utilities only.

## Inventory

| Script | Purpose |
| ------ | ------- |
| `build_wasm.sh` | Compile both workspace contracts to `target/wasm32-unknown-unknown/release/*.wasm`. |
| `check-wasm-size.sh` | Build (optional) and fail when any release WASM exceeds **100 KiB** (`THRESHOLD_BYTES=102400`). |
| `test_check_wasm_size.sh` | Focused guard tests for `check-wasm-size.sh` (synthetic artifacts, no build). |
| `clean_profraw.sh` | Remove stray `*.profraw` coverage files left over by `cargo llvm-cov`. |
| `check_workspace.sh` | Convenience wrapper around `cargo check --workspace`. |
| `list_contract_errors.py` | Print every `ContractError` variant declared in `contracts/credit/src/types.rs` with its discriminant. |
| `gas-regression.sh` | CI/regen orchestrator for the per-entrypoint CPU/memory budget regression. |
| `regen_budget_baseline.sh` | Regenerate `contracts/credit/test_snapshots/budget.json` from live measurements. |
| `wasm-size-baseline.sh` | Default: `--regen` (rebuild release WASM and overwrite `scripts/wasm-size-baseline.json`, then `git diff`). `--no-diff` keeps the regen write but suppresses the diff. `--check` is the CI guardrail: builds once, compares each crate's size to the baseline, fails on `delta > tolerance_bytes`, emits a `::warning::` on `delta < -tolerance_bytes`, and emits a `::notice::` for any non-zero within-tolerance drift. |

## Conventions

- Shell scripts target `bash` and use `set -euo pipefail`.
- Python scripts target Python 3.9+ and have no third-party deps.
- Scripts must be runnable from any working directory; they cd to the
  repo root themselves.
