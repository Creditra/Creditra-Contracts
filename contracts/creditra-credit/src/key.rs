//! # Borrower Key Encoding
//!
//! Deterministic, collision-free storage key generation for borrower addresses.
//!
//! This module provides a type-safe wrapper around borrower address serialization
//! for use as storage keys in `cw_storage_plus::Map`. All functions produce
//! keys that are:
//!
//! - **Deterministic:** the same borrower address always produces the same key,
//! - **Collision-free:** different borrower addresses always produce different keys,
//! - **Stable:** the encoding does not change across contract invocations or upgrades.
//!
//! ## Design
//!
//! CosmWasm `Addr` values have a canonical bech32 string representation. Each
//! valid Cosmos address maps to exactly one bech32 string, and the mapping is
//! bijective. Therefore, serializing the canonical bytes of an `Addr` yields a
//! key that is deterministic, collision-free, and stable by construction.
//!
//! The [`BorrowerKey`] struct wraps the serialized bytes and provides
//! convenience constructors and accessors used by the storage layer.

use cosmwasm_std::Addr;

/// A deterministic, collision-free storage key derived from a borrower address.
///
/// Internally stores the canonical bech32 address bytes. The encoding is
/// stable, bijective, and requires no hashing — the address itself is the key.
///
/// # Examples
///
/// ```
/// use creditra_credit::key::BorrowerKey;
/// use cosmwasm_std::Addr;
///
/// let addr = Addr::unchecked("cosmos1qyqszqgpqyqszqgpqyqszqgpqyqszqgpjnp7du");
/// let key = BorrowerKey::from_address(&addr);
/// assert_eq!(key.as_bytes(), addr.as_bytes());
/// ```
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BorrowerKey {
    key_bytes: Vec<u8>,
}

impl BorrowerKey {
    /// Create a `BorrowerKey` from a borrower address.
    ///
    /// The key is the canonical bech32 byte representation of the address.
    pub fn from_address(addr: &Addr) -> Self {
        Self {
            key_bytes: addr.as_bytes().to_vec(),
        }
    }

    /// Return the raw key bytes suitable for use as a `cw_storage_plus::Map` key.
    pub fn as_bytes(&self) -> &[u8] {
        &self.key_bytes
    }

    /// Return the length of the key in bytes.
    pub fn len(&self) -> usize {
        self.key_bytes.len()
    }

    /// Return `true` if the key is non-empty.
    pub fn is_empty(&self) -> bool {
        self.key_bytes.is_empty()
    }
}

impl AsRef<[u8]> for BorrowerKey {
    fn as_ref(&self) -> &[u8] {
        &self.key_bytes
    }
}

/// Produce a deterministic, collision-free storage key for a borrower address.
///
/// Returns the canonical bech32 bytes of the address. This function is
/// equivalent to `BorrowerKey::from_address(addr).as_bytes().to_vec()`.
///
/// # Stability Guarantee
///
/// The returned bytes are derived from the `Addr::as_bytes()` representation,
/// which is the UTF-8 encoded bech32 address string. This is stable across
/// all CosmWasm versions and contract upgrades.
pub fn borrower_key_bytes(addr: &Addr) -> Vec<u8> {
    addr.as_bytes().to_vec()
}

/// The fundamental storage kind used by cw-storage-plus.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StorageType {
    /// A single value stored at a fixed key.
    Item,
    /// Key-value mapping with length-prefixed namespace and typed keys.
    Map,
}

/// Metadata defining a canonical storage key family for upgrade audit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageKeyFamily {
    /// Canonical namespace string used in Item::new or Map::new.
    pub namespace: &'static str,
    /// Storage kind (Item or Map).
    pub storage_type: StorageType,
    /// Name of the key type used to index entries.
    pub key_type_name: &'static str,
    /// Name of the value type stored at this key.
    pub value_type_name: &'static str,
    /// Human-readable explanation of what this key family represents.
    pub description: &'static str,
}

impl StorageKeyFamily {
    /// Compute the raw storage key prefix for this family in cw-storage-plus.
    pub fn raw_prefix(&self) -> Vec<u8> {
        match self.storage_type {
            StorageType::Item => self.namespace.as_bytes().to_vec(),
            StorageType::Map => {
                let len = self.namespace.len() as u16;
                let mut prefix = Vec::with_capacity(2 + self.namespace.len());
                prefix.extend_from_slice(&len.to_be_bytes());
                prefix.extend_from_slice(self.namespace.as_bytes());
                prefix
            }
        }
    }

