# Creditra Contracts

Core smart contracts for the Creditra protocol, managing credit lines, draw operations, repayments, and risk parameters.

This repo contains the **credit** contract: it maintains credit lines, tracks utilization, enforces limits, and exposes methods for opening lines, drawing, repaying, and updating risk parameters. Draw logic includes a liquidity reserve check and token transfer flow.

**Contract data model:**

- `CreditStatus`: Active, Suspended, Defaulted, Closed, Restricted
- `CreditLineData`: borrower, credit_limit, utilized_amount, interest_rate_bps, risk_score, status, last_rate_update_ts, accrued_interest, last_accrual_ts

**Behavior notes:**
- after `suspend_credit_line`, `draw_credit` for that borrower reverts
- after `default_credit_line`, `draw_credit` reverts and `repay_credit` remains allowed
- `repay_credit` remains allowed while suspended or defaulted
- `freeze_draws` globally blocks all `draw_credit` calls without mutating any borrower's `CreditStatus`; `repay_credit` is never affected by the freeze flag

**Methods:** `init`, `set_liquidity_token`, `set_liquidity_source`, `open_credit_line`, `draw_credit`, `repay_credit`, `update_risk_parameters`, `suspend_credit_line`, `close_credit_line`, `default_credit_line`, `reinstate_credit_line`, `get_credit_line`, `freeze_draws`, `unfreeze_draws`, `is_draws_frozen`, `set_rate_change_limits`, `get_rate_change_limits`, `set_rate_formula_config`, `get_rate_formula_config`, `clear_rate_formula_config`.

### Liquidity reserve enforcement

- `draw_credit` now checks configured liquidity token balance at the configured liquidity source before transfer.
- If reserve balance is less than requested draw amount, the transaction reverts with: `Insufficient liquidity reserve for requested draw amount`.
- `init` defaults liquidity source to the contract address.
- `repay_credit` (when a liquidity token is configured) uses `transfer_from` to move tokens from the borrower to the configured liquidity source; borrowers must approve an allowance for the credit contract.
- Admin can configure:
  - `set_liquidity_token` — token contract used for reserve and draw transfers.
  - `set_liquidity_source` — reserve address to fund draws (contract or external source).

### Suspend credit line behavior

- `suspend_credit_line` is **admin only** and requires the credit line to exist.
- Only lines in `Active` status can be suspended.
- `draw_credit` rejects any draw when the line is not `Active` (including `Suspended`).
- Repayments are intended to remain allowed while suspended.

### Interest accrual design

- The contract already reserves `accrued_interest` and `last_accrual_ts` in storage for lazy interest accounting.
- The design note for implementing accrual is documented in [`docs/interest-accrual.md`](docs/interest-accrual.md).
- Current code does not yet apply periodic accrual to balances; the new document defines the intended behavior before implementation.

### Risk-score based rate formula

- Admin can enable an optional bounded piecewise-linear formula via `set_rate_formula_config(base_rate_bps, slope_bps_per_score, min_rate_bps, max_rate_bps)`.
- When enabled, `update_risk_parameters` automatically computes `interest_rate_bps` from the borrower's `risk_score`: `rate = clamp(base + score × slope, min, max)`.
- The computed rate always respects `MAX_INTEREST_RATE_BPS` (10,000 = 100%) and existing `RateChangeConfig` limits.
- When disabled (default or after `clear_rate_formula_config`), the manually supplied rate is used as before.
- Full formula documentation: [`docs/risk-based-rate-formula.md`](docs/risk-based-rate-formula.md).

## Workspace members

| Crate | Path | Purpose |
| ----- | ---- | ------- |
| `creditra-credit` | `contracts/credit/` | Primary credit-line contract: open/draw/repay/risk. |
| `gateway-auction` | `gateway-contract/contracts/auction_contract/` | Minimal auction contract consumed as a dev-dependency to exercise the credit contract's default-liquidation hook. |

Both crates target `wasm32-unknown-unknown` and share the workspace
release profile defined in the root `Cargo.toml`.

