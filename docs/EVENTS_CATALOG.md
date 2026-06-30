# Events Catalog

**Version:** 1.0  
**Status:** Authoritative for `main` at the time of writing  
**Scope:** `creditra-credit` (`contracts/credit/`) and `gateway-auction` (`gateway-contract/contracts/auction_contract/`)  
**Last updated:** 2026-06-28

---

## 1. Purpose

This document is the single authoritative reference for every event emitted by the
Creditra credit and auction contracts. It lists each event's topic shape, payload
struct, field order, field types, and stability status so that off-chain indexers,
orchestrators, and integrators can decode events without guessing.

Changes to this catalog should be proposed through the repo issue tracker and
PR process. Breaking changes require a new event topic with a `_vN` suffix and a
contract API version bump.

---

## 2. Versioning Policy

The contract API version is defined in `contracts/credit/src/lib.rs` as
`CONTRACT_API_VERSION = (1, 0, 0)`. Event schema follows SemVer-style rules:

- **Major:** Breaking changes require a new event topic with a `_vN` suffix and a
  contract API major version bump. Breaking changes include:
  - Renaming, removing, or reordering fields in a payload struct.
  - Changing a field's type.
  - Changing a topic string.
- **Minor:** A new event topic or a new optional field at the end of an existing
  payload struct. Requires a contract API minor version bump.
- **Patch:** Bug fixes only; no structural changes to topics or payloads.

### Topic suffix convention

When a breaking change is required, introduce a new topic with a suffix and keep
the old topic alive during a dual-publish window:

```
("credit", "drawn_v2")   // new version
("credit", "drawn")      // legacy version; still emitted
```

Remove the legacy topic only after downstream indexers confirm cutover.

---

## 3. Topic Encoding

Topics are Soroban `Symbol` values chosen to use the cheap `SCV_SYMBOL` on-chain
encoding (≤ 9 characters). Symbols longer than 9 characters use `Symbol::new(env,
"<longer-name>")` and cost more gas to publish.

| Encoding rule | Limit |
|---|---|
| `symbol_short!` macro | ≤ 9 characters |
| `Symbol::new` | Up to 32 characters |

The first topic in every published tuple is either `"credit"` (credit contract)
or a short identifier such as `"blk_chg"` or `"BID_RFDN"`.

---

## 4. Credit Contract Event Catalog

All topics are published under the `("credit", "...")` namespace unless otherwise
noted. Publishers live in `contracts/credit/src/events.rs`.

### 4.1 Lifecycle events

| Second topic | Payload struct | Field order & types | Version added | Stability |
|---|---|---|---|---|
| `"opened"` | `CreditLineEvent` | 1. `borrower: Address`, 2. `status: CreditStatus`, 3. `credit_limit: i128`, 4. `interest_rate_bps: u32`, 5. `risk_score: u32` | 1.0.0 | Stable |
| `"suspend"` | `CreditLineEvent` | Same as `opened` | 1.0.0 | Stable |
| `"closed"` | `CreditLineEvent` | Same as `opened` | 1.0.0 | Stable |
| `"default"` | `CreditLineEvent` | Same as `opened` | 1.0.0 | Stable |
| `"reinstate"` | `CreditLineEvent` | Same as `opened` | 1.0.0 | Stable |

### 4.2 Draw and repayment events

| Second topic | Payload struct | Field order & types | Version added | Stability |
|---|---|---|---|---|
| `"drawn"` | `DrawnEvent` | 1. `borrower: Address`, 2. `amount: i128`, 3. `new_utilized_amount: i128` | 1.0.0 | Stable |
| `"drawn_v2"` | `DrawnEventV2` | 1. `borrower: Address`, 2. `recipient: Address`, 3. `reserve_source: Address`, 4. `amount: i128`, 5. `new_utilized_amount: i128`, 6. `timestamp: u64` | 1.0.0 | Stable |
| `"repay"` | `RepaymentEvent` | 1. `borrower: Address`, 2. `amount: i128`, 3. `new_utilized_amount: i128` | 1.0.0 | Stable |

### 4.3 Accrual and fee events

