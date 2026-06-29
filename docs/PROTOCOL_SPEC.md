# Creditra Protocol Specification

**Version:** 1.0
**Status:** authoritative for `main` at the time of writing
**Scope:** `creditra-credit` (`contracts/credit/`) and `gateway-auction`
(`gateway-contract/contracts/auction_contract/`)

This document is the per-module contract surface specification. It is the
reference a protocol integrator or auditor consults to answer:

- *What entrypoints exist, and exactly what arguments do they take?*
- *What storage is touched by each entrypoint, and at which TTL tier?*
- *What invariants is each module responsible for upholding?*
- *Which `ContractError` variants can each entrypoint return, and what do they
  mean semantically?*
- *What is the upgrade path?*

Every signature, error, and constant in this document is taken directly from
the source. File paths use the form
`contracts/credit/src/<file>.rs:<line>` so a reviewer can verify each claim.

---

## 1. Module Topology

```
creditra-credit (contracts/credit/src/)
├── lib.rs            # #[contract] Credit + #[contractimpl] entrypoints (5449 LOC)
├── types.rs          # ContractError, CreditStatus, CreditLineData, configs
├── storage.rs        # DataKey, TTL constants, all storage helpers
├── auth.rs           # require_admin / require_admin_auth
├── borrow.rs         # draw_status_error helper
├── collateral.rs     # deposit/withdraw collateral, MinCollateralRatioBps
├── config.rs         # init, set_liquidity_token, set_liquidity_source
├── events.rs         # all #[contracttype] payloads + publishers
├── freeze.rs         # global draw freeze (admin)
├── lifecycle.rs      # state transitions, settle_default_liquidation
├── math_utils.rs     # mul_div, apply_bps, prorate_interest, Rounding
├── query.rs          # read-only helpers, is_delinquent
├── risk.rs           # rate formula, rate-change limits, update_risk_parameters
└── accrual.rs        # apply_accrual + penalty/grace branches

gateway-auction (gateway-contract/contracts/auction_contract/src/)
├── lib.rs            # #[contract] Auction + #[contractimpl] entrypoints
├── types.rs          # AuctionMode, AuctionStatus, AuctionConfig, AuctionState
├── storage.rs        # DataKey + AuctionKey, TTL constants
├── events.rs         # BidRefundedEvent, AuctionClosedEvent, ...
└── errors.rs         # AuctionError (12 variants)
```

---

## 2. `creditra-credit` — Entrypoint Specification

The contract struct is `Credit` (`contracts/credit/src/lib.rs:91`). All
entrypoints are inside a single `#[contractimpl]` block at
`contracts/credit/src/lib.rs:93`. Constants of note:

| Constant | Value | Location | Meaning |
|---|---|---|---|
| `CONTRACT_API_VERSION` | `(1, 0, 0)` | `lib.rs:60` | Major.minor.patch ABI version |
| `MAX_PROTOCOL_FEE_BPS` | `1_000` | `lib.rs:63` | 10 % cap on protocol fee |
| `BULK_BLOCK_MAX` | `50` | `lib.rs:74` | Bulk-blocklist batch cap |
| `ACCRUE_BATCH_MAX` | `50` | `lib.rs:78` | Keeper accrual batch cap |
| `MAX_INTEREST_RATE_BPS` | `10_000` | `risk.rs:24` | 100 % APR cap |
| `MAX_RISK_SCORE` | `100` | `risk.rs:27` | Score is 0..=100 |
| `MAX_ENUMERATION_LIMIT` | `100` | `storage.rs:102` | Page cap on enumeration |
| `LEDGER_BUMP_AMOUNT` | `3_110_400` | `storage.rs:122` | ~6 months at 5s/ledger |
| `LEDGER_BUMP_THRESHOLD` | `1_555_200` | `storage.rs:123` | ~3 months bump trigger |
| `INSTANCE_BUMP_AMOUNT/THRESHOLD` | mirror of above | `storage.rs:126-127` | |
| `SECONDS_PER_YEAR` (accrual) | `31_536_000` | `accrual.rs:60` | dead-code, legacy |
| `SECONDS_PER_YEAR` (math) | `31_557_600` | `math_utils.rs:60` | Julian — live |
| `BPS_DENOMINATOR` | `10_000` | `math_utils.rs:57` | |
| `BPS_YEAR_DENOM` | `315_576_000_000` | `math_utils.rs:66` | precomputed |

### 2.1 Initialization & admin rotation

#### `init(env: Env, admin: Address)`
`config.rs:20`. Writes `admin_key`, `LiquiditySource = current_contract_address`,
`CreditLineCount = 0`, `TotalUtilized = 0`, `SchemaVersion = 1`,
`MinCollateralRatioBps = 15_000`. Guards on existing `admin_key`.

- **Returns:** `Result<(), ContractError>` (proper return type per
  `contractimpl`).
- **Errors:** `AlreadyInitialized` (14) if `admin_key` is set.
- **Auth:** none (must be reachable for first deploy).
- **Events:** none.
- **Storage written:** `Symbol("admin")`, `DataKey::LiquiditySource`,
  `DataKey::CreditLineCount`, `DataKey::TotalUtilized`,
  `DataKey::SchemaVersion`, `DataKey::MinCollateralRatioBps` — all
  Instance.

