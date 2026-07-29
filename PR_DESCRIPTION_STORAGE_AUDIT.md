# PR: Storage Key and TTL Audit

## Overview

This PR completes a comprehensive storage audit for the Creditra credit contract, verifying that all storage keys use the correct Soroban storage types (instance, persistent, or temporary) and documenting TTL implications for production deployments.

## Changes

### Documentation Updates

1. **docs/credit.md** — Added comprehensive storage key appendix:
   - Storage type definitions and TTL behavior
   - Complete table of all instance storage keys with TTL notes
   - Complete table of all persistent storage keys with TTL notes
   - Audit findings summary with verification results
   - Security notes on TTL management and failure modes

2. **Code Comments** — Added storage documentation to all modules with storage write operations:
   - `storage.rs`: Reentrancy guard, pause flag, borrower blocking
   - `freeze.rs`: DrawsFrozen flag
   - `risk.rs`: Rate formula and rate change config
   - `lifecycle.rs`: Borrower credit lines and liquidation settlement markers
   - `auth.rs`: Admin address

### Storage Type Verification

All storage types have been verified as correct:

| Component | Storage Type | Status |
|-----------|--------------|--------|
| Admin address | Instance | ✅ Correct |
| Proposed admin/proposed_at | Instance | ✅ Correct |
| LiquidityToken | Instance | ✅ Correct |
| LiquiditySource | Instance | ✅ Correct |
| Reentrancy flag | Instance | ✅ Correct |
| Rate config (rate_cfg) | Instance | ✅ Correct |
| Rate formula config | Instance | ✅ Correct |
| Pause flag | Instance | ✅ Correct |
| MaxDrawAmount | Instance | ✅ Correct |
| DrawsFrozen | Instance | ✅ Correct |
| SchemaVersion | Instance | ✅ Correct |
| Borrower credit lines | Persistent | ✅ Correct |
| BlockedBorrower | Persistent | ✅ Correct |
| Liquidation settlement markers | Persistent | ✅ Correct |

## Key Findings

### 1. No Borrower Data on Instance Storage ✅

Verified that per-borrower data correctly uses persistent storage, avoiding the shared TTL pitfall where one borrower's activity could affect another's data availability.

### 2. Instance TTL is Critical ⚠️

All global configuration shares one TTL. If the instance is archived:
- Admin cannot be retrieved → all admin operations fail
- Liquidity config is lost → draws/repays may fail
- Reentrancy guard defaults to `false` → no reentrancy protection
- All protocol flags reset to defaults

**Recommendation**: Production deployments must implement TTL extension via `env.storage().instance().extend_ttl()` in frequently-called functions or a dedicated admin function.

### 3. Persistent TTL Per Borrower ✅

Each borrower's credit line has independent TTL. If a borrower's entry TTL expires:
- That borrower's credit line data is lost
- Other borrowers are unaffected
- The borrower would need to re-establish their credit line

**Recommendation**: Extend TTL on credit line access or via a keeper service.

### 4. Reentrancy Guard Semantics ℹ️

While stored in instance storage, the guard is functionally temporary (set on entry, cleared on all exits). This is safe but relies on correct implementation at all exit paths. Could optionally move to temporary storage for cleaner semantics.

## Security Notes

### Trust Boundaries

- **Instance storage**: Contains all admin-controlled configuration. Compromise of the admin key allows modification of all instance-stored values.
- **Persistent storage**: Contains borrower-specific data protected by different authorization rules (borrower auth for draws/repays, admin auth for lifecycle changes).

### Failure Modes

| Failure Mode | Impact | Mitigation |
|--------------|--------|------------|
| Instance TTL expires | Contract loses admin, config, and all protocol settings | Implement regular TTL extension |
| Borrower persistent TTL expires | That borrower's credit line data is lost | Extend TTL on access |
| Reentrancy guard not cleared | Contract becomes permanently locked | Verify all exit paths clear guard |

## Testing

Tests require a Rust/cargo environment. Before merging, run:

```bash
# Run all tests
cargo test -p creditra-credit

# Verify 95% line coverage
cargo llvm-cov --workspace --all-targets --fail-under-lines 95
```

## Checklist

- [x] Storage types verified for all keys
- [x] TTL implications documented
- [x] Code comments added at write sites
- [x] docs/credit.md updated with storage appendix
- [ ] Tests pass (`cargo test -p creditra-credit`)
- [ ] 95% line coverage verified (`cargo llvm-cov`)

## Related Issues

This PR addresses the storage audit requirements from the project TODO.