| Second topic | Payload struct | Field order & types | Version added | Stability |
|---|---|---|---|---|
| `"accrue"` | `InterestAccruedEvent` | 1. `borrower: Address`, 2. `accrued_amount: i128`, 3. `new_utilized_amount: i128` | 1.0.0 | Stable |
| `"fee_accrd"` | `FeeAccruedEvent` | 1. `borrower: Address`, 2. `fee_amount: i128`, 3. `treasury_amount: i128`, 4. `bounty_amount: i128`, 5. `new_treasury_balance: i128`, 6. `new_bounty_balance: i128` | 1.1.0 | Stable |
| `"late_fee"` | `LateFeeChargedEvent` | 1. `borrower: Address`, 2. `fee: i128`, 3. `installment_index: u64` | 1.0.0 | Stable |

### 4.4 Risk and parameter events

| Second topic | Payload struct | Field order & types | Version added | Stability |
|---|---|---|---|---|
| `"risk_upd"` | `RiskParametersUpdatedEvent` | 1. `borrower: Address`, 2. `credit_limit: i128`, 3. `interest_rate_bps: u32`, 4. `risk_score: u32` | 1.0.0 | Stable |
| `"drw_freeze"` | `DrawsFrozenEvent` | 1. `frozen: bool`, 2. `reason: FreezeReason` | 1.0.0 | Stable |
| `"line_frz"` | `CreditLineFreezeEvent` | 1. `borrower: Address`, 2. `reason: FreezeReason`, 3. `frozen: bool`, 4. `ledger: u32` | 1.0.0 | Stable |
| `"br_freeze"` | `BorrowerFrozenEvent` | 1. `borrower: Address`, 2. `frozen_until: u64`, 3. `ledger: u32` | 1.0.0 | Stable |
| `"pen_enter"` | `PenaltyRateEnteredEvent` | 1. `borrower: Address`, 2. `base_rate_bps: u32`, 3. `penalty_surcharge_bps: u32`, 4. `effective_rate_bps: u32` | 1.0.0 | Stable |
| `"pen_exit"` | `PenaltyRateExitedEvent` | 1. `borrower: Address`, 2. `previous_rate_bps: u32`, 3. `new_rate_bps: u32` | 1.0.0 | Stable |
| `"grace_wv"` | `GraceWaiverAppliedEvent` | 1. `borrower: Address`, 2. `waived_amount: i128`, 3. `mode: GraceWaiverMode` | 1.0.0 | Stable |

### 4.5 Admin and governance events

| Second topic | Payload struct | Field order & types | Version added | Stability |
|---|---|---|---|---|
| `"admin_prop"` | `AdminRotationProposedEvent` | 1. `proposed_admin: Address`, 2. `accept_after: u64` | 1.0.0 | Stable |
| `"admin_acc"` | `AdminRotationAcceptedEvent` | 1. `new_admin: Address` | 1.0.0 | Stable |
| `"tre_prop"` | `TreasuryWithdrawalProposedEvent` | 1. `recipient: Address`, 2. `amount: i128`, 3. `proposer: Address`, 4. `proposed_at: u64`, 5. `execute_after: u64` | 1.0.0 | Stable |
| `"tre_exec"` | `TreasuryWithdrawalExecutedEvent` | 1. `recipient: Address`, 2. `amount: i128`, 3. `executor: Address`, 4. `executed_at: u64` | 1.0.0 | Stable |
| `"upgraded"` | `ContractUpgradedEvent` | 1. `old_wasm_hash: BytesN<32>`, 2. `new_wasm_hash: BytesN<32>` | 1.0.0 | Stable |

### 4.6 Blocklist and freeze events

| Topic | Payload struct | Field order & types | Version added | Stability |
|---|---|---|---|---|
| `("blk_chg",)` | `BorrowerBlockedEvent` | 1. `borrower: Address`, 2. `blocked: bool`, 3. `ledger: u32` | 1.0.0 | Stable |

> Note: `BorrowerBlockedEvent` is emitted on a single-element topic tuple
> `("blk_chg",)`.

### 4.7 Collateral events

