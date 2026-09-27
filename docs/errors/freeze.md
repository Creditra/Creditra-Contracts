# Freeze Error Catalog

**Version: 2026-07-27**
**Source of truth: [`FreezeError`](../../contracts/freeze/src/errors.rs) enum in `contracts/freeze/src/errors.rs`.**

**Published contract crate: `creditra-freeze`.**

**CI guard: In-module tests pin every discriminant.**

This document is the canonical reference for the `FreezeError` catalog
emitted by the Creditra freeze domain. Integrators (TypeScript SDK, Rust
SDK, indexers) match against the integer codes here when decoding an error
emitted from the freeze contract.

---

## Stability Guarantee

Discriminants are **permanent and immutable** once assigned. The `#[repr(u32)]`
representation, combined with the Soroban `#[contracterror]` derive, means
these codes cross the contract ABI as raw `u32` values. Reordering or
renumbering would silently break every SDK client that matches on a numeric
code.

Rules enforced by CI (`contracts/freeze/src/errors.rs`):

- Every variant has an explicit `= N` assignment in `errors.rs`.
- No two variants share the same integer (`no_duplicate_discriminants`).
- New variants are always appended at the end with the next available integer.
- The `mirror_discriminants_match_canonical_credit_contract` test pins every mirror discriminant against the canonical credit contract `ContractError` table at [`docs/ERROR_CODES.md`](../ERROR_CODES.md).

---

## Tier System

The catalog uses a **two-tier** discriminant policy. Each tier serves a
distinct role:

| Tier | Codes | Purpose |
|------|-------|---------|
| **Mirror** | `3`, `16`, `19`, `40`, `46` | Semantically identical to canonical `ContractError` codes published by `contracts/credit/src/types.rs`. SDK consumers can map these integers directly to the canonical table at [`docs/ERROR_CODES.md`](../ERROR_CODES.md). |
| **Freeze-specific** | `100+` | Errors that have no canonical counterpart in the credit contract's `ContractError`. Reserved namespace with a `50`-slot buffer above the credit contract's `1..=49` range. |

The `100+` gap is intentional. It defends against accidental collisions if a
future PR appends either catalog, and it gives front-end integrators an
immediate visual distinction when an error code in the `100`-block surfaces in
their telemetry.

---

## Error Code Table

### Mirror Tier

| Code  | Variant                              | When it occurs | Resolution |
|-------|--------------------------------------|----------------|------------|
| `3`   | `CreditLineNotFound`                 | The requested borrower does not have an open credit line. | Create a credit line first. |
| `16`  | `BorrowerBlocked`                    | Borrower is on the admin-managed block list. | The borrower must contact admin. |
| `19`  | `DrawsFrozen`                        | Global draw freeze is active. | Temporary — repayments remain open. |
| `40`  | `BorrowerFrozen`                     | Borrower's draws are temporarily frozen until the specified expiry timestamp. | Wait for expiry or contact admin. |
| `46`  | `CreditLineFrozen`                   | Credit line draws are frozen by admin (compliance or investigation hold). | Wait for `unfreeze_credit_line`. |

> **SDK tip.** A mirror code from the freeze contract has the *same* recovery
> meaning as the equivalent code from the credit contract's `ContractError`. Use a single
> helper to decode.

### Freeze-Specific Tier (100+)

*No variants are currently defined for the freeze-specific tier. Space is reserved for future freeze-specific logic.*

---

## SDK Decoder Mapping

The mirror tier overlaps the credit contract's `ContractError` codes. SDK
clients should be able to decode these without needing to look up a
per-contract table — the canonical reference at
[`docs/ERROR_CODES.md`](../ERROR_CODES.md) covers both the credit contract and the
mirror tier of this catalog.

| Mirror code | Same-named variant in `ContractError`? | Identical meaning? |
|-------------|---------------------------------------|--------------------|
| 3           | `ContractError::CreditLineNotFound` (3) | Yes |
| 16          | `ContractError::BorrowerBlocked` (16)   | Yes |
| 19          | `ContractError::DrawsFrozen` (19)       | Yes |
| 40          | `ContractError::BorrowerFrozen` (40)    | Yes |
| 46          | `ContractError::CreditLineFrozen` (46)  | Yes |

---

## Categories

The freeze domain groups its mirror errors into the same top-level categories as the credit contract.

| Category | Mirror codes | Description |
|----------|--------------|-------------|
| `Misc`   | 3            | Entity not found. |
| `Block`  | 16, 19, 40, 46 | Draw-block conditions. |

---

## Related Documents

- [`docs/ERROR_CODES.md`](../ERROR_CODES.md) — canonical flat code table for the
  credit contract.
- [`docs/error-taxonomy.md`](../error-taxonomy.md) — error categories with
  SDK recovery actions.
- [`docs/errors.md`](../errors.md) — canonical reference for the credit
  contract's `ContractError`.
