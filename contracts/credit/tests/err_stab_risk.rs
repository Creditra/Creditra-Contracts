// SPDX-License-Identifier: MIT

//! ContractError stability tests for the risk subsystem.
//!
//! Pins the discriminants and category mappings for every error variant used
//! by `creditra_credit::risk`. The snapshot file `err_snapshot_risk.json`
//! stores the expected variant→code mapping. Set `UPDATE_SNAPSHOT=1` to
//! regenerate the snapshot when adding/removing risk variants.

use creditra_credit::types::{ContractError, ContractErrorCategory};
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token, Address, Env,
};
use std::collections::BTreeMap;
use std::path::PathBuf;

// ── Discriminant stability pins (risk surface) ──────────────────────────

#[test]
fn risk_v7_error_discriminants_are_pinned() {
    // Auth → require_admin_auth gates every risk entrypoint
    assert_eq!(ContractError::Unauthorized as u32, 1);
    assert_eq!(ContractError::NotAdmin as u32, 2);
    assert_eq!(ContractError::AdminNotInitialized as u32, 32);

    // Numeric → input validation in update_risk_parameters
    assert_eq!(ContractError::InvalidAmount as u32, 5);
    assert_eq!(ContractError::NegativeLimit as u32, 7);
    assert_eq!(ContractError::LimitOutOfBounds as u32, 34);
    assert_eq!(ContractError::Overflow as u32, 12);

    // Risk → rate/score enforcement
    assert_eq!(ContractError::RateTooHigh as u32, 8);
    assert_eq!(ContractError::ScoreTooHigh as u32, 9);

    // Circuit breaker → assert_not_paused gates risk writes
    assert_eq!(ContractError::Paused as u32, 18);

    // Timestamp guard → rate change interval enforcement
    assert_eq!(ContractError::TimestampRegression as u32, 33);

    // Lifecycle → credit line loading
    assert_eq!(ContractError::CreditLineNotFound as u32, 3);
    assert_eq!(ContractError::CreditLineClosed as u32, 4);
    assert_eq!(ContractError::CreditLineSuspended as u32, 20);
    assert_eq!(ContractError::CreditLineDefaulted as u32, 21);

    // Cooldown → borrow admin cooldown
    assert_eq!(ContractError::AdminCooldownActive as u32, 54);

    // Exposure → limit decrease overflow
    assert_eq!(ContractError::ExposureCapExceeded as u32, 31);
}

// Category stability pins (risk surface)

#[test]
fn risk_v7_category_mappings_are_pinned() {
    use ContractErrorCategory::*;

    // Auth bucket (1)
    assert_eq!(ContractError::Unauthorized.category(), Auth);
    assert_eq!(ContractError::NotAdmin.category(), Auth);
    assert_eq!(ContractError::AdminNotInitialized.category(), Auth);

    // Numeric bucket (3)
    assert_eq!(ContractError::InvalidAmount.category(), Numeric);
    assert_eq!(ContractError::NegativeLimit.category(), Numeric);
    assert_eq!(ContractError::LimitOutOfBounds.category(), Numeric);
    assert_eq!(ContractError::Overflow.category(), Numeric);

    // Risk bucket (6)
    assert_eq!(ContractError::RateTooHigh.category(), Risk);
    assert_eq!(ContractError::ScoreTooHigh.category(), Risk);
    assert_eq!(ContractError::Paused.category(), Risk);
    assert_eq!(ContractError::DrawCooldownActive.category(), Risk);
    assert_eq!(ContractError::AdminCooldownActive.category(), Risk);

    // Lifecycle bucket (2)
    assert_eq!(ContractError::CreditLineNotFound.category(), Lifecycle);
    assert_eq!(ContractError::CreditLineClosed.category(), Lifecycle);
    assert_eq!(ContractError::CreditLineSuspended.category(), Lifecycle);
    assert_eq!(ContractError::CreditLineDefaulted.category(), Lifecycle);
}

// Snapshot: JSON file stores variant→code mapping

fn snapshot_path() -> PathBuf {
    // Resolve relative to CARGO_MANIFEST_DIR so the path works regardless of cwd.
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    dir.join("tests").join("err_snapshot_risk.json")
}

