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
5. [`docs/storage-layout.md`](./storage-layout.md) — instance vs persistent
   storage tier reference.

For the rate / accrual formulas:

- [`docs/risk-based-rate-formula.md`](./risk-based-rate-formula.md) — terse
  normative reference.
- [`docs/interest-accrual.md`](./interest-accrual.md) — accrual normative
  reference.
- [`docs/RISK_PRICING.md`](./RISK_PRICING.md) — the algorithm in depth with
  worked examples.

---

## I am an operator / deployer

Read in this order:

1. [`docs/deploy.md`](./deploy.md) — quick deploy sequence.
2. [`docs/integration-initialization.md`](./integration-initialization.md) —
   deploy order, operator setup, and error recovery.
3. [`docs/EXECUTION_QUALITY.md`](./EXECUTION_QUALITY.md) §6 — testnet and
   mainnet checklists.
4. [`docs/upgrade-policy.md`](./upgrade-policy.md) — admin-gated WASM upgrade
   procedure.
5. [`docs/scripts.md`](./scripts.md) — helper scripts.
6. [`CIRCUIT_BREAKER_IMPLEMENTATION.md`](../CIRCUIT_BREAKER_IMPLEMENTATION.md)
   — pause / unpause semantics.

For the off-chain orchestrator that handles default auctions:

1. [`docs/default-liquidation-auction-hook.md`](./default-liquidation-auction-hook.md)
   — handoff protocol.
2. [`docs/default-oracle.md`](./default-oracle.md) — staged default-signal
   oracle design.

---

## I am a contributor

Read in this order:

1. [`README.md`](../README.md) — repo map and conventions.
2. [`docs/contributing-tests.md`](./contributing-tests.md) — test helper
   conventions.
3. [`docs/EXECUTION_QUALITY.md`](./EXECUTION_QUALITY.md) §1 — existing test
   catalog (find the file analogous to your change).
4. [`docs/PROTOCOL_SPEC.md`](./PROTOCOL_SPEC.md) — confirm your change fits
   the existing entrypoint surface.
5. The relevant source file's `//!` doc block — every module documents its
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
| `integration-initialization.md` | ~4 | Deploy order, operator setup, error recovery |
| `deploy.md` | ~2 | Deploy quickstart |
| `contributing-tests.md` | ~3 | Test helper conventions |
| `scripts.md` | ~2 | Helper script reference |

### Top-level companions

| File | Purpose |
|---|---|
| `WHITEPAPER.md` | Protocol-level design (the centerpiece) |
| `README.md` | Repo entry point |
| `CIRCUIT_BREAKER_IMPLEMENTATION.md` | Pause design rationale |
| `AUCTION_CLOSE_TIME_FIX.md` | Close-time off-by-one fix history |
| `SELF_SUSPEND_ARCHITECTURE.md` | Borrower self-suspend design |
| `STORAGE_KEY_ENCODING_DIAGRAMS.md`, `STORAGE_KEY_ENCODING_SUMMARY.md` | Storage key safety |
| `UNWRAP_AUDIT_REPORT.md` | Production unwrap removal |
| `POST_AUDIT_CHECKLIST.md` | Post-audit follow-ups |
| `AUDIT_SUMMARY.md`, `IMPLEMENTATION_STATUS.md` | Status snapshots |
| `INTEREST_ACCRUAL_SPIKE_RESULTS.md` | Accrual model spike |
| `TEST_COVERAGE_REPORT.md`, `COVERAGE_REPORT.md`, `TEST_COVERAGE.md`, `TEST_VALIDATION.md` | Coverage snapshots |

---

*Last updated alongside the documentation pass in June 2026.*
