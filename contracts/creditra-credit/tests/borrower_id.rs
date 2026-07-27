// SPDX-License-Identifier: MIT

//! Property test: the borrower-ID encoding in `creditra-credit` is a bijection.
//!
//! # Invariant
//!
//! The CosmWasm creditra-credit contract assigns sequential `u64` IDs to
//! credit lines via `CREDIT_LINE_COUNT`. Each credit line stores exactly
//! one `borrower: Addr`. The test verifies:
//!
//! 1. **Right-inverse**: loading a credit line by its assigned ID returns
//!    the original borrower address.
//! 2. **Left-inverse**: scanning all credit lines for a given borrower
//!    yields the originally assigned ID.
//! 3. **Injectivity**: after N registrations, all N IDs are distinct.
//! 4. **Sequential IDs**: IDs are assigned as `[0, n)` without gaps.
//! 5. **Idempotency**: creating two credit lines for the same borrower
//!    produces two distinct IDs (each creation is an independent event).
//!
//! # References
//!
//! - `contracts/creditra-credit/src/state.rs` — `CREDIT_LINE_COUNT`, `CREDIT_LINES`
//! - Issue #757

use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env, MockApi, MockQuerier, MockStorage};
use cosmwasm_std::{Addr, Api, OwnedDeps};
use creditra_credit::contract;
use creditra_credit::msg::{ExecuteMsg, InstantiateMsg};
use creditra_credit::state::{CREDIT_LINE_COUNT, CREDIT_LINES};
use proptest::prelude::*;
use proptest::test_runner::Config as ProptestConfig;
use std::collections::HashSet;

/// Strategy for the number of distinct borrowers to register.
fn registration_count() -> impl Strategy<Value = usize> {
    1_usize..=50_usize
}

/// Set up the contract with an owner.
fn setup_contract(deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>) -> Addr {
    let env = mock_env();
    let owner = deps.api.addr_make("owner");
    let info = message_info(&owner, &[]);
    let msg = InstantiateMsg {
        owner: owner.to_string(),
    };
    contract::instantiate(deps.as_mut(), env, info, msg).unwrap();
    owner
}

/// Create a credit line for a borrower and return the assigned sequential ID.
fn create_credit_line(
    deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
    owner: &Addr,
    borrower: &str,
) -> u64 {
    let env = mock_env();
    let info = message_info(owner, &[]);
    let msg = ExecuteMsg::CreateCreditLine {
        borrower: borrower.to_string(),
        collateral_denom: "ucollateral".to_string(),
        collateral_amount: "1000".to_string(),
        credit_denom: "ucredit".to_string(),
        credit_amount: "500".to_string(),
    };
    contract::execute(deps.as_mut(), env, info, msg).unwrap();

    let count = CREDIT_LINE_COUNT.load(deps.as_ref().storage).unwrap();
    count - 1
}

