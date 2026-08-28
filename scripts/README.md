# scripts/

Helper scripts for local development and CI of the Creditra Soroban
contracts. None of the files here are compiled into the contract WASM —
they are operator-facing utilities only.

## Inventory

| Script | Purpose |
| ------ | ------- |
| `build_wasm.sh` | Compile both workspace contracts to `target/wasm32-unknown-unknown/release/*.wasm`. Asserts the reproducible-build policy (pinned toolchain + `--verify-active`) and builds `--locked`. |
| `check-wasm-size.sh` | Build (optional) and fail when any release WASM exceeds **100 KiB** (`THRESHOLD_BYTES=102400`). |
| `test_check_wasm_size.sh` | Focused guard tests for `check-wasm-size.sh` (synthetic artifacts, no build). |
| `check-toolchain.sh` | Enforce the reproducible-build policy: exact toolchain pin in `rust-toolchain.toml`, required targets/components, CI workflow consumes the pin, lock files committed. `--verify-active` additionally fails when the active `rustc` does not match the pin. |
| `test_check_toolchain.sh` | Focused guard tests for `check-toolchain.sh` (synthetic fixtures, no toolchain install). |
| `clean_profraw.sh` | Remove stray `*.profraw` coverage files left over by `cargo llvm-cov`. |
| `check_workspace.sh` | Convenience wrapper around `cargo check --workspace --locked`. |
| `list_contract_errors.py` | Print every `ContractError` variant declared in `contracts/credit/src/types.rs` with its discriminant. |
| `gas-regression.sh` | Run per-entrypoint budget regression tests (or regenerate baselines with `--regen`). |
| `regen_budget_baseline.sh` | Regenerate `contracts/credit/test_snapshots/budget.json` via the `budget_baseline` example. |

## Conventions

- Shell scripts target `bash` and use `set -euo pipefail`.
- Python scripts target Python 3.9+ and have no third-party deps.
- Scripts must be runnable from any working directory; they cd to the
  repo root themselves.

## Reproducible builds

Builds must be byte-for-byte repeatable across machines and over time.
Three invariants keep them that way, all enforced by `check-toolchain.sh`
and CI:

1. **Pinned compiler.** `rust-toolchain.toml` pins `channel` to an exact
   `X.Y.Z` version. Floating channels (`stable`, `beta`, `nightly`) are
   rejected — they resolve to different compilers on different days.
2. **Pinned dependencies.** Every build uses `--locked` against committed
   `Cargo.lock` files, so dependency resolution cannot drift.
3. **One source of truth.** CI derives its toolchain from
   `rust-toolchain.toml` (never a floating action ref), and
   `--verify-active` fails when the active `rustc` does not match the pin
   (stray overrides, `RUSTUP_TOOLCHAIN`, rustup-less environments).

To upgrade the toolchain, bump `channel` deliberately and re-run the WASM
size baselines in the same commit.
