// SPDX-License-Identifier: MIT

//! Property test: the borrower-ID encoding is a bijection with `Address` values.
//!
//! # Invariant
//!
//! The contract maintains two complementary persistent mappings:
//!
//! - `CreditLineIdByBorrower(Address) -> u32`  (borrower → numeric ID)
//! - `CreditLineBorrowerById(u32) -> Address`   (numeric ID → borrower)
//!
//! These must form a **bijection** — every borrower maps to exactly one ID,
//! every ID maps back to exactly one borrower, and the roundtrip is the
//! identity function.
//!
//! # Properties verified
//!
//! 1. **Right-inverse**: `get_borrower_by_id(get_id(addr)) == Some(addr)`
//! 2. **Left-inverse**: `get_id(get_borrower_by_id(id)) == Some(id)`
//! 3. **Injectivity**: after N registrations, all N IDs are distinct
//! 4. **Idempotency**: calling `ensure_credit_line_id` twice for the same
//!    borrower returns the same ID
//! 5. **Sequential IDs**: IDs assigned as `[0, n)` without gaps
//!
//! # Strategy
//!
//! Generate a count of borrowers (1–50), register each via
//! [`ensure_credit_line_id`], then verify every property above.
//!
//! # References
//!
//! - [`crate::storage::ensure_credit_line_id`]
//! - [`crate::storage::get_credit_line_id`]
//! - [`crate::storage::get_borrower_by_credit_line_id`]
//! - [`crate::storage::DataKey::CreditLineIdByBorrower`]
//! - [`crate::storage::DataKey::CreditLineBorrowerById`]
//! - Issue #583

use creditra_credit::test_helpers::{
    ensure_credit_line_id, get_borrower_by_credit_line_id, get_credit_line_id,
};
use proptest::prelude::*;
use proptest::test_runner::Config as ProptestConfig;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};
use std::collections::HashSet;

/// Strategy for the number of distinct borrowers to register.
fn registration_count() -> impl Strategy<Value = usize> {
    1_usize..=50_usize
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, .. ProptestConfig::default() })]

    /// Verify the borrower-ID mapping is a bijection for any set of borrowers.
    ///
    /// 1. Register `n` distinct addresses via `ensure_credit_line_id`.
    /// 2. Verify every address → id → address roundtrip.
    /// 3. Verify every id → address → id roundtrip.
    /// 4. Verify all `n` IDs are unique (HashSet size == n).
    /// 5. Verify IDs are sequential from 0 (sorted[ i ] == i).
    #[test]
    fn borrower_id_bijection(n in registration_count()) {
        let env = Env::default();
        let contract_id = env.register(creditra_credit::Credit, ());

        let mut addrs: Vec<Address> = Vec::with_capacity(n);
        let mut ids: Vec<u32> = Vec::with_capacity(n);

        // Register each borrower
        for _ in 0..n {
            let addr = Address::generate(&env);
            addrs.push(addr.clone());

            let id = env.as_contract(&contract_id, || {
                ensure_credit_line_id(&env, &addr)
            });
            ids.push(id);
        }

        // ── Right-inverse: get_borrower_by_id(ensure_id(addr)) == Some(addr) ──
        for (addr, &id) in addrs.iter().zip(ids.iter()) {
            let recovered: Option<Address> = env.as_contract(&contract_id, || {
                get_borrower_by_credit_line_id(&env, id)
            });
            prop_assert_eq!(
                &recovered,
                &Some(addr.clone()),
                "Right-inverse failed for id={}",
                id,
            );
        }

        // ── Left-inverse: get_id(get_borrower_by_id(id)) == Some(id) ─────────
        for &id in &ids {
            let recovered: Option<Address> = env.as_contract(&contract_id, || {
                get_borrower_by_credit_line_id(&env, id)
            });
            let restored_id: Option<u32> = env.as_contract(&contract_id, || {
                recovered
                    .as_ref()
                    .and_then(|addr| get_credit_line_id(&env, addr))
            });
            prop_assert_eq!(
                restored_id,
                Some(id),
                "Left-inverse failed for id={}",
                id,
            );
        }

        // ── Injectivity: all N IDs are unique ────────────────────────────────
        let unique_ids: HashSet<u32> = ids.iter().copied().collect();
        prop_assert_eq!(
            unique_ids.len(),
            n,
            "Expected {} unique IDs but got {}",
            n,
            unique_ids.len(),
        );

        // ── Sequential: IDs are [0, n) without gaps ──────────────────────────
        let mut sorted = ids.clone();
        sorted.sort();
        for (i, &id) in sorted.iter().enumerate() {
            prop_assert_eq!(
                id as usize, i,
                "Non-sequential ID at position {}: expected {} but got {}",
                i, i, id,
            );
        }
    }

    /// Verify that `ensure_credit_line_id` is idempotent — calling it twice
    /// with the same borrower returns the same ID.
    #[test]
    fn idempotent_same_borrower(n in registration_count()) {
        let env = Env::default();
        let contract_id = env.register(creditra_credit::Credit, ());

        let mut addrs: Vec<Address> = Vec::with_capacity(n);

        for _ in 0..n {
            let addr = Address::generate(&env);
            addrs.push(addr.clone());
        }

        for addr in &addrs {
            let id1: u32 = env.as_contract(&contract_id, || {
                ensure_credit_line_id(&env, addr)
            });

            let id2: u32 = env.as_contract(&contract_id, || {
                ensure_credit_line_id(&env, addr)
            });

            prop_assert_eq!(
                id1, id2,
                "Idempotency violated: same borrower got different IDs ({} vs {})",
                id1, id2,
            );
        }
    }

    /// Verify that the ID mapping roundtrips even when addresses share similar
    /// prefixes (generated sequentially, which mimics adversarial addresses
    /// with common prefixes).
    #[test]
    fn sequential_addresses_roundtrip(n in registration_count()) {
        let env = Env::default();
        let contract_id = env.register(creditra_credit::Credit, ());

        let mut ids: Vec<u32> = Vec::with_capacity(n);

        for _ in 0..n {
            let addr = Address::generate(&env);
            let id = env.as_contract(&contract_id, || {
                ensure_credit_line_id(&env, &addr)
            });
            ids.push(id);

            // Immediate roundtrip before registering more addresses
            let recovered: Option<Address> = env.as_contract(&contract_id, || {
                get_borrower_by_credit_line_id(&env, id)
            });
            prop_assert_eq!(
                recovered,
                Some(addr),
                "Immediate roundtrip failed for id={}",
                id,
            );
        }
    }
}

