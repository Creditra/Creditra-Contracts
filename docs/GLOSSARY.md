# Creditra Glossary

Project-specific terms used across this repo's documentation. Where a term
appears in source as a named constant or symbol, the source location is
given.

---

## A

**`AccruedInterest`**. The borrower's outstanding interest portion of debt,
tracked separately from principal so repayments can be allocated
interest-first. Field on `CreditLineData`
(`contracts/credit/src/types.rs:173-200`). Updated by
`crate::accrual::apply_accrual`.

**Admin**. The single Address (in v1) with permission to call privileged
entrypoints (`open_credit_line`, `update_risk_parameters`,
`default_credit_line`, etc.). Stored under instance `Symbol("admin")`
(`contracts/credit/src/storage.rs:269`). Rotation is two-step
(`propose_admin` → `accept_admin` with delay).

**Anti-snipe**. End-time extension intended to push out an English
auction's close time when a bid lands inside the extension window. PR
#430 added the design; the live `place_bid` path does not extend.
Documented but not active in this release. See `WHITEPAPER.md` §6.3 and
`docs/SECURITY.md` §6.1.

**Auction (English)**. Ascending-price auction. Outbidders atomically
refund the prior highest bidder under the reentrancy guard. Closed
manually by admin after `end_time`. Source:
`gateway-contract/contracts/auction_contract/src/lib.rs`.

**Auction (Dutch)**. Descending-price auction with linear decay
`p(t) = p_0 - (p_0 - p_f) * min(t, T) / T`. First qualifying bid wins
and atomically closes the auction.

**`AuctionContract`**. Address of the deployed `gateway-auction` contract
that the credit contract calls on
`settle_default_liquidation`. Stored under instance
`DataKey::AuctionContract` (`contracts/credit/src/storage.rs:31-98`).

---

## B

**Bps (basis points)**. `1 bps = 1 / 10_000 = 0.01 %`. All interest rates,
penalty surcharges, deviation thresholds, and fee splits in Creditra are
expressed in bps. The denominator constant is
`BPS_DENOMINATOR = 10_000` (`contracts/credit/src/math_utils.rs:57`).

**Behavioral signal**. Off-chain measurement (wallet age, repayment
history, counterparty diversity, attestation, etc.) that feeds into the
risk score. Taxonomy is documented in `WHITEPAPER.md` §3.

---

## C

**Capitalize (interest)**. Folding accrued interest into `utilized_amount`
so subsequent interest is computed on principal + prior interest. Creditra
capitalizes on every state-mutating call via
`crate::accrual::apply_accrual`.

**CEI (Checks-Effects-Interactions)**. The ordering discipline:
validate → mutate state → call external. Creditra's external token CPIs
are wrapped by a reentrancy guard so the actual ordering is
**Checks → external call → Effects under guard**, which is equivalently
safe.

**Cooldown (draw)**. The per-borrower minimum interval between draws,
configured by `set_draw_min_interval` (`contracts/credit/src/lib.rs:731`).
Stored as `DataKey::DrawMinIntervalSeconds`. Default 0 = disabled.

**Credit limit**. The maximum `utilized_amount` for a credit line. Field
on `CreditLineData`. Set by `open_credit_line` / `update_risk_parameters`.
Bounded by protocol-wide `MinCreditLimit` / `MaxCreditLimit`.

**`CreditLineData`**. The per-borrower record. See
`docs/PROTOCOL_SPEC.md` §2 or `contracts/credit/src/types.rs:173-200`.

**`CreditStatus`**. 5-variant enum: `Active=0`, `Suspended=1`,
`Defaulted=2`, `Closed=3`, `Restricted=4`. See `docs/state-machine.md`.

---

## D

**Default**. Status transition to `Defaulted` via `default_credit_line`
(admin in v1). Emits `("credit","liq_req")` for the off-chain orchestrator
to construct an auction.

**Deviation (oracle)**. Bps deviation of a new price from the last
accepted price. Computed by `compute_deviation_bps`
(`contracts/credit/src/math_utils.rs:306`). Bound by
`OracleConfig.max_deviation_bps`.

