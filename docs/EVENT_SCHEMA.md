# Event Schema Documentation

**Version:** 1.0  
**Status:** Authoritative  
**Scope:** `creditra-credit` (`contracts/credit/`) and `gateway-auction` (`gateway-contract/contracts/auction_contract/`)  
**Last updated:** 2026-06-29

---

## 1. Overview

This document provides a structured schema for all events emitted by the Creditra credit and auction contracts. It documents every event topic, payload structure, field types, and version stability guarantees to enable off-chain indexers, orchestrators, and integrators to reliably decode and process events.

---

## 2. Versioning Policy

### Contract API Version

The contract API version is defined in `contracts/credit/src/lib.rs` as `CONTRACT_API_VERSION = (1, 0, 0)`.

### SemVer-Style Event Schema Versioning

Event schema follows strict SemVer-style rules:

- **Major (Breaking):** Breaking changes require a new event topic with a `_vN` suffix and a contract API major version bump. Breaking changes include:
  - Renaming, removing, or reordering fields in a payload struct
  - Changing a field's type
  - Changing a topic string
  - Semantic changes to event meaning

- **Minor:** A new event topic or a new optional field at the end of an existing payload struct. Requires a contract API minor version bump.

- **Patch:** Bug fixes only; no structural changes to topics or payloads.

### Topic Suffix Convention

When a breaking change is required, introduce a new topic with a suffix and keep the old topic alive during a dual-publish window:

```
("credit", "drawn_v2")   // new version
("credit", "drawn")      // legacy version; still emitted
```

Remove the legacy topic only after downstream indexers confirm cutover.

---

## 3. Topic Encoding

Topics are Soroban `Symbol` values chosen to use the cheap `SCV_SYMBOL` on-chain encoding (≤ 9 characters). Symbols longer than 9 characters use `Symbol::new(env, "<longer-name>")` and cost more gas to publish.

| Encoding rule | Limit |
|---|---|
| `symbol_short!` macro | ≤ 9 characters |
| `Symbol::new` | Up to 32 characters |

The first topic in every published tuple is either `"credit"` (credit contract) or a short identifier such as `"blk_chg"` or `"BID_RFDN"`.

---

## 4. Credit Contract Events

All credit contract events are published under the `("credit", "...")` namespace unless otherwise noted. Publishers live in `contracts/credit/src/events.rs`.

### 4.1 Lifecycle Events

#### Event: Credit Line State Changes

**Topic:** `("credit", "opened")`, `("credit", "suspend")`, `("credit", "closed")`, `("credit", "default")`, `("credit", "reinstate")`

**Payload Struct:** `CreditLineEvent`

**Fields:**
1. `borrower: Address` - The borrower whose credit line state changed
2. `status: CreditStatus` - New status of the credit line (Active=0, Suspended=1, Defaulted=2, Closed=3)
3. `credit_limit: i128` - Maximum credit limit
4. `interest_rate_bps: u32` - Interest rate in basis points
5. `risk_score: u32` - Risk assessment score

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_credit_line_event`

---

### 4.2 Draw and Repayment Events

#### Event: Draw Executed

**Topic:** `("credit", "drawn")`

**Payload Struct:** `DrawnEvent`

**Fields:**
1. `borrower: Address` - The borrower who drew funds
2. `amount: i128` - Amount drawn
3. `new_utilized_amount: i128` - Total utilized amount after draw

**Version Added:** 1.0.0  
**Stability:** Stable (legacy)  
**Publisher:** `publish_drawn_event`

---

#### Event: Draw Executed (V2)

**Topic:** `("credit", "drawn_v2")`

**Payload Struct:** `DrawnEventV2`

**Fields:**
1. `borrower: Address` - The borrower who drew funds
2. `recipient: Address` - Recipient of the drawn funds
3. `reserve_source: Address` - Source reserve address
4. `amount: i128` - Amount drawn
5. `new_utilized_amount: i128` - Total utilized amount after draw
6. `timestamp: u64` - Ledger timestamp of the draw

**Version Added:** 1.0.0  
**Stability:** Stable (new default)  
**Publisher:** `publish_drawn_event_v2`

---

#### Event: Repayment Made

**Topic:** `("credit", "repay")`

**Payload Struct:** `RepaymentEvent`

**Fields:**
1. `borrower: Address` - The borrower who made the repayment
2. `amount: i128` - Amount repaid
3. `new_utilized_amount: i128` - Total utilized amount after repayment

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_repayment_event`