// ── Deterministic edge-case tests ──────────────────────────────────────────

#[cfg(test)]
mod edge_cases {
    use super::*;

    /// Single borrower: right-inverse and left-inverse hold trivially.
    #[test]
    fn single_borrower_roundtrip() {
        let env = Env::default();
        let contract_id = env.register(creditra_credit::Credit, ());
        let addr = Address::generate(&env);

        let id = env.as_contract(&contract_id, || {
            ensure_credit_line_id(&env, &addr)
        });

        // Right-inverse
        let recovered = env.as_contract(&contract_id, || {
            get_borrower_by_credit_line_id(&env, id)
        });
        assert_eq!(recovered, Some(addr));
    }

    /// Two borrowers get distinct IDs.
    #[test]
    fn two_borrowers_distinct_ids() {
        let env = Env::default();
        let contract_id = env.register(creditra_credit::Credit, ());
        let a = Address::generate(&env);
        let b = Address::generate(&env);

        let id_a = env.as_contract(&contract_id, || {
            ensure_credit_line_id(&env, &a)
        });
        let id_b = env.as_contract(&contract_id, || {
            ensure_credit_line_id(&env, &b)
        });

        assert_ne!(id_a, id_b, "Two borrowers must get distinct IDs");
        assert_eq!(id_a, 0, "First borrower must get ID 0");
        assert_eq!(id_b, 1, "Second borrower must get ID 1");
    }

    /// Idempotency: three calls to the same borrower returns the same ID.
    #[test]
    fn three_calls_idempotent() {
        let env = Env::default();
        let contract_id = env.register(creditra_credit::Credit, ());
        let addr = Address::generate(&env);

        let id1 = env.as_contract(&contract_id, || {
            ensure_credit_line_id(&env, &addr)
        });
        let id2 = env.as_contract(&contract_id, || {
            ensure_credit_line_id(&env, &addr)
        });
        let id3 = env.as_contract(&contract_id, || {
            ensure_credit_line_id(&env, &addr)
        });

        assert_eq!(id1, id2, "Second call must return same ID");
        assert_eq!(id2, id3, "Third call must return same ID");
    }

    /// Many borrowers: all roundtrips succeed and all IDs are unique.
    #[test]
    fn many_borrowers_all_unique() {
        let env = Env::default();
        let contract_id = env.register(creditra_credit::Credit, ());
        let count = 100;

        let mut ids = Vec::with_capacity(count);
        let mut addrs = Vec::with_capacity(count);

        for _ in 0..count {
            let addr = Address::generate(&env);
            addrs.push(addr.clone());
            let id = env.as_contract(&contract_id, || {
                ensure_credit_line_id(&env, &addr)
            });
            ids.push(id);
        }

        // All roundtrips succeed
        for (addr, &id) in addrs.iter().zip(ids.iter()) {
            let recovered = env.as_contract(&contract_id, || {
                get_borrower_by_credit_line_id(&env, id)
            });
            assert_eq!(recovered, Some(addr.clone()));

            let restored_id = env.as_contract(&contract_id, || {
                get_credit_line_id(&env, addr)
            });
            assert_eq!(restored_id, Some(id));
        }

        // All IDs are unique
        let unique: HashSet<u32> = ids.iter().copied().collect();
        assert_eq!(unique.len(), count);

        // Sequential from 0
        let mut sorted = ids.clone();
        sorted.sort();
        for (i, &id) in sorted.iter().enumerate() {
            assert_eq!(id as usize, i, "ID at position {} should be {}", i, i);
        }
    }
}
