# Collateral Error Catalog

**Version: 2026-04-24**
**Source of truth: [`CollateralError`](../contracts/collateral/src/errors.rs) enum in `contracts/collateral/src/errors.rs`.**

**Published contract crate: `creditra-collateral`.**

**CI guard: [`tests/catalog.rs`](../contracts/collateral/tests/catalog.rs) pins every discriminant.**

This document is the canonical reference for the `CollateralError` catalog
emitted by the Creditra collateral domain. Integrators (TypeScript SDK, Rust
SDK, indexers) match against the integer codes here when decoding an error
emitted from the collateral contract.

---

## Stability Guarantee

Discriminants are **permanent and immutable** once assigned. The `#[repr(u32)]`
representation, combined with the Soroban `#[contracterror]` derive, means
these codes cross the contract ABI as raw `u32` values. Reordering or
renumbering would silently break every SDK client that matches on a numeric
code.

Rules enforced by CI (`contracts/collateral/tests/catalog.rs`):

- Every variant has an explicit `= N` assignment in `errors.rs`.
- No two variants share the same integer (`no_duplicate_discriminants`).
- New variants are always appended at the end with the next available integer.
- The integration test must be updated alongside any enum change so the
  discriminant count and uniqueness set stay in sync.
- The `discriminants_are_stable` and `mirror_matches_canonical_credit_contract_error_table`
  tests pin every mirror discriminant against the canonical credit contract
  `ContractError` table at [`docs/ERROR_CODES.md`](ERROR_CODES.md).

---

## Tier System

The catalog uses a **two-tier** discriminant policy. Each tier serves a
distinct role:

| Tier | Codes | Purpose |
|------|-------|---------|
| **Mirror** | `5`, `12`, `22`, `35`, `39` | Semantically identical to canonical `ContractError` codes published by `contracts/credit/src/types.rs`. SDK consumers can map these integers directly to the canonical table at [`docs/ERROR_CODES.md`](ERROR_CODES.md). |
| **Collateral-specific** | `100+` | Errors that have no canonical counterpart in the credit contract's `ContractError`. Reserved namespace with a `50`-slot buffer above the credit contract's `1..=49` range. |

The `100+` gap is intentional. It defends against accidental collisions if a
future PR appends either catalog, and it gives front-end integrators an
immediate visual distinction when an error code in the `100`-block surfaces in
their telemetry.

---

## Error Code Table

### Mirror Tier

| Code  | Variant                              | When it occurs | Resolution |
|-------|--------------------------------------|----------------|------------|
| `5`   | `InvalidAmount`                      | Deposit, withdrawal, partial-release, or admin operation supplied `amount <= 0`. | Pass a strictly positive `i128` value. |
| `12`  | `Overflow`                           | `checked_add` on collateral balances or `checked_mul` on `utilized * min_ratio_bps / 10_000` would overflow `i128`. | Reduce amounts so they stay well below `i128::MAX / 10_000`. |
| `22`  | `MissingLiquidityToken`              | Collateral token address has not been configured (`set_collateral_token` never called), **or** the multi-collateral path received a token that is not on the admin allowlist. | Configure a collateral token before deposits / withdrawals; for multi-collateral use an allowlisted token. |
| `35`  | `CollateralRatioBelowMinimum`        | Withdrawal (or draw) would leave `(post_balance * 10_000) / utilized` strictly below `MinCollateralRatioBps`. | Reduce the withdrawal amount, repay some utilization, or raise `MinCollateralRatioBps` (admin only). |
| `39`  | `InsufficientCollateralBalance`       | Withdrawal amount strictly exceeds the borrower's stored collateral balance. | Query `get_balance_for_token` (or `get_collateral` for the single-token path) and reduce the requested amount. |

> **SDK tip.** A `5` from the collateral contract has the *same* recovery
> meaning as a `5` from the credit contract's `ContractError`. Use a single
> helper to decode.

### Collateral-Specific Tier (100+)

| Code   | Variant                              | When it occurs | Resolution |
|--------|--------------------------------------|----------------|------------|
| `100`  | `CollateralTokenNotAllowed`          | Multi-collateral deposit/withdraw received a token address that the admin has explicitly **not** allowlisted. Distinguished from code `22`: `22` is "token not configured at all", `100` is "token is known but rejected". | Use an allowlisted token, or request admin allowlist addition via governance. |
| `101`  | `CollateralRiskWeightOutOfRange`     | A `risk_weight_bps` configuration update fell outside the configured `[min_risk_weight_bps, max_risk_weight_bps]` bounds. | Adjust the proposed weight to fall inside the bounds returned by the risk-tier config view. |
| `102`  | `CollateralTokenMismatch`            | Deposit-then-withdraw (or atomic release) flow supplied a token address that does not match the token currently bound to that borrower's collateral position. | Query the borrower's per-token balance view, then use the matching token address. |
| `103`  | `CollateralPositionLocked`           | An outstanding draw is open against the position; deposits and withdraws that would disturb liquidation economics are blocked until the draw is cured. | Repay the outstanding draw (full or partial repayment, depending on policy), then retry. |
| `104`  | `CollateralBalanceForTokenNotFound`  | A zero-balance lookup path (`get_balance_for_token`) was called for a borrower who has no balance under the requested token. | Call `set_balance_for_token` to seed the entry, or use a different token. |

