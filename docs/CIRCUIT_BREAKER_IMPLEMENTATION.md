# Circuit Breaker (Emergency Pause) Implementation

**Date:** 2026-04-24  
**Feature:** Emergency protocol pause with repay_credit exception

---

## Summary

Implemented a protocol-wide circuit breaker that allows the admin to halt all mutating operations in case of an exploit or critical bug, with the explicit exception of `repay_credit` to ensure users can always reduce their debt exposure.

---

## Changes

### Core Implementation

**`src/types.rs`**
- Added `ContractError::Paused = 18` to the stable error enum
- Updated discriminant table documentation

**`src/storage.rs`**
- Added `paused_key()` for instance storage
- Added `is_paused(env) -> bool` getter
- Added `set_paused(env, paused)` setter (caller must enforce admin auth)
- Added `assert_not_paused(env)` guard function

**`src/events.rs`**
- Added `ProtocolPausedEvent { admin, paused, timestamp }`
- Added `publish_protocol_paused_event()` emitting `("credit", "paused")` or `("credit", "unpaused")`

**`src/lib.rs`**
- Added `set_protocol_paused(paused: bool)` admin-only method
- Added `is_protocol_paused() -> bool` view method
- Injected `assert_not_paused(&env)` guard into:
  - `open_credit_line`
  - `draw_credit`
  - `set_liquidity_token`
  - `set_liquidity_source`
  - `set_rate_change_limits`
  - `set_max_draw_amount`
- **Explicitly excluded** `repay_credit` from the guard

**`src/lifecycle.rs`**
- Injected `assert_not_paused(&env)` into:
  - `suspend_credit_line`
  - `close_credit_line`
  - `default_credit_line`
  - `reinstate_credit_line`

**`src/risk.rs`**
- Injected `assert_not_paused(&env)` into `update_risk_parameters`

### Test Coverage

**`tests/circuit_breaker.rs`** — 25 tests covering:

1. **Authorization**
   - `admin_can_pause_protocol`
   - `admin_can_unpause_protocol`
   - `non_admin_cannot_pause`

2. **Event Emission**
   - `pause_emits_event`
   - `unpause_emits_event`

3. **Blocked Operations** (11 tests)
   - `open_credit_line_blocked_when_paused`
   - `draw_credit_blocked_when_paused`
   - `update_risk_parameters_blocked_when_paused`
   - `suspend_credit_line_blocked_when_paused`
   - `close_credit_line_blocked_when_paused`
   - `default_credit_line_blocked_when_paused`
   - `reinstate_credit_line_blocked_when_paused`
   - `set_liquidity_token_blocked_when_paused`
   - `set_liquidity_source_blocked_when_paused`
   - `set_rate_change_limits_blocked_when_paused`
   - `set_max_draw_amount_blocked_when_paused`

4. **repay_credit Exception** (critical safety feature)
   - `repay_credit_works_when_paused`
   - `repay_credit_full_repayment_when_paused`

5. **Read-Only Operations**
   - `get_credit_line_works_when_paused`
   - `is_protocol_paused_always_works`
   - `get_rate_change_limits_works_when_paused`
   - `get_max_draw_amount_works_when_paused`

6. **Idempotency**
   - `pause_when_already_paused_is_idempotent`
   - `unpause_when_already_unpaused_is_idempotent`

7. **Resume After Unpause**
   - `operations_resume_after_unpause`

**`tests/error_discriminants.rs`**
- Updated to include `ContractError::Paused = 18`
- Updated variant count to 18

### Documentation

**`docs/errors.md`**
- Added error code 18 (`Paused`) to the reference table
- Added security notes on the pause mechanism

**`docs/credit.md`**
- Added "Circuit Breaker (Emergency Pause)" section
- Documented pause control methods
- Listed blocked vs. active operations when paused
- Documented events
- Added threat model covering trust assumptions, failure modes, and design rationale

---

## Threat Model

### Trust Assumptions

