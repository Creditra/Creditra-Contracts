# Validation Layer Design for Oracle Price Settlement

## Overview

This document describes the module structure and interfaces for the oracle input validation layer integrated into `settle_default_liquidation`.

## Architecture

### 1. Module Structure

```
contracts/credit/src/
├── lifecycle.rs (modified)
│   └── settle_default_liquidation() — PUBLIC ENTRY
│       └── calls validate_settlement_oracle_price()
├── oracle_validation.rs (NEW)
│   ├── validate_settlement_oracle_price() — INTERNAL
│   ├── OracleValidationResult — STRUCT
│   ├── ResolvedOraclePrice — ENUM
│   └── Unit tests
└── oracles.rs (existing)
    ├── resolve_quorum_price()
    ├── get_oracle_quorum_price()
    └── Unit tests
```

### 2. New Module: `oracle_validation.rs`

**Purpose**: Encapsulate all oracle validation logic for settlement.

**Exports**:
- `validate_settlement_oracle_price()` — core validation function
- Type definitions for validation results

**Dependencies**:
- `crate::storage::*` (oracle getters)
- `crate::types::*` (errors, configs)
- `crate::math_utils::compute_deviation_bps()`
- `crate::oracles::resolve_quorum_price()`
- `soroban_sdk::Env`

### 3. Core Validation Function

```rust
/// Validate oracle price(s) for settlement before state mutation.
///
/// # Behavior
///
/// 1. If quorum config is set → uses stored quorum price (single-oracle arg ignored)
/// 2. Else if single-oracle config is set → validates supplied `oracle_price` arg
/// 3. Else → price is optional (backward compatible, no validation)
///
/// # Parameters
/// - `env`: Soroban environment
/// - `oracle_price`: optional price supplied by caller (single-oracle mode)
///
/// # Returns
/// `ResolvedOraclePrice` enum:
/// - `NotConfigured` — neither oracle config is set (no validation needed)
/// - `QuorumMode(price)` — quorum price successfully validated
/// - `SingleOracleMode(price)` — single-oracle price successfully validated
///
/// # Errors
/// Panics with:
/// - `OraclePriceInvalid` (36) — price is zero, negative, or missing when required
/// - `OraclePriceStale` (37) — price exceeds max_age_seconds
/// - `OraclePriceDeviation` (38) — price deviates from last accepted price
/// - `OracleQuorumNotMet` (50) — quorum price not yet submitted
///
/// # Side effects
/// None — pure validation, no storage mutations.
pub fn validate_settlement_oracle_price(
    env: &Env,
    oracle_price: Option<i128>,
) -> ResolvedOraclePrice
```

### 4. Result Type Definition

```rust
/// Outcome of oracle validation for settlement.
///
/// Each variant represents a successful validation in a different mode.
/// Failures panic with typed `ContractError` before returning.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolvedOraclePrice {
    /// Neither oracle config is set; price is optional and validation is skipped.
    /// Settlement proceeds without oracle gating (backward compatible).
    NotConfigured,

    /// Quorum config was active; the supplied single-oracle `oracle_price` arg
    /// is ignored. The stored quorum price was validated and accepted.
    /// Contains the validated quorum price (for observability/logging).
    QuorumMode(i128),

    /// Single-oracle config was active. The supplied `oracle_price` arg
    /// was validated and accepted. Contains the validated price.
    SingleOracleMode(i128),
}

impl ResolvedOraclePrice {
    /// Extract the validated price, if one was resolved.
    pub fn price(&self) -> Option<i128> {
        match self {
            ResolvedOraclePrice::NotConfigured => None,
            ResolvedOraclePrice::QuorumMode(p) => Some(*p),
            ResolvedOraclePrice::SingleOracleMode(p) => Some(*p),
        }
    }
}
```

### 5. Internal Validation Stages

Each stage is tested independently; failures panic immediately.

#### Stage 1: Configuration Resolution

```rust
fn resolve_oracle_configs(env: &Env) -> (Option<OracleConfig>, Option<OracleQuorumConfig>)
```

- Load both configs from storage
- Determine active mode: quorum > single-oracle > none
- Return tuple for downstream validation

#### Stage 2: Quorum Mode Validation (if applicable)

```rust
fn validate_quorum_price(env: &Env, cfg: &OracleQuorumConfig) -> i128
```

- Load stored quorum price + timestamp
- Check not None (OracleQuorumNotMet if missing)
- Check freshness: `now - ts <= cfg.max_age_seconds` (OraclePriceStale)
- Return validated price

#### Stage 3: Single-Oracle Mode Validation (if applicable)

