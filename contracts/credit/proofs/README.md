# Kani proof harnesses

This directory contains the model-checking harnesses for the credit protocol math and auction pricing logic.

Local validation:

```bash
cargo kani -p creditra-credit
cargo kani -p gateway-auction
```

The credit harnesses are included via `cfg(kani)` in the contract crate, and the auction harnesses are likewise compiled under the same flag in the auction crate. The CI workflow runs both commands with a bounded time budget so arithmetic regressions cannot slip through silently.
