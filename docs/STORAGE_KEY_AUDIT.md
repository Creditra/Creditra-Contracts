# Storage Key Audit & Upgrade Compatibility Reference

Authoritative inventory and specification for CosmWasm storage keys in `contracts/creditra-credit`.

## 1. Overview & Purpose

Changing key namespaces or serialization schemes can make deployed credit state unreadable across contract upgrades. This document inventories all 22 persisted storage key families, specifies their exact binary serialization formats, and documents collision-freedom guarantees and upgrade migration protocols.

## 2. Authoritative Key Inventory

| # | Namespace | Kind | Key Type | Value Type | Description |
|---|---|---|---|---|---|
| 1 | `config` | `Item` | `()` | `Config` | Singleton contract configuration containing administrative owner address. |
| 2 | `clc` | `Item` | `()` | `u64` | Monotonically increasing counter of all credit lines ever created. |
| 3 | `cl` | `Map` | `u64` | `CreditLine` | Credit line records keyed by stable numeric identifier. |
| 4 | `dcnt` | `Map` | `u64` | `u64` | Per-credit-line draw counter tracking total draws created on that line. |
| 5 | `dr` | `Map` | `(u64, u64)` | `Draw` | Draw records keyed by composite credit line and draw identifiers. |
| 6 | `dacnt` | `Map` | `(u64, u64)` | `u64` | Per-draw audit sequence counter. |
| 7 | `da` | `Map` | `(u64, u64, u64)` | `DrawAuditEntry` | Immutable append-only audit trail entries for draw actions. |
| 8 | `bid` | `Map` | `Addr` | `u64` | Mapping from borrower address to stable numeric credit line identifier. |
| 9 | `orc_qcfg` | `Item` | `()` | `OracleQuorumConfig` | Multi-oracle quorum configuration for redundancy median resolution. |
| 10 | `orc_prc` | `Item` | `()` | `OraclePriceRecord` | Stored quorum-resolved canonical price record. |
| 11 | `orc_lst` | `Item` | `()` | `Vec<Addr>` | Authorized oracle feed provider addresses. |
| 12 | `orc_w` | `Map` | `Addr` | `u32` | Individual oracle voting weight in basis points. |
| 13 | `orc_rpt` | `Map` | `Addr` | `OracleReportData` | Latest submitted price report data per oracle address. |
| 14 | `lfc` | `Item` | `()` | `LateFeeConfig` | Structured late-fee configuration (flat or APR surcharge). |
| 15 | `bct` | `Map` | `&Addr` | `Vec<String>` | Multi-collateral tokens currently posted by each borrower. |
| 16 | `cb` | `Map` | `(&Addr, &str)` | `Uint128` | Raw collateral balance keyed by borrower and token denomination. |
| 17 | `crw` | `Map` | `&str` | `u32` | Optional risk-weight overrides in basis points keyed by denomination. |
| 18 | `cta` | `Item` | `()` | `Vec<String>` | Admin-managed allowlist of accepted collateral token denominations. |
| 19 | `default_fee_share` | `Item` | `()` | `u32` | Default treasury fee share in basis points (0..=10_000). |
| 20 | `mkt_fee_share` | `Map` | `&str` | `u32` | Per-market treasury fee share override in basis points. |
| 21 | `treasury_bal` | `Map` | `&str` | `Uint128` | Per-market accumulated treasury balance held in contract. |
| 22 | `bounty_bal` | `Map` | `&str` | `Uint128` | Per-market accumulated bounty pool balance held in contract. |

## 3. Storage Key Encoding & Collision Resistance

### 3.1 CosmWasm Key Serialization Architecture

CosmWasm storage primitives provided by `cw_storage_plus` serialize keys according to the following invariants:

1. **`Item<T>` Keys:** Serialized directly as their exact UTF-8 ASCII bytes:
   ```text
   raw_key = namespace.as_bytes()
   ```
   All item namespaces in `creditra-credit` begin with lowercase ASCII characters (`'a'..='z'`), whose first byte is in the range `0x61..=0x7A`.

2. **`Map<K, V>` Keys:** Serialized with a 2-byte big-endian length prefix denoting the length of the namespace string, followed by the namespace bytes and serialized primary key:
   ```text
   raw_key = (namespace.len() as u16).to_be_bytes() || namespace.as_bytes() || primary_key_bytes
   ```
   Because all namespace strings in the contract have lengths $\le 255$ bytes, the first byte of `(namespace.len() as u16).to_be_bytes()` is **always `0x00`**.

### 3.2 Mathematical Proof of Collision Freedom

1. **Item vs Map Disjoint Domains:**
   - Any `Item` key begins with byte $\ge 0x61$.
   - Any `Map` key begins with byte $0x00$.
   - Because $0x00 \neq 0x61..=0x7A$, an `Item` key and a `Map` key can never collide under any circumstances, even if they share the identical namespace string.

2. **Map vs Map Disjoint Prefixes:**
   - For any two distinct `Map` namespaces $N_1$ and $N_2$ where $N_1 \neq N_2$:
     - If $|N_1| \neq |N_2|$, the 2-byte length prefixes differ.
     - If $|N_1| = |N_2|$, the length prefixes are identical but the subsequent namespace bytes differ at some offset.
     - Substring prefixes (such as `"cl"` and `"clc"`) cannot overlap because their length headers differ (`[0x00, 0x02]` vs `[0x00, 0x03]`).

### 3.3 Automated CI & Runtime Validation

To prevent future PRs from inadvertently introducing colliding keys, `contracts/creditra-credit/src/key.rs` provides programmatic guards:
- `validate_storage_key_catalog() -> Result<(), &'static str>`: Exhaustively iterates all pairs in `ALL_STORAGE_KEY_FAMILIES` to ensure zero duplicate namespaces and zero byte-prefix collisions.
- `check_new_namespace_collision(namespace: &str, storage_type: StorageType) -> Result<(), &'static str>`: Validates any prospective new key against the entire existing catalog before addition.

## 4. Upgrade Compatibility & Migration Protocol

### 4.1 Empty State Migration
When migrating a contract with no active loans or draws:
- Singleton config and counters remain intact.
- Monotonic counters default to zero.
- Subsequent credit line creation and draw execution proceed without error.

### 4.2 Populated State Migration
When migrating a contract with active credit lines, open draws, audit logs, and collateral deposits:
- Raw byte fixtures verify 100% data fidelity with zero serialization mismatch.
- Health factor calculations, proof of reserve queries, and draw audit trails reflect existing data without mutation.
- Post-migration transactions (e.g. `RepayDraw`, `CreateDraw`) link seamlessly to pre-migration records.