**Delinquent**. A line whose `next_due_ts + grace < now`. Detected by
`crate::query::is_delinquent`. Triggers the penalty-surcharge branch in
accrual.

**`DrawAudit`**. Persistent per-`(borrower, ts)` record of original draw
amounts, used by `reverse_draw` to bound the reversible amount.

---

## E

**`ExposureCapExceeded`**. `ContractError::ExposureCapExceeded = 31`.
Reverts when `TotalUtilized + amount > MaxTotalExposure`.

---

## F

**Factory contract**. The auction contract's term for the only entity
allowed to call its `settle_default_liquidation` — i.e. the credit
contract address. Stored under `DataKey::FactoryContract`
(`gateway-contract/contracts/auction_contract/src/types.rs`).

**Forgive debt**. Admin write-off via `forgive_debt(borrower, amount)`
(`contracts/credit/src/lifecycle.rs:499`). Reduces `accrued_interest`
first, then `utilized_amount`.

**Freeze (draws)**. Global admin-only kill-switch on `draw_credit` calls
(`set_draws_frozen`). Distinct from per-line `Suspended` and from
contract-wide `Paused`. See `contracts/credit/src/freeze.rs` for the
comparison table.

---

## G

**Grace period**. Seconds after `next_due_ts` during which a suspended
line accrues at a waived or reduced rate. Configured by
`set_grace_period_config(seconds, waiver_mode, reduced_rate_bps)`. Two
modes: `GraceWaiverMode::FullWaiver = 0` (interest waived entirely) and
`GraceWaiverMode::ReducedRate` (interest at `reduced_rate_bps`).

---

## I

**Indexer**. Off-chain process consuming Soroban events to reconstruct
protocol state. See `docs/indexer-integration.md`.

**Instance storage**. Soroban storage tier for hot, always-loaded
contract-level configuration. Shared TTL across all instance keys.

**`init`**. One-time initialization. `contracts/credit/src/config.rs:20`.
Second call reverts `AlreadyInitialized = 14`.

---

## J

**Julian year**. 365.25 days × 86 400 s = 31 557 600 s. Used as
`SECONDS_PER_YEAR` in `contracts/credit/src/math_utils.rs:60`.

---

## L

**`LiquidityToken`**. The SAC / token contract used for `transfer` and
`transfer_from` operations. Set by `set_liquidity_token(addr)`. Stored
under instance `DataKey::LiquidityToken`.

**`LiquiditySource`**. The reserve address that funds draws. Defaults to
the credit contract's own address; production deployments point it at a
separate reserve pool. Stored under instance `DataKey::LiquiditySource`.

**Lazy accrual**. Accrual realized only on a state-mutating call rather
than on every block or via a periodic keeper. Creditra's model. See
`docs/interest-accrual.md`.

---

## M

**Multisig**. Recommended production form of the admin Address — a
contract whose `require_auth` requires m-of-n signatures. Creditra does
not embed a multisig; it relies on Soroban's authorization framework.

**`MaxTotalExposure`**. Global cap on `TotalUtilized + new_draw`. The
protocol-wide circuit breaker on absolute loss. Set by
`set_max_total_exposure`.

---

## O

**Oracle (price)**. The price feed consulted on
`settle_default_liquidation`. Subject to staleness
(`max_age_seconds`) and deviation (`max_deviation_bps`) circuit breakers.

**Oracle (default signal)**. Staged design for a signature-verified
default attestation envelope. Not yet implemented. See
`docs/default-oracle.md`.

---

## P

**Pause**. Protocol-wide circuit breaker via `pause_protocol`. Blocks
every mutating entrypoint **except `repay_credit`** — borrowers must
always be able to deleverage. Stored under instance `Symbol("paused")`.

**Penalty surcharge**. Additive bps added to `interest_rate_bps` during
accrual when a line is delinquent. Configured by
`set_penalty_surcharge_bps`. Applied as
`min(rate + penalty, MAX_INTEREST_RATE_BPS)`.

**Persistent storage**. Soroban storage tier for keyed, per-borrower
data with per-key TTL. Auto-bumped by Creditra on access.