    /// Check if this storage key family can collide with another family.
    pub fn can_collide_with(&self, other: &StorageKeyFamily) -> bool {
        if self.namespace == other.namespace {
            return true;
        }
        let p1 = self.raw_prefix();
        let p2 = other.raw_prefix();
        p1 == p2
    }
}

/// Authoritative inventory of all persisted storage key families in the credit contract.
pub const ALL_STORAGE_KEY_FAMILIES: &[StorageKeyFamily] = &[
    StorageKeyFamily {
        namespace: "config",
        storage_type: StorageType::Item,
        key_type_name: "()",
        value_type_name: "Config",
        description: "Singleton contract configuration containing administrative owner address",
    },
    StorageKeyFamily {
        namespace: "clc",
        storage_type: StorageType::Item,
        key_type_name: "()",
        value_type_name: "u64",
        description: "Monotonically increasing counter of all credit lines ever created",
    },
    StorageKeyFamily {
        namespace: "cl",
        storage_type: StorageType::Map,
        key_type_name: "u64",
        value_type_name: "CreditLine",
        description: "Credit line records keyed by stable numeric identifier",
    },
    StorageKeyFamily {
        namespace: "dcnt",
        storage_type: StorageType::Map,
        key_type_name: "u64",
        value_type_name: "u64",
        description: "Per-credit-line draw counter tracking total draws created on that line",
    },
    StorageKeyFamily {
        namespace: "dr",
        storage_type: StorageType::Map,
        key_type_name: "(u64, u64)",
        value_type_name: "Draw",
        description: "Draw records keyed by composite credit line and draw identifiers",
    },
    StorageKeyFamily {
        namespace: "dacnt",
        storage_type: StorageType::Map,
        key_type_name: "(u64, u64)",
        value_type_name: "u64",
        description: "Per-draw audit sequence counter",
    },
    StorageKeyFamily {
        namespace: "da",
        storage_type: StorageType::Map,
        key_type_name: "(u64, u64, u64)",
        value_type_name: "DrawAuditEntry",
        description: "Immutable append-only audit trail entries for draw actions",
    },
    StorageKeyFamily {
        namespace: "bid",
        storage_type: StorageType::Map,
        key_type_name: "Addr",
        value_type_name: "u64",
        description: "Mapping from borrower canonical address to stable credit line identifier",
    },
    StorageKeyFamily {
        namespace: "orc_qcfg",
        storage_type: StorageType::Item,
        key_type_name: "()",
        value_type_name: "OracleQuorumConfig",
        description: "Multi-oracle quorum configuration for price resolution",
    },
    StorageKeyFamily {
        namespace: "orc_prc",
        storage_type: StorageType::Item,
        key_type_name: "()",
        value_type_name: "OraclePriceRecord",
        description: "Stored canonical quorum-resolved price and ledger timestamp",
    },
    StorageKeyFamily {
        namespace: "orc_lst",
        storage_type: StorageType::Item,
        key_type_name: "()",
        value_type_name: "Vec<Addr>",
        description: "List of authorized oracle feed provider addresses",
    },
    StorageKeyFamily {
        namespace: "orc_w",
        storage_type: StorageType::Map,
        key_type_name: "Addr",
        value_type_name: "u32",
        description: "Individual oracle voting weight in basis points",
    },
    StorageKeyFamily {
        namespace: "orc_rpt",
        storage_type: StorageType::Map,
        key_type_name: "Addr",
        value_type_name: "OracleReportData",
        description: "Latest submitted price report data per oracle",
    },
    StorageKeyFamily {
        namespace: "lfc",
        storage_type: StorageType::Item,
        key_type_name: "()",
        value_type_name: "LateFeeConfig",
        description: "Structured late fee configuration (Flat or Apr)",
    },
    StorageKeyFamily {
        namespace: "bct",
        storage_type: StorageType::Map,
        key_type_name: "&Addr",
        value_type_name: "Vec<String>",
        description: "List of collateral token denominations posted by each borrower",
    },
    StorageKeyFamily {
        namespace: "cb",
        storage_type: StorageType::Map,
        key_type_name: "(&Addr, &str)",
        value_type_name: "Uint128",
        description: "Posted collateral balance keyed by borrower address and token denomination",
    },
    StorageKeyFamily {
        namespace: "crw",
        storage_type: StorageType::Map,
        key_type_name: "&str",
        value_type_name: "u32",
        description: "Collateral risk weight in basis points keyed by token denomination",
    },
    StorageKeyFamily {
        namespace: "cta",
        storage_type: StorageType::Item,
        key_type_name: "()",
        value_type_name: "Vec<String>",
        description: "Allowlist of accepted collateral token denominations",
    },
    StorageKeyFamily {
        namespace: "default_fee_share",
        storage_type: StorageType::Item,
        key_type_name: "()",
        value_type_name: "u32",
        description: "Default treasury fee share in basis points",
    },
    StorageKeyFamily {
        namespace: "mkt_fee_share",
        storage_type: StorageType::Map,
        key_type_name: "&str",
        value_type_name: "u32",
        description: "Per-market treasury fee share override keyed by market denomination",
    },
    StorageKeyFamily {
        namespace: "treasury_bal",
        storage_type: StorageType::Map,
        key_type_name: "&str",
        value_type_name: "Uint128",
        description: "Accumulated treasury fee balance keyed by market denomination",
    },
    StorageKeyFamily {
        namespace: "bounty_bal",
        storage_type: StorageType::Map,
        key_type_name: "&str",
        value_type_name: "Uint128",
        description: "Accumulated bounty fee share balance keyed by market denomination",
    },
];

