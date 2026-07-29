# Creditra Security & Threat Model

**Scope:** `creditra-credit` (`contracts/credit/`) and `gateway-auction`
(`gateway-contract/contracts/auction_contract/`).
**Last updated:** June 2026, aligned with `main` at the documentation pass.
**Companion:** `docs/threat-model.md` (authorization matrix, role definitions).

This document is the protocol's adversarial review surface. It enumerates
the realistic attacker capabilities the contract is designed to resist, maps
each capability to a concrete mitigation in the source, lists the auditor
checklist a reviewer should walk before signing off, and discloses the
assumptions and trust roots the model rests on.

---

## 1. Attacker Capabilities

The protocol assumes adversaries with all of the following capabilities. The
mitigations below address each.

| Capability                                                                                       | Realistic? | Granted by |
|--------------------------------------------------------------------------------------------------|------------|------------|
| **Mempool visibility & ordering**: see pending transactions and front-run.                       | Yes        | Soroban public ledger |
| **Hostile token contract**: the configured `LiquidityToken` re-enters or mis-reports balances.   | Yes        | Admin may misconfigure or be tricked into setting an upgradeable token. |
| **Malicious oracle**: the price feed pushes manipulated values.                                  | Yes        | Whichever entity calls `settle_default_liquidation` with `oracle_price`. |
| **MEV-style sequencing**: validator(s) reorder transactions in a block.                          | Plausible  | Network layer |
| **Governance / admin capture**: the admin key is compromised or coerced.                         | Yes        | Single-Address admin model in v1. |
| **Time-warp / ledger lag**: ledger timestamp moves forward unexpectedly or sub-second granularity. | Plausible | Soroban host |
| **Borrower collusion**: a borrower controls or pays the off-chain scorer.                        | Yes        | Off-chain scoring is the protocol's trust input. |
| **Auction sniping**: bidding at the end of an English auction window.                            | Yes        | Public auction surface. |
| **Storage TTL expiry**: persistent state expires before refresh.                                 | Yes        | Soroban TTL model. |
| **Dust / denial-of-service via paging**: enumeration calls forced to scan unbounded state.       | Yes        | Public read entrypoint. |

---

## 2. Threats × Mitigations

The dominant risks and their concrete mitigations.