#### `get_contract_version() -> (u32, u32, u32)`
`lib.rs:99`. Returns `CONTRACT_API_VERSION = (1, 0, 0)`. Used by indexers and
clients to gate breaking event-schema changes (see
`docs/indexer-integration.md`).

#### `propose_admin(env, new_admin: Address, delay_seconds: u64)`
`lib.rs:103`. Admin only.
- Writes `Symbol("proposed_admin") = new_admin`,
  `Symbol("proposed_at") = now`.
- Emits `AdminRotationProposedEvent` on topic `("credit","admin_prop")`.

#### `accept_admin(env)`
`lib.rs:117`. Must be called by the proposed admin.
- Checks `now >= proposed_at + delay`.
- **Errors:** `Unauthorized` (caller not proposed), `AdminAcceptTooEarly`
  (delay not elapsed).
- Atomically rotates `Symbol("admin")`, clears the proposed slot.
- Emits `AdminRotationAcceptedEvent` on `("credit","admin_acc")`.

### 2.2 Credit line CRUD

#### `open_credit_line(env, borrower, credit_limit, interest_rate_bps, risk_score)`
`lib.rs:181` → `lifecycle::open_credit_line` (`lifecycle.rs:247`).
- **Auth:** admin (`require_admin_auth`), pause check.
- **Validation order:**
  1. `assert_not_paused`
  2. `credit_limit >= 0` (else `NegativeLimit`)
  3. `interest_rate_bps <= MAX_INTEREST_RATE_BPS` (else `RateTooHigh`)
  4. `risk_score <= MAX_RISK_SCORE` (else `ScoreTooHigh`)
  5. `validate_credit_limit_bounds(credit_limit)` against `MinCreditLimit`,
     `MaxCreditLimit` (`lifecycle.rs:126`) — else `LimitOutOfBounds`
  6. If existing line: must be non-`Active` (else returns existing — reopens
     Closed/Defaulted under admin auth)
- **State written:** `DataKey::CreditLineIdByBorrower(borrower)` (Persistent),
  `DataKey::CreditLineBorrowerById(id)` (Persistent), the line itself
  (Persistent), `DataKey::CreditLineCount` (Instance).
- **Events:** `CreditLineEvent` on `("credit","opened")`.

#### `draw_credit(env, borrower, amount)`
`lib.rs:261`. **The canonical borrower entrypoint.** Reentrancy-guarded.
Pause-gated.

The full ordered validation chain is documented in `docs/ARCHITECTURE.md`
(sequence diagram); the abbreviated list:

1. `assert_not_paused` (else `Paused`)
2. `set_reentrancy_guard` (else `Reentrancy`)
3. `borrower.require_auth()` (Soroban auth host fn)
4. `amount > 0` (else `InvalidAmount`)
5. `!is_draws_frozen()` (else `DrawsFrozen`)
6. `amount <= MaxDrawAmount` (else `DrawExceedsMaxAmount`)
7. `is_borrower_blocked(borrower) == false` (else `BorrowerBlocked`)
8. Load line; `CreditLineNotFound` if absent
9. `apply_accrual` — capitalizes accrued interest into `utilized_amount`
10. `borrow::draw_status_error` (Suspended → `CreditLineSuspended`;
    Defaulted → `CreditLineDefaulted`; Closed → `CreditLineClosed`;
    Active/Restricted → pass)
11. Cooldown: `now - LastDrawTs > DrawMinIntervalSeconds`
    (else `DrawCooldownActive`)
12. `utilized + amount` via `checked_add` (else `Overflow`)
13. Updated utilized `<= credit_limit` (else `OverLimit`)
14. Collateral ratio: `utilized * MinCollateralRatioBps / 10_000 <=
    CollateralBalance(borrower)` (else `CollateralRatioBelowMinimum`)
15. Per-borrower utilization cap:
    `updated_utilized <= credit_limit * cap_bps / 10_000`
16. Global cap: `TotalUtilized + amount <= MaxTotalExposure`
    (else `ExposureCapExceeded`)
17. Liquidity token configured (else `MissingLiquidityToken`)
18. Liquidity source configured (else `MissingLiquiditySource`)
19. Reserve balance check (else `InsufficientLiquidityReserve`)
20. `token::Client::transfer(reserve, borrower, amount)` (token CPI)
21. `persist_credit_line` (writes new utilization + atomically adjusts
    `TotalUtilized`)
22. `set_last_draw_ts(borrower, now)`
23. `clear_reentrancy_guard`
24. Write `DataKey::DrawAudit(borrower, now)` and persist
25. Emit `DrawnEvent` on `("credit","drawn")`

**Note (CEI ordering):** The token transfer in step 20 is the external call.
The reentrancy guard set in step 2 ensures that any malicious token contract
attempting to re-enter `draw_credit` reverts with `Reentrancy`. State persist
(step 21) happens after the transfer; the guard is what makes that ordering
safe.

#### `repay_credit(env, borrower, amount)`
`lib.rs:437`. Reentrancy-guarded. **Not pause-gated** — users must always be
able to deleverage during emergencies.