```rust
fn validate_single_oracle_price(
    env: &Env,
    price: Option<i128>,
    cfg: &OracleConfig,
) -> i128
```

- Check price is Some (OraclePriceInvalid if missing when config set)
- Check price > 0 (OraclePriceInvalid)
- Validate freshness:
  - If first price: accept any positive price
  - Else check: `now - last_ts <= cfg.max_age_seconds` (OraclePriceStale)
- Validate deviation (if last price exists):
  - compute `deviation_bps(price, last_price)`
  - check `dev <= cfg.max_deviation_bps` (OraclePriceDeviation)
- Return validated price

#### Stage 4: Price Update (after settlement succeeds)

```rust
pub fn record_accepted_oracle_price(env: &Env, price: i128) -> ()
```

- Called AFTER settlement completes successfully
- Updates `OracleLastPrice` and `OracleLastPriceTs` in instance storage
- Single atomic write for both (preserves consistency)

### 6. Integration into `settle_default_liquidation`

**Calling sequence**:

```rust
pub fn settle_default_liquidation(
    env: Env,
    borrower: Address,
    recovered_amount: i128,
    settlement_id: Symbol,
    close_factor_bps: u32,
    oracle_price: Option<i128>,
) {
    // Step 1: Authorization & replay protection (existing)
    require_admin_auth(&env);
    let settlement_key = liquidation_settlement_key(&borrower, &settlement_id);
    if env.storage().persistent().has(&settlement_key) {
        env.panic_with_error(ContractError::AlreadyInitialized);
    }

    // Step 2: Numeric validation (existing)
    if recovered_amount <= 0 || close_factor_bps == 0 || close_factor_bps > 10_000 {
        env.panic_with_error(ContractError::InvalidAmount);
    }
    let max_close_factor = crate::storage::get_close_factor_bps(&env);
    if close_factor_bps > max_close_factor {
        env.panic_with_error(ContractError::OverLimit);
    }

    // Step 3: Oracle validation (NEW - BEFORE state mutation)
    let oracle_result = crate::oracle_validation::validate_settlement_oracle_price(
        &env,
        oracle_price,
    );
    // oracle_result is consumed for logging/metrics but not needed for logic

    // Step 4: Credit line read & accrual (existing)
    let stored_line: CreditLineData = crate::storage::get_credit_line(&env, &borrower)
        .unwrap_or_else(|| env.panic_with_error(ContractError::CreditLineNotFound));
    let previous_utilized = stored_line.utilized_amount;
    let mut credit_line = crate::accrual::apply_accrual(&env, stored_line);

    if credit_line.status != CreditStatus::Defaulted {
        env.panic_with_error(ContractError::CreditLineDefaulted);
    }

    // Step 5: Economic validation (existing, FIXED bug here)
    let max_recoverable = credit_line
        .utilized_amount
        .checked_mul(close_factor_bps as i128)
        .unwrap_or_else(|| env.panic_with_error(ContractError::Overflow))
        .checked_div(10_000)
        .unwrap_or_else(|| env.panic_with_error(ContractError::Overflow));

    if recovered_amount > max_recoverable {
        env.panic_with_error(ContractError::OverLimit);
    }

    // Step 6: State mutation
    credit_line.utilized_amount = credit_line
        .utilized_amount
        .checked_sub(recovered_amount)  // FIXED: was undefined actual_recovery
        .unwrap_or_else(|| env.panic_with_error(ContractError::Overflow));

    let previous_status = credit_line.status;
    if credit_line.utilized_amount == 0 {
        credit_line.status = CreditStatus::Closed;
    }

    persist_credit_line(
        &env,
        &borrower,
        &credit_line,
        previous_utilized,
        Some(previous_status),
    );

    if credit_line.status == CreditStatus::Closed {
        clear_repayment_schedule(&env, &borrower);
    }

    // Step 7: Replay protection & oracle price recording (NEW)
    env.storage().persistent().set(&settlement_key, &true);
    if oracle_result.price().is_some() {
        crate::oracle_validation::record_accepted_oracle_price(
            &env,
            oracle_result.price().unwrap(),
        );
    }

    // Step 8: Events (existing)
    if credit_line.status == CreditStatus::Closed {
        publish_credit_line_event(
            &env,
            (symbol_short!("credit"), symbol_short!("closed")),
            CreditLineEvent {
                borrower: borrower.clone(),
                status: CreditStatus::Closed,
                credit_limit: credit_line.credit_limit,
                interest_rate_bps: credit_line.interest_rate_bps,
                risk_score: credit_line.risk_score,
            },
        );
    }

    publish_default_liquidation_settled_event(
        &env,
        DefaultLiquidationSettledEvent {
            borrower,
            settlement_id,
            recovered_amount,
            remaining_utilized_amount: credit_line.utilized_amount,
            status: credit_line.status,
            close_factor_bps,
        },
    );
}
```