| # | Threat | Mitigation | Source location |
|---|---|---|---|
| T1 | **Reentrancy via token CPI** (malicious `LiquidityToken` re-enters draw / repay during transfer). | Explicit reentrancy guard set before the external call; `Reentrancy = 11` revert on re-entry. Guard cleared on every exit path. | `storage.rs:316`, `lib.rs:261/437/953` |
| T2 | **Reentrancy in auction refund** (malicious token used as bid currency re-enters during the prior-bid refund). | Same `Symbol("reentrancy")` guard pattern; `AuctionError::Reentrancy = 10`. | `gateway-contract/.../storage.rs`, `lib.rs` (`place_bid`) |
| T3 | **Cross-contract settlement replay** (admin or attacker re-runs `settle_default_liquidation` with the same `settlement_id`). | Per-`(borrower, settlement_id)` persistent flag `(Symbol("liq_seen"), borrower, settlement_id)`; second call reverts `AlreadyInitialized = 14`. Auction side: `AuctionKey::LiquidationSettled(auction_id)`. | `lifecycle.rs:39-48,539-630`, gateway-auction `lib.rs` |
| T4 | **Settlement amount tampering** (auction returns one value, admin records another). | `settle_default_liquidation` asserts that the cross-contract call's return equals the admin-supplied `recovered_amount`; mismatch reverts `InvalidAmount = 5`. | `lib.rs:953` (final assertion) |
| T5 | **Oracle price manipulation** (push a single-block extreme price to grief settlement). | `OracleConfig { max_deviation_bps, max_age_seconds }` circuit breaker: stale (`OraclePriceStale = 37`) or deviation-exceeding (`OraclePriceDeviation = 38`) prices revert. Price+ts persisted atomically. | `lib.rs:1055`, `storage.rs:561-593`, `math_utils.rs:306` |
| T6 | **Front-running risk-parameter updates** (race to draw before a rate hike). | `RateChangeConfig { max_rate_change_bps, rate_change_min_interval }` bounds both magnitude and cadence. Rate hike >`max_rate_change_bps` reverts `RateTooHigh = 8`; hike within `rate_change_min_interval` reverts `TimestampRegression = 33`. | `risk.rs:207`, `types.rs:217-222` |
| T7 | **Mempool front-run on draw cap depletion** (drain a credit line just before a rate change). | Per-borrower draw cooldown (`DrawMinIntervalSeconds`) plus per-tx draw cap (`MaxDrawAmount`) plus global cap (`MaxTotalExposure`). | `lib.rs:261-424` steps 6, 11, 16 |
| T8 | **Admin compromise → instant rate to 100% → grief all borrowers.** | `MAX_INTEREST_RATE_BPS = 10_000` is the hard ceiling that even admin cannot bypass. `RateChangeConfig` further bounds change *per update*, so a compromised admin can at most raise rates by `max_rate_change_bps` per `rate_change_min_interval` window. Repayment is **never pause-blocked** so borrowers can always escape. | `risk.rs:24,207`, `lib.rs:437` |
| T9 | **Admin compromise → drain treasury / drain reserve.** | `withdraw_treasury` only moves the `TreasuryBalance` accumulator (fees collected from interest); cannot touch reserve or borrower funds. The liquidity reserve is an external address; the credit contract has no write capability over it beyond `transfer_from` calls authorized by the borrower. | `lib.rs:770` |
| T10 | **Admin compromise → impostor upgrade with backdoor WASM.** | `upgrade` is admin-only, but the proposal model is the second-layer mitigation: in production deployments the admin SHOULD be a `m-of-n` multisig with diverse key custody. The contract enforces a `propose_admin` + `accept_admin` flow with a configurable delay (`AdminAcceptTooEarly = 15`), so a stolen admin key cannot rotate without a time window for response. | `lib.rs:103-157`, `lib.rs:1330` |
| T11 | **Borrower collusion with scorer** (off-chain scorer assigns favorable risk score for a fee). | Out-of-protocol mitigation: scorer should be a stake-weighted committee in production; the on-chain `OracleConfig` + `RateChangeConfig` bound the blast radius. `MaxTotalExposure` and per-borrower limits cap absolute loss. | `lib.rs:827` (`set_max_total_exposure`) |
| T12 | **Auction sniping at close** (bid in the last block to suppress competition). | `AUCTION_CLOSE_TIME_FIX.md` switched the comparison to `>=` to prevent off-by-one closes. The full anti-snipe extension is in PR #430's design but is *not* active in the live `place_bid` path (see `WHITEPAPER.md` §6.3); this is a known gap (see §6 below). | `gateway-contract/.../lib.rs` (place_bid) |
| T13 | **English auction grief: 1-stroop overbid spam.** | `min_increment_bps` enforces a minimum bid increment; `min_next_bid = max(highest_bid * (1 + inc/10000), highest_bid + 1)`. Each spam bid pays the refund-CPI gas and the increment-bound new bid amount. | `gateway-contract/.../lib.rs` (helper `min_next_bid`) |
| T14 | **Dutch auction race after-close.** | First qualifying bid atomically flips status `Open → Closed` in the same transaction that records the bid. No second bid can land. | `gateway-contract/.../lib.rs` (place_bid Dutch branch) |
| T15 | **Time-warp accrual** (ledger ts jumps forward by large delta, blowing up interest). | Lazy accrual uses `now - last_accrual_ts`; the *math* uses `prorate_interest` with checked-mul; an overflow reverts `Overflow = 12` rather than wrapping. Realistic ledger jumps are bounded by Soroban's host. | `accrual.rs:87`, `math_utils.rs:244` |
| T16 | **Backward timestamp on suspension / rate update.** | `assert_ts_monotonic` reverts `TimestampRegression = 33`. | `storage.rs:538`, `risk.rs:207` |
| T17 | **DoS via unbounded enumeration.** | `enumerate_credit_lines(start_after, limit)` is capped at `MAX_ENUMERATION_LIMIT = 100`. `accrue_batch` capped at 50. `bulk_block_borrowers` capped at 50. | `storage.rs:102`, `lib.rs:885,1112,1133` |
| T18 | **State TTL expiry on dormant borrower** (line becomes inaccessible). | `LEDGER_BUMP_THRESHOLD = 1_555_200` / `LEDGER_BUMP_AMOUNT = 3_110_400` keep an active borrower's data refreshed automatically. A dormant borrower (~6 months no activity) requires admin republish; the `accrue_batch` keeper hook lets indexers cheaply re-bump dormant lines. | `storage.rs:122-127,1133` |
| T19 | **Storage-key collision across borrowers.** | Storage keys use the `DataKey::*(Address)` discriminator + the Address itself, plus a per-borrower id mapping. Tested in `tests/borrower_key_encoding.rs`. | `storage.rs:31-98`, `tests/borrower_key_encoding.rs` |
| T20 | **Discriminant reorder breaks SDK ABI.** | CI test `tests/error_discriminants.rs` reverts on any reorder/renumber of `ContractError`. Same for event topic stability via `tests/event_topic_stability.rs`. | `tests/error_discriminants.rs`, `tests/event_topic_stability.rs` |
| T21 | **Pause griefing** (admin pauses the protocol indefinitely). | `repay_credit` is the **only entrypoint excluded from the pause check** — borrowers can always reduce debt and avoid penalty accrual even during indefinite pause. | `lib.rs:437`, `CIRCUIT_BREAKER_IMPLEMENTATION.md` |
| T22 | **Token-failure mid-CPI leaves inconsistent state.** | The reentrancy guard's clear-on-exit is paired with Soroban host's panic-revert: a token CPI panic causes the whole tx to revert (state untouched), including the persist call. Tested in `tests/token_failure_rollback.rs`. | `lib.rs:261/437`, `tests/token_failure_rollback.rs` |
| T23 | **Collateral over-withdraw racing utilization growth.** | `withdraw_collateral` re-evaluates `utilized * MinCollateralRatioBps / 10_000 <= post_balance` against the **current** utilized amount; concurrent draws raise utilized first under the same lock. | `collateral.rs:69-126` |
| T24 | **Borrower self-suspend abused to dodge default.** | `self_suspend_credit_line` cannot transition out of `Suspended` on the borrower side; reinstatement is admin-only. A borrower cannot self-default or self-reinstate. | `lifecycle.rs:342,630`, `SELF_SUSPEND_ARCHITECTURE.md` |