---

#### Event: Draw Reversed

**Topic:** `("credit", "draw_rev")`

**Payload Struct:** `DrawReversedEvent`

**Fields:**
1. `borrower: Address` - The borrower whose draw was reversed
2. `amount: i128` - Amount reversed
3. `original_ts: u64` - Original draw timestamp
4. `reason_code: u32` - Reason code for reversal
5. `new_utilized_amount: i128` - Total utilized amount after reversal
6. `timestamp: u64` - Reversal timestamp
7. `admin: Address` - Admin who performed the reversal
8. `accounting_only: bool` - Whether reversal was accounting-only

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_draw_reversed_event`

---

### 4.3 Accrual and Fee Events

#### Event: Interest Accrued

**Topic:** `("credit", "accrue")`

**Payload Struct:** `InterestAccruedEvent`

**Fields:**
1. `borrower: Address` - The borrower for whom interest accrued
2. `accrued_amount: i128` - Amount of interest accrued
3. `new_utilized_amount: i128` - Total utilized amount after accrual

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_interest_accrued_event`

---

#### Event: Fee Accrued

**Topic:** `("credit", "fee_accrd")`

**Payload Struct:** `FeeAccruedEvent`

**Fields:**
1. `borrower: Address` - The borrower for whom fees were accrued
2. `fee_amount: i128` - Total protocol fee skimmed from repayment
3. `treasury_amount: i128` - Treasury portion credited to TreasuryBalance
4. `bounty_amount: i128` - Bounty pool portion credited to BountyBalance
5. `new_treasury_balance: i128` - New treasury balance after fee
6. `new_bounty_balance: i128` - New bounty balance after fee

**Version Added:** 1.1.0  
**Stability:** Stable  
**Publisher:** `publish_fee_accrued_event`

---

#### Event: Late Fee Charged

**Topic:** `("credit", "late_fee")`

**Payload Struct:** `LateFeeChargedEvent`

**Fields:**
1. `borrower: Address` - The borrower charged with late fee
2. `fee: i128` - Late fee amount
3. `installment_index: u64` - Index of the missed installment

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_late_fee_charged_event`

---

### 4.4 Risk and Parameter Events

#### Event: Risk Parameters Updated

**Topic:** `("credit", "risk_upd")`

**Payload Struct:** `RiskParametersUpdatedEvent`

**Fields:**
1. `borrower: Address` - The borrower whose risk parameters were updated
2. `credit_limit: i128` - New credit limit
3. `interest_rate_bps: u32` - New interest rate in basis points
4. `risk_score: u32` - New risk score

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_risk_parameters_updated`

---

#### Event: Draws Frozen

**Topic:** `("credit", "drw_freeze")`

**Payload Struct:** `DrawsFrozenEvent`

**Fields:**
1. `frozen: bool` - Whether draws are frozen (true) or unfrozen (false)
2. `reason: FreezeReason` - Reason for the freeze action

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_draws_frozen_event`

---

#### Event: Credit Line Freeze

**Topic:** `("credit", "line_frz")`

**Payload Struct:** `CreditLineFreezeEvent`

**Fields:**
1. `borrower: Address` - The borrower whose credit line was frozen/unfrozen
2. `reason: FreezeReason` - Structured reason for the freeze action
3. `frozen: bool` - true when frozen; false when unfrozen
4. `ledger: u32` - Ledger sequence at time of change (for off-chain indexers)

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_credit_line_freeze_event`

---

#### Event: Borrower Frozen

