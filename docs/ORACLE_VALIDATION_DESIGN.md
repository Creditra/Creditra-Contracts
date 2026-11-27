# Oracle Input Validation Before Price-Dependent Settlement

## Issue #1168: Design Document

### 1. State Model

#### 1.1 Oracle Configurations

**Single-Oracle Circuit Breaker (`OracleConfig`)**
```rust
pub struct OracleConfig {
    pub max_deviation_bps: u32,    // 1-10000 basis points
    pub max_age_seconds: u64,       // freshness window
}
```
- Optional: if not set, no oracle validation required
- Stores last accepted price and timestamp in instance storage
- Used when `OracleQuorumConfig` is NOT set

**Multi-Oracle Quorum (`OracleQuorumConfig`)**
```rust
pub struct OracleQuorumConfig {
    pub min_quorum_k: u32,          // k >= 2 feeds required
    pub max_deviation_bps: u32,     // 1-10000 basis points
    pub max_age_seconds: u64,       // freshness window
}
```
- Optional: if not set, falls back to single-oracle mode
- Stores resolved median price and timestamp after `submit_oracle_prices`
- Takes precedence over single-oracle mode when both are configured

#### 1.2 Settlement State

Per-borrower credit line with status machine:
- `Active` → draw-capable, cannot settle
- `Defaulted` → awaiting settlement (target state for liquidation)
- `Closed` → terminal after successful settlement

Replay protection marker:
- Key: `(Symbol("liq_seen"), borrower, settlement_id)`
- Prevents double-settlement of the same (borrower, settlement_id) pair

### 2. Failure Scenarios & Invariants

#### 2.1 Price Validity Invariants

| Invariant | Condition | Error | Impact |
|-----------|-----------|-------|--------|
| **Positivity** | `price > 0` | `OraclePriceInvalid(36)` | Silent loss prevention: zero/negative prices corrupt liquidation math |
| **Freshness** | `now - timestamp ≤ max_age_seconds` | `OraclePriceStale(37)` | Prevents stale oracle outages from liquidating at wrong prices |
| **Deviation** | `\|(price - last_price) / last_price\| ≤ max_deviation_bps` | `OraclePriceDeviation(38)` | Prevents flash-loan or manipulation attacks on settlement |
| **Presence** | If oracle config set, price required | `OraclePriceInvalid(36)` | Enforces consistent validation across all settlements |

#### 2.2 Authorization & Replay Invariants

| Invariant | Condition | Error | Impact |
|-----------|-----------|-------|--------|
| **Admin Auth** | Caller must be admin | `NotAdmin(2)` | Settlement gate to prevent unauthorized recovery |
| **Defaulted Status** | Credit line status must be `Defaulted` | `CreditLineDefaulted(21)` | Only defaulted lines can settle |
| **Replay Protection** | `(borrower, settlement_id)` not yet seen | `AlreadyInitialized(14)` | Prevents double-settlement |

#### 2.3 Numeric Invariants

| Invariant | Condition | Error | Impact |
|-----------|-----------|-------|--------|
| **Close Factor Bounds** | `0 < close_factor_bps ≤ 10_000` | `InvalidAmount(5)` | Prevents invalid settlement percentages |
| **Close Factor Cap** | `close_factor_bps ≤ protocol_max` | `OverLimit(6)` | Enforces global settlement cap |
| **Recovered Amount** | `recovered_amount > 0` | `InvalidAmount(5)` | Prevents zero/negative recovery |
| **Recovery vs Capacity** | `recovered_amount ≤ utilized * close_factor / 10_000` | `OverLimit(6)` | Cannot recover more than what can be written off |
| **Overflow Safety** | All arithmetic uses `checked_*` primitives | `Overflow(12)` | Prevents silent wraparound |

### 3. Validation Entry Point: `settle_default_liquidation`

```rust
pub fn settle_default_liquidation(
    env: Env,
    borrower: Address,
    recovered_amount: i128,
    settlement_id: Symbol,
    close_factor_bps: u32,
    oracle_price: Option<i128>,  // NEW: explicit parameter
) -> Result<(), ContractError>
```

