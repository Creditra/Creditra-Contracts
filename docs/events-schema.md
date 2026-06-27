# Creditra Event Schema Reference

**Version:** 1.0
**Status:** authoritative for `main` at the time of writing
**Scope:** `creditra-credit` (`contracts/credit/`)

---

## 1. Overview

This document is the canonical reference for all events emitted by the Creditra credit contract. Every event topic, payload struct, and field order is documented here, along with a clear versioning policy for safe schema evolution.

---

## 2. Versioning Policy (SemVer-style)

### Schema Stability Rules

The contract API version is defined in `contracts/credit/src/lib.rs:60` as `CONTRACT_API_VERSION = (1, 0, 0)`. Event schema follows:

- **Major:** Breaking changes (rename/remove/reorder fields, topic name changes, semantic changes require a new version with a `_vN` suffix. Examples:
  - `("credit","drawn_v2`
  - `("credit","repay_v2`

- **Minor:** New event topics or new optional fields at the *end* of existing payload structs, but only via new fields in same topic only with a version suffix (not existing payloads still backward compatible.

- **Patch:** Bug fixes only; no breaking changes.

### Topic Encoding Rationale

Topics use `symbol_short!` (≤ 9 chars) to use cheap `SCV_SYMBOL` on-chain encoding, when ≤9 is `Symbol::new(env, "<longer-name` (up to 32 chars) when length > 9.

---

## 3. Event Catalog (Full)

| Topic Symbols | Payload Struct | Field Order & Types | Version Added | Version Deprecated
|---|---|---|---|---
| ("credit","opened") | CreditLineEvent | 1. borrower: Address, 2. status: CreditStatus, 3. credit_limit: i128, 4. interest_rate_bps: u32, 5. risk_score: u32 | 1.0.0 | -
| ("credit","suspend") | CreditLineEvent | 1. borrower: Address, 2. status: CreditStatus, 3. credit_limit: i128, 4. interest_rate_bps: u32, 5. risk_score: u32 | 1.0.0 | -
| ("credit","closed") | CreditLineEvent | 1. borrower: Address, 2. status: CreditStatus, 3. credit_limit: i128, 4. interest_rate_bps: u32, 5. risk_score: u32 | 1.0.0 | -
| ("credit","default") | CreditLineEvent | 1. borrower: Address, 2. status: CreditStatus, 3. credit_limit: i128, 4. interest_rate_bps: u32, 5. risk_score: u32 | 1.0.0 | -
| ("credit","reinstate") | CreditLineEvent | 1. borrower: Address, 2. status: CreditStatus, 3. credit_limit: i128, 4. interest_rate_bps: u32, 5. risk_score: u32 | 1.0.0 | -
| ("credit","drawn") | DrawnEvent | 1. borrower: Address, 2. amount: i128, 3. new_utilized_amount: i128 | 1.0.0 | -
| ("credit","drawn_v2") | DrawnEventV2 | 1. borrower: Address, 2. recipient: Address, 3. reserve_source: Address, 4. amount: i128, 5. new_utilized_amount: i128, 6. timestamp: u64 | 1.0.0 | -
| ("credit","repay") | RepaymentEvent | 1. borrower: Address, 2. amount: i128, 3. new_utilized_amount: i128 | 1.0.0 | -
| ("credit","accrue") | InterestAccruedEvent | 1. borrower: Address, 2. accrued_amount: i128, 3. new_utilized_amount: i128 | 1.0.0 | -
| ("credit","fee_accrd") | FeeAccruedEvent | 1. borrower: Address, 2. fee_amount: i128, 3. new_treasury_balance: i128 | 1.0.0 | -
| ("credit","admin_prop") | AdminRotationProposedEvent | 1. proposed_admin: Address, 2. accept_after: u64 | 1.0.0 | -
| ("credit","admin_acc") | AdminRotationAcceptedEvent | 1. new_admin: Address | 1.0.0 | -
| ("credit","risk_upd") | RiskParametersUpdatedEvent | 1. borrower: Address, 2. credit_limit: i128, 3. interest_rate_bps: u32, 4. risk_score: u32 | 1.0.0 | -
| ("credit","draw_rev") | DrawReversedEvent | 1. borrower: Address, 2. amount: i128, 3. original_ts: u64, 4. reason_code: u32, 5. new_utilized_amount: i128, 6. timestamp: u64, 7. admin: Address, 8. accounting_only: bool | 1.0.0 | -
| ("credit","drw_freeze") | DrawsFrozenEvent | 1. frozen: bool, 2. reason: FreezeReason | 1.0.0 | -
| ("credit","line_frz") | CreditLineFreezeEvent | 1. borrower: Address, 2. reason: FreezeReason, 3. frozen: bool, 4. ledger: u32 | 1.0.0 | -
| ("credit","rate_form") | bool | (single value, no struct | 1.0.0 | -
| ("credit","liq_req") | (Address, i128) | 1. borrower: Address, 2. utilized_amount: i128 | 1.0.0 | -
| ("credit","liq_setl") | DefaultLiquidationSettledEvent | 1. borrower: Address, 2. settlement_id: Symbol, 3. recovered_amount: i128, 4. remaining_utilized_amount: i128, 5. status: CreditStatus, 6. close_factor_bps: u32 | 1.0.0 | -
| ("credit","paused") | bool | single boolean value | 1.0.0 | -
| ("credit","unpaused") | bool | single boolean value | 1.0.0 | -
| ("blk_chg",) | BorrowerBlockedEvent | 1. borrower: Address, 2. blocked: bool, 3. ledger: u32 | 1.0.0 | -
| ("credit","pen_enter") | PenaltyRateEnteredEvent | 1. borrower: Address, 2. base_rate_bps: u32, 3. penalty_surcharge_bps: u32, 4. effective_rate_bps: u32 | 1.0.0 | -
| ("credit","pen_exit") | PenaltyRateExitedEvent | 1. borrower: Address, 2. previous_rate_bps: u32, 3. new_rate_bps: u32 | 1.0.0 | -
| ("credit","col_dep") | CollateralDepositedEvent | 1. borrower: Address, 2. amount: i128, 3. new_balance: i128 | 1.0.0 | -
| ("credit","col_wit") | CollateralWithdrawnEvent | 1. borrower: Address, 2. amount: i128, 3. new_balance: i128 | 1.0.0 | -
| ("credit","upgraded") | ContractUpgradedEvent | 1. old_wasm_hash: BytesN<32>, 2. new_wasm_hash: BytesN<32> | 1.0.0 | -
| ("credit","orc_cfg") | (u32, u64) | 1. max_deviation_bps: u32, 2. max_age_seconds: u64 | 1.0.0 | -
| ("credit","orc_price") | (i128, u64) | 1. price: i128, 2. timestamp: u64 | 1.0.0 | -

---

## 4. Type Definitions

All payload structs are defined in `contracts/credit/src/events.rs`. The definitions there with `#[contracttype]`.

See also:
- `docs/indexer-integration.md`
- `docs/PROTOCOL_SPEC.md`
- `contracts/credit/src/events.rs`