**Topic:** `("br_freeze",)`

**Payload Struct:** `BorrowerFrozenEvent`

**Fields:**
1. `borrower: Address` - The borrower frozen/unfrozen
2. `frozen_until: u64` - Timestamp (ledger seconds) until which draws are frozen
3. `ledger: u32` - Ledger sequence at time of change (for off-chain indexers)

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_borrower_frozen_event`

---

#### Event: Penalty Rate Entered

**Topic:** `("credit", "pen_enter")`

**Payload Struct:** `PenaltyRateEnteredEvent`

**Fields:**
1. `borrower: Address` - The borrower entering penalty rate
2. `base_rate_bps: u32` - Base interest rate in basis points
3. `penalty_surcharge_bps: u32` - Penalty surcharge in basis points
4. `effective_rate_bps: u32` - Effective total rate in basis points

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_penalty_rate_entered_event`

---

#### Event: Penalty Rate Exited

**Topic:** `("credit", "pen_exit")`

**Payload Struct:** `PenaltyRateExitedEvent`

**Fields:**
1. `borrower: Address` - The borrower exiting penalty rate
2. `previous_rate_bps: u32` - Previous penalty rate in basis points
3. `new_rate_bps: u32` - New rate in basis points

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_penalty_rate_exited_event`

---

#### Event: Grace Waiver Receipt

**Topic:** `("credit", "grace_wv")`

**Payload Struct:** `GraceWaiverReceiptEvent`

**Fields:**
1. `borrower: Address` - The borrower receiving grace waiver
2. `waived_amount: i128` - Amount of interest waived
3. `mode: GraceWaiverMode` - Mode of grace waiver (FullWaiver or ReducedRate)

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_grace_waiver_receipt_event`

---

### 4.5 Admin and Governance Events

#### Event: Admin Rotation Proposed

**Topic:** `("credit", "admin_prop")`

**Payload Struct:** `AdminRotationProposedEvent`

**Fields:**
1. `proposed_admin: Address` - Address of the proposed new admin
2. `accept_after: u64` - Earliest timestamp when the proposal can be accepted

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_admin_rotation_proposed`

---

#### Event: Admin Rotation Accepted

**Topic:** `("credit", "admin_acc")`

**Payload Struct:** `AdminRotationAcceptedEvent`

**Fields:**
1. `new_admin: Address` - Address of the new admin

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_admin_rotation_accepted`

---

#### Event: Treasury Withdrawal Proposed

**Topic:** `("credit", "tre_prop")`

**Payload Struct:** `TreasuryWithdrawalProposedEvent`

**Fields:**
1. `recipient: Address` - Treasury recipient address
2. `amount: i128` - Snapshot of treasury balance at proposal time
3. `proposer: Address` - Admin who submitted the proposal
4. `proposed_at: u64` - Ledger timestamp when proposal was created
5. `execute_after: u64` - Earliest timestamp for execution (proposed_at + 86_400)

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_treasury_withdrawal_proposed`

---

#### Event: Treasury Withdrawal Executed

**Topic:** `("credit", "tre_exec")`

**Payload Struct:** `TreasuryWithdrawalExecutedEvent`

**Fields:**
1. `recipient: Address` - Treasury recipient address
2. `amount: i128` - Amount transferred
3. `executor: Address` - Admin who executed the withdrawal
4. `executed_at: u64` - Ledger timestamp at execution

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_treasury_withdrawal_executed`

---

#### Event: Contract Upgraded

**Topic:** `("credit", "upgraded")`

**Payload Struct:** `ContractUpgradedEvent`

**Fields:**
1. `old_wasm_hash: BytesN<32>` - Previous WASM hash
2. `new_wasm_hash: BytesN<32>` - New WASM hash

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_contract_upgraded_event`

---

### 4.6 Blocklist Events

#### Event: Borrower Blocked/Unblocked

**Topic:** `("blk_chg",)`

**Payload Struct:** `BorrowerBlockedEvent`

**Fields:**
1. `borrower: Address` - The borrower blocked/unblocked
2. `blocked: bool` - true = blocked; false = unblocked
3. `ledger: u32` - Ledger sequence at time of change (for off-chain indexers)

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_borrower_blocked_event`

