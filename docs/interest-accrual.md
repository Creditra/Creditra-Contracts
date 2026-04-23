  # Interest Accrual Design

**Version: 2026-04-22 (Final Implementation)**

This document captures the intended design for issue `#119`: introduce deterministic interest accrual for the `credit` contract without breaking existing storage or indexer assumptions.

## Current baseline

- `CreditLineData` already stores `accrued_interest` and `last_accrual_ts`.
- No contract entrypoint currently applies time-based accrual to `utilized_amount`.
- Existing flows treat `utilized_amount` as principal-only outstanding debt.

## Goals

- Accrue interest only on outstanding borrowed principal.
- Keep accrual deterministic from Soroban ledger timestamps.
- Avoid background jobs or per-ledger iteration.
- Preserve backward compatibility for existing lines with `last_accrual_ts == 0`.
- Emit an explicit event whenever interest is materialized on-chain.

## Non-goals

- Compounding every ledger close.
- Variable-rate interpolation between historical rate changes.
- Off-chain oracle inputs for time or rate calculation.
- Penalty interest for defaulted positions in the first version.

## Proposed accounting model

- Interest uses simple per-second accrual derived from the annual `interest_rate_bps`.
- Accrual is lazy: it is computed only when a credit line is touched by a state-changing operation or by an explicit accrual entrypoint added later.
- Newly accrued interest is capitalized into debt and tracked in `accrued_interest`.

Formula:

```text
elapsed_seconds = now - last_accrual_checkpoint
annual_rate = interest_rate_bps / 10_000
accrued = floor(utilized_amount * annual_rate * elapsed_seconds / SECONDS_PER_YEAR)
```

With integer math:

```text
accrued = floor(utilized_amount * interest_rate_bps * elapsed_seconds / (10_000 * 31_536_000))
```

`SECONDS_PER_YEAR` is fixed at `31_536_000` (`365 * 24 * 60 * 60`).

## Rounding and overflow policy

- Use floor rounding toward zero.
- If `utilized_amount == 0`, accrual returns zero and only the checkpoint advances when appropriate.
- All multiplication/division paths must use checked math and revert with `ContractError::Overflow` on overflow.
- Fractional dust remains unmaterialized until enough time elapses to produce at least `1` unit.

## Accrual checkpoint rules

- For a newly opened or reopened line:
  - `accrued_interest = 0`
  - `last_accrual_ts = 0`
- On the first accrual-aware mutation of a line with `last_accrual_ts == 0`:
  - set `last_accrual_ts = now`
  - do not back-charge historical interest before the feature existed
- On subsequent accrual-aware mutations:
  - compute elapsed time from `last_accrual_ts`
  - add materialized interest to both `utilized_amount` and `accrued_interest`
  - update `last_accrual_ts = now`

## Methods that should trigger accrual

Before their main state mutation, these methods should settle pending interest:

- `draw_credit`
- `repay_credit`
- `update_risk_parameters`
- `close_credit_line`
- `default_credit_line`
- `reinstate_credit_line`

For `suspend_credit_line`, either behavior is defensible:

- Accrue on suspension so the status checkpoint and financial checkpoint align.
- Skip accrual on suspension because no financial balance changes.

Recommended choice: accrue on suspension for consistency across lifecycle mutations.

## Status-specific behavior

- `Active`: accrues normally.
- `Suspended`: accrues normally; suspension blocks new draws, not time.
- `Defaulted`: v1 should continue accruing at the same contractual rate unless a later policy adds default penalties.
- `Closed`: never accrues.
- `Restricted`: accrues normally while debt remains above the reduced limit.

## Event model

When positive interest is materialized, emit `("credit", "accrue")` with:

- `borrower`
- `accrued_amount`
- `total_accrued_interest`
- `new_utilized_amount`
- `timestamp`

No event is required when elapsed time produces zero newly materialized interest.

## Query behavior

Two acceptable models exist:

- Stored-value queries only: `get_credit_line` returns persisted balances and does not simulate accrual.
- Preview queries: add a separate view function later that returns hypothetical accrued debt at `now`.

Recommended choice: keep `get_credit_line` as stored-value only and add a separate preview method later if needed. That avoids hidden mutation semantics in read paths and keeps indexer behavior simple.

## Migration and backward compatibility

- Existing lines with `last_accrual_ts == 0` must not accrue from contract creation time or from an unknown historical timestamp.
- The first post-upgrade touch establishes the checkpoint.
- Existing event schemas remain valid; `InterestAccruedEvent` is additive.

## Test plan

Required tests for implementation:

- Zero utilization does not accrue interest.
- First post-upgrade touch initializes `last_accrual_ts` without retroactive charges.
- Positive elapsed time accrues the expected floored amount.
- Repeated accrual checkpoints do not double-count elapsed time.
- Repayment after accrual reduces the post-accrual debt, not just principal.
- Closed lines never accrue.
- Suspended and defaulted lines accrue according to the chosen policy.
- Overflow paths revert deterministically.
- `InterestAccruedEvent` payload matches stored results.
- Repayments apply to interest first.

## Interest-First Repayment

The contract implements an "interest-first" repayment policy. When a borrower repays:
1. The repayment amount is first compared against the `accrued_interest` balance.
2. `accrued_interest` is reduced by `min(repayment_amount, accrued_interest)`.
3. `utilized_amount` is reduced by the full repayment amount (clamped at zero).

This ensures that capitalized interest is always settled before principal, which is a standard financial practice.

## Open decisions

- Whether `suspend_credit_line` should settle accrual before changing status.
- Whether defaulted lines should accrue contractual rate or a separate penalty rate in a later issue.
- Whether a dedicated `accrue_interest` admin/user entrypoint is needed for proactive settlement.