#### 3.1 Validation Order

1. **Authorization** (gate before any reads)
   - Admin auth required
   - Reentrancy guard check

2. **Replay Protection** (gate before state reads)
   - Check `(borrower, settlement_id)` not settled before

3. **Numeric Validation** (cheap checks)
   - `recovered_amount > 0`
   - `close_factor_bps ∈ (0, 10_000]`
   - `close_factor_bps ≤ protocol_max`

4. **Credit Line Read** (hot path with TTL bump)
   - Load credit line
   - Apply accrual
   - Verify status == `Defaulted`

5. **Oracle Validation** (before state mutation)
   - Load oracle config(s)
   - Validate price:
     - If quorum config → use stored quorum price (ignore `oracle_price` arg)
     - Else if single-oracle config → validate `oracle_price` arg
     - Else → price optional (backward compatible)
   - Check positivity, staleness, deviation

6. **Economic Validation** (amount checks)
   - Compute max_recoverable = utilized * close_factor / 10_000
   - Verify `recovered_amount ≤ max_recoverable`

7. **State Mutation** (committed only after all validation)
   - Reduce `utilized_amount`
   - Transition to `Closed` if utilized → 0
   - Persist credit line
   - Mark settlement as seen
   - Emit events

#### 3.2 Failure Modes

**Retries & Idempotency:**
- If oracle price fails validation → entire settlement reverts
- Retries with same settlement_id → blocked by replay protection
- Retries with new settlement_id → allowed (new attempt)
- No partial state: either full success or full rollback (Soroban semantics)

**Concurrent Execution:**
- Soroban single-threaded: no genuine concurrency
- Ledger finality: settlement committed atomically
- Cross-contract calls to auction: separate reentrancy guard

**Partial Failure:**
- If oracle becomes stale mid-settlement → revert (no rollback needed, atomic transaction)
- If admin changes oracle config post-validation → settlement still uses captured price (no TOCTOU)

### 4. Data Model & Storage

#### 4.1 Instance Storage (Global)

```
OracleConfig:              (max_deviation_bps, max_age_seconds)
OracleLastPrice:           i128
OracleLastPriceTs:         u64

OracleQuorumConfig:        (min_quorum_k, max_deviation_bps, max_age_seconds)
OracleQuorumPrice:         i128
OracleQuorumPriceTs:       u64
```

#### 4.2 Persistent Storage (Per-borrower, Replay Protection)

```
LiquidationSettlementSeen: (Symbol("liq_seen"), borrower, settlement_id) → bool
```

### 5. Test Strategy

#### 5.1 Unit Tests (validator module)

**Validity checks:**
- ✓ Positive price accepted
- ✗ Zero price rejected (OraclePriceInvalid)
- ✗ Negative price rejected (OraclePriceInvalid)
- ✗ Overflow on `i128::MIN` rejected

**Staleness checks:**
- ✓ Fresh price accepted
- ✓ Price at exact max_age accepted
- ✗ Price 1s beyond max_age rejected (OraclePriceStale)
- ✓ First price accepted (no prior reference)

**Deviation checks:**
- ✓ Zero deviation accepted (identical prices)
- ✓ Within-bound deviation accepted
- ✓ Boundary: exactly at max_deviation accepted
- ✗ Over-deviation upward rejected (OraclePriceDeviation)
- ✗ Over-deviation downward rejected (OraclePriceDeviation)

**Configuration resolution:**
- ✓ Quorum mode used when both modes set (precedence)
- ✓ Single-oracle fallback when quorum not configured
- ✓ No validation when neither configured (backward compat)
- ✗ Missing price when single-oracle required (OraclePriceInvalid)

#### 5.2 Integration Tests (settlement)

**Normal operation:**
- ✓ Settlement succeeds with valid oracle price
- ✓ Settlement updates credit line status
- ✓ Replay attempt fails with AlreadyInitialized

**Oracle outages:**
- ✗ Stale oracle blocks settlement
- ✗ Deviation blocks settlement
- ✓ Admin can extend max_age or widen deviation to recover

