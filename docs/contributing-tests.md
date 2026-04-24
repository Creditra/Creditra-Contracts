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

## Scope Boundary

`MockLiquidityToken` is test-only (`#[cfg(test)]`) and must not be imported
into contract runtime logic.