---

## Examples

### Rust

```rust
use creditra_collateral::CollateralError;

fn handle_collateral_error(code: u32) -> &'static str {
    match code {
        x if x == CollateralError::InvalidAmount as u32 => "invalid amount supplied",
        x if x == CollateralError::InsufficientCollateralBalance as u32 => {
            "withdrawal exceeds deposited balance"
        }
        x if x == CollateralError::CollateralRatioBelowMinimum as u32 => {
            "withdrawal would breach minimum collateral ratio"
        }
        x if x == CollateralError::CollateralTokenNotAllowed as u32 => {
            "collateral token is not on the allowlist"
        }
        _ => "unknown / future variant",
    }
}
```

### TypeScript (Soroban SDK)

```typescript
import { CollateralError } from "@creditra/collateral-sdk";

try {
  await collateralContract.withdraw({ borrower, token, amount });
} catch (err) {
  switch (err.code) {
    case 39 /* InsufficientCollateralBalance */:
      console.error("withdrawal exceeds deposited balance");
      break;
    case 35 /* CollateralRatioBelowMinimum */:
      console.error("withdrawal would breach min ratio");
      break;
    case 100 /* CollateralTokenNotAllowed */:
      console.error("token not on collateral allowlist");
      break;
    default:
      console.warn("unhandled collateral error", err.code);
  }
}
```

---

## SDK Decoder Mapping

The mirror tier overlaps the credit contract's `ContractError` codes. SDK
clients should be able to decode these without needing to look up a
per-contract table — the canonical reference at
[`docs/ERROR_CODES.md`](ERROR_CODES.md) covers both the credit contract and the
mirror tier of this catalog.

| Mirror code | Same-named variant in `ContractError`? | Identical meaning? |
|-------------|---------------------------------------|--------------------|
| 5           | `ContractError::InvalidAmount` (5)     | Yes                |
| 12          | `ContractError::Overflow` (12)         | Yes                |
| 22          | `ContractError::MissingLiquidityToken` (22) | Yes (collateral reuses the same code, including for "token not on multi-collateral allowlist" since the credit contract does not distinguish) |
| 35          | `ContractError::CollateralRatioBelowMinimum` (35) | Yes |
| 39          | `ContractError::InsufficientCollateralBalance` (39) | Yes |

---

## Cross-Contract Trust Notes

| Source contract | SDK should use            |
|-----------------|---------------------------|
| `creditra-credit` | `ContractError` from `contracts/credit/src/types.rs`. |
| `creditra-collateral` (current and future) | `CollateralError` from `contracts/collateral/src/errors.rs`. SDK clients decoupling from a single contract should still match the integer code; both enums reach the same protocol-wide meaning for the mirror tier. |

If a transaction targets the `creditra-credit` contract and the host returns
error `35`, the SDK decodes via `ContractError::CollateralRatioBelowMinimum`.
If a transaction targets `creditra-collateral` and the host returns `35`, the
SDK decodes via `CollateralError::CollateralRatioBelowMinimum` (or — being
agnostic — via the bytes `35` and the canonical table at
[`docs/ERROR_CODES.md`](ERROR_CODES.md)).

The two decode paths converge because the variants share their *semantic*
meaning. Code `35` on either contract means exactly one thing across the
protocol: *"the (post-actions) collateral ratio is below the configured
minimum ratio floor"*.

---

## Categories

The collateral domain groups its mirror errors into the same four
top-level categories as the credit contract. Categories are
documentation-only here (the runtime `ContractErrorCategory` lives in the
credit crate). For SDK side-decoding, prefer the canonical category table at
[`docs/error-taxonomy.md`](error-taxonomy.md).

| Category    | Mirror codes | Description                               |
|-------------|--------------|-------------------------------------------|
| `Numeric`   | 5, 12        | Bad input or arithmetic overflow.         |
| `Liquidity` | 22           | Collateral / liquidity token misconfiguration. |
| `Collateral`| 35, 39       | Ratio floor breach, balance shortfall.    |

The `100+` tier is **collateral-domain only** — it has no canonical category
mapping; SDK clients should treat each variant as its own bucket.

---

## Backwards Compatibility

- **Mirror tier codes (`5`, `12`, `22`, `35`, `39`) are frozen.** Re-binding
  any of them to a different semantic is a breaking change and would force
  SDK consumers to recognise two distinct meanings for the same integer.
- **Collateral-specific tier codes (`100+`) may be appended.** New variants
  must use the next available integer and must be paired with a CI test
  update. They must not reorder or remove existing variants.
- **No change to the canonical credit `ContractError`.** This catalog is
  intentionally scoped to the collateral contract; the canonical table is
  unaffected.

---

## Related Documents

- [`docs/ERROR_CODES.md`](ERROR_CODES.md) — canonical flat code table for the
  credit contract.
- [`docs/error-taxonomy.md`](error-taxonomy.md) — error categories with
  SDK recovery actions.
- [`docs/errors.md`](errors.md) — canonical reference for the credit
  contract's `ContractError`.
- [`docs/storage-layout.md`](storage-layout.md) — storage keys for collateral
  balances (relevant context for understanding
  `CollateralBalanceForTokenNotFound`).
