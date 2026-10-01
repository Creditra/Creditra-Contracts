# Event Catalog — Creditra Contracts

**Issue:** #1305  
**Version:** 1.2
**Status:** Authoritative — single source of truth for all Creditra events  
**Scope:** `creditra-credit` (`contracts/credit/`), `gateway-auction`
(`gateway-contract/contracts/auction_contract/`), `creditra-accrual`
(`contracts/accrual/`)  
**Validated by:** `contracts/credit/tests/events_catalog.rs`

```bash
cargo test -p creditra-credit --test events_catalog
```

This document replaces the former `EVENT_SCHEMA.md`, `events-schema.md`, and
`EVENTS_CATALOG.md` files (all deleted). The source of truth for every topic
string and payload field is `contracts/credit/src/events.rs`. Any change to
that file must be reflected here.

---

## Versioning Policy

The contract API version is defined in `contracts/credit/src/lib.rs` as
`CONTRACT_API_VERSION = (1, 1, 0)`. Event schema follows SemVer-style rules:

- **Major:** Breaking changes (rename/remove/reorder fields, topic name change)
  require a new topic with a `_vN` suffix and a contract API major bump.
- **Minor:** New event topic or new optional field at the end of an existing
  payload struct. Requires a minor version bump.
- **Patch:** Bug fixes only; no structural changes.

When a breaking change is required, the old topic stays alive during a
dual-publish window while downstream indexers migrate, then is removed.

---

## Topic Encoding

Topics are Soroban `Symbol` values. Short forms (≤ 9 chars) use the cheap
`SCV_SYMBOL` encoding via `symbol_short!`; longer forms use `Symbol::new`.
**Symbols must not contain hyphens** — use underscores.

---

## 1. Credit Contract Events

Source: `contracts/credit/src/events.rs`. All topics are in the `("credit", ...)`
namespace unless noted.

### 1.1 Lifecycle events

| Second topic | Payload struct | Field order & types | Added |
|---|---|---|---|
| `"opened"` | `CreditLineEvent` | `borrower: Address`, `status: CreditStatus`, `credit_limit: i128`, `interest_rate_bps: u32`, `risk_score: u32` | 1.0.0 |
| `"suspend"` | `CreditLineEvent` | same as opened | 1.0.0 |
| `"closed"` | `CreditLineEvent` | same as opened | 1.0.0 |
| `"defaulted"` | `CreditLineEvent` | same as opened | 1.0.0 |
| `"reinstate"` | `CreditLineEvent` | same as opened | 1.0.0 |

**Publisher:** `publish_credit_line_event`

### 1.2 Draw and repayment events

| Second topic | Payload struct | Field order & types | Added |
|---|---|---|---|
| `"drawn"` | `DrawnEvent` | `borrower: Address`, `amount: i128`, `new_utilized_amount: i128`, `timestamp: u64` | 1.0.0 |
| `"drawn_v2"` | `DrawnEventV2` | `borrower: Address`, `recipient: Address`, `reserve_source: Address`, `amount: i128`, `new_utilized_amount: i128`, `timestamp: u64` | 1.0.0 |
| `"repay"` | `RepaymentEvent` | `borrower: Address`, `amount: i128`, `new_utilized_amount: i128` | 1.0.0 |
| `"draw_rev"` | `DrawReversedEvent` | `borrower: Address`, `amount: i128`, `original_ts: u64`, `reason_code: u32`, `new_utilized_amount: i128`, `timestamp: u64`, `admin: Address`, `accounting_only: bool` | 1.0.0 |

**Publishers:** `publish_drawn_event`, `publish_drawn_event_v2`,
`publish_repayment_event`, `publish_draw_reversed_event`

### 1.3 Accrual and fee events

| Second topic | Payload struct | Field order & types | Added |
|---|---|---|---|
| `"accrue"` | `InterestAccruedEvent` | `borrower: Address`, `accrued_amount: i128`, `new_utilized_amount: i128` | 1.0.0 |
| `"fee_accrd"` | `FeeAccruedEvent` | `borrower: Address`, `fee_amount: i128`, `treasury_amount: i128`, `bounty_amount: i128`, `new_treasury_balance: i128`, `new_bounty_balance: i128` | 1.1.0 |
| `"late_fee"` | `LateFeeChargedEvent` | `borrower: Address`, `fee: i128`, `installment_index: u64` | 1.0.0 |

**Publishers:** `publish_interest_accrued_event`, `publish_fee_accrued_event`,
`publish_late_fee_charged_event`

### 1.4 Risk and parameter events

