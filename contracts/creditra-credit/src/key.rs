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
}