| Second topic | Payload struct | Field order & types | Version added | Stability |
|---|---|---|---|---|
| `"col_dep"` | `CollateralDepositedEvent` | 1. `borrower: Address`, 2. `amount: i128`, 3. `new_balance: i128` | 1.0.0 | Stable |
| `"col_wit"` | `CollateralWithdrawnEvent` | 1. `borrower: Address`, 2. `amount: i128`, 3. `new_balance: i128` | 1.0.0 | Stable |

### 4.8 Default liquidation events

| Second topic | Payload struct | Field order & types | Version added | Stability |
|---|---|---|---|---|
| `"liq_req"` | `(Address, i128)` | 1. `borrower: Address`, 2. `utilized_amount: i128` | 1.0.0 | Stable |
| `"liq_setl"` | `DefaultLiquidationSettledEvent` | 1. `borrower: Address`, 2. `settlement_id: Symbol`, 3. `recovered_amount: i128`, 4. `remaining_utilized_amount: i128`, 5. `status: CreditStatus`, 6. `close_factor_bps: u32` | 1.0.0 | Stable |

### 4.9 Attestation events

| Second topic | Payload struct | Field order & types | Version added | Stability |
|---|---|---|---|---|
| `"atst_bat"` | `AttestationBatchCommittedEvent` | 1. `borrower: Address`, 2. `merkle_root: BytesN<32>`, 3. `count: u32` | 1.0.0 | Stable |

### 4.10 Rescue and upgrade events

| Second topic | Payload struct | Field order & types | Version added | Stability |
|---|---|---|---|---|
| `"tok_resc"` | `TokenRescuedEvent` | 1. `token: Address`, 2. `recipient: Address`, 3. `amount: i128` | 1.0.0 | Stable |

### 4.11 Raw-value events (no struct)

| Second topic | Payload type | Value | Version added | Stability |
|---|---|---|---|---|
| `"rate_form"` | `bool` | `true` = rate formula enabled; `false` = disabled | 1.0.0 | Stable |
| `"paused"` | `bool` | `true` = contract is paused | 1.0.0 | Stable |
| `"unpaused"` | `bool` | `false` = contract is unpaused | 1.0.0 | Stable |
| `"fee_set"` | `u32` | Protocol fee in basis points | 1.0.0 | Stable |
| `"fee_bnd"` | `(u32, u32)` | 1. `min_bps: u32`, 2. `max_bps: u32` | 1.0.0 | Stable |
| `"clsfctr"` | `u32` | Close factor in basis points | 1.0.0 | Stable |
| `"orc_cfg"` | `(u32, u64)` | 1. `max_deviation_bps: u32`, 2. `max_age_seconds: u64` | 1.0.0 | Stable |
| `"orc_price"` | `(i128, u64)` | 1. `price: i128`, 2. `timestamp: u64` | 1.0.0 | Stable |

---

## 5. Auction Contract Event Catalog

All topics are published under the `gateway-auction` contract. Publishers live in
`gateway-contract/contracts/auction_contract/src/events.rs`.

### 5.1 English auction events

| First topic | Second topic | Payload struct | Field order & types | Version added | Stability |
|---|---|---|---|---|---|
| `"BID_RFDN"` | `"auction"` | `BidRefundedEvent` | 1. `prev_bidder: Address`, 2. `amount: i128` | 1.0.0 | Stable |

### 5.2 Auction close event

| First topic | Second topic | Payload struct | Field order & types | Version added | Stability |
|---|---|---|---|---|---|
| `"AUC_CLOSE"` | `"auction"` | `AuctionClosedEvent` | 1. `auction_id: Symbol`, 2. `winner: Option<Address>`, 3. `amount: i128` | 1.0.0 | Stable |

### 5.3 Default liquidation settlement event

| First topic | Second topic | Payload struct | Field order & types | Version added | Stability |
|---|---|---|---|---|---|
| `"LIQ_SETL"` | `"auction"` | `DefaultLiquidationSettlementEvent` | 1. `auction_id: Symbol`, 2. `credit_contract: Address`, 3. `borrower: Address`, 4. `winner: Address`, 5. `recovered_amount: i128` | 1.0.0 | Stable |

---

## 6. Stability Matrix