| Second topic | Payload struct | Field order & types | Added |
|---|---|---|---|
| `"risk_upd"` | `RiskParametersUpdatedEvent` | `borrower: Address`, `credit_limit: i128`, `interest_rate_bps: u32`, `risk_score: u32` | 1.0.0 |
| `"drw_freeze"` | `DrawsFrozenEvent` | `frozen: bool`, `reason: FreezeReason` | 1.0.0 |
| `"line_frz"` | `CreditLineFreezeEvent` | `borrower: Address`, `frozen: bool`, `reason: FreezeReason`, `ledger: u32` | 1.0.0 |
| `"pen_enter"` | `PenaltyRateEnteredEvent` | `borrower: Address`, `base_rate_bps: u32`, `penalty_surcharge_bps: u32`, `effective_rate_bps: u32` | 1.0.0 |
| `"pen_exit"` | `PenaltyRateExitedEvent` | `borrower: Address`, `previous_rate_bps: u32`, `new_rate_bps: u32` | 1.0.0 |
| `"grace_wv"` | `GraceWaiverReceiptEvent` | `borrower: Address`, `waived_amount: i128`, `mode: GraceWaiverMode` | 1.0.0 |

**Publishers:** `publish_risk_parameters_updated`, `publish_draws_frozen_event`,
`publish_credit_line_freeze_event`, `publish_penalty_rate_entered_event`,
`publish_penalty_rate_exited_event`, `publish_grace_waiver_receipt_event`

### 1.5 Admin and governance events

| Second topic | Payload struct | Field order & types | Added |
|---|---|---|---|
| `"admin_prop"` | `AdminRotationProposedEvent` | `proposed_admin: Address`, `accept_after: u64` | 1.0.0 |
| `"admin_acc"` | `AdminRotationAcceptedEvent` | `new_admin: Address` | 1.0.0 |
| `"tre_prop"` | `TreasuryWithdrawalProposedEvent` | `recipient: Address`, `amount: i128`, `proposer: Address`, `proposed_at: u64`, `execute_after: u64` | 1.0.0 |
| `"tre_exec"` | `TreasuryWithdrawalExecutedEvent` | `recipient: Address`, `amount: i128`, `executor: Address`, `executed_at: u64` | 1.0.0 |
| `"upgraded"` | `ContractUpgradedEvent` | `old_wasm_hash: BytesN<32>`, `new_wasm_hash: BytesN<32>` | 1.0.0 |

**Publishers:** `publish_admin_rotation_proposed`, `publish_admin_rotation_accepted`,
`publish_treasury_withdrawal_proposed`, `publish_treasury_withdrawal_executed`,
`publish_contract_upgraded_event`

### 1.6 Blocklist and freeze events

| Topic tuple | Payload struct | Field order & types | Added |
|---|---|---|---|
| `("blk_chg",)` | `BorrowerBlockedEvent` | `borrower: Address`, `blocked: bool`, `ledger: u32` | 1.0.0 |
| `("br_freeze",)` | `BorrowerFrozenEvent` | `borrower: Address`, `frozen_until: u64`, `ledger: u32` | 1.0.0 |

> These events use a **single-element** topic tuple — indexers must handle
> `topics.len() == 1` for these two topics.

**Publishers:** `publish_borrower_blocked_event`, `publish_borrower_frozen_event`

### 1.7 Collateral events

| Second topic | Payload struct | Field order & types | Added |
|---|---|---|---|
| `"col_dep"` | `CollateralDepositedEvent` | `borrower: Address`, `amount: i128`, `new_balance: i128` | 1.0.0 |
| `"col_wit"` | `CollateralWithdrawnEvent` | `borrower: Address`, `amount: i128`, `new_balance: i128` | 1.0.0 |
| `"col_prel"` | `CollateralPartialReleasedEvent` | `borrower: Address`, `amount_released: i128`, `new_balance: i128`, `health_factor_bps: u32` | 1.0.0 |

**Publishers:** `publish_collateral_deposited_event`,
`publish_collateral_withdrawn_event`, `publish_collateral_partial_released_event`

### 1.8 Default liquidation events

| Second topic | Payload | Field order & types | Added |
|---|---|---|---|
| `"liq_req"` | Raw tuple `(Address, i128)` | `borrower: Address`, `utilized_amount: i128` | 1.0.0 |
| `"liq_setl"` | `DefaultLiquidationSettledEvent` | `borrower: Address`, `settlement_id: Symbol`, `recovered_amount: i128`, `remaining_utilized_amount: i128`, `status: CreditStatus`, `close_factor_bps: u32` | 1.0.0 |