### 7. Error Handling Strategy

**All validation failures are fatal** (panic immediately):

```rust
// ✗ Invalid price
if price <= 0 {
    env.panic_with_error(ContractError::OraclePriceInvalid);
}

// ✗ Stale price
if now - ts > cfg.max_age_seconds {
    env.panic_with_error(ContractError::OraclePriceStale);
}

// ✗ Excessive deviation
if deviation_bps > cfg.max_deviation_bps {
    env.panic_with_error(ContractError::OraclePriceDeviation);
}

// ✗ Missing quorum price
if quorum_price.is_none() {
    env.panic_with_error(ContractError::OracleQuorumNotMet);
}

// ✓ Success: settlement proceeds
```

**Rationale**: Settlement is all-or-nothing; partial failure (partial state mutation) is worse than full rollback. Caller retry with corrected input or new settlement_id.

### 8. Storage Interaction

**Reads** (no mutations yet):
- `get_oracle_config()` → `Option<OracleConfig>`
- `get_oracle_quorum_config()` → `Option<OracleQuorumConfig>`
- `get_oracle_last_price()` → `Option<i128>`
- `get_oracle_last_price_ts()` → `Option<u64>`
- `get_oracle_quorum_price()` → `Option<i128>` (new accessor)
- `get_oracle_quorum_price_ts()` → `Option<u64>` (new accessor)

**Writes** (after settlement succeeds):
- `set_oracle_last_price(price, ts)` — atomic pair update
- `set_oracle_quorum_price(price, ts)` — if quorum mode (existing via `submit_oracle_prices`)

### 9. Observability & Logging

Each validation stage can emit for diagnostics (not required for logic):

```rust
// Conceptual logging (actual impl uses Soroban events)
log_oracle_validation_started();
match (oracle_cfg, quorum_cfg) {
    (None, None) => {
        log_oracle_not_configured();
        // proceeding without validation
    }
    (_, Some(qcfg)) => {
        log_oracle_quorum_mode_active();
        log_oracle_quorum_price_validated(quorum_price);
    }
    (Some(cfg), None) => {
        log_oracle_single_mode_active();
        log_oracle_price_checked_positive(price);
        log_oracle_price_freshness_ok(age, cfg.max_age_seconds);
        log_oracle_price_deviation_ok(deviation_bps, cfg.max_deviation_bps);
    }
}
```

**Events** (emitted after success):
- `OraclePriceAccepted(price, timestamp)` for single-oracle mode
- `DefaultLiquidationSettled(...)` includes settlement outcome

### 10. Testing Strategy

See separate task #6 for detailed test specifications.

**Unit tests** (in `oracle_validation.rs`):
- Each validation stage tested in isolation
- Edge cases: boundary prices, exact max_age, ceiling deviation
- Error cases: invalid prices, stale, over-deviation

**Integration tests** (in `settlement_oracle_validation.rs`):
- Full settlement flow with oracle validation
- Multiple settlements reusing last price
- Oracle config updates between settlements
- Quorum vs single-oracle precedence
- Backward compat: no oracle config

### 11. Backward Compatibility

**Guarantee**: Existing integrations continue to work.

- `oracle_price` parameter added as optional argument
- If no oracle config: validation skipped (settlement works as before)
- If oracle config added later: NEW settlements validate, old ones don't need replay
- Quorum mode: additive, doesn't break single-oracle

### 12. Security Considerations

**Threat Model**:
1. **Flash-loan price attack**: quorum + deviation check prevents
2. **Stale oracle outage**: max_age_seconds + staleness check prevents
3. **Manipulation via incorrect price**: deviation check detects sudden swings
4. **Double-settlement**: replay protection prevents
5. **Unauthorized settlement**: admin auth required

**Mitigations**:
- All validation BEFORE state mutation
- All errors are fatal (atomic rollback)
- All prices validated to strict bounds
- Deterministic validation order
- No TOCTOU issues (price frozen at validation time)

---

## Summary

The validation layer cleanly separates oracle concerns from settlement logic:

1. **Encapsulation**: `oracle_validation.rs` owns all oracle logic
2. **Clarity**: Validation order is explicit and linear
3. **Testability**: Each stage tested independently
4. **Safety**: All failures are fast-fail, before state mutation
5. **Backward Compat**: Existing code unaffected when oracle config not set
6. **Forward Compat**: Can add oracle providers without redeployment