1. `set_reentrancy_guard`
2. `borrower.require_auth()`
3. `amount > 0` (else `InvalidAmount`)
4. `amount <= MaxRepayAmount` (else `RepayExceedsMaxAmount`)
5. Load line (else `CreditLineNotFound`)
6. `apply_accrual`
7. Status != `Closed` (else `CreditLineClosed`)
8. `effective_repay = min(amount, utilized_amount)`
9. Interest-first split:
    - `interest_repaid = min(effective_repay, accrued_interest)`
    - `principal_repaid = effective_repay - interest_repaid`
10. Compute `fee = interest_repaid * protocol_fee_bps / 10_000` (floor),
    `reserve_amount = effective_repay - fee`
11. `token::Client::transfer_from(borrower, contract_address, fee)`
12. `token::Client::transfer_from(borrower, reserve, reserve_amount)`
13. Decrement `accrued_interest` and `utilized_amount`
14. `persist_credit_line` (atomically adjusts `TotalUtilized`)
15. `advance_repayment_schedule_after_repay` (advance `next_due_ts` by
    `installments_paid * period_seconds`, saturating)
16. Emit `FeeAccruedEvent`, `InterestAccruedEvent`, `RepaymentEvent`
17. `clear_reentrancy_guard`

**Errors:** `InvalidAmount`, `RepayExceedsMaxAmount`, `CreditLineNotFound`,
`CreditLineClosed`, `MissingLiquidityToken`, `InsufficientRepaymentAllowance`,
`InsufficientRepaymentBalance`, `Overflow`, `Reentrancy`.

### 2.3 Lifecycle transitions

| Entrypoint | File | Auth | Pause | Notes |
|---|---|---|---|---|
| `suspend_credit_line(borrower)` | `lib.rs:918` → `lifecycle.rs:147` | admin | yes | Apply accrual; `Active→Suspended`; set `suspension_ts` monotonically. |
| `self_suspend_credit_line(borrower)` | `lib.rs:922` → `lifecycle.rs:342` | borrower | yes | Same effect, borrower-initiated. |
| `close_credit_line(borrower, closer)` | `lib.rs:926` → `lifecycle.rs:385` | admin OR borrower if `utilized==0` | yes | Idempotent on `Closed`. |
| `default_credit_line(borrower)` | `lib.rs:930` → `lifecycle.rs:450` | admin | yes | Emits `("credit","liq_req")`. |
| `forgive_debt(borrower, amount)` | `lifecycle.rs:499` | admin | yes | Caps to `utilized_amount`; reduces `accrued_interest` first. |
| `reinstate_credit_line(borrower, target_status)` | `lib.rs:940` → `lifecycle.rs:630` | admin | yes | `target ∈ {Active, Restricted}`; current must be `Defaulted`. Clears `suspension_ts`. |

All transitions invoke `apply_accrual` first and persist via
`persist_credit_line` with the captured `previous_utilized` so the global
`TotalUtilized` accumulator stays consistent.

### 2.4 Risk parameters

#### `update_risk_parameters(env, borrower, credit_limit, interest_rate_bps, risk_score)`
`lib.rs:559` → `risk.rs:207`. Admin + pause.

1. `assert_not_paused`, `require_admin_auth`
2. Load line; `CreditLineNotFound` if absent
3. `apply_accrual`
4. `credit_limit >= 0` (else `NegativeLimit`)
5. `risk_score <= 100` (else `ScoreTooHigh`)
6. Validate credit_limit bounds
7. Compute `effective_rate`:
   - If `RateFormulaConfig` set: `compute_rate_from_score(cfg, risk_score)`
   - Else: provided `interest_rate_bps`
8. Apply per-borrower `RateFloorBps` (max of effective_rate and floor)
9. Apply per-borrower `RateCeilingBps` (min of floor-adjusted rate and ceiling)
10. If `RateChangeConfig` set:
   - `|new_rate - old_rate| <= max_rate_change_bps` (else `RateTooHigh`)
   - `now - last_rate_update_ts >= rate_change_min_interval`
     (else `TimestampRegression`)
11. `effective_rate <= MAX_INTEREST_RATE_BPS` (sanity)
12. If `utilized_amount > new credit_limit`: status → `Restricted`
13. Persist; emit `RiskParametersUpdatedEvent` on `("credit","risk_upd")`.

#### `set_rate_change_limits(env, max_rate_change_bps, rate_change_min_interval)`
`lib.rs:569`. Admin + pause. Writes `Symbol("rate_cfg")`.

#### `set_borrower_rate_floor(env, borrower, floor_bps: Option<u32>)`
`lib.rs:578`. Admin. Asserts `floor <= 10_000` and rejects
`floor > RateCeilingBps(borrower)` when a ceiling is configured. `None`
clears.

#### `set_borrower_rate_ceiling(env, borrower, ceiling_bps: Option<u32>)`
`lib.rs:775`. Admin. Asserts `ceiling <= 10_000` and rejects
`ceiling < RateFloorBps(borrower)` when a floor is configured. `None`
clears. The value is applied during `update_risk_parameters` after any
per-borrower floor and before rate-change guardrails.

#### `set_penalty_surcharge_bps(env, bps)`
`lib.rs:587`. Admin + pause. Surcharge added to base rate (and clamped to
`MAX_INTEREST_RATE_BPS`) when `is_delinquent` is true.