**Publishers:** `publish_default_liquidation_requested_event`,
`publish_default_liquidation_settled_event`

### 1.9 Attestation events

| Second topic | Payload struct | Field order & types | Added |
|---|---|---|---|
| `"atst_bat"` | `AttestationBatchCommittedEvent` | `borrower: Address`, `merkle_root: BytesN<32>`, `count: u32` | 1.0.0 |

**Publisher:** `publish_attestation_batch_committed`

### 1.10 Rescue events

| Second topic | Payload struct | Field order & types | Added |
|---|---|---|---|
| `"tok_resc"` | `TokenRescuedEvent` | `token: Address`, `recipient: Address`, `amount: i128` | 1.0.0 |

**Publisher:** `publish_token_rescued_event`

### 1.11 Raw-value events (no struct)

| Second topic | Payload type | Value | Publisher |
|---|---|---|---|
| `"rate_form"` | `bool` | `true` = formula enabled | `publish_rate_formula_config_event` |
| `"paused"` | `bool` | `true` = paused | `publish_paused_event` |
| `"unpaused"` | `bool` | `false` = unpaused | `publish_paused_event` |
| `"fee_bps"` | `u32` | Protocol fee bps | `publish_protocol_fee_bps_set_event` |
| `"fee_bnds"` | `(u32, u32)` | `(min_bps, max_bps)` | `publish_protocol_fee_bounds_set_event` |
| `"clsfctr"` | `u32` | Close factor bps | `publish_close_factor_bps_set_event` |
| `"orc_cfg"` | `(u32, u64)` | `(max_deviation_bps, max_age_seconds)` | `publish_oracle_config_set_event` |
| `"orc_price"` | `(i128, u64)` | `(price, timestamp)` | `publish_oracle_price_accepted_event` |
| `"orc_qcfg"` | `(u32, u32, u64)` | `(min_quorum_k, max_deviation_bps, max_age_seconds)` | `publish_oracle_quorum_config_set_event` |
| `"orc_qprc"` | `(i128, u32, u64)` | `(price, quorum_k, timestamp)` | `publish_oracle_quorum_price_set_event` |

### 1.12 Oracle reference recovery event

| Second topic | Payload struct | Field order & types | Added |
|---|---|---|---|
| `"orc_refreshed"` | `OracleReferenceRefreshedEvent` | `previous_price: i128`, `previous_timestamp: u64`, `new_price: i128`, `timestamp: u64` | 1.2.0 |

**Publisher:** `publish_oracle_reference_refreshed_event`. Emitted when the
admin re-anchors a stale single-oracle reference through
`refresh_oracle_reference`.

### 1.13 Oracle registry events

| Second topic | Payload struct | Fields (in order) | Version added |
|---|---|---|---|
| `"orc_add"` | `OracleAddedEvent` | `oracle: Address`, `weight: u32`, `timestamp: u64` | 1.1.0 |
| `"orc_rmv"` | `OracleRemovedEvent` | `oracle: Address`, `timestamp: u64` | 1.1.0 |
| `"orc_qthrs"` | `OracleQuorumThresholdSetEvent` | `threshold: u32`, `timestamp: u64` | 1.1.0 |
| `"orc_win"` | `OracleReportingWindowSetEvent` | `window_seconds: u64`, `timestamp: u64` | 1.1.0 |
| `"orc_rpt"` | `OracleValueReportedEvent` | `oracle: Address`, `value: u128`, `timestamp: u64` | 1.1.0 |

