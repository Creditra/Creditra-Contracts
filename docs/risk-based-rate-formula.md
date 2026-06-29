# Risk-Score Based Dynamic Interest Rate Formula

**Version: 2026-04-22**

This document defines the bounded piecewise-linear formula introduced in issue `#265` to compute effective interest rates from borrower risk scores.

## Overview

When enabled via `set_rate_formula_config`, the contract automatically derives `interest_rate_bps` from the borrower's `risk_score` during `update_risk_parameters`, instead of using the manually supplied rate. This provides deterministic, auditable, and consistent rate-setting across all borrowers.

## Formula

The rate formula uses a piecewise-linear mapping:

```text
raw_rate = base_rate_bps + (risk_score × slope_bps_per_score)
effective_rate = clamp(raw_rate, min_rate_bps, min(max_rate_bps, MAX_INTEREST_RATE_BPS))
```

Where:
- `base_rate_bps` — Base annual interest rate in basis points at risk_score = 0.
- `slope_bps_per_score` — Additional bps per unit increase in risk_score.
- `min_rate_bps` — Floor on the computed rate.
- `max_rate_bps` — Ceiling on the computed rate (never exceeds 10,000 = 100%).
- `MAX_INTEREST_RATE_BPS` = 10,000 (contract-level hard cap).

## Configuration

### Setting the formula

Admin calls `set_rate_formula_config`:

```
set_rate_formula_config(
    base_rate_bps: 200,       // 2% base rate
    slope_bps_per_score: 50,  // +0.5% per risk score unit
    min_rate_bps: 200,        // 2% floor
    max_rate_bps: 5000        // 50% ceiling
)
```

### Validation rules

| Constraint | Enforced |
|---|---|
| `min_rate_bps ≤ max_rate_bps` | Panics if violated |
| `max_rate_bps ≤ 10,000` | Panics if violated |
| `base_rate_bps ≤ 10,000` | Panics if violated |

### Disabling the formula

Admin calls `clear_rate_formula_config()` to remove the formula and revert to manual rate mode.

### Querying the formula

`get_rate_formula_config()` returns `Option<RateFormulaConfig>`:
- `Some(config)` — formula is active
- `None` — manual mode

## Examples

Given configuration: `base=200, slope=50, min=200, max=5000`

| Risk Score | Raw Rate | Clamped Rate | Annual % |
|---|---|---|---|
| 0 (lowest risk) | 200 | 200 | 2.00% |
| 10 | 700 | 700 | 7.00% |
| 25 | 1450 | 1450 | 14.50% |
| 50 (medium risk) | 2700 | 2700 | 27.00% |
| 75 | 3950 | 3950 | 39.50% |
| 96 | 5000 | 5000 | 50.00% |
| 100 (highest risk) | 5200 | 5000 | 50.00% (clamped) |

Given configuration: `base=100, slope=80, min=300, max=8000`

| Risk Score | Raw Rate | Clamped Rate | Annual % |
|---|---|---|---|
| 0 | 100 | 300 | 3.00% (floored to min) |
| 5 | 500 | 500 | 5.00% |
| 50 | 4100 | 4100 | 41.00% |
| 100 | 8100 | 8000 | 80.00% (clamped to max) |

## Behavior in `update_risk_parameters`

1. Admin calls `update_risk_parameters(borrower, credit_limit, interest_rate_bps, risk_score)`.
2. **If formula config exists**: The passed `interest_rate_bps` is **ignored** and the effective rate is computed from `risk_score` using the formula.
3. **If no formula config**: The passed `interest_rate_bps` is used directly (original behavior).
4. Rate-change limits (`RateChangeConfig`) still apply to the computed rate.
5. The effective rate is stored in `CreditLineData.interest_rate_bps`.

## Integer arithmetic and safety

- All operations use **saturating arithmetic** (`saturating_add`, `saturating_mul`) to prevent overflow. If `risk_score × slope_bps_per_score` overflows `u32`, it saturates to `u32::MAX` and is then clamped to `max_rate_bps`.
- The result is bounded by `clamp(raw, min, max)` ensuring it always falls within the configured range.
- No floating-point math is used — all values are integer basis points.

## Events

### Formula config set
Topic: `("credit", "rate_cfg")`

Payload: `RateFormulaConfigEvent { base_rate_bps, slope_bps_per_score, min_rate_bps, max_rate_bps, enabled: true }`

### Formula config cleared
Topic: `("credit", "rate_cfg")`

Payload: `RateFormulaConfigEvent { base_rate_bps: 0, slope_bps_per_score: 0, min_rate_bps: 0, max_rate_bps: 0, enabled: false }`

### Risk parameters updated
The existing `("credit", "risk_upd")` event is emitted with `interest_rate_bps` set to the **effective** (computed or manual) rate.

## Interaction with rate-change limits

When a `RateChangeConfig` is set:
- The delta between the old stored rate and the new effective rate (whether computed or manual) is checked against `max_rate_change_bps`.
- The minimum interval between rate changes is enforced.
- This prevents the formula from causing abrupt rate changes even if the risk score changes drastically.

## Storage design

The formula config is stored in **instance storage** under the key `"rate_form"` as a `RateFormulaConfig`. This is read during `update_risk_parameters` to determine whether to compute or passthrough the rate. The computed rate is then stored in `CreditLineData.interest_rate_bps` as usual — downstream code (accrual, queries) requires no changes.

## Migration and backward compatibility

- **No schema changes**: `CreditLineData` is unchanged.
- **No retroactive changes**: Existing credit lines keep their current rates until the next `update_risk_parameters` call.
- **Opt-in**: The formula only activates when explicitly configured.
- **Reversible**: `clear_rate_formula_config()` fully reverts to manual mode.

## Test coverage

See `contracts/credit/src/risk_formula_tests.rs` for comprehensive tests covering:
- Edge scores (0, 50, 100)
- Clamping to min/max bounds
- Overflow saturation
- Backward compatibility (manual mode)
- Config validation
- Set → clear → set lifecycle
- Rate-change limits with formula
- Admin authorization