#### `set_rate_formula_config(env, base_rate_bps, slope_bps_per_score, min_rate_bps, max_rate_bps)`
`lib.rs:1159`. Admin + pause. Validates `min_rate <= max_rate <= 10_000` and
emits `("credit","rate_form")` with `true`.

#### `clear_rate_formula_config(env)`
`lib.rs:1189`. Admin. Emits `("credit","rate_form")` with `false`.

### 2.5 Caps, limits, schedule

| Entrypoint | File | Storage | Notes |
|---|---|---|---|
| `set_max_draw_amount(amount)` | `lib.rs:699` | `DataKey::MaxDrawAmount` (Instance) | `amount > 0`. |
| `set_max_repay_amount(amount)` | `lib.rs:714` | `DataKey::MaxRepayAmount` | |
| `set_draw_min_interval(seconds)` | `lib.rs:731` | `DataKey::DrawMinIntervalSeconds` | `0` disables cooldown. |
| `set_utilization_cap(borrower, cap_bps)` | `lib.rs:607` | `DataKey::UtilizationCapBps(borrower)` (Persistent) | `cap_bps ∈ 1..=10000`; `0` clears. |
| `set_max_total_exposure(amount)` | `lib.rs:827` | `DataKey::MaxTotalExposure` | `0` removes the cap. |
| `set_credit_limit_bounds(min, max)` | `lib.rs:862` | `MinCreditLimit`, `MaxCreditLimit` | `min >= 0`, `max >= min`. |
| `set_repayment_schedule(borrower, amount_per_period, period_seconds, first_due_ts)` | `lifecycle.rs:182` | `DataKey::RepaymentSchedule(borrower)` (Persistent) | All > 0. |
| `set_grace_period_config(grace_period_seconds, waiver_mode, reduced_rate_bps)` | `lib.rs:646` | `Symbol("grace_cfg")` (Instance) | `reduced_rate <= 10000`. |

### 2.6 Collateral

| Entrypoint | File | Effect |
|---|---|---|
| `deposit_collateral(borrower, amount)` | `lib.rs:805` → `collateral.rs:34` | `token::transfer` borrower → contract; `CollateralBalance += amount`. Emits `CollateralDepositedEvent`. |
| `withdraw_collateral(borrower, amount)` | `lib.rs:809` → `collateral.rs:69` | Compute `post_balance`; require `utilized * MinCollateralRatioBps / 10000 <= post_balance` else `CollateralRatioBelowMinimum`; transfer out; persist; emit. |
| `get_collateral(borrower) -> i128` | `lib.rs:813` → `collateral.rs:124` | Read-only. |

`InsufficientRepaymentBalance` is intentionally reused for over-withdraw
(see `collateral.rs:78-83` comment). Clients should disambiguate by entrypoint.

### 2.7 Treasury, bounty pool & protocol fee

| Entrypoint | Effect |
|---|---|
| `set_protocol_fee_bps(bps)` | Admin; `bps <= MAX_PROTOCOL_FEE_BPS = 1_000`. Returns `Overflow` if exceeded. |
| `set_treasury_fee_share_bps(bps)` | Admin; treasury share of skimmed fees in `0..=10_000`. Bounty pool receives the remainder. Default unset = 10_000 (100 % treasury). |
| `get_treasury_fee_share_bps()` | Returns configured share or `None` when unset. |
| `set_treasury(admin, treasury)` | Double-auth (admin arg + `require_admin_auth`). |
| `withdraw_treasury(admin)` | Transfers `TreasuryBalance` from contract to `TreasuryAddress`; clears balance. Errors: `TreasuryNotSet`, `MissingLiquidityToken`. |
| `set_bounty(admin, bounty)` | Double-auth; configures bounty pool withdrawal address. |
| `get_bounty()` | Returns configured bounty address, if any. |
| `withdraw_bounty(admin)` | Transfers `BountyBalance` to `BountyAddress`; clears balance. Errors: `BountyNotSet`, `MissingLiquidityToken`. |

On `repay_credit`, the protocol fee skim is split per `TreasuryFeeShareBps` into
`TreasuryBalance` and `BountyBalance` (floor to treasury, remainder to bounty).
See `contracts/credit/src/fees.rs`.

### 2.8 Settlement & oracle

#### `settle_default_liquidation(env, borrower, recovered_amount, settlement_id: Symbol, oracle_price: Option<i128>)`
`lib.rs:953`. Admin + pause + reentrancy guard.

1. `set_reentrancy_guard`
2. If `OracleConfig` is set:
   - `oracle_price.is_some()` and value > 0 (else `OraclePriceInvalid`)
   - `now - OracleLastPriceTs <= max_age_seconds` (else `OraclePriceStale`)
   - `compute_deviation_bps(new, last) <= max_deviation_bps`
     (else `OraclePriceDeviation`)
   - Atomically write `OracleLastPrice`, `OracleLastPriceTs`; emit
     `("credit","orc_price")`
3. If `AuctionContract` is set, call
   `AuctionClient::settle_default_liquidation(settlement_id, contract, borrower) -> i128`
   and assert returned == `recovered_amount` (else `InvalidAmount`)
