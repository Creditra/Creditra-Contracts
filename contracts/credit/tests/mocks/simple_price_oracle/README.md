# SimplePriceOracle mock

Deployable Soroban contract used by credit integration tests that exercise
default liquidation with an external price feed.

## Why

`Credit::settle_default_liquidation` accepts an optional `oracle_price` when
oracle circuit-breaker config is enabled. Cross-contract tests need a real
`#[contract]` deployment to mirror the off-chain flow:

1. Deploy oracle mock
2. Admin sets price
3. Test reads `get_price()` and passes it into settlement

## Build

The crate is a workspace member and produces WASM via `cdylib`:

```bash
cargo build -p simple-price-oracle --target wasm32-unknown-unknown --release
```

## Run unit tests

```bash
cargo test -p simple-price-oracle
```

## Run credit integration tests that use the mock

```bash
cargo test -p creditra-credit oracle::with_mock
```

## Security notes

- `set_price` requires the stored admin to authorize the call
- `init` is idempotent-guarded (second call panics)
- No privileged `get_price` path — read access is intentionally public for test ergonomics
