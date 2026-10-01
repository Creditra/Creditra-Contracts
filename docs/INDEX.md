# Creditra Documentation Index

A single page that tells you where to start, by audience.

---

## I am a grant reviewer / technical evaluator

You have ~15 minutes. Read in this order:

1. [`README.md`](../README.md) — the one-pager.
2. [`WHITEPAPER.md`](../WHITEPAPER.md) — protocol-level model, math, comparison
   to Aave/Compound/Maker, known limitations.
3. [`docs/RISK_PRICING.md`](./RISK_PRICING.md) — the rate / accrual /
   settlement algorithm with worked numerical examples.
4. Skim [`docs/EXECUTION_QUALITY.md`](./EXECUTION_QUALITY.md) — test catalog,
   coverage, CI, PR cadence — the reproducible proof.

If something seems wrong, [`docs/SECURITY.md`](./SECURITY.md) §6 ("Known gaps
& future work") states the open items explicitly so you don't have to dig.

---

## I am an auditor / security reviewer

Read in this order:

1. [`docs/SECURITY.md`](./SECURITY.md) — 24-row threats × mitigations table,
   trust roots, auditor checklist.
2. [`docs/threat-model.md`](./threat-model.md) — authorization matrix per
   entrypoint.
3. [`docs/PROTOCOL_SPEC.md`](./PROTOCOL_SPEC.md) — per-entrypoint validation
   chain (the 25-step `draw_credit` ordering is in §2.2).
4. [`docs/ARCHITECTURE.md`](./ARCHITECTURE.md) — sequence diagrams for draw,
   repay, default→auction→settle, plus call topology and storage tiers.
5. Then the source under `contracts/credit/src/` and
   `gateway-contract/contracts/auction_contract/src/`. Every module has a
   `//!` block with WHAT / HOW / WHY pointers; `lib.rs` is the
   `#[contractimpl]` chokepoint.

Reproducible verification:

```bash
cargo llvm-cov --workspace --all-targets --fail-under-lines 95
cargo build --release --target wasm32-unknown-unknown -p creditra-credit
ls -l target/wasm32-unknown-unknown/release/creditra_credit.wasm  # < 50 KB
python3 scripts/list_contract_errors.py --json | jq 'length'      # 38
```

---

## I am a protocol integrator / SDK consumer

Read in this order:

1. [`docs/PROTOCOL_SPEC.md`](./PROTOCOL_SPEC.md) — every entrypoint with exact
   signature, validation order, error returns, storage tiers.
2. [`docs/contract-errors.md`](./contract-errors.md) — 38-row error table.
3. [`docs/state-machine.md`](./state-machine.md) — authoritative
   `CreditStatus` transition table.
4. [`docs/indexer-integration.md`](./indexer-integration.md) — event topics,
   payload field layouts, sample `getEvents` JSON.
5. [`docs/STORAGE_LAYOUT.md`](./STORAGE_LAYOUT.md) — instance vs persistent
   storage tier reference.

For the rate / accrual formulas:

- [`docs/risk-based-rate-formula.md`](./risk-based-rate-formula.md) — terse
  normative reference.
- [`docs/interest-accrual.md`](./interest-accrual.md) — accrual normative
  reference.
- [`docs/RISK_PRICING.md`](./RISK_PRICING.md) — the algorithm in depth with
  worked examples.

For protocol fees, the treasury and bounty pools, and how fee revenue leaves the
contract:

- [`docs/treasury.md`](./treasury.md) — fee lifecycle end to end: accrual
  sources, the value-conserving split rule with worked examples, bounty vs
  treasury, the `AuctionActive` freeze, and both withdrawal paths (immediate
  and 24-hour timelocked).


For event schema:
- [`docs/EVENT_CATALOG.md`](./EVENT_CATALOG.md) — **canonical event catalog and
  versioning policy** (replaces the former `EVENTS_CATALOG.md`, `EVENT_SCHEMA.md`,
  and `events-schema.md`; validated by `events_catalog.rs`).

---

## I am an operator / deployer

Read in this order:

1. [`docs/deploy.md`](./deploy.md) — quick deploy sequence.
2. [`docs/EXECUTION_QUALITY.md`](./EXECUTION_QUALITY.md) §6 — testnet and
   mainnet checklists.
3. [`docs/upgrade-policy.md`](./upgrade-policy.md) — admin-gated WASM upgrade
   procedure.
4. [`docs/scripts.md`](./scripts.md) — helper scripts.
5. [`docs/CIRCUIT_BREAKER_IMPLEMENTATION.md`](./CIRCUIT_BREAKER_IMPLEMENTATION.md)
   — pause / unpause semantics.

For oracle behaviour and settlement gating:

- [`docs/oracle-mechanisms.md`](./oracle-mechanisms.md) — **canonical reference for
  all three oracle mechanisms** (single-price circuit breaker, quorum-of-K, and the
  weighted-median registry): precedence table, staleness rules per mechanism, and
  failure codes. Start here.

For the off-chain orchestrator that handles default auctions:

1. [`docs/default-liquidation-auction-hook.md`](./default-liquidation-auction-hook.md)
   — handoff protocol.
2. [`docs/default-oracle.md`](./default-oracle.md) — staged default-signal
   oracle design (unimplemented; see
   [`docs/oracle-mechanisms.md`](./oracle-mechanisms.md) for the price oracles).

---

## I am a contributor

Read in this order:

1. [`README.md`](../README.md) — repo map and conventions.
2. [`docs/CONTRIBUTING.md`](./CONTRIBUTING.md) — contributing standards and
   PR write-up guidelines.
3. [`docs/contributing-tests.md`](./contributing-tests.md) — test helper
   conventions.
4. [`docs/EXECUTION_QUALITY.md`](./EXECUTION_QUALITY.md) §1 — existing test
   catalog (find the file analogous to your change).
5. [`docs/PROTOCOL_SPEC.md`](./PROTOCOL_SPEC.md) — confirm your change fits
   the existing entrypoint surface.
6. The relevant source file's `//!` doc block — every module documents its
   WHAT / HOW / WHY before the code starts.

Commit style: conventional commits (`docs:`, `feat:`, `fix:`,
`security:`, `chore:`, `test:`). Atomic; one logical change per commit.

---

## Document inventory

### Long-form references (this directory)

| File | Pages | Purpose |
|---|---|---|
| `INDEX.md` | 1 | This page |
| `PROTOCOL_SPEC.md` | ~12 | Per-module contract surface |
| `ARCHITECTURE.md` | ~10 | Sequence + state + topology diagrams |
| `RISK_PRICING.md` | ~12 | Pricing algorithm + worked examples |
| `SECURITY.md` | ~8 | Threat model + auditor checklist |
| `EXECUTION_QUALITY.md` | ~10 | Tests + CI + deployment + PR cadence |
| `state-machine.md` | ~4 | Normative state-transition table |
| `interest-accrual.md` | ~3 | Accrual normative reference |
| `interest-accrual-design.md` | ~6 | Accrual design spec |
| `risk-based-rate-formula.md` | ~3 | Rate formula normative reference |
| `contract-errors.md`, `errors.md` | ~4 each | Error code tables |
| `storage-layout.md` | ~4 | Storage tier reference |
| `threat-model.md` | ~4 | Authorization matrix |
| `default-liquidation-auction-hook.md` | ~3 | Cross-contract handoff |
| `default-oracle.md` | ~5 | Staged default-signal oracle |
| `credit.md` | ~15 | Master credit-contract reference |
| `upgrade-policy.md` | ~3 | Upgrade procedure |
| `utilization-cap.md` | ~3 | Per-borrower utilization cap |
| `indexer-integration.md` | ~4 | Off-chain event decoding |
| `EVENT_CATALOG.md` | ~6 | **Authoritative event catalog and versioning policy** (validated by `events_catalog.rs`) |
| `deploy.md` | ~2 | Deploy quickstart |
| `contributing-tests.md` | ~3 | Test helper conventions |
| `scripts.md` | ~2 | Helper script reference |
## Complete document inventory

Every `docs/*.md` file is linked below, routed by audience. Nothing in this
directory should be reachable only by guessing its filename.

`scripts/check_docs_index.sh` enforces this: it fails when a `docs/*.md` file
exists but is not referenced from this page (and is run in CI).

| Audience | File | Purpose |
| --- | --- | --- |
| Auditors / integrators | [`ARCHITECTURE.md`](./ARCHITECTURE.md) | Creditra System Architecture |
| Auditors / contributors | [`AUCTION_CLOSE_TIME_FIX.md`](./AUCTION_CLOSE_TIME_FIX.md) | Auction close-time hardening specification |
| Auditors / operators | [`CIRCUIT_BREAKER_IMPLEMENTATION.md`](./CIRCUIT_BREAKER_IMPLEMENTATION.md) | Pause / unpause design rationale |
| Contributors | [`CONTRIBUTING.md`](./CONTRIBUTING.md) | Contributing guidelines and PR workflow conventions |
| Grant reviewers / contributors | [`COVERAGE.md`](./COVERAGE.md) | Coverage Guide |
| Auditors / integrators | [`CROSS_CONTRACT_HANDSHAKE.md`](./CROSS_CONTRACT_HANDSHAKE.md) | Cross-Contract Handshake Protocol |
| Integrators | [`ERROR_CODES.md`](./ERROR_CODES.md) | ContractError Codes — Categorized Reference |
| Integrators | [`ERROR_MIGRATION.md`](./ERROR_MIGRATION.md) | V1 to V2 `ContractError` encoding migration |
| Integrators | [`EVENTS_CATALOG.md`](./EVENTS_CATALOG.md) | **Authoritative event catalog and versioning policy** |
| Integrators | [`EVENT_SCHEMA.md`](./EVENT_SCHEMA.md) | Event Schema Documentation |
| Grant reviewers / contributors | [`EXECUTION_QUALITY.md`](./EXECUTION_QUALITY.md) | Creditra Execution Quality — The Receipts |
| Everyone | [`GLOSSARY.md`](./GLOSSARY.md) | Creditra Glossary |
| Everyone | [`INDEX.md`](./INDEX.md) | Creditra Documentation Index |
| Operators / auditors | [`ORACLE_OUTAGE_SIMULATION.md`](./ORACLE_OUTAGE_SIMULATION.md) | Multi-Oracle Outage Simulation & Recovery Guidelines (`creditra-credit`) |
| Auditors / integrators | [`ORACLE_VALIDATION_DESIGN.md`](./ORACLE_VALIDATION_DESIGN.md) | Oracle input validation before price-dependent settlement |
| Auditors / integrators | [`PROTOCOL_SPEC.md`](./PROTOCOL_SPEC.md) | Creditra Protocol Specification |
| Grant reviewers / integrators | [`RISK_PRICING.md`](./RISK_PRICING.md) | Creditra Risk-Pricing Algorithm — In Depth |
| Auditors / integrators | [`SCORING.md`](./SCORING.md) | Credit Score VRF Commitment |
| Auditors / operators | [`SECURITY.md`](./SECURITY.md) | Creditra Security & Threat Model |
| Auditors / integrators | [`SELF_SUSPEND_ARCHITECTURE.md`](./SELF_SUSPEND_ARCHITECTURE.md) | Borrower self-suspend architecture & sequence diagrams |
| Auditors / integrators | [`STORAGE_KEY_ENCODING_DIAGRAMS.md`](./STORAGE_KEY_ENCODING_DIAGRAMS.md) | Soroban storage key encoding & collision resistance diagrams |
| Integrators | [`STORAGE_LAYOUT.md`](./STORAGE_LAYOUT.md) | **Authoritative storage layout** (DataKey source of truth) |
| Auditors / integrators | [`VALIDATION_LAYER_DESIGN.md`](./VALIDATION_LAYER_DESIGN.md) | Oracle validation layer architecture & interfaces |
| Integrators | [`contract-errors.md`](./contract-errors.md) | `ContractError` reference |
| Contributors | [`contributing-tests.md`](./contributing-tests.md) | Contributing Tests |
| Everyone | [`credit.md`](./credit.md) | Master credit-contract reference |
| Operators / auditors | [`default-liquidation-auction-hook.md`](./default-liquidation-auction-hook.md) | Default Liquidation Auction Hook |
| Operators / auditors | [`default-oracle.md`](./default-oracle.md) | Default Oracle Design (Stellar/Soroban) |
| Operators | [`deploy.md`](./deploy.md) | Deployment Guide |
| Auditors / integrators | [`error-taxonomy.md`](./error-taxonomy.md) | `ContractError` Taxonomy — Recovery Actions by Category |
| Integrators | [`errors.md`](./errors.md) | ContractError Reference |
| Integrators | [`events-schema.md`](./events-schema.md) | Creditra Event Schema Reference |
| Integrators | [`indexer-integration.md`](./indexer-integration.md) | Indexer Integration Guide (Soroban Events) |
| Auditors / contributors | [`interest-accrual-design.md`](./interest-accrual-design.md) | Interest Accrual Design Specification |
| Integrators | [`interest-accrual.md`](./interest-accrual.md) | Accrual normative reference |
| Operators / auditors | [`oracle-mechanisms.md`](./oracle-mechanisms.md) | Canonical reference for all three oracle mechanisms |
| Integrators | [`risk-based-rate-formula.md`](./risk-based-rate-formula.md) | Risk-Score Based Dynamic Interest Rate Formula |
| Operators / contributors | [`scripts.md`](./scripts.md) | Helper scripts |
| Integrators / auditors | [`state-machine.md`](./state-machine.md) | Repayment Schedule State Machine |
| Auditors | [`threat-model.md`](./threat-model.md) | Threat Model — Authorization Matrix |
| Operators / auditors | [`treasury.md`](./treasury.md) | **Treasury fee lifecycle end to end** (accrual → split → withdrawal) |
| Operators | [`upgrade-policy.md`](./upgrade-policy.md) | Upgrade Policy: Native WASM Upgrade Path |
| Operators / integrators | [`utilization-cap.md`](./utilization-cap.md) | Per-Borrower Utilization Ratio Cap |

### Superseded duplicates

These files are kept only so old links keep resolving. Treat the canonical
column as the single source of truth and do not extend the duplicates.

| Superseded | Canonical | Reason |
| --- | --- | --- |
| [`errors.md`](./errors.md) | [`contract-errors.md`](./contract-errors.md) | legacy copy of the error-code table |
| [`ERROR_CODES.md`](./ERROR_CODES.md) | [`contract-errors.md`](./contract-errors.md) | upper-case duplicate of the error-code table |
| [`EVENT_SCHEMA.md`](./EVENT_SCHEMA.md) | [`EVENTS_CATALOG.md`](./EVENTS_CATALOG.md) | legacy event schema (catalogue is authoritative) |
| [`events-schema.md`](./events-schema.md) | [`EVENTS_CATALOG.md`](./EVENTS_CATALOG.md) | legacy event schema (catalogue is authoritative) |
| [`interest-accrual-design.md`](./interest-accrual-design.md) | [`interest-accrual.md`](./interest-accrual.md) | design history for the normative accrual reference |
| [`ERROR_MIGRATION.md`](./ERROR_MIGRATION.md) | [`error-taxonomy.md`](./error-taxonomy.md) | error taxonomy migration note |

### Top-level companions

| File | Purpose |
| --- | --- |
| [`WHITEPAPER.md`](../WHITEPAPER.md) | Protocol-level design (the centerpiece) |
| [`README.md`](../README.md) | Repo entry point |

### Contract-local READMEs

Per-crate references that sit next to the code they describe. These are the
READMEs published with each crate, so they are versioned with the contract they
document rather than with this index.

| Crate | README | Purpose |
|---|---|---|
| `creditra-risk` | [`contracts/risk/README.md`](../contracts/risk/README.md) | Standalone risk-admin cooldown contract: entrypoints, error codes, storage keys and TTL policy, and — importantly — how it relates to the credit contract's *separate, duplicated* cooldown (`set_risk_admin_cooldown`) and who calls `record_risk_admin_action` |
| `creditra-borrow` | [`contracts/borrow/README.md`](../contracts/borrow/README.md) | Borrow module reference |
| `creditra-collateral` | [`contracts/collateral/README.md`](../contracts/collateral/README.md) | Collateral module reference |
| `creditra-freeze` | [`contracts/freeze/README.md`](../contracts/freeze/README.md) | Global draws-frozen toggle reference |
| `creditra-lifecycle` | [`contracts/lifecycle/README.md`](../contracts/lifecycle/README.md) | State-transition reference |
| `creditra-query` | [`contracts/query/README.md`](../contracts/query/README.md) | Read-only query reference |

---

*Documentation index is checked by `scripts/check_docs_index.sh`.*