Emitted by `add_oracle`, `remove_oracle`, `set_quorum_threshold`,
`set_reporting_window`, and `report_value` so indexers can audit the
weighted-median oracle registry (Issue #1264).

---

## 2. Accrual Contract Events

Source: `contracts/accrual/src/events.rs`. Namespace: `("accrual", ...)`.

| Second topic | Payload struct | Field order & types | Added |
|---|---|---|---|
| `"batch"` | `AccrualBatchCompletedEvent` | `borrowers_processed: u32`, `lines_accrued: u32`, `total_interest_accrued: i128`, `timestamp: u64` | 1.0.0 |
| `"accrue"` | `accrual::InterestAccruedEvent` | `borrower: Address`, `accrued_amount: i128`, `new_utilized_amount: i128`, `new_accrued_interest: i128`, `elapsed_seconds: u64`, `timestamp: u64` | 1.0.0 |

**Publishers:** `publish_accrual_batch_completed`, `publish_interest_accrued`

---

## 3. Auction Contract Events

Source: `gateway-contract/contracts/auction_contract/src/events.rs`.

| First topic | Second topic | Payload struct | Field order & types | Added |
|---|---|---|---|---|
| `"BID_RFDN"` | `"auction"` | `BidRefundedEvent` | `prev_bidder: Address`, `amount: i128` | 1.0.0 |
| `"AUC_CLOSE"` | `"auction"` | `AuctionClosedEvent` | `auction_id: Symbol`, `winner: Option<Address>`, `amount: i128` | 1.0.0 |
| `"LIQ_SETL"` | `"auction"` | `DefaultLiquidationSettlementEvent` | `auction_id: Symbol`, `credit_contract: Address`, `borrower: Address`, `winner: Address`, `recovered_amount: i128` | 1.0.0 |

**Publishers:** `publish_bid_refunded_event`, `publish_auction_closed_event`,
`publish_default_liquidation_settlement_event`

---

## 4. Shared Type Definitions

All defined in `contracts/credit/src/types.rs`:

| Type | Variants |
|---|---|
| `CreditStatus` | `Active=0`, `Suspended=1`, `Defaulted=2`, `Closed=3`, `Restricted=4` |
| `FreezeReason` | `LiquidityReserve=0`, `Compliance=1`, `RiskInvestigation=2`, `OperationalMaintenance=3`, `BorrowerRequest=4`, `AdminAction=5` |
| `GraceWaiverMode` | `FullWaiver`, `ReducedRate` |

---

## 5. Complete Publisher Reference

| Publisher function | Topic |
|---|---|
| `publish_credit_line_event` | `("credit", "opened"\|"suspend"\|"closed"\|"defaulted"\|"reinstate")` |
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
| `publish_grace_waiver_receipt_event` | `("credit", "grace_wv")` |
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
| `publish_collateral_partial_released_event` | `("credit", "col_prel")` |
| `publish_token_rescued_event` | `("credit", "tok_resc")` |
| `publish_attestation_batch_committed` | `("credit", "atst_bat")` |
| `publish_rate_formula_config_event` | `("credit", "rate_form")` |
| `publish_paused_event` | `("credit", "paused")` / `("credit", "unpaused")` |
| `publish_protocol_fee_bps_set_event` | `("credit", "fee_bps")` |
| `publish_protocol_fee_bounds_set_event` | `("credit", "fee_bnds")` |
| `publish_close_factor_bps_set_event` | `("credit", "clsfctr")` |
| `publish_oracle_config_set_event` | `("credit", "orc_cfg")` |
| `publish_oracle_price_accepted_event` | `("credit", "orc_price")` |
| `publish_oracle_reference_refreshed_event` | `("credit", "orc_refreshed")` |
| `publish_oracle_quorum_config_set_event` | `("credit", "orc_qcfg")` |
| `publish_oracle_quorum_price_set_event` | `("credit", "orc_qprc")` |
| `publish_oracle_added_event` | `("credit", "orc_add")` |
| `publish_oracle_removed_event` | `("credit", "orc_rmv")` |
| `publish_oracle_quorum_threshold_set_event` | `("credit", "orc_qthrs")` |
| `publish_oracle_reporting_window_set_event` | `("credit", "orc_win")` |
| `publish_oracle_value_reported_event` | `("credit", "orc_rpt")` |
| `publish_risk_parameters_updated` | `("credit", "risk_upd")` |
| `publish_bid_refunded_event` | `("BID_RFDN", "auction")` |
| `publish_auction_closed_event` | `("AUC_CLOSE", "auction")` |
| `publish_default_liquidation_settlement_event` | `("LIQ_SETL", "auction")` |
| `publish_accrual_batch_completed` | `("accrual", "batch")` |
| `publish_interest_accrued` | `("accrual", "accrue")` |

---

## 6. Maintenance

When adding a new `publish_*` function to `events.rs`:

1. Add a row to the appropriate section above.
2. Add a test to `contracts/credit/tests/events_catalog.rs` that calls the
   publisher inside `env.as_contract(...)` and asserts the topic tuple.
3. Run `cargo test -p creditra-credit --test events_catalog` to confirm.

---

## 7. Related Documentation

- [`docs/indexer-integration.md`](./indexer-integration.md) — decoder patterns and RPC examples
- [`docs/PROTOCOL_SPEC.md`](./PROTOCOL_SPEC.md) — per-entrypoint event-emission table
- [`docs/ARCHITECTURE.md`](./ARCHITECTURE.md) — event topology diagrams
- `contracts/credit/src/events.rs` — source of truth for all credit event structs and publishers
- `contracts/credit/tests/events_catalog.rs` — validation test (run on every CI pass)
- `contracts/credit/tests/event_topic_stability.rs` — topic string pin tests