4. Delegate to `lifecycle::settle_default_liquidation` for accounting:
   - Status must be `Defaulted` (else `CreditLineDefaulted` mismatch)
   - Replay: `(Symbol("liq_seen"), borrower, settlement_id)` must be unset
     (else `AlreadyInitialized`)
   - `recovered_amount <= utilized_amount` (else `OverLimit`)
   - Decrement `utilized_amount` and `accrued_interest` pro-rata
   - If `utilized_amount == 0`: status → `Closed`, clear repayment schedule
   - Emit `DefaultLiquidationSettledEvent` on `("credit","liq_setl")`
5. `clear_reentrancy_guard`

#### `set_oracle_config(env, max_deviation_bps, max_age_seconds)`
`lib.rs:1055`. Admin + pause. Validates `deviation in 1..=10_000` (else
`InvalidAmount`), `max_age_seconds > 0`. Emits `("credit","orc_cfg")`.

### 2.9 Operational controls

| Entrypoint | Effect |
|---|---|
| `freeze_draws(env, reason)` / `unfreeze_draws(env)` | Global flag + [`FreezeReason`]; admin; emits `DrawsFrozenEvent` on `("credit","drw_freeze")`. |
| `freeze_credit_line(env, borrower, reason)` / `unfreeze_credit_line(env, borrower)` | Per-line draw freeze with reason taxonomy; admin; emits `CreditLineFreezeEvent` on `("credit","line_frz")`. |
| `is_draws_frozen() -> bool` / `get_draws_freeze_reason()` / `is_credit_line_frozen(borrower)` / `get_credit_line_freeze_reason(borrower)` | Read-only freeze state. |
| `block_borrower(admin, borrower)` / `unblock_borrower` / `bulk_block_borrowers` | Admin; `bulk_*` capped at `BULK_BLOCK_MAX=50`. Emits `BorrowerBlockedEvent` on `("blk_chg",)`. |
| `accrue_batch(borrowers)` | No auth (pause-gated). Capped at `ACCRUE_BATCH_MAX=50`. Keeper hook. |
| `reverse_draw(borrower, amount, original_ts, reason_code)` | Admin + pause. Time window enforced (constant `DRAW_REVERSAL_WINDOW_SECS`). Decrements utilized; emits `DrawReversedEvent` on `("credit","draw_rev")`. |
| Pause toggles (`pause_protocol`, `unpause_protocol` — naming may differ) | Admin; flips `Symbol("paused")`; emits `("credit","paused")`/`("credit","unpaused")`. |

### 2.10 Read-only queries

| Entrypoint | Returns |
|---|---|
| `get_credit_line(borrower)` | `Option<CreditLineData>` |
| `get_credit_line_summary(borrower)` | `Option<CreditLineData>` (alias) |
| `get_credit_line_count()` | `u32` |
| `enumerate_credit_lines(start_after, limit)` | `Vec<(u32, CreditLineData)>`, `limit <= 100` |
| `get_total_utilized()` | `i128` |
| `get_repayment_schedule(borrower)` | `Option<RepaymentSchedule>` |
| `is_delinquent(borrower)` | `bool` (see `query.rs:57`) |
| `get_protocol_config()` | `ProtocolConfig { liquidity_token, liquidity_source }` |
| `get_liquidity_source()` | `Address` |
| `get_oracle_config()` | `Option<OracleConfig>` |
| `get_rate_formula_config()` | `Option<RateFormulaConfig>` |
| `get_rate_change_limits()` | `Option<RateChangeConfig>` |
| `get_borrower_rate_floor(borrower)` | `Option<u32>` |
| `get_borrower_rate_ceiling(borrower)` | `Option<u32>` |
| `get_grace_period_config()` | `Option<GracePeriodConfig>` |
| `get_penalty_surcharge_bps()` | `u32` |
| `get_max_total_exposure()` | `Option<i128>` |
| `get_credit_limit_bounds()` | `(Option<i128>, Option<i128>)` |
| `get_max_draw_amount/get_max_repay_amount/get_draw_min_interval` | scalars |
| `get_auction_contract()` | `Option<Address>` |
| `get_treasury()` | `Option<Address>` |
| `get_protocol_fee_bps()` | `Option<u32>` |
| `get_collateral(borrower)` | `i128` |
| `get_health_factor(borrower)` | `u32` (bps-scaled, `u32::MAX` when no debt; `< 10_000` = liquidatable; see `query.rs:get_health_factor`) |

Reads with persistent borrower data invoke `bump_credit_line_ttl` (a write,
but cheap and idempotent — see `storage.rs:146`).

### 2.11 Upgrade

#### `upgrade(env, new_wasm_hash: BytesN<32>)`
`lib.rs:1330`. Admin + pause.
1. `require_admin_auth`; `assert_not_paused`
2. Read `old_wasm_hash` (for the event)
3. Bump `DataKey::SchemaVersion`
4. `env.deployer().update_current_contract_wasm(new_wasm_hash)` — atomic
   in-place WASM hash swap (`soroban-sdk 22.0.11`)
5. Emit `ContractUpgradedEvent { old_wasm_hash, new_wasm_hash }` on
   `("credit","upgraded")`

**Migration guards:** none beyond the version bump in this version. New
storage layouts must be additive (new `DataKey` variants); existing
discriminants are pinned by CI test `tests/error_discriminants.rs` and the
absence of a v1 → v2 schema migration script (a v2 release would have to
ship a `migrate()` entrypoint as the first call).

