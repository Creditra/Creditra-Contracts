# Lifecycle (v7) Cargo Fuzz Target

This directory contains the `cargo-fuzz` target for testing the Creditra credit-line lifecycle subsystem (v7).

## Fuzz Target

- **`lifecycle`** (`targets/main.rs`): Generates arbitrary state-transition sequences (`LifecycleOp`) and executes them against a simulated Soroban `Env`, checking structural invariants on every operation.

## Invariants Verified

1. **No panics**: Every lifecycle call either succeeds or terminates with a valid `ContractError` discriminant.
2. **Valid status**: Credit line status is always one of `Active`, `Suspended`, `Defaulted`, `Closed`, or `Restricted`.
3. **Closed is terminal**: Once `CreditStatus::Closed` is reached, the line cannot transition back to any non-closed status.
4. **Overflow safety**: `utilized_amount`, `accrued_interest`, and `credit_limit` remain non-negative and bounded.
5. **Compile-time discriminant pins**: Pins all lifecycle-relevant `ContractError` discriminants at compile time.

## Running the Fuzzer

```bash
cargo fuzz run --manifest-path contracts/lifecycle/fuzz/Cargo.toml lifecycle -- -max_total_time=60
```