---

## 3. Auditor Checklist

Items a reviewer should walk before signing off on the contract.

### 3.1 Authorization coverage

- [ ] Every `set_*` and `*_credit_line` admin entrypoint begins with
  `require_admin_auth` (`auth.rs:40`) and / or an `admin: Address` argument
  followed by `admin.require_auth()`.
- [ ] Every borrower entrypoint begins with `borrower.require_auth()`.
- [ ] `tests/unauthorized_matrix.rs` covers every privileged entrypoint
  with a negative test.
- [ ] Admin rotation uses two-step `propose_admin` → `accept_admin` with a
  positive delay.

### 3.2 Arithmetic safety

- [ ] All `i128` math uses `checked_add` / `checked_mul`; failure path is
  `Overflow = 12`, not wrapping.
- [ ] `math_utils::prorate_interest` and `math_utils::mul_div` use checked
  primitives.
- [ ] `compute_rate_from_score` uses saturating arithmetic; result is
  clamped to `[r_min, min(r_max, MAX_INTEREST_RATE_BPS)]`.
- [ ] No `unwrap()` / `expect()` on production paths.
  Tracked in `UNWRAP_AUDIT_REPORT.md` (PR #418).

### 3.3 Reentrancy ordering

- [ ] `draw_credit`: guard set → CEI checks → token transfer →
  state persist → guard clear.
- [ ] `repay_credit`: guard set → CEI checks → `transfer_from`(s) →
  state persist → guard clear.
- [ ] `settle_default_liquidation`: guard set → oracle check →
  cross-contract call → accounting → guard clear.
- [ ] Auction `place_bid` English mode: guard set around refund CPI.
- [ ] Auction `claim_auction`: guard set around payout CPI.

### 3.4 Oracle deviation bounds

- [ ] `OracleConfig.max_deviation_bps` is in `1..=10_000`.
- [ ] `OracleConfig.max_age_seconds > 0`.
- [ ] First-write case (no prior price) is handled: `compute_deviation_bps`
  returns `None` for `last_price <= 0` (`math_utils.rs:306`).
- [ ] Atomic price + ts persist (no intermediate state).

### 3.5 Storage TTL

- [ ] Every persistent read or write goes through helpers that bump the
  ledger TTL.
- [ ] `MAX_ENUMERATION_LIMIT = 100`, `ACCRUE_BATCH_MAX = 50`,
  `BULK_BLOCK_MAX = 50` are all enforced.
- [ ] `tests/storage_ttl.rs` covers the bump regression.

### 3.6 Event stability

- [ ] No topic-string change without a major version bump in
  `CONTRACT_API_VERSION`.
- [ ] `tests/event_topic_stability.rs` covers every topic.

### 3.7 Cross-contract safety

- [ ] `AuctionContract` address is admin-set and not mutable mid-settlement.
- [ ] Settlement is replay-protected on **both** sides.
- [ ] Cross-contract return value (`i128` recovered amount) is asserted
  against the admin-supplied value.

---

## 4. Trust Roots & Assumptions

The contract's correctness is conditional on the following assumptions.
Auditors should validate each.

1. **Admin key custody.** The `admin` is assumed to be a key (or contract,
   e.g. a Soroban multisig) whose compromise is detectable within the
   `propose_admin` delay window. Default deployment recommends a 3-of-5
   multisig with off-chain key diversity.

2. **`LiquidityToken` honesty.** The configured token contract is assumed to
   implement the Stellar token interface honestly:
   - `transfer` either succeeds or reverts atomically.
   - `transfer_from` honors allowance correctly.
   - `balance` cannot be falsely inflated.
   The reentrancy guard defends against a *malicious* token from re-entering,
   but a token that lies about balances can still cause economic loss bounded
   by `MaxTotalExposure`.

3. **Off-chain scoring oracle.** The `risk_score` passed to
   `update_risk_parameters` is assumed to be produced by a scoring stack with
   integrity. The on-chain mitigations bound damage but cannot detect a
   subtly biased score. The path to decentralization is in
   `docs/default-oracle.md`.

4. **Ledger timestamp honesty.** `env.ledger().timestamp()` is assumed to be
   strictly non-decreasing and within a few seconds of wall-clock at validator
   level. `assert_ts_monotonic` defends against timestamp regression in
   contract logic but cannot defend against systemic time-warp attacks at the
   network layer.

5. **Storage TTL semantics.** Persistent storage is assumed to be retrievable
   for at least `LEDGER_BUMP_AMOUNT ≈ 6 months` after the last touch. Soroban
   guarantees this in the host environment; archival recovery is out of
   protocol scope.

6. **Soroban SDK correctness.** The contract depends on `soroban-sdk 22.0.11`.
   `update_current_contract_wasm` and `require_auth` are trusted host
   functions.

---

## 5. Severity Matrix

The protocol's risk profile mapped to severity:

| Severity | Examples | Mitigations in this release |
|---|---|---|
| Critical (loss of all borrower funds) | Reentrancy on draw, replay of settlement, admin upgrade to malicious WASM | Reentrancy guard, two-side replay marker, two-step admin rotation w/ delay |
| High (loss of one borrower's funds, or oracle griefing) | Hostile token CPI, price manipulation at settlement | Reentrancy guard, oracle deviation + staleness breaker, `MaxTotalExposure` |
| Medium (degraded UX, recoverable) | Pause griefing, draw cooldown abuse, enumeration DoS | Repay-exception during pause, bounded batch sizes |
| Low (information leak, ABI churn) | Event reordering, topic change | CI guards on discriminants and topic stability |

---

## 6. Known Gaps & Future Work

These are explicit and tracked. A reviewer should not assume they will be
addressed before mainnet.

1. **Anti-snipe is documented but not active.** The auction `place_bid`
   currently hard-rejects bids when `now >= end_time`. The end-time extension
   logic described in PR #430 is not exercised in the live path after the
   `AUCTION_CLOSE_TIME_FIX.md` reconciliation. Tracked for the next auction
   release.

2. **Default-signal oracle is staged, not live.** `default_credit_line` is
   admin-only today. The signed-attestation path in `docs/default-oracle.md`
   is designed but not implemented in this release.

3. **No formal verification.** The state machine, the rate clamp, and the
   `TotalUtilized` invariant are amenable to formal verification (e.g., via
   Kani or symbolic execution). Today they are protected by unit + property
   tests only.

4. **Single-Address admin.** In v1 the admin is a single Soroban Address.
   Deployments should use a multisig contract as that address, but the
   protocol does not enforce that.

5. **Year-length mismatch.** `accrual::SECONDS_PER_YEAR = 31_536_000` (365 d)
   is dead code; the live accrual goes through `math_utils::SECONDS_PER_YEAR =
   31_557_600` (Julian, 365.25 d). The dead constant should be removed in a
   follow-up to avoid reader confusion.

6. **Pre-existing build failures.** A baseline `cargo check --workspace`
   reports 65 errors at the documentation cutoff, all in
   `contracts/credit/src/lifecycle.rs` and `contracts/credit/src/risk.rs`
   from a merge artifact (duplicate function bodies). These are tracked and
   do not impact the documentation pass, which is doc-only.

---

## 7. Bug Bounty Scope & Disclosure

**In scope:**

- `creditra-credit` (`contracts/credit/`)
- `gateway-auction` (`gateway-contract/contracts/auction_contract/`)
- The cross-contract handoff in
  `lifecycle.rs:settle_default_liquidation` and the auction's
  `settle_default_liquidation` and `claim_auction`.

**Out of scope (today):**

- Off-chain scoring stack.
- Off-chain auction orchestrator.
- Soroban SDK and host-function bugs (report to Stellar).
- Front-end / wallet integrations.
- DoS at the network layer.

**Severity & rewards** (illustrative; subject to deployment-time tuning):

| Severity | Reward bracket |
|---|---|
| Critical | Negotiable; up to TVL-percentage cap |
| High | Fixed tier |
| Medium | Fixed tier |
| Low | Acknowledgement + bounty |

**Disclosure policy:**

- Report via security contact in `Cargo.toml` (`authors` / repo issue tracker)
  with `[SECURITY]` prefix and request a private response channel before
  disclosure.
- 90-day coordinated disclosure window; extensions granted for active
  remediation.
- Responsible disclosure is rewarded; public disclosure pre-fix forfeits
  bounty.

---

## 8. References

- `docs/threat-model.md` — authorization matrix
- `docs/PROTOCOL_SPEC.md` — per-entrypoint validation order
- `WHITEPAPER.md` — protocol-level design
- `docs/upgrade-policy.md` — upgrade procedure
- `docs/EXECUTION_QUALITY.md` — test catalog
- `docs/error-taxonomy.md` — categorized error variants with SDK recovery hints
- `CIRCUIT_BREAKER_IMPLEMENTATION.md` — pause design
- `AUCTION_CLOSE_TIME_FIX.md` — close-time off-by-one fix
- `UNWRAP_AUDIT_REPORT.md` — production-unwrap removal (PR #418)
- `SELF_SUSPEND_ARCHITECTURE.md` — borrower self-suspend design