---

### 4.7 Collateral Events

#### Event: Collateral Deposited

**Topic:** `("credit", "col_dep")`

**Payload Struct:** `CollateralDepositedEvent`

**Fields:**
1. `borrower: Address` - The borrower depositing collateral
2. `amount: i128` - Amount deposited
3. `new_balance: i128` - New collateral balance after deposit

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_collateral_deposited_event`

---

#### Event: Collateral Partial Released

**Topic:** `("credit", "col_prel")`

**Payload Struct:** `CollateralPartialReleasedEvent`

**Fields:**
1. `borrower: Address` - Borrower whose collateral is being partially released
2. `amount_released: i128` - Token amount returned to the borrower
3. `new_balance: i128` - Collateral balance remaining after the release
4. `health_factor_bps: u32` - Health factor after release (u32::MAX when utilized_amount == 0)

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_collateral_partial_released_event`

---

#### Event: Collateral Withdrawn

**Topic:** `("credit", "col_wit")`

**Payload Struct:** `CollateralWithdrawnEvent`

**Fields:**
1. `borrower: Address` - The borrower withdrawing collateral
2. `amount: i128` - Amount withdrawn
3. `new_balance: i128` - New collateral balance after withdrawal

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_collateral_withdrawn_event`

---

### 4.8 Default Liquidation Events

#### Event: Default Liquidation Requested

**Topic:** `("credit", "liq_req")`

**Payload Type:** Raw tuple `(Address, i128)`

**Fields:**
1. `borrower: Address` - The borrower in default
2. `utilized_amount: i128` - Amount utilized at time of default

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_default_liquidation_requested_event`

---

#### Event: Default Liquidation Settled

**Topic:** `("credit", "liq_setl")`

**Payload Struct:** `DefaultLiquidationSettledEvent`

**Fields:**
1. `borrower: Address` - The borrower whose liquidation was settled
2. `settlement_id: Symbol` - Unique identifier for the settlement
3. `recovered_amount: i128` - Amount recovered from auction
4. `remaining_utilized_amount: i128` - Remaining debt after settlement
5. `status: CreditStatus` - New credit line status after settlement
6. `close_factor_bps: u32` - Close factor in basis points

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_default_liquidation_settled_event`

---

### 4.9 Attestation Events

#### Event: Attestation Batch Committed

**Topic:** `("credit", "atst_bat")`

**Payload Struct:** `AttestationBatchCommittedEvent`

**Fields:**
1. `borrower: Address` - Borrower whose attestation batch was updated
2. `merkle_root: BytesN<32>` - SHA-256 Merkle root of all leaf hashes in the committed batch
3. `count: u32` - Number of leaves in the batch (informational)

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_attestation_batch_committed`

---

### 4.10 Rescue Events

#### Event: Token Rescued

**Topic:** `("credit", "tok_resc")`

**Payload Struct:** `TokenRescuedEvent`

**Fields:**
1. `token: Address` - Token address being rescued
2. `recipient: Address` - Recipient of rescued tokens
3. `amount: i128` - Amount rescued

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_token_rescued_event`

---

### 4.11 Raw-Value Events (No Struct)

#### Event: Rate Formula Configured

**Topic:** `("credit", "rate_form")`

**Payload Type:** `bool`

**Value:** `true` = rate formula enabled; `false` = disabled

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_rate_formula_config_event`

---

#### Event: Contract Paused/Unpaused

**Topic:** `("credit", "paused")` or `("credit", "unpaused")`

**Payload Type:** `bool`

**Value:** `true` = contract is paused; `false` = contract is unpaused

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_paused_event`

---

#### Event: Protocol Fee BPS Set

**Topic:** `("credit", "fee_set")`

**Payload Type:** `u32`

**Value:** Protocol fee in basis points

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_protocol_fee_bps_set_event`

