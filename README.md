# Creditra Contracts

Soroban smart contracts for the Creditra adaptive credit protocol on Stellar.

## About

This repo contains the **credit** contract: it maintains credit lines, tracks utilization, enforces limits, and exposes methods for opening lines, drawing, repaying, and updating risk parameters. Token transfers and interest accrual are still TODO.

**Behavior notes:**

- after `suspend_credit_line`, `draw_credit` for that borrower reverts
- `repay_credit` remains allowed while suspended
This repo contains the **credit** contract: it will maintain credit lines, track utilization, enforce limits, and expose methods for opening lines, drawing, repaying, and updating risk parameters. The contract includes core logic for state management and events; additional features like interest accrual and token transfers are future enhancements.

**Contract data model:**

- `CreditStatus`: Active, Suspended, Defaulted, Closed
- `CreditLineData`: borrower, credit_limit, utilized_amount, interest_rate_bps, risk_score, status

**Methods:** `init`, `open_credit_line`, `draw_credit`, `repay_credit`, `update_risk_parameters`, `suspend_credit_line`, `close_credit_line`, `default_credit_line`, `get_credit_line`.

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

```bash
cd creditra-contracts
cargo build --release -p creditra-credit
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

Avoid large dependencies; prefer minimal use of the Soroban SDK surface to stay within practical Soroban deployment limits.

### Run tests

```bash
cargo test -p creditra-credit
```

### Deploy (with Soroban CLI)

Once the Soroban CLI and a network are configured:

```bash
make build
```

Or manually:

```bash
soroban contract deploy --wasm target/wasm32-unknown-unknown/release/creditra_credit.wasm --source <identity> --network <network>
```

See [Stellar Soroban docs](https://developers.stellar.org/docs/smart-contracts) for details.

- `contracts/credit/` — credit line contract
  - `Cargo.toml` — crate config, soroban-sdk dependency
  - `src/lib.rs` — contract types and implementation
  - `src/lib.rs` — contract types and impl

## Testing and Coverage

### Run tests

```bash
make test
```

### Test Coverage

This project uses `cargo-tarpaulin` to generate test coverage reports. We enforce a minimum of **95%** line coverage.

#### Local Coverage Report

To run coverage locally, install `cargo-tarpaulin`:

```bash
cargo install cargo-tarpaulin
```

Then run the coverage command:

```bash
make coverage
```

The report will be generated in `coverage/tarpaulin-report.html`.

#### Maintaining Coverage

To maintain high coverage:
1. Ensure every new function has corresponding unit tests.
2. Test both success and failure (panic) paths using `#[should_panic]`.
3. Use `cargo tarpaulin --fail-under 95` as a gatekeeper before merging.

This repo is a standalone git repository. After adding your remote:

```bash
git remote add origin <your-creditra-contracts-repo-url>
git push -u origin main
```
