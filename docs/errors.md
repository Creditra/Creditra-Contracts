# ContractError Reference

**Version: 2026-04-24**  
**Source of truth: `contracts/credit/src/types.rs`**

This document is the canonical reference for all `ContractError` discriminants in the
Creditra Credit contract. Integrators (TypeScript SDK, Rust SDK, indexers) must use
these integer codes to identify failure reasons.

---

## Stability Guarantee

Discriminants are **permanent and immutable** once assigned. The contract is deployed
on Stellar Soroban and cannot be upgraded without a migration. Changing or reordering
a discriminant would silently break all SDK clients that match on integer codes.

Rules enforced by CI (`tests/error_discriminants.rs`):

- Every variant has an explicit `= N` assignment in `types.rs`.
- No two variants share the same integer.
- New variants are always appended at the end with the next available integer.
- The assertion test file must be updated alongside any enum change.

---

## Error Code Table

| Code | Variant                          | When it occurs | Resolution |
|------|----------------------------------|----------------|------------|
| `1`  | `Unauthorized`                   | Caller is not the authorized party for this operation (e.g., non-borrower calling `draw_credit`). | Ensure the correct signer is authorizing the transaction. |
| `2`  | `NotAdmin`                       | Caller does not hold the admin role stored in instance storage. | Use the admin keypair or rotate admin via `propose_admin` / `accept_admin`. |
| `3`  | `CreditLineNotFound`             | No credit line exists in persistent storage for the given borrower address. | Verify the borrower address and that `open_credit_line` was called successfully. |
| `4`  | `CreditLineClosed`               | The credit line status is `Closed`; draws and repayments are both blocked. | A closed line cannot be reopened. Open a new credit line for the borrower. |
| `5`  | `InvalidAmount`                  | The supplied amount is zero, negative, or otherwise outside the valid range. | Pass a strictly positive `i128` value. |
| `6`  | `OverLimit`                      | The requested draw would push `utilized_amount` above `credit_limit`. | Reduce the draw amount or request a limit increase via `update_risk_parameters`. |
| `7`  | `NegativeLimit`                  | A credit limit of zero or below was supplied to `update_risk_parameters`. | Supply a positive `i128` credit limit. |
| `8`  | `RateTooHigh`                    | `interest_rate_bps` exceeds `10 000` (100 %) or the configured `max_rate_change_bps` delta. | Use a rate in the range `0–10 000` bps. |
| `9`  | `ScoreTooHigh`                   | `risk_score` exceeds `100`. | Supply a score in the range `0–100`. |
| `10` | `UtilizationNotZero`             | An operation that requires zero utilization was attempted while the borrower still has an outstanding balance. | Repay the full balance before retrying. |
| `11` | `Reentrancy`                     | A reentrant call was detected via the reentrancy guard on `draw_credit` or `repay_credit`. | This should not occur with standard Stellar Asset Contracts. Investigate the token contract for unexpected callbacks. |
| `12` | `Overflow`                       | An arithmetic operation (e.g., `checked_add` on `utilized_amount`) would overflow `i128`. | Amounts near `i128::MAX` are not supported. Reduce the draw or limit value. |
| `13` | `LimitDecreaseRequiresRepayment` | A limit decrease was requested that would push `credit_limit` below `utilized_amount`. The line transitions to `Restricted` status. | Borrower must repay the excess (`utilized_amount - new_limit`) before the limit can be lowered further. |
| `14` | `AlreadyInitialized`             | `init` was called on a contract that already has an admin stored. | `init` is a one-time operation. Do not call it again after deployment. |
| `15` | `AdminAcceptTooEarly`            | `accept_admin` was called before the `delay_seconds` window set in `propose_admin` has elapsed. | Wait until `env.ledger().timestamp() >= accept_after` and retry. |
| `16` | `BorrowerBlocked`                | The borrower address is on the admin-managed block list; draws are disabled. | Contact the protocol admin to remove the block, or use a different borrower address. |
| `17` | `DrawExceedsMaxAmount`           | The requested draw amount exceeds the per-transaction cap set via `set_max_draw_amount`. | Split the draw into smaller transactions or request a cap increase from the admin. |
| `18` | `Paused`                         | The protocol is paused via the emergency circuit breaker; operation is blocked. | Wait for the admin to unpause the protocol via `set_protocol_paused(false)`. `repay_credit` remains active during a pause. |

---

## SDK Usage Examples

### Rust

```rust
use creditra_credit::types::ContractError;

match result {
    Err(e) if e == ContractError::OverLimit as u32 => {
        // handle over-limit
    }
    Err(e) if e == ContractError::CreditLineNotFound as u32 => {
        // handle not found
    }
    _ => {}
}
```

### TypeScript (Soroban SDK)

```typescript
import { ContractError } from "@creditra/credit-sdk";

try {
  await client.drawCredit({ borrower, amount });
} catch (err) {
  if (err.code === 6 /* OverLimit */) {
    console.error("Draw exceeds credit limit");
  } else if (err.code === 3 /* CreditLineNotFound */) {
    console.error("No credit line found for borrower");
  }
}
```

---

## Security Notes

### Failure Modes and Trust Boundaries

**Reentrancy (code 11)**  
The reentrancy guard on `draw_credit` and `repay_credit` is defense-in-depth.
Standard Stellar Asset Contracts do not invoke callbacks into the caller, so this
error should never appear in production. If it does, the token contract being used
is non-standard and must be audited before use.

**Overflow (code 12)**  
All arithmetic on `utilized_amount` uses `checked_add`; overflow reverts the
transaction with no state change. Amounts near `i128::MAX` (~1.7 × 10³⁸) are
outside the intended operating range of the protocol.

**AlreadyInitialized (code 14)**  
The `init` guard prevents admin takeover via re-initialization. The check reads
instance storage before writing; a rejected second call leaves storage unchanged.

**AdminAcceptTooEarly (code 15)**  
The two-step admin rotation (`propose_admin` → `accept_admin`) includes an optional
time-lock. The delay is enforced against `env.ledger().timestamp()`, which is
network-provided and monotonic enough for coarse governance windows. It is not
suitable for sub-second precision.

**BorrowerBlocked (code 16)**  
The block list is admin-controlled. Blocking a borrower prevents draws but does not
affect repayments — a blocked borrower can still repay outstanding debt.

**DrawExceedsMaxAmount (code 17)**  
The per-transaction draw cap is a risk-management control, not a security boundary.
It limits the blast radius of a compromised borrower key or a buggy integration.

**Paused (code 18)**  
The protocol pause is an emergency circuit breaker controlled by the admin. When
activated, all state-mutating operations are blocked except `repay_credit`, which
remains active to allow users to reduce their debt exposure even during an incident.
The pause state is stored in instance storage and checked at the entry of every
guarded function. Read-only operations (`get_credit_line`, `is_protocol_paused`, etc.)
are never blocked.

### General Trust Model

| Actor           | Trusted for |
|-----------------|-------------|
| Admin           | Lifecycle operations, risk parameters, block list, liquidity config |
| Borrower        | Drawing and repaying their own credit line only |
| Liquidity token | Standard Stellar Asset Contract behavior (no callbacks) |
| Ledger timestamp| Coarse monotonic ordering (governance delays, accrual intervals) |

Errors in the `1–2` range (`Unauthorized`, `NotAdmin`) indicate an access-control
violation and should be treated as security-relevant events by monitoring systems.