---

#### Event: Protocol Fee Bounds Set

**Topic:** `("credit", "fee_bnd")`

**Payload Type:** `(u32, u32)`

**Fields:**
1. `min_bps: u32` - Minimum fee in basis points
2. `max_bps: u32` - Maximum fee in basis points

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_protocol_fee_bounds_set_event`

---

#### Event: Close Factor BPS Set

**Topic:** `("credit", "clsfctr")`

**Payload Type:** `u32`

**Value:** Close factor in basis points

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_close_factor_bps_set_event`

---

#### Event: Oracle Config Set

**Topic:** `("credit", "orc_cfg")`

**Payload Type:** `(u32, u64)`

**Fields:**
1. `max_deviation_bps: u32` - Maximum allowed price deviation in basis points
2. `max_age_seconds: u64` - Maximum age of oracle price in seconds

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_oracle_config_set_event`

---

#### Event: Oracle Price Accepted

**Topic:** `("credit", "orc_price")`

**Payload Type:** `(i128, u64)`

**Fields:**
1. `price: i128` - Accepted oracle price
2. `timestamp: u64` - Price timestamp

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_oracle_price_accepted_event`

---

#### Event: Oracle Quorum Config Set

**Topic:** `("credit", "orc_qcfg")`

**Payload Type:** `(u32, u32, u64)`

**Fields:**
1. `min_quorum_k: u32` - Minimum quorum size
2. `max_deviation_bps: u32` - Maximum deviation for quorum
3. `max_age_seconds: u64` - Maximum age for quorum

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_oracle_quorum_config_set_event`

---

#### Event: Oracle Quorum Price Set

**Topic:** `("credit", "orc_qprc")`

**Payload Type:** `(i128, u32, u64)`

**Fields:**
1. `price: i128` - Resolved quorum price
2. `quorum_k: u32` - Quorum size used
3. `timestamp: u64` - Price timestamp

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_oracle_quorum_price_set_event`

---

## 5. Auction Contract Events

All auction contract events are published under the `gateway-auction` contract. Publishers live in `gateway-contract/contracts/auction_contract/src/events.rs`.

### 5.1 English Auction Events

#### Event: Bid Refunded

**Topic:** `("BID_RFDN", "auction")`

**Payload Struct:** `BidRefundedEvent`

**Fields:**
1. `prev_bidder: Address` - Previous highest bidder being refunded
2. `amount: i128` - Amount refunded

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_bid_refunded_event`

---

### 5.2 Auction Close Events

#### Event: Auction Closed

**Topic:** `("AUC_CLOSE", "auction")`

**Payload Struct:** `AuctionClosedEvent`

**Fields:**
1. `auction_id: Symbol` - Unique identifier for the auction
2. `winner: Option<Address>` - Winning bidder address (None if no winner)
3. `amount: i128` - Winning bid amount

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_auction_closed_event`

---

### 5.3 Default Liquidation Settlement Events

#### Event: Default Liquidation Settlement

**Topic:** `("LIQ_SETL", "auction")`

**Payload Struct:** `DefaultLiquidationSettlementEvent`

**Fields:**
1. `auction_id: Symbol` - Unique identifier for the auction
2. `credit_contract: Address` - Credit contract address
3. `borrower: Address` - Borrower being liquidated
4. `winner: Address` - Auction winner
5. `recovered_amount: i128` - Amount recovered from auction

**Version Added:** 1.0.0  
**Stability:** Stable  
**Publisher:** `publish_default_liquidation_settlement_event`

---

## 6. Type Definitions

### Shared Types

- **CreditStatus** - Enum defined in `contracts/credit/src/types.rs`:
  - `Active = 0`
  - `Suspended = 1`
  - `Defaulted = 2`
  - `Closed = 3`

- **FreezeReason** - Enum defined in `contracts/credit/src/types.rs` for freeze source.

- **GraceWaiverMode** - Enum defined in `contracts/credit/src/types.rs`:
  - `FullWaiver`
  - `ReducedRate`

