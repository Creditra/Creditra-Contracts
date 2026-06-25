# Contributing Tests

This guide covers test-only helpers used in `contracts/credit/src/lib.rs` for
draw/repay integration scenarios.

## Liquidity Test Helpers

The main contract test module keeps liquidity setup lightweight with helper
functions around the real Soroban token client rather than a separate fake
token implementation.

Use these helpers in `contracts/credit/src/lib.rs` when a test needs to model
balance changes across multiple calls:
- `setup(...)` to deploy the contract, configure the liquidity token, and seed
	the initial reserve;
- `mint_liquidity(...)` to top up the reserve or borrower between calls;
- `liquidity_balance(...)` to assert reserve depletion and repayment effects;
- `approve(...)` for repay-path allowance setup.

## When To Use It

- Draw scenarios that need explicit reserve funding checks.
- Repay scenarios that need borrower balance/allowance fixtures.
- Any new integration-style test that currently duplicates token setup code.

## Reserve Depletion Sequences

Reserve-sensitive draw regressions should snapshot both state and events around
the failing call:
- perform one successful draw to consume part of the reserve;
- record `utilized_amount`, `last_accrual_ts`, and event counts;
- attempt a second draw that exceeds the remaining reserve;
- assert the panic message, unchanged reserve balance, unchanged stored credit
	line fields, and no additional `drawn` or `accrue` events.

Cover both a single borrower issuing sequential draws and multiple borrowers
sharing the same reserve so shared-liquidity regressions are caught.

## Reentrancy guard lifecycle (`token_failure_rollback.rs`)

Integration tests in `contracts/credit/tests/token_failure_rollback.rs` assert
that `draw_credit` / `repay_credit` clear the reentrancy guard after both
pre-transfer validation failures and mid-transfer CPI failures:

```bash
cargo test -p creditra-credit --test token_failure_rollback rollback
```

- **Pre-transfer failures** use the real Stellar asset contract (insufficient
  reserve / allowance) with `catch_unwind` to continue the same test after panic.
- **Mid-transfer failures** use the in-test `FailingTokenContract` mock (internal
  balances, configurable `set_fail_transfer` / `set_fail_transfer_from`) for
  draw-fail-then-draw and repay-fail-then-repay sequencing.

## Scope Boundary

`MockLiquidityToken` is test-only (`#[cfg(test)]`) and must not be imported
into contract runtime logic.

## SimplePriceOracle mock (`tests/mocks/simple_price_oracle/`)

Cross-contract default-liquidation tests deploy this minimal `#[contract]`
oracle to mirror the off-chain flow of reading a price before calling
`settle_default_liquidation`:

```bash
cargo test -p simple-price-oracle
cargo test -p creditra-credit oracle::with_mock
```

- Deploy with `env.register(SimplePriceOracle, ())`, then `init` + `set_price`.
- Read `get_price()` and pass the value as `Some(price)` into settlement.
- `set_price` is admin-guarded; see `contracts/credit/tests/oracle/with_mock.rs`.