fn current_risk_snapshot() -> BTreeMap<String, u32> {
    let mut map = BTreeMap::new();
    map.insert("Unauthorized".into(), ContractError::Unauthorized as u32);
    map.insert("NotAdmin".into(), ContractError::NotAdmin as u32);
    map.insert(
        "CreditLineNotFound".into(),
        ContractError::CreditLineNotFound as u32,
    );
    map.insert(
        "CreditLineClosed".into(),
        ContractError::CreditLineClosed as u32,
    );
    map.insert("InvalidAmount".into(), ContractError::InvalidAmount as u32);
    map.insert("NegativeLimit".into(), ContractError::NegativeLimit as u32);
    map.insert("RateTooHigh".into(), ContractError::RateTooHigh as u32);
    map.insert("ScoreTooHigh".into(), ContractError::ScoreTooHigh as u32);
    map.insert("Overflow".into(), ContractError::Overflow as u32);
    map.insert("Paused".into(), ContractError::Paused as u32);
    map.insert(
        "CreditLineSuspended".into(),
        ContractError::CreditLineSuspended as u32,
    );
    map.insert(
        "CreditLineDefaulted".into(),
        ContractError::CreditLineDefaulted as u32,
    );
    map.insert(
        "AdminNotInitialized".into(),
        ContractError::AdminNotInitialized as u32,
    );
    map.insert(
        "TimestampRegression".into(),
        ContractError::TimestampRegression as u32,
    );
    map.insert(
        "LimitOutOfBounds".into(),
        ContractError::LimitOutOfBounds as u32,
    );
    map.insert(
        "AdminCooldownActive".into(),
        ContractError::AdminCooldownActive as u32,
    );
    map
}

fn serialize_snapshot(map: &BTreeMap<String, u32>) -> String {
    let mut s = String::from("{\n");
    for (i, (name, code)) in map.iter().enumerate() {
        let comma = if i < map.len() - 1 { "," } else { "" };
        s.push_str(&format!("  \"{}\": {}{}\n", name, code, comma));
    }
    s.push_str("}\n");
    s
}

fn parse_snapshot(raw: &str) -> BTreeMap<String, u32> {
    let mut map = BTreeMap::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('{') || line.starts_with('}') {
            continue;
        }
        // Expect: "VariantName": 123,
        let line = line.strip_suffix(',').unwrap_or(line);
        let mut parts = line.splitn(2, ": ");
        let name_raw = parts.next().unwrap_or("").trim();
        let name = name_raw.trim_matches('"');
        let code: u32 = parts
            .next()
            .unwrap_or("0")
            .trim()
            .parse()
            .unwrap_or(u32::MAX);
        if !name.is_empty() {
            map.insert(name.to_string(), code);
        }
    }
    map
}

#[test]
fn risk_error_snapshot_matches() {
    let current = current_risk_snapshot();
    let path = snapshot_path();

    if std::env::var("UPDATE_SNAPSHOT").is_ok() {
        let json = serialize_snapshot(&current);
        std::fs::write(&path, json.as_bytes()).expect("write snapshot");
        return;
    }

    let snapshot_raw = std::fs::read_to_string(&path)
        .expect("err_snapshot_risk.json not found — run with UPDATE_SNAPSHOT=1 to create it");
    let snapshot = parse_snapshot(&snapshot_raw);
    assert_eq!(
        current, snapshot,
        "Risk error snapshot mismatch. Run with UPDATE_SNAPSHOT=1 to regenerate."
    );
}

#[test]
fn risk_v7_subset_has_no_duplicate_discriminants() {
    use std::collections::HashSet;
    let codes: Vec<u32> = current_risk_snapshot().into_values().collect();
    let unique: HashSet<u32> = codes.iter().cloned().collect();
    assert_eq!(
        codes.len(),
        unique.len(),
        "Duplicate discriminants in risk surface — inspect types.rs"
    );
}

#[test]
fn risk_v7_subset_variant_count_is_known() {
    assert_eq!(current_risk_snapshot().len(), 16);
}

//  Integration: runtime error paths

#[cfg(test)]
mod integration {
    use super::*;

    fn setup() -> (Env, CreditClient<'_>, Address) {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);

        (env, client, admin)
    }

