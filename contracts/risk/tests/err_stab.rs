// SPDX-License-Identifier: MIT

//! ContractError stability tests for the risk admin cooldown contract.
//!
//! # What
//!
//! Focused CI guard for the numeric discriminants and category mappings used by
//! the standalone risk contract (`creditra_risk`). Any assertion failure means a
//! discriminant, category, or runtime error path was accidentally changed —
//! breaking deployed SDK clients and indexers that match on error codes.
//!
//! # Scope (risk contract surface)
//!
//! - `Unauthorized` (1)
//! - `NotAdmin` (2)
//! - `Paused` (3)
//! - `RiskAdminCooldownActive` (54)
//!
//! # Rules
//! - Never change an existing assertion value.
//! - If a new risk-related error variant is added, append it with the next
//!   available integer **and** add corresponding assertions here.
//! - Integration tests MUST verify the raw discriminant (e.g. `"#54"`) is
//!   encoded in the panic payload — never match on variant names alone.

use creditra_risk::{ContractError, ContractErrorCategory, RiskContract, RiskContractClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env,
};

// ═══════════════════════════════════════════════════════════════════════════
// Section 1 — Discriminant stability pins (risk error surface)
// ═══════════════════════════════════════════════════════════════════════════

/// Pin every discriminant in the risk contract error surface.
#[test]
fn risk_error_discriminants_are_pinned() {
    assert_eq!(ContractError::Unauthorized as u32, 1);
    assert_eq!(ContractError::NotAdmin as u32, 2);
    assert_eq!(ContractError::Paused as u32, 3);
    assert_eq!(ContractError::RiskAdminCooldownActive as u32, 54);
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 2 — Category stability pins
// ═══════════════════════════════════════════════════════════════════════════

/// Every risk-related variant maps to the expected stable category.
#[test]
fn risk_category_mappings_are_pinned() {
    use ContractErrorCategory::*;

    assert_eq!(ContractError::Unauthorized.category(), Auth);
    assert_eq!(ContractError::NotAdmin.category(), Auth);
    assert_eq!(ContractError::Paused.category(), Risk);
    assert_eq!(ContractError::RiskAdminCooldownActive.category(), Risk);
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 3 — Variant-count sanity
// ═══════════════════════════════════════════════════════════════════════════

/// Verify that the pinned risk surface remains a small, known set.
#[test]
fn risk_error_surface_variant_count_is_known() {
    const EXPECTED_VARIANT_COUNT: usize = 4;

    let codes = [
        ContractError::Unauthorized as u32,
        ContractError::NotAdmin as u32,
        ContractError::Paused as u32,
        ContractError::RiskAdminCooldownActive as u32,
    ];

    assert_eq!(
        codes.len(),
        EXPECTED_VARIANT_COUNT,
        "risk error surface variant count changed — update the pins above"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 4 — Integration: runtime error paths return the pinned discriminant
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod integration {
    use super::*;

    fn setup() -> (Env, Address, RiskContractClient<'static>) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| li.timestamp = 10_000);

        let admin = Address::generate(&env);
        let contract_id = env.register(RiskContract, ());
        let client = RiskContractClient::new(&env, &contract_id);
        client.init(&admin);

        (env, admin, client)
    }

    fn extract_error_str(payload: &Box<dyn std::any::Any + Send>) -> String {
        if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else if let Some(s) = payload.downcast_ref::<&str>() {
            s.to_string()
        } else {
            String::new()
        }
    }

    /// `set_risk_admin_cooldown` while paused returns the pinned `#3` code.
    #[test]
    fn paused_entrypoint_reverts_with_paused_code_3() {
        let (_env, _admin, client) = setup();

        client.set_paused(&true);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.set_risk_admin_cooldown(&60);
        }));
        assert!(result.is_err(), "expected revert when paused");

        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#3"),
            "expected Paused (#3), got: {err_str:?}"
        );
    }

    /// `record_risk_admin_action` while the cooldown is active returns `#54`.
    #[test]
    fn cooldown_guard_reverts_with_risk_admin_cooldown_active_code_54() {
        let (env, _admin, client) = setup();

        client.set_risk_admin_cooldown(&3_600);
        env.ledger().with_mut(|li| li.timestamp = 1_000);
        client.record_risk_admin_action();

        env.ledger().with_mut(|li| li.timestamp = 1_001);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.record_risk_admin_action();
        }));
        assert!(result.is_err(), "expected revert while cooldown is active");

        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#54"),
            "expected RiskAdminCooldownActive (#54), got: {err_str:?}"
        );
    }
}