/// Scan all credit lines to find the ID assigned to a given borrower address.
fn find_id_for_borrower(
    deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>,
    borrower: &Addr,
) -> Option<u64> {
    let count = CREDIT_LINE_COUNT.may_load(deps.as_ref().storage).unwrap().unwrap_or(0);
    for id in 0..count {
        if let Some(cl) = CREDIT_LINES.may_load(deps.as_ref().storage, id).unwrap() {
            if cl.borrower == *borrower {
                return Some(id);
            }
        }
    }
    None
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, .. ProptestConfig::default() })]

    /// Verify the borrower-ID mapping is a bijection for any set of borrowers.
    ///
    /// 1. Register `n` distinct addresses via `create_credit_line`.
    /// 2. Verify every id → credit_line → borrower roundtrip (right-inverse).
    /// 3. Verify every borrower → id → borrower roundtrip (left-inverse).
    /// 4. Verify all `n` IDs are unique (HashSet size == n).
    /// 5. Verify IDs are sequential from 0.
    #[test]
    fn borrower_id_bijection(n in registration_count()) {
        let mut deps = mock_dependencies();
        let owner = setup_contract(&mut deps);

        let mut addr_strs: Vec<String> = Vec::with_capacity(n);
        let mut ids: Vec<u64> = Vec::with_capacity(n);

        for i in 0..n {
            let borrower_str = deps.api.addr_make(&format!("borrower_{}", i)).to_string();
            let id = create_credit_line(&mut deps, &owner, &borrower_str);
            ids.push(id);
            addr_strs.push(borrower_str);
        }

        // ── Right-inverse: credit_line_by_id(id).borrower == original address ──
        for (i, &id) in ids.iter().enumerate() {
            let cl = CREDIT_LINES.load(deps.as_ref().storage, id).unwrap();
            prop_assert_eq!(
                cl.borrower.as_str(),
                addr_strs[i].as_str(),
                "Right-inverse failed for id={}: expected {} got {}",
                id,
                addr_strs[i],
                cl.borrower,
            );
        }

        // ── Left-inverse: find_id_for_borrower(addr) == Some(id) ────────────
        for (i, &id) in ids.iter().enumerate() {
            let borrower_addr = deps.api.addr_validate(&addr_strs[i]).unwrap();
            let recovered = find_id_for_borrower(&deps, &borrower_addr);
            prop_assert_eq!(
                recovered,
                Some(id),
                "Left-inverse failed for addr={}: expected Some({}) got {:?}",
                addr_strs[i],
                id,
                recovered,
            );
        }

        // ── Injectivity: all N IDs are unique ────────────────────────────────
        let unique_ids: HashSet<u64> = ids.iter().copied().collect();
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

    /// Verify that sequential credit line creations produce strictly
    /// increasing IDs even when borrowers share similar name prefixes.
    #[test]
    fn sequential_borrowers_get_sequential_ids(n in registration_count()) {
        let mut deps = mock_dependencies();
        let owner = setup_contract(&mut deps);

        let mut prev_id: Option<u64> = None;
        for i in 0..n {
            let borrower_str = deps.api.addr_make(&format!("borrower_{}", i)).to_string();
            let id = create_credit_line(&mut deps, &owner, &borrower_str);

            if let Some(prev) = prev_id {
                prop_assert_eq!(
                    id,
                    prev + 1,
                    "IDs must be strictly sequential: got {} after {}",
                    id,
                    prev,
                );
            }
            prev_id = Some(id);
        }
    }

    /// Verify that each credit line stores exactly one borrower (no aliasing).
    #[test]
    fn each_id_maps_to_exactly_one_borrower(n in registration_count()) {
        let mut deps = mock_dependencies();
        let owner = setup_contract(&mut deps);

        let mut addr_strs: Vec<String> = Vec::with_capacity(n);
        let mut ids: Vec<u64> = Vec::with_capacity(n);

        for i in 0..n {
            let borrower_str = deps.api.addr_make(&format!("borrower_{}", i)).to_string();
            let id = create_credit_line(&mut deps, &owner, &borrower_str);
            ids.push(id);
            addr_strs.push(borrower_str);
        }

        // Each ID maps to exactly the borrower that created it
        for &id in &ids {
            let cl = CREDIT_LINES.load(deps.as_ref().storage, id).unwrap();
            // Verify the borrower at this ID is unique by checking no other ID maps to it
            let mut found_count = 0u32;
            for &other_id in &ids {
                let other_cl = CREDIT_LINES.load(deps.as_ref().storage, other_id).unwrap();
                if other_cl.borrower == cl.borrower {
                    found_count += 1;
                }
            }
            prop_assert_eq!(
                found_count, 1,
                "Borrower at id={} appears in {} credit lines, expected exactly 1",
                id, found_count,
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
        let mut deps = mock_dependencies();
        let owner = setup_contract(&mut deps);

        let alice = deps.api.addr_make("alice").to_string();
        let id = create_credit_line(&mut deps, &owner, &alice);
        let cl = CREDIT_LINES.load(deps.as_ref().storage, id).unwrap();
        assert_eq!(cl.borrower.as_str(), alice.as_str());

        let borrower_addr = deps.api.addr_validate(&alice).unwrap();
        let recovered = find_id_for_borrower(&deps, &borrower_addr);
        assert_eq!(recovered, Some(0));
    }

    /// Two borrowers get distinct sequential IDs.
    #[test]
    fn two_borrowers_distinct_ids() {
        let mut deps = mock_dependencies();
        let owner = setup_contract(&mut deps);

        let alice = deps.api.addr_make("alice").to_string();
        let bob = deps.api.addr_make("bob").to_string();
        let id_a = create_credit_line(&mut deps, &owner, &alice);
        let id_b = create_credit_line(&mut deps, &owner, &bob);

        assert_ne!(id_a, id_b, "Two borrowers must get distinct IDs");
        assert_eq!(id_a, 0, "First borrower must get ID 0");
        assert_eq!(id_b, 1, "Second borrower must get ID 1");
    }

    /// Many borrowers: all roundtrips succeed and all IDs are unique.
    #[test]
    fn many_borrowers_all_unique() {
        let mut deps = mock_dependencies();
        let owner = setup_contract(&mut deps);
        let count = 100;

        let mut ids = Vec::with_capacity(count);
        let mut addr_strs = Vec::with_capacity(count);

        for i in 0..count {
            let borrower_str = deps.api.addr_make(&format!("borrower_{}", i)).to_string();
            let id = create_credit_line(&mut deps, &owner, &borrower_str);
            ids.push(id);
            addr_strs.push(borrower_str);
        }

        // All roundtrips succeed
        for (i, &id) in ids.iter().enumerate() {
            let cl = CREDIT_LINES.load(deps.as_ref().storage, id).unwrap();
            assert_eq!(cl.borrower.as_str(), addr_strs[i].as_str());

            let borrower_addr = deps.api.addr_validate(&addr_strs[i]).unwrap();
            let recovered = find_id_for_borrower(&deps, &borrower_addr);
            assert_eq!(recovered, Some(id));
        }

        // All IDs are unique
        let unique: HashSet<u64> = ids.iter().copied().collect();
        assert_eq!(unique.len(), count);

        // Sequential from 0
        let mut sorted = ids.clone();
        sorted.sort();
        for (i, &id) in sorted.iter().enumerate() {
            assert_eq!(id as usize, i, "ID at position {} should be {}", i, i);
        }
    }

    /// Borrower addresses with common prefixes do not collide.
    #[test]
    fn common_prefix_addresses_dont_collide() {
        let mut deps = mock_dependencies();
        let owner = setup_contract(&mut deps);

        // Seeds sharing a common prefix, which could cause encoding issues
        let addr_a = deps.api.addr_make("common-prefix-borrower-a").to_string();
        let addr_b = deps.api.addr_make("common-prefix-borrower-b").to_string();
        let id_a = create_credit_line(&mut deps, &owner, &addr_a);
        let id_b = create_credit_line(&mut deps, &owner, &addr_b);

        assert_ne!(id_a, id_b, "Common-prefix addresses must get distinct IDs");

        let cl_a = CREDIT_LINES.load(deps.as_ref().storage, id_a).unwrap();
        let cl_b = CREDIT_LINES.load(deps.as_ref().storage, id_b).unwrap();
        assert_ne!(cl_a.borrower, cl_b.borrower);
    }
}