---

## 3. Storage Model

### 3.1 Tier classification

Soroban storage has three tiers: `Temporary`, `Instance`, `Persistent`. The
credit contract uses **only Instance and Persistent**.

**Instance** is the contract's small, always-loaded scratchpad. Used for:
configuration constants, switches, the admin slot, the reentrancy guard, the
pause flag, oracle state, rate-formula state, grace config, treasury config,
credit-line counters and global accumulators.

**Persistent** is per-key on-chain state with explicit TTL. Used for:
per-borrower data (the line itself, last draw timestamp, blocklist flag,
utilization cap, rate floor, repayment schedule, collateral balance, draw
audit trail), and the `(borrower, settlement_id)` replay marker.

### 3.2 `DataKey` enum (full)

(Source: `contracts/credit/src/storage.rs:31-98`. The tier-per-variant
table is also reflected in `docs/storage-layout.md`.)

| Variant | Tier | Notes |
|---|---|---|
| `LiquidityToken` | Instance | SAC / token contract address |
| `LiquiditySource` | Instance | Reserve address funding draws |
| `DrawsFrozen` | Instance | Global draw kill-switch (`bool`) |
| `SchemaVersion` | Instance | Storage schema version (`u32`) |
| `CreditLineCount` | Instance | Monotonic borrower count |
| `CreditLineIdByBorrower(Address)` | Persistent | Borrower → id |
| `CreditLineBorrowerById(u32)` | Persistent | Id → borrower (enumeration) |
| `TotalUtilized` | Instance | Sum of all `utilized_amount` |
| `MaxDrawAmount` | Instance | Per-tx draw cap (`i128`) |
| `MaxRepayAmount` | Instance | Per-tx repay cap |
| `DrawMinIntervalSeconds` | Instance | Per-borrower cooldown (`u64`) |
| `LastDrawTs(Address)` | Persistent | Last successful draw timestamp |
| `BlockedBorrower(Address)` | Persistent | Blocklist flag |
| `UtilizationCapBps(Address)` | Persistent | Per-borrower utilization cap |
| `RateFloorBps(Address)` | Persistent | Per-borrower rate floor |
| `RateCeilingBps(Address)` | Persistent | Per-borrower rate ceiling |
| `RepaymentSchedule(Address)` | Persistent | `RepaymentSchedule` payload |
| `MinCreditLimit` | Instance | Lower bound on new lines |
| `MaxCreditLimit` | Instance | Upper bound on new lines |
| `PenaltySurchargeBps` | Instance | Delinquency surcharge |
| `AuctionContract` | Instance | Auction hook address |
| `MaxTotalExposure` | Instance | Global exposure cap |
| `ProtocolFeeBps` | Instance | Fee on interest portion |
| `TreasuryAddress` | Instance | Withdrawal recipient |
| `TreasuryBalance` | Instance | Accrued fees |
| `CollateralBalance(Address)` | Persistent | Per-borrower collateral |
| `MinCollateralRatioBps` | Instance | Collateral floor (default 15000) |
| `DrawAudit(Address, u64)` | Persistent | `(borrower, ts) → original draw amount` |
| `DrawReversedAmount(Address, u64)` | Persistent | Reversed total so far |
| `OracleConfig` | Instance | `(max_deviation_bps, max_age_seconds)` |
| `OracleLastPrice` | Instance | Last accepted price |
| `OracleLastPriceTs` | Instance | Last accepted ts |

**Instance Symbol keys** (small, hot, low-allocation; see
`storage.rs:269-302`):

| Symbol | Meaning |
|---|---|
| `"admin"` | Admin address |
| `"proposed_admin"` | Pending admin rotation |
| `"proposed_at"` | Timestamp the rotation was proposed |
| `"reentrancy"` | Boolean guard |
| `"rate_cfg"` | `RateChangeConfig` |
| `"rate_form"` | `RateFormulaConfig` |
| `"paused"` | Circuit-breaker flag |
| `"grace_cfg"` | `GracePeriodConfig` |

**Persistent Symbol tuple:**
`(symbol_short!("liq_seen"), borrower, settlement_id)` — settlement replay
marker (`lifecycle.rs:39-48`).

### 3.3 TTL bump schedule

Every read or write that touches a persistent key checks the remaining TTL
and, if it is below `LEDGER_BUMP_THRESHOLD ≈ 3 months`, extends it to
`LEDGER_BUMP_AMOUNT ≈ 6 months`
(`contracts/credit/src/storage.rs:122-127`). This means an active borrower's
data is automatically refreshed every time they (or a keeper) touch the
contract. A dormant borrower's data expires after ~6 months; recovery
requires admin republish from off-chain state. The `accrue_batch` entrypoint
(`lib.rs:1133`) exists primarily to let an indexer-driven keeper re-bump
dormant lines cheaply.

The auction contract uses shorter TTLs:
`PERSISTENT_BUMP_AMOUNT = 518_400` (~30 d) and
`PERSISTENT_LIFETIME_THRESHOLD = 120_960` (~7 d)
(`gateway-contract/contracts/auction_contract/src/storage.rs`).

---

## 4. Invariants