**Boundary cases:**
- ✓ Partial close factor works
- ✓ Full close factor (10_000 bps) closes line
- ✓ Multiple borrowers settle independently
- ✗ Concurrent settlements (impossible in Soroban, but test isolation)

**Backward compatibility:**
- ✓ Settlement without oracle config still works
- ✓ Existing `oracle_price` single-oracle param accepted

#### 5.3 E2E Tests

- Multi-step scenario: deposit collateral → draw → default → settle with pricing
- Oracle failure recovery path
- Cross-contract auction integration

### 6. Key Design Decisions

#### 6.1 Why Validate Before State Mutation

- **Atomicity guarantee**: If oracle fails, entire settlement reverts
- **No partial state**: Borrower never sees half-closed credit line
- **No silent failure**: Admin sees error, not mysterious settlement gaps
- **Auditability**: Clear log of what was validated when

#### 6.2 Why Quorum Takes Precedence

- **Safety property**: Multi-oracle consensus is stronger than single-oracle
- **Gradual migration**: Deploy quorum config alongside single-oracle, later deprecate
- **Clear semantics**: Single oracle_price arg becomes "hint only" in quorum mode

#### 6.3 Why Optional oracle_price Parameter

- **Backward compatibility**: Existing integrations don't break
- **Forward compatibility**: Can add oracle providers without redeploying contracts
- **Clear intent**: Caller explicitly passes price when needed

#### 6.4 Why Preserve Least Privilege

- **Authorization**: Only admin can settle (not borrower, not keepers)
- **Validation**: Tight bounds on price deviation (circuit breaker)
- **Replay**: Each (borrower, settlement_id) pair can only settle once
- **State machine**: Only Defaulted lines can settle

### 7. Error Taxonomy

| Error | Code | Category | Recovery |
|-------|------|----------|----------|
| `OraclePriceInvalid` | 36 | Oracle | Provide valid price or adjust config |
| `OraclePriceStale` | 37 | Oracle | Wait for fresh price or extend max_age |
| `OraclePriceDeviation` | 38 | Oracle | Wait for price convergence or widen bound |
| `OracleQuorumNotMet` | 50 | Oracle | Submit more oracle prices or lower k |
| `NotAdmin` | 2 | Auth | Use admin key |
| `CreditLineDefaulted` | 21 | Lifecycle | Credit line not in Defaulted state |
| `AlreadyInitialized` | 14 | Replay | Use new settlement_id |
| `InvalidAmount` | 5 | Numeric | Check price/amount bounds |
| `OverLimit` | 6 | Numeric | Check recovery cap |

### 8. Observability

#### 8.1 Events Emitted

- `OraclePriceAccepted(price, timestamp)` — when single-oracle price validated
- `OracleQuorumPriceSet(price, timestamp)` — when quorum resolved
- `DefaultLiquidationSettled(borrower, settlement_id, recovered_amount, ...)` — final outcome

#### 8.2 Logs

- Price validation entry and exit
- Oracle config resolution (which mode active)
- Deviation computation (actual vs max)
- Settlement state transitions

### 9. Non-Goals

- Removing oracle validation
- Weakening deviation bounds to make tests pass
- Trusting a single oracle without validation
- Allowing unsigned/unauthenticated price updates
- Dynamic reconfig mid-settlement (TOCTOU risk)

---

## Summary: Acceptance Criteria Mapping

| Criterion | Design Element | Validation |
|-----------|---|---|
| Deterministic for valid inputs | Validation order #3.1, unit tests #5.1 | ✓ Same inputs → same outcome |
| Deterministic for invalid inputs | Error matrix #7, unit tests | ✓ Invalid → consistent error |
| Authorization/validation invariants preserved | Section #2.2, gate in #3.1 step 1-2 | ✓ Admin auth + replay protection |
| Retries & concurrency safe | Section #3.2 failure modes | ✓ Atomic txn, replay-protected |
| Focused tests | Section #5.1-5.2 | ✓ Unit + integration + E2E |
| Backward compatible | Section #6.3, Section #3.2 last bullet | ✓ oracle_price optional |
| Diagnostics without sensitive data | Section #8 observability | ✓ Price/amount logs only |
