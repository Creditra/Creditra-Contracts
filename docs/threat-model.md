# Credit Contract Threat Model and Trust Assumptions

This document describes the security model for `contracts/credit`, including
actors, trust boundaries, assumptions, and expected failure modes.

## Scope

In-scope:

- `Credit` contract state transitions and authorization checks.
- Interactions with an external token contract during draw flows.
- Admin-operated configuration endpoints and operational controls.

Out-of-scope:

- Off-chain risk engine correctness.
- Wallet/device security of protocol operators and borrowers.
- Chain-level consensus failures.

## Security Objectives

1. Preserve correctness of borrower credit state (`credit_limit`, `utilized_amount`, `status`).
2. Prevent unauthorized administrative changes.
3. Prevent borrowers from drawing beyond allowed limits.
4. Ensure failed external token operations do not leave partial on-chain state changes.

## Actors and Roles

- **Admin (trusted operator)**  
  Can configure liquidity/token settings and perform privileged line management.
- **Borrower (partially trusted user)**  
  Can draw and repay only against their own credit line.
- **Indexer / Observer (untrusted reader)**  
  Reads state and events, cannot mutate contract state.
- **Token contract (external dependency)**  
  Invoked during draw path for reserve checks and token transfer.
- **Soroban runtime / ledger (trusted platform assumption)**  
  Provides transaction atomicity, auth primitives, and deterministic execution.

## Assets and Invariants

Critical assets:

- Contract admin authority.
- Borrower credit line records in persistent storage.
- Liquidity configuration (token contract address, reserve/source address).

Key invariants:

- `utilized_amount` never exceeds `credit_limit`.
- `utilized_amount` never drops below zero.
- Closed lines cannot be drawn or repaid.
- Only authorized roles perform admin actions.

## Trust Boundaries

### Boundary A: Contract caller -> Credit contract

- Borrower authorization is required on borrower-driven write paths.
- Admin authorization is required on admin-only paths.
- Any missing/incorrect authorization is treated as a hard failure.

### Boundary B: Credit contract -> External token contract

- Draw path depends on token contract behavior for `balance` and `transfer`.
- Assumption: token implements expected Soroban token semantics.
- If token call fails, transaction reverts atomically.

### Boundary C: Protocol operations -> On-chain config

- Admin key custody and operational discipline directly affect security.
- Misconfiguration (wrong token/source) can halt or misroute liquidity.

## Threats and Mitigations

### 1) Unauthorized admin actions

Threat: attacker attempts to set config or mutate credit lines without admin rights.  
Mitigation: admin-only paths require admin auth.  
Residual risk: admin private key compromise bypasses this control.

### 2) Unauthorized borrower actions

Threat: attacker repays/draws for another borrower or manipulates line lifecycle.  
Mitigation: borrower-driven methods require borrower auth and use borrower-keyed records.

### 3) Reentrancy and callback-style interference

Threat: external contract call causes reentrant execution and state corruption.  
Mitigation: explicit reentrancy guard on draw/repay critical paths (defense-in-depth).  
Assumption: standard token contracts do not callback into caller.

### 4) Malicious or non-standard token contract

Threat: configured token contract lies about balances, has unexpected behavior, or blocks transfers.  
Mitigation:

- token trust is explicit and administrative;
- failed token operations revert transaction atomically;
- operationally restrict token allowlist to vetted contracts.

Residual risk: if admin configures a malicious token, integrity/liveness can be degraded.

### 5) Admin key compromise

Threat: compromised admin key changes config, force-closes lines, or defaults borrowers.  
Impact: full protocol control loss for this deployment.  
Mitigations (operational):

- hardware-backed/multisig admin account;
- strict key rotation and break-glass procedure;
- on-chain monitoring/alerts for admin method calls.

Two-step admin rotation mitigation now exists on-chain:

- `propose_admin(new_admin, delay_seconds)` by current admin only;
- `accept_admin()` by proposed admin only;
- optional delay window enforced via stored acceptance timestamp;
- each phase emits an audit event for monitoring.

### 6) Operational and liveness risks

Threats:

- Wrong liquidity source address.
- Inadequate reserve balance.
- Stale operational processes (no monitoring).
- `freeze_draws` left active outside a declared maintenance window.