**`ProtocolFeeBps`**. The protocol's cut of the interest portion of a
repayment. Capped at `MAX_PROTOCOL_FEE_BPS = 1_000` (10 % of *interest*,
never principal).

---

## R

**Rate floor**. Per-borrower minimum interest rate that overrides the
formula-computed rate. Stored under `DataKey::RateFloorBps(Address)`.
Configured by `set_borrower_rate_floor`.

**`RateChangeConfig`**. Magnitude and cadence cap on rate changes per
`update_risk_parameters` call. `max_rate_change_bps` and
`rate_change_min_interval` (seconds).

**`RateFormulaConfig`**. Piecewise-linear formula parameters
`(base_rate_bps b, slope_bps_per_score s, min_rate_bps r_min, max_rate_bps r_max)`.

**Reentrancy guard**. Boolean flag at instance `Symbol("reentrancy")`.
Set on entry to `draw_credit`, `repay_credit`,
`settle_default_liquidation`, and the auction's `place_bid` refund branch
and `claim_auction`. Reverts `Reentrancy = 11` on re-entry.

**Restricted**. Cure state of a credit line: limit was reduced below
`utilized_amount`. Repayments cure back to Active automatically.

**Reverse draw**. Admin-only reversal of a recent draw within
`DRAW_REVERSAL_WINDOW_SECS` (= 3600 by docs). Decrements `utilized` and
emits `DrawReversedEvent`.

---

## S

**SAC**. Stellar Asset Contract. The Soroban token interface
implementation provided by the Stellar host for native assets and
classic asset trustlines.

**Schema version**. `DataKey::SchemaVersion`, currently 1. Bumped by
`upgrade`. Used by off-chain indexers to detect breaking changes.

**Settlement (default liquidation)**. The cross-contract handoff that
records auction recovery against the defaulted line.
`settle_default_liquidation` on both contracts; replay-protected on both
sides.

**Stroop**. The smallest unit of XLM (1 stroop = 10⁻⁷ XLM). All on-chain
i128 amounts in this repo are in stroop units (or token equivalents).

**Suspended**. Status. Draws blocked, repayments allowed. Grace policy
optional. Reachable from Active by admin (`suspend_credit_line`) or
borrower (`self_suspend_credit_line`).

---

## T

**`TotalUtilized`**. Global accumulator: sum of `utilized_amount` over
all open lines. Stored under instance `DataKey::TotalUtilized`.
Maintained by `crate::storage::persist_credit_line` using the
caller-captured `previous_utilized`.

**TTL (Time To Live)**. Soroban's per-key storage lifetime. Creditra
bumps to `LEDGER_BUMP_AMOUNT ≈ 6 months` whenever remaining TTL drops
below `LEDGER_BUMP_THRESHOLD ≈ 3 months`. Constants in
`contracts/credit/src/storage.rs:122-127`.

**Topic (event)**. The Soroban event-publish key, typically a tuple of
`Symbol`s. Creditra uses `(symbol_short!("credit"), symbol_short!(name))`
for almost every event. Stability pinned by
`tests/event_topic_stability.rs`.

**Treasury**. The address where protocol fees withdrawn from the credit
contract land. Set by `set_treasury(admin, treasury_addr)`. Drained by
`withdraw_treasury(admin)`.

---

## U

**Utilization**. `utilized_amount / credit_limit`. The fraction of the
credit line currently drawn.

**Utilization cap (per-borrower)**. A per-borrower bps ratio bound on
`utilized / credit_limit`. Configured by
`set_utilization_cap(borrower, cap_bps)`. Stored under
`DataKey::UtilizationCapBps(Address)`. See `docs/utilization-cap.md`.

**Upgrade**. Admin-gated WASM swap via
`env.deployer().update_current_contract_wasm(new_wasm_hash)`. Bumps
`SchemaVersion`. Emits `ContractUpgradedEvent`.

---

## W

**WASM size budget**. Hard CI limit: 50 KB
(`THRESHOLD_BYTES=51200` in `.github/workflows/ci.yml`). Achieved via
`opt-level = "z"`, full LTO, stripped symbols, single codegen unit.
