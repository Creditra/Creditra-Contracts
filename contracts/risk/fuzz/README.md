# Risk Admin Cooldown (v7) Cargo Fuzz Target

This directory contains the `cargo-fuzz` target for testing the Creditra Risk Admin Cooldown contract (`creditra-risk`, v7).

## Fuzz Target

- **`main`** (`targets/main.rs`): Generates arbitrary state-transition sequences (`RiskAction`) and executes them against a simulated Soroban `Env`, checking structural invariants, authorization rules, circuit breaker bounds, and overflow safety on every operation.

## Invariants Verified

1. **Auth Enforcement**: Every state-changing entrypoint (`init`, `set_risk_admin_cooldown`, `set_paused`, `record_risk_admin_action`) requires valid admin authorization.
2. **Cooldown Enforcement**: Critical admin actions fail during an active cooldown window and succeed once the cooldown elapses.
3. **Pause Circuit Breaker**: State mutations are blocked when paused, while read-only queries and unpausing remain available.
4. **First Action Invariant**: Initial actions always succeed regardless of configured cooldown.
5. **Overflow Safety**: All timestamp and cooldown math uses overflow-safe arithmetic without panicking.
6. **No Unwraps**: Production call paths do not panic unexpectedly on arbitrary fuzz inputs.

## Running the Fuzzer

```bash
cargo fuzz run --manifest-path contracts/risk/fuzz/Cargo.toml main -- -max_total_time=60
```