| Event topic | Stable since | Deprecated | Removal target | Notes |
|---|---|---|---|---|
| `("credit","opened")` through `("credit","reinstate")` | 1.0.0 | No | — | Lifecycle lifecycle events share `CreditLineEvent` shape |
| `("credit","drawn")` | 1.0.0 | No | — | Use `drawn_v2` for richer traceability |
| `("credit","drawn_v2")` | 1.0.0 | No | — | New default draw event |
| `("credit","repay")` | 1.0.0 | No | — | |
| `("credit","accrue")` | 1.0.0 | No | — | |
| `("credit","fee_accrd")` | 1.1.0 | No | — | Extended with fee split fields in 1.1.0 |
| `("credit","late_fee")` | 1.0.0 | No | — | |
| `("credit","risk_upd")` | 1.0.0 | No | — | |
| `("credit","drw_freeze")` | 1.0.0 | No | — | Global toggle |
| `("credit","line_frz")` | 1.0.0 | No | — | Per-borrower toggle |
| `("credit","br_freeze")` | 1.0.0 | No | — | Time-bounded per-borrower freeze |
| `("credit","pen_enter")` | 1.0.0 | No | — | |
| `("credit","pen_exit")` | 1.0.0 | No | — | |
| `("credit","grace_wv")` | 1.0.0 | No | — | |
| `("credit","admin_prop")` | 1.0.0 | No | — | |
| `("credit","admin_acc")` | 1.0.0 | No | — | |
| `("credit","tre_prop")` | 1.0.0 | No | — | |
| `("credit","tre_exec")` | 1.0.0 | No | — | |
| `("credit","upgraded")` | 1.0.0 | No | — | |
| `("credit","liq_req")` | 1.0.0 | No | — | Raw tuple payload |
| `("credit","liq_setl")` | 1.0.0 | No | — | |
| `("credit","col_dep")` | 1.0.0 | No | — | |
| `("credit","col_wit")` | 1.0.0 | No | — | |
| `("credit","tok_resc")` | 1.0.0 | No | — | |
| `("credit","atst_bat")` | 1.0.0 | No | — | |
| `("credit","rate_form")` | 1.0.0 | No | — | Raw `bool` payload |
| `("credit","paused")` | 1.0.0 | No | — | Raw `bool` payload |
| `("credit","unpaused")` | 1.0.0 | No | — | Raw `bool` payload |
| `("credit","fee_set")` | 1.0.0 | No | — | Raw `u32` payload |
| `("credit","fee_bnd")` | 1.0.0 | No | — | Raw tuple payload |
| `("credit","clsfctr")` | 1.0.0 | No | — | Raw `u32` payload |
| `("credit","orc_cfg")` | 1.0.0 | No | — | Raw tuple payload |
| `("credit","orc_price")` | 1.0.0 | No | — | Raw tuple payload |
| `("blk_chg",)` | 1.0.0 | No | — | Single-element topic tuple |
| `("BID_RFDN","auction")` | 1.0.0 | No | — | |
| `("AUC_CLOSE","auction")` | 1.0.0 | No | — | |
| `("LIQ_SETL","auction")` | 1.0.0 | No | — | |

---

## 7. Type Definitions

All payload structs are defined in `contracts/credit/src/events.rs` or
`gateway-contract/contracts/auction_contract/src/events.rs` with `#[contracttype]`.

Shared types referenced in events:

- `CreditStatus` — `Active = 0`, `Suspended = 1`, `Defaulted = 2`, `Closed = 3`
  (defined in `contracts/credit/src/types.rs`).
- `FreezeReason` — enum for freeze source (defined in
  `contracts/credit/src/types.rs`).
- `GraceWaiverMode` — `FullWaiver`, `ReducedRate` (defined in
  `contracts/credit/src/types.rs`).

---

## 8. Publisher Reference

All publisher functions live in `contracts/credit/src/events.rs` (credit) and
`gateway-contract/contracts/auction_contract/src/events.rs` (auction). Each
publisher takes `&Env` plus the event-specific payload fields and calls
`env.events().publish(topic, payload)`.