- The admin key is secure and controlled by a trusted operator or multisig.
- The pause mechanism is a last-resort incident response tool, not a routine operational control.

### Failure Modes

| Failure Mode | Risk | Mitigation |
|--------------|------|------------|
| Admin key compromise | Attacker can pause the protocol indefinitely, causing denial-of-service | Use a multisig or hardware wallet for the admin key; monitor pause events |
| Accidental pause | Protocol operations are halted unintentionally | Implement operational procedures requiring confirmation before pausing; emit clear events |
| Pause during active draws | Users with pending draws cannot complete them | `repay_credit` remains active so users can reduce exposure; unpause as soon as safe |

### Design Rationale

1. **repay_credit exception**: Users must always be able to reduce their debt exposure, even during an emergency. This prevents a paused protocol from trapping user funds.

2. **Instance storage**: The pause flag is stored in instance storage (not persistent) for fast access. The guard function reads this flag on every mutating call, so minimizing latency is critical.

3. **Guard placement**: `assert_not_paused` is injected at the entry of every mutating operation, before any state reads or auth checks, to fail fast and minimize compute overhead.

4. **Read-only operations**: View functions (`get_credit_line`, `is_protocol_paused`, etc.) are never blocked, allowing monitoring and observability during an incident.

---

## Performance Impact

- **Overhead per guarded call**: 1 instance storage read (`is_paused`)
- **Storage cost**: 1 boolean in instance storage (negligible)
- **Compute cost**: Minimal — the guard is a single `if` check that short-circuits on the common path (unpaused)

---

## Testing

Run the circuit breaker test suite:

```bash
cargo test -p creditra-credit --test circuit_breaker
```

Run all tests including the new discriminant assertions:

```bash
cargo test -p creditra-credit
```

Check coverage (requires `cargo-llvm-cov`):

```bash
cargo llvm-cov --package creditra-credit --html
```

Expected coverage: ≥95% line coverage (circuit breaker paths are fully tested).

---

## Usage Example

```rust
// Admin pauses the protocol during an incident
client.set_protocol_paused(&true);

// All mutating operations now fail with ContractError::Paused
let result = client.draw_credit(&borrower, &500);
// → Err(ContractError::Paused)

// Users can still repay to reduce exposure
client.repay_credit(&borrower, &200);
// → Ok(())

// Admin unpauses after the incident is resolved
client.set_protocol_paused(&false);

// Operations resume normally
client.draw_credit(&borrower, &500);
// → Ok(())
```

---

## Security Notes

- The pause mechanism is **not** a substitute for proper access control, input validation, or secure coding practices.
- It is a **last-resort** incident response tool for halting the protocol when a critical vulnerability is discovered.
- The admin key must be secured with the same rigor as any other privileged key in the system (multisig, hardware wallet, etc.).
- Pause events should be monitored by off-chain systems to detect unauthorized or accidental pauses.

---

## Future Enhancements

Potential improvements for future iterations:

1. **Time-locked unpause**: Require a delay between pause and unpause to prevent rapid toggling.
2. **Pause reason**: Include a reason string in the pause event for incident tracking.
3. **Partial pause**: Allow pausing specific operations (e.g., only draws) while keeping others active.
4. **Multisig pause**: Require multiple admin signatures to pause the protocol.
5. **Auto-unpause**: Automatically unpause after a configured duration if no explicit unpause is called.

---

## Checklist

- [x] `ContractError::Paused` added with stable discriminant 18
- [x] `set_protocol_paused` and `is_protocol_paused` methods implemented
- [x] Guard injected into all mutating operations except `repay_credit`
- [x] `repay_credit` explicitly excluded from the guard
- [x] Events emitted on pause/unpause
- [x] 25 tests covering authorization, blocked operations, repay exception, and idempotency
- [x] Documentation updated in `docs/errors.md` and `docs/credit.md`
- [x] Threat model documented
- [x] No diagnostics or compile errors