## Tech Stack

- **Rust** (edition 2021)
- **soroban-sdk** (Stellar Soroban)
- Build target: **wasm32** for Soroban

## Prerequisites

- Rust 1.75+ (recommend latest stable)
- `wasm32` target:

  ```bash
  rustup target add wasm32-unknown-unknown
  ```

- [Stellar Soroban CLI](https://developers.stellar.org/docs/smart-contracts/getting-started/setup) for deploy and invoke (optional for local build).

## Setup and build

### Build
```bash
cargo build
```

### WASM build (release profile, size-optimized)

The workspace uses a release profile tuned for contract size (opt-level `"z"`, LTO, strip symbols). To build the contract for Soroban:

```bash
rustup target add wasm32-unknown-unknown
cargo build --release --target wasm32-unknown-unknown -p creditra-credit
```

WASM output is at `target/wasm32-unknown-unknown/release/creditra_credit.wasm`. Size is kept small by:

- `opt-level = "z"` (optimize for size)
- `lto = true` (link-time optimization)
- `strip = "symbols"` (no debug symbols in release)
- `codegen-units = 1` (better optimization)

CI enforces a size budget of 50 KB (`51200` bytes) for this artifact to ensure deployability and fast runtime.

Avoid large dependencies; prefer minimal use of the Soroban SDK surface to stay within practical Soroban deployment limits.

### Run tests

```bash
cargo test -p creditra-credit
```

### Coverage
```bash
cargo llvm-cov --workspace --all-targets --fail-under-lines 95
```

Current result:

- Regions: `99.51%`
- Lines: `98.94%`

This satisfies the 95% minimum coverage target.

## Helper scripts

The `scripts/` directory contains operator-facing utilities. None of the
scripts are required for the contract to compile or run — they exist to
keep common chores reproducible.

| Script | Use |
| ------ | --- |
| `scripts/build_wasm.sh [all\|credit\|auction]` | Build release-mode WASM artifacts. |
| `scripts/check_workspace.sh [args]` | `cargo check --workspace` wrapper, forwards extra args. |
| `scripts/clean_profraw.sh [--dry-run]` | Remove stray `*.profraw` coverage profiles outside `target/`. |
| `scripts/list_contract_errors.py [--json]` | Print every `ContractError` variant with its discriminant. |

See [`scripts/README.md`](scripts/README.md) for conventions.

## Security Documentation

- Threat model and trust assumptions: [`docs/threat-model.md`](docs/threat-model.md)

### Deploy (with Soroban CLI)

Once the Soroban CLI and a network are configured:

```bash
soroban contract deploy --wasm target/wasm32-unknown-unknown/release/creditra_credit.wasm --source <identity> --network <network>
```

See [Stellar Soroban docs](https://developers.stellar.org/docs/smart-contracts) for details.

## Project layout

- `Cargo.toml` — workspace and release profile (opt for contract size)
- `contracts/credit/` — credit line contract
  - `Cargo.toml` — crate config, soroban-sdk dependency
  - `src/lib.rs` — contract entrypoints (`#[contractimpl]`)
  - `src/types.rs` — `CreditLineData`, `ContractError`, config structs
  - `src/storage.rs` — persistent/instance storage keys and helpers
  - `src/auth.rs` — admin access control
  - `src/risk.rs` / `src/accrual.rs` — rate formula and interest accrual
  - `src/lifecycle.rs` — open/suspend/default/reinstate/close state transitions
  - `src/borrow.rs` / `src/collateral.rs` / `src/freeze.rs` — draw, collateral, freeze helpers
- `gateway-contract/contracts/auction_contract/` — auction contract used
  in credit-contract integration tests to exercise the default-liquidation hook
- `docs/` — protocol-level reference (errors, state-machine, threat model, etc.)
- `scripts/` — operator-facing helpers (build, coverage cleanup, introspection)

## Merging to remote

This repo is a standalone git repository. After adding your remote:

```bash
git remote add origin <your-creditra-contracts-repo-url>
git push -u origin main
```
