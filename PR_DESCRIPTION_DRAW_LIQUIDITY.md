# feat(credit): enforce liquidity reserve on draw

## Summary

Adds a reserve-enforcement gate to `draw_credit` in `contracts/credit/src/lib.rs`.

- Reads the configured liquidity source balance through the Stellar Asset Contract interface before any draw transfer.
- Reverts with `ContractError::InsufficientLiquidity` (15) when reserve balance is lower than the requested draw amount.
- Preserves borrower accounting on failure because the reserve check runs before utilization is updated.
- Keeps draw transfers SAC-compatible by using `balance` and `transfer`.

## Tests

Added reserve-focused tests in `contracts/credit/src/lib.rs`:

- `draw_credit_with_exact_reserve_balance_succeeds`
- `draw_credit_reverts_when_reserve_is_underfunded`
- `draw_credit_uses_external_reserve_balance_and_transfer_path`

## Test Output

```text
$ cargo test -p creditra-credit
Downloading crates ... done
error: linker `link.exe` not found
note: the msvc targets depend on the msvc linker but `link.exe` was not found
```

Dependency download succeeded after enabling network access, but the local environment could not complete compilation because the MSVC linker is not installed or not available on `PATH`.

## Security Notes

- Assumptions: the configured liquidity token follows the Stellar Asset Contract interface for `balance` and `transfer`.
- Trust boundaries: admin-controlled `LiquidityToken` and `LiquiditySource` remain privileged configuration and must be set correctly.
- Failure modes: an underfunded reserve blocks draws; a malicious or non-standard token contract can still misreport balances or fail transfers; an incorrect reserve address can strand or misroute liquidity.