Invariants the contract maintains, by module.

### 4.1 Global

- **`TotalUtilized` conservation.**
  `TotalUtilized == Σ over all open credit lines of utilized_amount`. Enforced
  by routing every credit-line mutation through `persist_credit_line(env,
  borrower, line, previous_utilized)` (`storage.rs:257`), which atomically
  updates the line *and* the accumulator using `adjust_total_utilized`
  (`storage.rs:239`). Test: `tests/total_utilized_invariant.rs`.

- **No overflow.** Every arithmetic operation that can grow beyond `i128::MAX`
  uses `checked_add` / `checked_mul` and reverts with `Overflow = 12` instead
  of wrapping. Test: `tests/accrual_overflow_audit.rs`.

- **Monotonic timestamps.** `last_accrual_ts`, `last_rate_update_ts`,
  `suspension_ts` are non-decreasing. Enforced via `assert_ts_monotonic`
  (`storage.rs:538`); violation reverts with `TimestampRegression = 33`.
  Test: `tests/monotonic_timestamps.rs`.

- **Single-shot init.** `init` reverts with `AlreadyInitialized = 14` if
  re-invoked. Test: `tests/init_idempotency.rs`.

- **Stable error discriminants.** `tests/error_discriminants.rs` reverts CI
  on reorder or renumber of `ContractError`.

- **Stable event topics.** `tests/event_topic_stability.rs` pins the topic
  symbol strings.

### 4.2 Credit-line module

- **Status reachability.** Only the transitions in §4 of `WHITEPAPER.md` are
  reachable. `Closed` is terminal. Test:
  `tests/state_transition_invariants.rs`.

- **Repayment never blocked except for Closed.** Pause is the only switch
  that can block draws; `repay_credit` bypasses it. Test:
  `tests/circuit_breaker.rs`.

- **`Restricted` is reversible by repayment.** A line in `Restricted` whose
  `utilized_amount` falls below the (new) `credit_limit` is auto-promoted to
  `Active` on the next mutation. Test: `tests/restricted_status.rs`.

- **Settlement replay safety.** `(borrower, settlement_id)` is the dedup key.
  Test: `tests/default_liquidation_settled_event.rs`.

### 4.3 Risk / accrual

- **Rate cap.** `interest_rate_bps <= MAX_INTEREST_RATE_BPS = 10_000` after
  every mutation. Tests: `tests/risk_formula_tests.rs` (inline),
  `tests/penalty_surcharge.rs`.

- **Floor-rounded interest.** `compute_interest` rounds to floor; the
  borrower never overpays one stroop of theoretical interest. Test:
  `accrual_tests.rs` (inline), `tests/accrual_overflow_audit.rs`.

- **Score range.** `risk_score <= 100`. Test: inline `risk_formula_tests.rs`.

### 4.4 Reentrancy

- The guard is set on `draw_credit`, `repay_credit`,
  `settle_default_liquidation` (credit), and `place_bid` (English refund) /
  `claim_auction` (auction). Cleared on every exit path including error.
  Stored under instance `Symbol("reentrancy")`. Tests:
  `tests/token_failure_rollback.rs` and auction `test.rs` reentrancy tests.

### 4.5 Auction

- One-shot settlement per `auction_id`. Stored at
  `AuctionKey::LiquidationSettled(Symbol)`. Test: cross-contract
  `tests/credit_auction_e2e.rs`.
- English mode atomically refunds the prior bidder under the reentrancy
  guard before recording the new bid.
- Dutch mode closes on first qualifying bid (atomic status flip from `Open`
  to `Closed`).

---

## 5. Error Taxonomy

The full enum (38 variants, `#[repr(u32)]`, discriminants stable ABI). The
table also appears in `docs/contract-errors.md` and is the source of truth
for off-chain decoders.