Mitigations:

- pre-deployment and post-change checklist;
- automated reserve health checks;
- incident runbooks and rollback plans for config mistakes;
- monitoring alert on `DrawsFrozenEvent { frozen: true }` outside declared windows and on freeze durations exceeding operational thresholds (e.g. > 1 hour).

### 7) Admin abuses global draw freeze

Threat: compromised or malicious admin calls `freeze_draws` to block all borrowers from drawing, causing protocol-wide liveness failure.

Impact: all `draw_credit` calls revert with `ContractError::DrawsFrozen` (15) until unfrozen. Repayments are unaffected — borrowers can always reduce their debt.

Mitigations:

- `is_draws_frozen` is publicly readable; off-chain monitoring can detect and alert on unexpected freezes immediately.
- `DrawsFrozenEvent` includes `actor` and `timestamp` fields for governance audit trails.
- Operational policy: require multi-party approval or a declared maintenance window before invoking `freeze_draws`.
- The flag is distinct from per-line `Suspended` — it does not mutate borrower state, so no remediation of individual lines is needed after unfreeze.

Residual risk: admin key compromise remains the root threat. Mitigated operationally by hardware-backed/multisig admin accounts and real-time monitoring.

## Immutable Upgrade Posture

Current posture: **assume immutable deployment unless a separate governance or migration process is explicitly introduced.**

Implications:

- Code defects require contract migration to a new deployment.
- Security hotfixes are operationally heavier than in upgradeable architectures.
- Documentation and runbooks must include migration procedures.

Recommended operational policy:

1. treat contract release as immutable,
2. maintain tested migration scripts,
3. announce and execute controlled migration if critical issues are found.

## Assumptions

1. Soroban authorization and transaction atomicity are correct.
2. Token contract follows expected token interface semantics.
3. Admin keys are protected by strong operational controls.
4. Off-chain risk decisions are sane and not adversarial.

## Failure Modes

- **Fail-closed:** unauthorized calls, invalid state transitions, or failing token calls revert.
- **Liveness degradation:** low reserve or token misbehavior can block draws.
- **Governance failure:** admin compromise can cause protocol-wide misuse.

## Security Review Notes

- Recommended before production: independent review focused on auth boundaries,
  external token trust assumptions, and admin key operational controls.
- Re-run threat model on each material contract behavior change.


### 7) Large single-transaction draw (compromised borrower key or buggy integrator)

Threat: A compromised borrower private key or a buggy integrator submits an
oversized single-transaction draw, draining a disproportionate share of the
liquidity reserve in one ledger.

Mitigation: Admin can configure a protocol-wide per-transaction draw cap via
`set_max_draw_amount`. Draws above the cap revert with
`ContractError::DrawExceedsMaxAmount` before any state or token transfer
occurs.

Residual risk:
- Cap is unset by default; operators must actively configure it for the
  protection to apply.
- A compromised admin key can raise or remove the cap.
- Multiple sequential draws just at or under the cap are not rate-limited
  by this control; separate rate-limiting or circuit-breaker logic would
  be needed to address that threat.

Operational recommendation: set `max_draw_amount` to a value reflecting the
largest legitimate single draw expected during normal protocol operation
immediately after deployment initialization.

### 8) Admin draw reversal misuse and token-accounting divergence

Threat: An operator uses `reverse_draw` outside intended operational procedure,
or users incorrectly assume reversal automatically claws tokens back from the
borrower.

Mitigation:
- `reverse_draw` is admin-authenticated and time-bounded to a 1-hour window from
  `original_ts`.
- Reversal is borrower-bound and draw-bound using stored audit records keyed by
  `(borrower, original_ts)`.
- Every reversal emits a dedicated `("credit", "draw_rev")` audit event with
  reason code, actor, and amount for monitoring and post-incident review.
- Contract behavior is explicit: reversal is accounting-only and does not call
  token transfer APIs.

Residual risk:
- Accounting reversal can reduce outstanding debt while drawn tokens remain with
  the borrower; this is a conscious emergency-correction tradeoff, not a
  clawback primitive.
- Compromised admin key can still misuse reversal controls within the allowed
  window.