# Per-Borrower Utilization Ratio Cap

## Overview

The utilization cap allows an admin to restrict how much of a borrower's nominal credit limit they can actually draw, expressed as a ratio in basis points (bps). This is independent of the credit limit itself and acts as an additional ceiling below it.

## Semantics

- **Cap formula:** `cap_amount = credit_limit * cap_bps / 10_000`
- **Enforcement:** In `draw_credit`, after the credit-limit check, if a cap is configured the contract verifies: `utilized_amount + draw_amount <= cap_amount`. If not, the transaction reverts with `"exceeds utilization cap"`.
- **No cap set:** If no cap is configured for a borrower, the full credit limit applies (existing behavior is unchanged).

## Configuration

| Method | Auth | Description |
|---|---|---|
| `set_utilization_cap(borrower, cap_bps)` | Admin only | Set cap. `cap_bps=0` removes the cap. Valid range: 1–10_000. |
| `get_utilization_cap(borrower)` | Anyone | Returns `Some(cap_bps)` if set, `None` otherwise. |

## Examples

| credit_limit | cap_bps | cap_amount | Max draw |
|---|---|---|---|
| 1_000 | 8_000 (80%) | 800 | 800 |
| 1_000 | 5_000 (50%) | 500 | 500 |
| 1_000 | 10_000 (100%) | 1_000 | 1_000 (same as limit) |
| 1_000 | not set | — | 1_000 (full limit) |

## Interaction with credit limit updates

When `update_risk_parameters` changes `credit_limit`, the cap ratio (bps) is unchanged. The effective cap amount recalculates automatically on the next draw because it is derived from the current `credit_limit` at draw time.

**Example:** borrower has `credit_limit=1_000`, `cap_bps=8_000` (cap_amount=800). Admin raises limit to 2_000. On the next draw, cap_amount becomes 1_600 automatically — no cap reconfiguration needed.

## Interaction with interest accrual

The cap is applied to `utilized_amount` (principal + capitalized interest). If accrued interest pushes `utilized_amount` above the cap, no new draws are possible until the borrower repays below the cap threshold. The cap does not block repayments.

## Security notes

- Only the admin can set or remove a cap (`require_admin_auth` enforced).
- `cap_bps > 10_000` is rejected to prevent nonsensical configurations.
- The cap is stored per-borrower in instance storage; each borrower's cap is independent.
- Removing a cap (passing `cap_bps=0`) deletes the storage entry, restoring full-limit behavior.
