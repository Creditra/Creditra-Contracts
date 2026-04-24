# Indexer Integration Guide (Soroban Events)

This guide explains how indexers subscribe to Credit contract events and decode:

- `CreditLineEvent`
- `DrawnEvent`
- `RepaymentEvent`
- `RiskParametersUpdatedEvent`
- `DefaultLiquidationRequestedEvent`
- `DefaultLiquidationSettledEvent`

Source of truth for schemas: `contracts/credit/src/events.rs`.

---

## 1) Event channels and topics

The contract publishes Soroban events under a `credit` namespace.

| Event payload | Topic tuple | Emitted by |
|---|---|---|
| `CreditLineEvent` | `("credit", "opened" \| "suspend" \| "closed" \| "default" \| "reinstate")` | `open_credit_line`, `suspend_credit_line`, `close_credit_line`, `default_credit_line`, `reinstate_credit_line` |
| `DrawnEvent` | `("credit", "drawn")` | `draw_credit` |
| `RepaymentEvent` | `("credit", "repay")` | `repay_credit` |
| `RiskParametersUpdatedEvent` | `("credit", "risk_upd")` | `update_risk_parameters` |
| `InterestAccruedEvent` | `("credit", "accrue")` | `draw_credit`, `repay_credit` |
| `DefaultLiquidationRequestedEvent` | `("credit", "liq_req")` | `default_credit_line` |
| `DefaultLiquidationSettledEvent` | `("credit", "liq_setl")` | `settle_default_liquidation` |

For `CreditLineEvent`, `event_type` in the payload mirrors the second topic symbol.

---

## 2) Canonical field lists (from `events.rs`)

### `CreditLineEvent`

| Field | Type | Notes |
|---|---|---|
| `event_type` | `Symbol` | One of `opened`, `suspend`, `closed`, `default`, `reinstate` |
| `borrower` | `Address` | Borrower account/contract address |
| `status` | `CreditStatus` | Enum: `Active=0`, `Suspended=1`, `Defaulted=2`, `Closed=3` |
| `credit_limit` | `i128` | Current credit limit |
| `interest_rate_bps` | `u32` | Rate in basis points |
| `risk_score` | `u32` | Risk score |

### `DrawnEvent`

| Field | Type | Notes |
|---|---|---|
| `borrower` | `Address` | Borrower address |
| `amount` | `i128` | Draw amount |
| `new_utilized_amount` | `i128` | Post-draw utilized amount |
| `timestamp` | `u64` | Ledger timestamp at emit time |

### `RepaymentEvent`

| Field | Type | Notes |
|---|---|---|
| `borrower` | `Address` | Borrower address |
| `amount` | `i128` | Repaid amount recorded by contract |
| `new_utilized_amount` | `i128` | Post-repay utilized amount |
| `timestamp` | `u64` | Ledger timestamp at emit time |

### `InterestAccruedEvent`

| Field | Type | Notes |
|---|---|---|
| `borrower` | `Address` | Borrower address |
| `accrued_amount` | `i128` | Amount of interest accrued in this step |
| `total_accrued_interest` | `i128` | Cumulative interest accrued |
| `new_utilized_amount` | `i128` | Utilized amount including the new interest |
| `timestamp` | `u64` | Ledger timestamp at emit time |

### `RiskParametersUpdatedEvent`

| Field | Type | Notes |
|---|---|---|
| `borrower` | `Address` | Borrower address |
| `credit_limit` | `i128` | Updated limit |
| `interest_rate_bps` | `u32` | Updated rate |
| `risk_score` | `u32` | Updated score |

### `DefaultLiquidationRequestedEvent`

| Field | Type | Notes |
|---|---|---|
| `borrower` | `Address` | Defaulted borrower |
| `utilized_amount` | `i128` | Debt at request time |
| `timestamp` | `u64` | Ledger timestamp at emit time |

### `DefaultLiquidationSettledEvent`

| Field | Type | Notes |
|---|---|---|
| `borrower` | `Address` | Borrower being settled |
| `settlement_id` | `Symbol` | Idempotency key (typically auction id) |
| `recovered_amount` | `i128` | Amount applied from liquidation proceeds |
| `remaining_utilized_amount` | `i128` | Debt remaining after settlement |
| `status` | `CreditStatus` | `Defaulted` for partial, `Closed` for full |
| `timestamp` | `u64` | Ledger timestamp at emit time |

---

## 3) Subscription/query patterns

Most indexers use RPC polling with cursor checkpoints.

### JSON-RPC `getEvents` example

Use strict topic filters to reduce bandwidth and decode costs.

```bash
curl -s "$SOROBAN_RPC_URL" \
  -H 'Content-Type: application/json' \
  -d '{
    "jsonrpc":"2.0",
    "id":"credit-events-1",
    "method":"getEvents",
    "params":{
      "startLedger":123456,
      "filters":[
        {
          "type":"contract",
          "contractIds":["'$CREDIT_CONTRACT_ID'"],
          "topics":["credit"]
        }
      ],
      "pagination":{"limit":100}
    }
  }'
```