---

## 7. Stability Matrix

| Event topic | Stable since | Deprecated | Removal target | Notes |
|---|---|---|---|---|
| Credit lifecycle events | 1.0.0 | No | — | Share `CreditLineEvent` shape |
| `("credit","drawn")` | 1.0.0 | No | — | Legacy; use `drawn_v2` |
| `("credit","drawn_v2")` | 1.0.0 | No | — | New default draw event |
| `("credit","repay")` | 1.0.0 | No | — | |
| `("credit","accrue")` | 1.0.0 | No | — | |
| `("credit","fee_accrd")` | 1.1.0 | No | — | Extended with fee split fields |
| `("credit","late_fee")` | 1.0.0 | No | — | |
| `("credit","risk_upd")` | 1.0.0 | No | — | |
| `("credit","drw_freeze")` | 1.0.0 | No | — | Global toggle |
| `("credit","line_frz")` | 1.0.0 | No | — | Per-borrower toggle |
| `("br_freeze",)` | 1.0.0 | No | — | Time-bounded per-borrower freeze |
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
| `("credit","orc_qcfg")` | 1.0.0 | No | — | Raw tuple payload |
| `("credit","orc_qprc")` | 1.0.0 | No | — | Raw tuple payload |
| `("blk_chg",)` | 1.0.0 | No | — | Single-element topic tuple |
| `("BID_RFDN","auction")` | 1.0.0 | No | — | |
| `("AUC_CLOSE","auction")` | 1.0.0 | No | — | |
| `("LIQ_SETL","auction")` | 1.0.0 | No | — | |

---

## 8. Publisher Reference

### Credit Contract Publishers

All credit contract publishers live in `contracts/credit/src/events.rs`:

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
| `publish_token_rescued_event` | `("credit", "tok_resc")` |
| `publish_attestation_batch_committed` | `("credit", "atst_bat")` |
| `publish_rate_formula_config_event` | `("credit", "rate_form")` |
| `publish_paused_event` | `("credit", "paused")` / `("credit", "unpaused")` |
| `publish_protocol_fee_bps_set_event` | `("credit", "fee_set")` |
| `publish_protocol_fee_bounds_set_event` | `("credit", "fee_bnd")` |
| `publish_close_factor_bps_set_event` | `("credit", "clsfctr")` |
| `publish_oracle_config_set_event` | `("credit", "orc_cfg")` |
| `publish_oracle_price_accepted_event` | `("credit", "orc_price")` |
| `publish_oracle_quorum_config_set_event` | `("credit", "orc_qcfg")` |
| `publish_oracle_quorum_price_set_event` | `("credit", "orc_qprc")` |
| `publish_risk_parameters_updated` | `("credit", "risk_upd")` |

### Auction Contract Publishers

All auction contract publishers live in `gateway-contract/contracts/auction_contract/src/events.rs`:

| Publisher function | Event topic |
|---|---|
| `publish_bid_refunded_event` | `("BID_RFDN", "auction")` |
| `publish_auction_closed_event` | `("AUC_CLOSE", "auction")` |
| `publish_default_liquidation_settlement_event` | `("LIQ_SETL", "auction")` |

---

## 9. Related Documentation

- [`docs/EVENTS_CATALOG.md`](./EVENTS_CATALOG.md) — Authoritative event catalog with stability status
- [`docs/events-schema.md`](./events-schema.md) — Legacy schema reference (superseded)
- [`docs/indexer-integration.md`](./indexer-integration.md) — Decoder patterns and RPC examples
- [`docs/PROTOCOL_SPEC.md`](./PROTOCOL_SPEC.md) — Per-entrypoint event-emission table
- [`docs/ARCHITECTURE.md`](./ARCHITECTURE.md) — Event topology diagrams
- [`docs/credit.md`](./credit.md) — Master credit contract reference
- [`docs/upgrade-policy.md`](./upgrade-policy.md) — WASM upgrade procedure
