# Storage Layout — Creditra Contracts
**Issue:** #594  
**Branch:** task/storage-tier-matrix  
**Campaign:** GrantFox / Stellar Wave

---

## Overview

Soroban smart contracts have three storage tiers, each with different
lifetime, cost, and eviction characteristics. This document tabulates
every data key used across the Creditra contract suite and maps it to
the correct tier with rationale.

---

## Storage Tier Reference

| Tier | Soroban API | Ledger Lifetime | Eviction | Cost | Best For |
|------|-------------|----------------|----------|------|----------|
| **Instance** | `env.storage().instance()` | Tied to contract instance TTL | Never (while instance lives) | Medium | Small, always-needed contract state |
| **Persistent** | `env.storage().persistent()` | Independent TTL; must be bumped | Yes (if TTL expires) | Higher | Long-lived user/protocol data |
| **Temporary** | `env.storage().temporary()` | Short TTL (auto-expires) | Yes (automatic) | Lowest | Nonces, session data, short-lived flags |

---

## Storage Key Matrix

### 🏦 Creditra Core Contract

| Data Key | Type | Tier | TTL Policy | Rationale |
|----------|------|------|------------|-----------|
| `Admin` | `Address` | **Instance** | Contract instance TTL | Single admin address; always needed when contract is invoked |
| `Initialized` | `bool` | **Instance** | Contract instance TTL | One-time init flag; tiny and always relevant |
| `TotalSupply` | `i128` | **Instance** | Contract instance TTL | Aggregate protocol metric; read on almost every call |
| `ProtocolFee` | `u32` | **Instance** | Contract instance TTL | Global fee config; rarely changes, always needed |
| `Paused` | `bool` | **Instance** | Contract instance TTL | Circuit-breaker flag; must always be accessible |

---

### 👤 User / Account Data

| Data Key | Type | Tier | TTL Policy | Rationale |
|----------|------|------|------------|-----------|
| `Balance(Address)` | `i128` | **Persistent** | Bump on every read/write | User balances outlive any single session; must survive eviction |
| `CreditScore(Address)` | `u32` | **Persistent** | Bump on read | Credit scores are long-lived user attributes |
| `UserMetadata(Address)` | `Bytes` | **Persistent** | Bump on write | KYC / profile data; must not be lost |
| `Allowance(Address, Address)` | `i128` | **Persistent** | Bump on approval | Standard ERC-20-style allowance; needs to persist |
| `Nonce(Address)` | `u64` | **Temporary** | Short TTL (auto-expires) | Replay protection; only valid within a short window |

---

### 🏛️ Loan / Credit Facility

| Data Key | Type | Tier | TTL Policy | Rationale |
|----------|------|------|------------|-----------|
| `Loan(u64)` | `LoanState` | **Persistent** | Bump on state change | Active loans must persist for their full term |
| `LoanCount` | `u64` | **Instance** | Contract instance TTL | Monotonic counter; always needed for new loan IDs |
| `RepaymentSchedule(u64)` | `Vec<u64>` | **Persistent** | Bump on creation | Schedule must outlive the loan term |
| `DefaultFlag(u64)` | `bool` | **Temporary** | Short TTL | Temporary flag set during liquidation window; auto-expires |
| `AuctionState(u64)` | `AuctionData` | **Temporary** | TTL = auction duration | Auction data only valid during bidding window |

---

### 🌉 Gateway / Bridge Contract

| Data Key | Type | Tier | TTL Policy | Rationale |
|----------|------|------|------------|-----------|
| `BridgeAdmin` | `Address` | **Instance** | Contract instance TTL | Always needed for auth checks |
| `SupportedAsset(Address)` | `bool` | **Persistent** | Bump on update | Asset whitelist is long-lived protocol config |
| `PendingTransfer(Bytes32)` | `TransferState` | **Persistent** | Bump on status change | Cross-chain transfers may take days to settle |
| `ProcessedHash(Bytes32)` | `bool` | **Temporary** | TTL = finality window | Replay guard; only needed during finality window |
| `BridgeFee` | `u32` | **Instance** | Contract instance TTL | Global config; always needed |

---

## TTL Bump Policy

```rust
// Recommended TTL constants (in ledgers; 1 ledger ≈ 5 seconds)
const PERSISTENT_BUMP_AMOUNT: u32 = 518_400;  // ~30 days
const PERSISTENT_THRESHOLD:   u32 = 259_200;  // bump when < 15 days remain

const INSTANCE_BUMP_AMOUNT:   u32 = 34_560;   // ~2 days
const INSTANCE_THRESHOLD:     u32 = 17_280;   // bump when < 1 day remains

const TEMPORARY_TTL:          u32 = 1_440;    // ~2 hours (nonces, flags)
const AUCTION_TTL:            u32 = 17_280;   // ~1 day (auction state)
```

Usage pattern:
```rust
// Bump persistent entry on every meaningful access
env.storage().persistent().bump(
    &DataKey::Balance(user.clone()),
    PERSISTENT_THRESHOLD,
    PERSISTENT_BUMP_AMOUNT,
);

// Bump instance storage once per transaction
env.storage().instance().bump(INSTANCE_THRESHOLD, INSTANCE_BUMP_AMOUNT);
```

---

## Decision Rules (when to pick each tier)
---

## Security Considerations

- **Persistent keys** that are not bumped will be evicted by the network.
  Always bump on read for user-facing data.
- **Temporary keys** must never store value that needs to survive restores.
  Do not use for balances or loan state.
- **Instance storage** is shared across all storage entries in the instance;
  keep it small (< ~1 KB total) to avoid high ledger fees.
- Access control must be enforced **before** any storage write — never
  write first and validate later.

---

## API Changes

No new public entry-points are introduced by this change.
The document formalises the storage model already implemented in the contracts.
If future PRs add new `DataKey` variants, they **must** update this matrix.

---

## References

- [Soroban Storage Docs](https://developers.stellar.org/docs/learn/smart-contract-internals/state-archival)
- [State Archival & TTL](https://developers.stellar.org/docs/learn/smart-contract-internals/state-archival#time-to-live-ttl)
- Issue: [#594 — Document storage tier matrix per data key](../../issues/594)