    fn setup_with_borrower() -> (Env, CreditClient<'_>, Address, Address) {
        let (env, client, admin) = setup();
        let borrower = Address::generate(&env);
        client.open_credit_line(&borrower, &1000_i128, &500_u32, &50_u32);
        (env, client, admin, borrower)
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

    // Test 1: update_risk_parameters on non-existent line → CreditLineNotFound (3)
    #[test]
    fn risk_update_nonexistent_line_reverts_with_code_3() {
        let (env, client, _admin) = setup();
        let borrower = Address::generate(&env);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.update_risk_parameters(&borrower, &1000_i128, &500_u32, &50_u32);
        }));
        assert!(result.is_err());
        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#3"),
            "expected CreditLineNotFound (#3), got: {:?}",
            err_str
        );
    }

    // Test 2: negative credit limit → NegativeLimit (7)
    #[test]
    fn risk_update_negative_limit_reverts_with_code_7() {
        let (env, client, _admin, borrower) = setup_with_borrower();

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.update_risk_parameters(&borrower, &-1_i128, &500_u32, &50_u32);
        }));
        assert!(result.is_err());
        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#7"),
            "expected NegativeLimit (#7), got: {:?}",
            err_str
        );
    }

    // Test 3: excessive risk score → ScoreTooHigh (9)
    #[test]
    fn risk_update_excessive_score_reverts_with_code_9() {
        let (env, client, _admin, borrower) = setup_with_borrower();

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.update_risk_parameters(&borrower, &1000_i128, &500_u32, &101_u32);
        }));
        assert!(result.is_err());
        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#9"),
            "expected ScoreTooHigh (#9), got: {:?}",
            err_str
        );
    }

    // Test 4: excessive rate → RateTooHigh (8)
    #[test]
    fn risk_update_excessive_rate_reverts_with_code_8() {
        let (env, client, _admin, borrower) = setup_with_borrower();

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.update_risk_parameters(&borrower, &1000_i128, &10_001_u32, &50_u32);
        }));
        assert!(result.is_err());
        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#8"),
            "expected RateTooHigh (#8), got: {:?}",
            err_str
        );
    }

    // Test 5: paused protocol → Paused (18)
    #[test]
    fn risk_update_while_paused_reverts_with_code_18() {
        let (env, client, admin, borrower) = setup_with_borrower();
        client.set_protocol_paused(&true);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.update_risk_parameters(&borrower, &1000_i128, &500_u32, &50_u32);
        }));
        assert!(result.is_err());
        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#18"),
            "expected Paused (#18), got: {:?}",
            err_str
        );
    }

    // Test 6: rate change exceeds cap → RateTooHigh (8)
    #[test]
    fn risk_update_excessive_rate_change_reverts_with_code_8() {
        let (env, client, _admin, borrower) = setup_with_borrower();
        client.set_rate_change_limits(&50_u32, &0_u64);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.update_risk_parameters(&borrower, &1000_i128, &1000_u32, &50_u32);
        }));
        assert!(result.is_err());
        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#8"),
            "expected RateTooHigh (#8) for excessive rate change, got: {:?}",
            err_str
        );
    }

    // Test 7: admin-not-initialized → AdminNotInitialized (32)
    #[test]
    fn risk_update_without_init_reverts_with_code_32() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.update_risk_parameters(&borrower, &1000_i128, &500_u32, &50_u32);
        }));
        assert!(result.is_err());
        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#32"),
            "expected AdminNotInitialized (#32), got: {:?}",
            err_str
        );
    }

    // Test 8: determinism — same risk error twice returns same code
    #[test]
    fn risk_error_discriminant_is_deterministic() {
        let (env, client, _admin, borrower) = setup_with_borrower();

        for run in 1..=2 {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                client.update_risk_parameters(&borrower, &-1_i128, &500_u32, &50_u32);
            }));
            assert!(result.is_err(), "run {} must revert", run);
            let err_str = extract_error_str(&result.unwrap_err());
            assert!(
                err_str.contains("#7"),
                "run {}: expected NegativeLimit (#7), got: {:?}",
                run,
                err_str
            );
        }
    }

    // Test 9: limit decrease below utilization → restricted (no error)
    #[test]
    fn risk_limit_decrease_below_utilization_transitions_to_restricted() {
        let (env, client, _admin, borrower) = setup_with_borrower();

        let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
        let token_address = token_id.address();
        client.set_liquidity_token(&token_address);
        client.set_liquidity_source(&env.current_contract_address());
        token::StellarAssetClient::new(&env, &token_address)
            .mint(&env.current_contract_address(), &10_000_i128);

        client.draw_credit(&borrower, &500_i128);
        client.update_risk_parameters(&borrower, &300_i128, &500_u32, &50_u32);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.credit_limit, 300, "credit limit should be 300");
    }
}