| Code | Variant | Semantic |
|---|---|---|
| 1 | `Unauthorized` | Caller fails an auth check (not admin / not borrower) |
| 2 | `NotAdmin` | Caller lacks admin privilege specifically |
| 3 | `CreditLineNotFound` | Borrower has no credit line in storage |
| 4 | `CreditLineClosed` | Line is in terminal `Closed` state |
| 5 | `InvalidAmount` | Amount is zero, negative, or out-of-range |
| 6 | `OverLimit` | Draw would exceed `credit_limit` |
| 7 | `NegativeLimit` | Credit limit < 0 |
| 8 | `RateTooHigh` | Rate delta exceeds cap, or rate > 10_000 |
| 9 | `ScoreTooHigh` | Risk score > 100 |
| 10 | `UtilizationNotZero` | Operation requires zero utilization |
| 11 | `Reentrancy` | Reentrancy detected |
| 12 | `Overflow` | Arithmetic overflow |
| 13 | `LimitDecreaseRequiresRepayment` | Lower limit blocked by current utilization |
| 14 | `AlreadyInitialized` | `init` already ran, or `(borrower, settlement_id)` replay |
| 15 | `AdminAcceptTooEarly` | Rotation delay not elapsed |
| 16 | `BorrowerBlocked` | Borrower on blocklist |
| 17 | `DrawExceedsMaxAmount` | Per-tx draw cap |
| 18 | `Paused` | Circuit breaker active |
| 19 | `DrawsFrozen` | Global draws frozen |
| 20 | `CreditLineSuspended` | Line suspended |
| 21 | `CreditLineDefaulted` | Line defaulted |
| 22 | `MissingLiquidityToken` | Liquidity token unset |
| 23 | `MissingLiquiditySource` | Liquidity source unset |
| 24 | `InsufficientLiquidityReserve` | Reserve balance insufficient |
| 25 | `LiquidityTokenCallFailed` | Token call failed observably |
| 26 | `InsufficientRepaymentAllowance` | Allowance below repay amount |
| 27 | `InsufficientRepaymentBalance` | Balance below repay amount (also collateral over-withdraw) |
| 28 | `RepayExceedsMaxAmount` | Per-tx repay cap |
| 29 | `DrawCooldownActive` | Draw cooldown not elapsed |
| 30 | `TreasuryNotSet` | Treasury address unset |
| 31 | `ExposureCapExceeded` | Global exposure cap |
| 32 | `AdminNotInitialized` | Admin missing from instance storage |
| 33 | `TimestampRegression` | Timestamp moved backwards |
| 34 | `LimitOutOfBounds` | Outside min/max credit-limit bounds |
| 35 | `CollateralRatioBelowMinimum` | Under-collateralized |
| 36 | `OraclePriceInvalid` | Oracle price ≤ 0 or malformed |
| 37 | `OraclePriceStale` | Exceeds `max_age_seconds` |
| 38 | `OraclePriceDeviation` | Exceeds `max_deviation_bps` |

Auction errors (`AuctionError`, 12 variants, see
`gateway-contract/contracts/auction_contract/src/errors.rs`):
`NotWinner=1, AlreadyClaimed=2, NotClosed=3, NoFactoryContract=4,
Unauthorized=5, InvalidState=6, BidTooLow=7, AuctionNotOpen=8,
AuctionNotClosed=9, Reentrancy=10, NoWinner=11, NotFound=12`.

---

## 6. Reentrancy & CEI

The credit contract uses **explicit reentrancy guard + strict CEI** for the
two paths that perform external token CPI:

- **`draw_credit`:** guard set → all checks → external transfer → state
  persist → guard cleared. The post-transfer persist is safe because the
  guard prevents a re-entered `draw_credit` from advancing state.
- **`repay_credit`:** guard set → all checks → two `transfer_from` CPIs
  (fee, then reserve) → state persist → guard cleared.
- **`settle_default_liquidation`:** guard set → oracle check → cross-contract
  `AuctionClient::settle_default_liquidation` → accounting → guard cleared.

The guard storage is `Symbol("reentrancy")` (instance). `set_reentrancy_guard`
reverts with `Reentrancy = 11` if already set. `clear_reentrancy_guard` is
idempotent and writes `false`.

The auction contract uses the same primitive (`Symbol("reentrancy")` instance
storage, `AuctionError::Reentrancy = 10`) wrapping the English-mode prior-bid
refund and the (placeholder) winner payout in `claim_auction`.

---

## 7. Upgrade Path

Soroban supports in-place WASM swap via
`env.deployer().update_current_contract_wasm(new_wasm_hash)`. The credit
contract exposes this through `upgrade(new_wasm_hash)` (admin + pause).

**Preserved across upgrade:** persistent and instance storage, contract
address, TTLs.
**Not preserved:** in-flight transactions, the reentrancy-guard
state (instance, but reset by the upgrade pause cycle), wasm-side runtime
state.

**Migration model:** additive only.
- New `DataKey` variants are safe to add (existing discriminants stable).
- New `ContractError` variants must be appended (CI guards against reorder).
- New event topics are safe.
- *Breaking* storage migrations require a one-shot `migrate()` entrypoint
  shipped as the first call against the new WASM; none required in v1.

**Schema version bump:** `upgrade` increments `SchemaVersion` so off-chain
indexers can refuse decoding events with a higher major version than they
understand (see `docs/indexer-integration.md`).

**Pause + upgrade interaction:** `upgrade` requires the contract to be
unpaused (`assert_not_paused`). A safe upgrade procedure is therefore:

1. Pause via `pause_protocol` (drains in-flight draws; repayments still go
   through).
2. Wait one ledger close.
3. Unpause + `upgrade(new_hash)` in the same admin transaction (or sequenced
   transactions if the admin is a multisig).

---

## 8. Cross-references

| Concept | Location |
|---|---|
| State machine | `docs/state-machine.md`, `WHITEPAPER.md` §4 |
| Risk pricing | `docs/risk-based-rate-formula.md`, `docs/RISK_PRICING.md` |
| Accrual | `docs/interest-accrual.md`, `docs/interest-accrual-design.md` |
| Storage layout | `docs/storage-layout.md` |
| Threat model | `docs/threat-model.md`, `docs/SECURITY.md` |
| Liquidation handoff | `docs/default-liquidation-auction-hook.md` |
| Oracle (price) | `WHITEPAPER.md` §7.1 |
| Oracle (default signal, staged) | `docs/default-oracle.md` |
| Upgrade policy | `docs/upgrade-policy.md` |
| Event catalog | `docs/events-schema.md` |
| Deployment | `docs/deploy.md` |
| Test catalog | `docs/EXECUTION_QUALITY.md` |