| Publisher function | Event topic |
|---|---|
| `publish_credit_line_event` | `("credit", "opened"\|"suspend"\|"closed"\|"default"\|"reinstate")` |
| `publish_drawn_event` | `("credit", "drawn")` |
| `publish_drawn_event_v2` | `("credit", "drawn_v2")` |
| `publish_repayment_event` | `("credit", "repay")` |
| `publish_interest_accrued_event` | `("credit", "accrue")` |
| `publish_fee_accrued_event` | `("credit", "fee_accrd")` |
| `publish_late_fee_charged_event` | `("credit", "late_fee")` |
| `publish_draw_reversed_event` | `("credit", "draw_rev")` |
| `publish_draws_frozen_event` | `("credit", "drw_freeze")` |
| `publish_credit_line_freeze_event` | `("credit", "line_frz")` |
| `publish_borrower_frozen_event` | `("br_freeze",)` |
| `publish_penalty_rate_entered_event` | `("credit", "pen_enter")` |
| `publish_penalty_rate_exited_event` | `("credit", "pen_exit")` |
| `publish_grace_waiver_applied_event` | `("credit", "grace_wv")` |
| `publish_admin_rotation_proposed` | `("credit", "admin_prop")` |
| `publish_admin_rotation_accepted` | `("credit", "admin_acc")` |
| `publish_treasury_withdrawal_proposed` | `("credit", "tre_prop")` |
| `publish_treasury_withdrawal_executed` | `("credit", "tre_exec")` |
| `publish_contract_upgraded_event` | `("credit", "upgraded")` |
| `publish_default_liquidation_requested_event` | `("credit", "liq_req")` |
| `publish_default_liquidation_settled_event` | `("credit", "liq_setl")` |
| `publish_borrower_blocked_event` | `("blk_chg",)` |
| `publish_collateral_deposited_event` | `("credit", "col_dep")` |
| `publish_collateral_withdrawn_event` | `("credit", "col_wit")` |
| `publish_token_rescued_event` | `("credit", "tok_resc")` |
| `publish_attestation_batch_committed` | `("credit", "atst_bat")` |
| `publish_rate_formula_config_event` | `("credit", "rate_form")` |
| `publish_paused_event` | `("credit", "paused")` / `("credit", "unpaused")` |
| `publish_protocol_fee_bps_set_event` | `("credit", "fee_set")` |
| `publish_protocol_fee_bounds_set_event` | `("credit", "fee_bnd")` |
| `publish_close_factor_bps_set_event` | `("credit", "clsfctr")` |
| `publish_oracle_config_set_event` | `("credit", "orc_cfg")` |
| `publish_oracle_price_accepted_event` | `("credit", "orc_price")` |
| `publish_risk_parameters_updated` | `("credit", "risk_upd")` |
| `publish_bid_refunded_event` | `("BID_RFDN", "auction")` |
| `publish_auction_closed_event` | `("AUC_CLOSE", "auction")` |
| `publish_default_liquidation_settlement_event` | `("LIQ_SETL", "auction")` |

---

## 9. API / Visible Changes Log

| Date | Change | Impact |
|---|---|---|
| 2026-06-28 | Created `docs/EVENTS_CATALOG.md` | Replaces `docs/events-schema.md` as the single authoritative catalog. `events-schema.md` retained for backward-compatible linking but no longer updated. |
| 2026-06-28 | Added `AttestationBatchCommittedEvent` and `publish_attestation_batch_committed` to `contracts/credit/src/events.rs` | Fixed broken import in `contracts/credit/src/attestation.rs`. No contract ABI change; event was already emitted by `commit_attestation_batch` but the struct and publisher were missing. |
| 2026-06-28 | Added `contracts/credit/tests/events_catalog.rs` | New integration test verifying every cataloged event is emitted with the correct topic and payload shape. |

---

## 10. Related Documentation

- [`docs/events-schema.md`](./events-schema.md) — legacy schema reference (superseded by this file).
- [`docs/indexer-integration.md`](./indexer-integration.md) — decoder patterns and RPC examples.
- [`docs/PROTOCOL_SPEC.md`](./PROTOCOL_SPEC.md) — per-entrypoint event-emission table.
- [`docs/ARCHITECTURE.md`](./ARCHITECTURE.md) — event topology diagrams.
- [`docs/credit.md`](./credit.md) — master credit contract reference.
- [`docs/upgrade-policy.md`](./upgrade-policy.md) — WASM upgrade procedure.