To isolate one stream, filter by the second topic as well (for example `drawn`, `repay`, `risk_upd`).

### JS SDK decode pattern (topic + value)

```ts
import { xdr, scValToNative } from "@stellar/stellar-sdk";

type RawEvent = {
  topic: string[];       // base64 XDR ScVal entries from RPC
  value: string;         // base64 XDR ScVal payload from RPC
  ledger: number;
  id: string;
};

function decodeScVal(base64Xdr: string) {
  return xdr.ScVal.fromXDR(base64Xdr, "base64");
}

function decodeEvent(evt: RawEvent) {
  const topics = evt.topic.map((t) => scValToNative(decodeScVal(t)));
  const data = scValToNative(decodeScVal(evt.value));

  // topics[0] === "credit"
  // topics[1] is one of: opened, suspend, closed, default, reinstate, drawn, repay, risk_upd
  return { topics, data, ledger: evt.ledger, id: evt.id };
}
```

### Rust XDR decode pattern

```rust
use stellar_xdr::{Limits, ReadXdr, ScVal};

fn decode_scval_base64(input: &str) -> Result<ScVal, Box<dyn std::error::Error>> {
    let bytes = base64::decode(input)?;
    let scv = ScVal::read_xdr(&mut bytes.as_slice(), Limits::none())?;
    Ok(scv)
}
```

After decoding `ScVal`, map by topic pair to the corresponding strongly-typed event schema your indexer owns.

---

## 4) Recommended indexer pipeline

1. Query from last finalized cursor (`startLedger` or `cursor`).
2. Filter by `contractId` + topic prefix `credit`.
3. Decode topic XDR and payload XDR.
4. Route by second topic symbol:
   - lifecycle: `opened|suspend|closed|default|reinstate` -> `CreditLineEvent`
   - `drawn` -> `DrawnEvent`
   - `repay` -> `RepaymentEvent`
   - `risk_upd` -> `RiskParametersUpdatedEvent`
   - `accrue` -> `InterestAccruedEvent`
  - `liq_req` -> `DefaultLiquidationRequestedEvent`
  - `liq_setl` -> `DefaultLiquidationSettledEvent`
5. Validate payload fields and ranges (for example non-negative numeric invariants where expected).
6. Upsert into event store with idempotency key (`event.id` + ledger/tx metadata).
7. Advance checkpoint only after durable write.

---

## 5) Versioning policy for schema/topic changes

Use additive-first evolution and explicit version markers for breaking changes.

- **Non-breaking changes**: adding optional fields at the end of payload structs is allowed; indexers should ignore unknown fields.
- **Breaking changes**: rename/remove/retype fields, topic name changes, or semantic changes must introduce a new versioned stream.
- **Topic versioning**: append version suffix in second topic symbol, for example `drawn_v2`, `repay_v2`, `risk_upd_v2`, or lifecycle `opened_v2` as needed.
- **Dual-publish window**: publish both old and new versioned events during migration to allow indexers to cut over safely.
- **Deprecation policy**: announce deprecation window in release notes and remove old stream only after downstream confirmation.

Suggested contract for consumers:

- Treat `(contract_id, topics[], tx_hash, event_index)` as unique identity.
- Never assume field ordering beyond the documented schema.
- Fail closed on unknown required fields for a known version.

### Contract API version probe

Before ingesting events from a deployed credit contract, indexers should call the read-only `get_contract_version()` query and record the returned `ContractVersion { major, minor, patch }` alongside the contract ID. Route decoders by `major` and gate on unsupported majors so that a re-deployed contract with a breaking schema change cannot silently corrupt downstream state. See `docs/credit.md` for the full versioning policy.

---

## 6) Operational and security notes

### Assumptions

- RPC responses are eventually consistent and may be paginated.
- Reorg/finality behavior follows network guarantees; consumers should delay irreversible side effects until desired confirmation depth.

### Trust boundaries

- **Trusted**: on-chain event content after consensus finality.
- **Partially trusted**: RPC transport and availability (can drop, delay, or duplicate responses).
- **Untrusted input**: decoded payloads before schema validation.

### Failure modes and mitigations

- **Duplicate delivery**: enforce idempotent writes keyed by event identity.
- **Out-of-order pages**: use monotonic cursoring and deterministic sort by `(ledger, tx, event_index)` where available.
- **Schema drift**: route by explicit topic version and keep per-version decoders.
- **Decoder errors**: dead-letter unknown/invalid payloads with raw XDR retained for replay.
- **Backfill gaps**: periodic reconciliation job over ledger ranges.

---

## 7) Quick checklist for integrators

- Subscribe/query by `contractId` + `credit` topic namespace.
- Implement decoders for `CreditLineEvent`, `DrawnEvent`, `RepaymentEvent`, `RiskParametersUpdatedEvent`, `InterestAccruedEvent`, `DefaultLiquidationRequestedEvent`, `DefaultLiquidationSettledEvent`.
- Store raw XDR alongside normalized records for audit/replay.
- Make ingestion idempotent and checkpointed.
- Support versioned topic suffixes (`*_v2`, etc.) for future migrations.