/// Validate that the entire storage key catalog is free of collisions and duplicate namespaces.
pub fn validate_storage_key_catalog() -> Result<(), &'static str> {
    for (i, f1) in ALL_STORAGE_KEY_FAMILIES.iter().enumerate() {
        for f2 in ALL_STORAGE_KEY_FAMILIES.iter().skip(i + 1) {
            if f1.namespace == f2.namespace {
                return Err("Duplicate storage namespace detected");
            }
            if f1.can_collide_with(f2) {
                return Err("Storage key prefix collision detected");
            }
        }
    }
    Ok(())
}

/// Check whether a prospective new namespace would collide with any existing key family.
pub fn check_new_namespace_collision(
    namespace: &str,
    storage_type: StorageType,
) -> Result<(), &'static str> {
    if namespace.is_empty() {
        return Err("Storage namespace cannot be empty");
    }
    let prospective_prefix = match storage_type {
        StorageType::Item => namespace.as_bytes().to_vec(),
        StorageType::Map => {
            let len = namespace.len() as u16;
            let mut prefix = Vec::with_capacity(2 + namespace.len());
            prefix.extend_from_slice(&len.to_be_bytes());
            prefix.extend_from_slice(namespace.as_bytes());
            prefix
        }
    };

    for family in ALL_STORAGE_KEY_FAMILIES.iter() {
        if family.namespace == namespace {
            return Err("Namespace matches an existing storage family");
        }
        let existing_prefix = family.raw_prefix();
        if prospective_prefix == existing_prefix {
            return Err("Raw storage key prefix matches an existing storage family");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::Addr;

    // ── Helpers ──────────────────────────────────────────────────────────

    fn make_addr(s: &str) -> Addr {
        Addr::unchecked(s)
    }

    // ── BorrowerKey tests ────────────────────────────────────────────────

    #[test]
    fn borrower_key_is_deterministic() {
        let addr = make_addr("cosmos1qyqszqgpqyqszqgpqyqszqgpqyqszqgpjnp7du");

        let key1 = BorrowerKey::from_address(&addr);
        let key2 = BorrowerKey::from_address(&addr);
        let key3 = BorrowerKey::from_address(&addr);

        assert_eq!(key1, key2);
        assert_eq!(key2, key3);
        assert_eq!(key1.as_bytes(), key2.as_bytes());
    }

    #[test]
    fn borrower_key_is_collision_free() {
        let addr_a = make_addr("cosmos1qyqszqgpqyqszqgpqyqszqgpqyqszqgpjnp7du");
        let addr_b = make_addr("cosmos1xv9tklw7d7se6rjketkxvqpn9h2v9pxm2sfvpn");

        let key_a = BorrowerKey::from_address(&addr_a);
        let key_b = BorrowerKey::from_address(&addr_b);

        assert_ne!(key_a, key_b);
        assert_ne!(key_a.as_bytes(), key_b.as_bytes());
    }

    #[test]
    fn borrower_key_is_non_empty() {
        let addr = make_addr("cosmos1qyqszqgpqyqszqgpqyqszqgpqyqszqgpjnp7du");
        let key = BorrowerKey::from_address(&addr);

        assert!(!key.is_empty());
    }

    #[test]
    fn borrower_key_bytes_is_deterministic() {
        let addr = make_addr("cosmos1qyqszqgpqyqszqgpqyqszqgpqyqszqgpjnp7du");

        let bytes1 = borrower_key_bytes(&addr);
        let bytes2 = borrower_key_bytes(&addr);
        let bytes3 = borrower_key_bytes(&addr);

        assert_eq!(bytes1, bytes2);
        assert_eq!(bytes2, bytes3);
    }

    #[test]
    fn borrower_key_bytes_is_collision_free() {
        let bytes_a =
            borrower_key_bytes(&make_addr("cosmos1qyqszqgpqyqszqgpqyqszqgpqyqszqgpjnp7du"));
        let bytes_b =
            borrower_key_bytes(&make_addr("cosmos1xv9tklw7d7se6rjketkxvqpn9h2v9pxm2sfvpn"));

        assert_ne!(bytes_a, bytes_b);
    }

    #[test]
    fn borrower_key_as_ref_works() {
        let addr = make_addr("cosmos1qyqszqgpqyqszqgpqyqszqgpqyqszqgpjnp7du");
        let key = BorrowerKey::from_address(&addr);
        let r: &[u8] = key.as_ref();
        assert_eq!(r, addr.as_bytes());
    }

    #[test]
    fn borrower_key_clone_is_equal() {
        let addr = make_addr("cosmos1test");
        let key = BorrowerKey::from_address(&addr);
        let cloned = key.clone();
        assert_eq!(key, cloned);
        assert_eq!(key.as_bytes(), cloned.as_bytes());
    }

    #[test]
    fn borrower_key_debug_format_contains_bytes() {
        let addr = make_addr("cosmos1test");
        let key = BorrowerKey::from_address(&addr);
        let debug_str = format!("{:?}", key);
        assert!(debug_str.contains("BorrowerKey"));
    }

    #[test]
    fn storage_catalog_validates_without_collisions() {
        assert!(validate_storage_key_catalog().is_ok());
    }

    #[test]
    fn storage_catalog_contains_expected_count() {
        assert_eq!(ALL_STORAGE_KEY_FAMILIES.len(), 22);
    }

    #[test]
    fn storage_catalog_namespaces_are_all_unique() {
        let mut seen = std::collections::HashSet::new();
        for family in ALL_STORAGE_KEY_FAMILIES {
            assert!(seen.insert(family.namespace));
        }
    }

    #[test]
    fn item_and_map_prefixes_are_strictly_disjoint() {
        for family in ALL_STORAGE_KEY_FAMILIES {
            let prefix = family.raw_prefix();
            match family.storage_type {
                StorageType::Item => {
                    assert!(!prefix.is_empty());
                    assert!(prefix[0] >= 0x20);
                }
                StorageType::Map => {
                    assert!(prefix.len() >= 3);
                    assert_eq!(prefix[0], 0x00);
                }
            }
        }
    }

    #[test]
    fn check_new_namespace_collision_rejects_duplicates() {
        let err = check_new_namespace_collision("config", StorageType::Item);
        assert!(err.is_err());
        assert_eq!(
            err.unwrap_err(),
            "Namespace matches an existing storage family"
        );

        let err_map = check_new_namespace_collision("cl", StorageType::Map);
        assert!(err_map.is_err());
        assert_eq!(
            err_map.unwrap_err(),
            "Namespace matches an existing storage family"
        );
    }

    #[test]
    fn check_new_namespace_collision_rejects_empty() {
        let err = check_new_namespace_collision("", StorageType::Item);
        assert!(err.is_err());
        assert_eq!(err.unwrap_err(), "Storage namespace cannot be empty");
    }

    #[test]
    fn check_new_namespace_collision_accepts_valid_unique_namespace() {
        let ok = check_new_namespace_collision("new_unique_namespace", StorageType::Item);
        assert!(ok.is_ok());

        let ok_map = check_new_namespace_collision("new_unique_map", StorageType::Map);
        assert!(ok_map.is_ok());
    }
}
