// SPDX-License-Identifier: MIT

use std::collections::HashSet;

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

fn setup(env: &Env) -> (CreditClient<'_>, Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    (client, contract_id, admin)
}

// ── G-address (user account) round-trip ─────────────────────────────────────

#[test]
fn test_g_address_to_string_from_string_round_trip() {
    let env = Env::default();
    let borrower = Address::generate(&env);

    let strkey = borrower.to_string();
    let restored = Address::from_string(&strkey);

    assert_eq!(restored, borrower);
}

#[test]
fn test_g_address_to_string_from_str_round_trip() {
    let env = Env::default();
    let borrower = Address::generate(&env);

    let strkey = borrower.to_string();
    let restored = Address::from_str(&env, strkey.as_ref());

    assert_eq!(restored, borrower);
}

#[test]
fn test_g_address_storage_round_trip() {
    let env = Env::default();
    let (client, _contract_id, _admin) = setup(&env);

    let borrower = Address::generate(&env);
    let strkey = borrower.to_string();
    let restored = Address::from_string(&strkey);

    // Write with original, read with restored
    client.open_credit_line(&borrower, &10_000_i128, &300_u32, &50_u32);
    let line = client.get_credit_line(&restored).unwrap();
    assert_eq!(line.credit_limit, 10_000);
    assert_eq!(line.borrower, borrower);
    assert_eq!(line.status as u32, 0); // Active

    // Write with restored, read with original
    client.close_credit_line(&restored, &borrower);
    client.open_credit_line(&restored, &20_000_i128, &400_u32, &60_u32);
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.credit_limit, 20_000);
    assert_eq!(line.interest_rate_bps, 400);
    assert_eq!(line.risk_score, 60);
}

// ── C-address (contract) round-trip ─────────────────────────────────────────

#[test]
fn test_c_address_to_string_from_string_round_trip() {
    let env = Env::default();
    let contract_id = env.register(Credit, ());

    let strkey = contract_id.to_string();
    let restored = Address::from_string(&strkey);

    assert_eq!(restored, contract_id);
}

#[test]
fn test_c_address_to_string_from_str_round_trip() {
    let env = Env::default();
    let contract_id = env.register(Credit, ());

    let strkey = contract_id.to_string();
    let restored = Address::from_str(&env, strkey.as_ref());

    assert_eq!(restored, contract_id);
}

#[test]
fn test_c_address_storage_round_trip() {
    let env = Env::default();
    let (client, contract_id, _admin) = setup(&env);

    let strkey = contract_id.to_string();
    let restored = Address::from_string(&strkey);

    // Use the contract address itself as a borrower — the contract accepts
    // any Address as a borrower key regardless of type.
    client.open_credit_line(&contract_id, &15_000_i128, &350_u32, &55_u32);
    let line = client.get_credit_line(&restored).unwrap();
    assert_eq!(line.credit_limit, 15_000);
    assert_eq!(line.borrower, contract_id);
}

// ── Strkey prefix verification ──────────────────────────────────────────────

#[test]
fn test_g_address_strkey_prefix() {
    let env = Env::default();
    let user = Address::generate(&env);
    let s = user.to_string();
    let bytes = s.as_ref().as_bytes();
    assert!(!bytes.is_empty(), "G-address strkey must not be empty");
    let first = bytes[0] as char;
    assert!(
        first == 'G' || first == 'C',
        "Address strkey must start with 'G' or 'C', got '{}'",
        first
    );
}

#[test]
fn test_c_address_strkey_prefix() {
    let env = Env::default();
    let contract_id = env.register(Credit, ());
    let s = contract_id.to_string();
    let bytes = s.as_ref().as_bytes();
    assert!(!bytes.is_empty());
    let first = bytes[0] as char;
    assert_eq!(first, 'C', "Contract address strkey must start with 'C'");
}

#[test]
fn test_g_and_c_strkey_prefixes_are_distinct() {
    let env = Env::default();
    let user = Address::generate(&env);
    let contract_id = env.register(Credit, ());

    let user_str = user.to_string();
    let contract_str = contract_id.to_string();

    assert_ne!(user_str, contract_str);

    let user_first = user_str.as_ref().as_bytes()[0] as char;
    let contract_first = contract_str.as_ref().as_bytes()[0] as char;
    assert_eq!(user_first, 'G', "User address strkey should start with 'G'");
    assert_eq!(
        contract_first, 'C',
        "Contract address strkey should start with 'C'"
    );
}

// ── Collision resistance ────────────────────────────────────────────────────

#[test]
fn test_different_addresses_produce_different_strkeys() {
    let env = Env::default();

    let addr1 = Address::generate(&env);
    let addr2 = Address::generate(&env);

    let s1 = addr1.to_string();
    let s2 = addr2.to_string();

    assert_ne!(
        s1, s2,
        "Different G-addresses must produce different strkey strings"
    );
}

#[test]
fn test_different_addresses_access_different_storage() {
    let env = Env::default();
    let (client, _contract_id, _admin) = setup(&env);

    let borrower1 = Address::generate(&env);
    let borrower2 = Address::generate(&env);

    client.open_credit_line(&borrower1, &5_000_i128, &300_u32, &50_u32);

    // borrower2 has no credit line
    assert!(client.get_credit_line(&borrower2).is_none());

    // Verify borrower1 still works
    assert!(client.get_credit_line(&borrower1).is_some());

    // Now restore borrower2 from strkey and verify no collision
    let s2 = borrower2.to_string();
    let restored2 = Address::from_string(&s2);
    assert!(client.get_credit_line(&restored2).is_none());
}

// ── Multiple round-trips (stress) ───────────────────────────────────────────

#[test]
fn test_multiple_addresses_independent_round_trips() {
    let env = Env::default();
    let (client, _contract_id, _admin) = setup(&env);

    let mut original_addresses = Vec::new();

    // Generate and open credit lines for 30 addresses
    for i in 0..30u32 {
        let addr = Address::generate(&env);
        let limit = ((i as i128) + 1) * 1_000;
        let rate = 300 + i * 10;
        let score = 50 + i;
        client.open_credit_line(&addr, &limit, &rate, &score);
        original_addresses.push((addr, limit, rate, score));
    }

    // For each address: round-trip strkey, then verify storage
    for (original_addr, expected_limit, expected_rate, expected_score) in &original_addresses {
        let strkey = original_addr.to_string();
        let restored = Address::from_string(&strkey);
        assert_eq!(&restored, original_addr);

        let line = client.get_credit_line(&restored).unwrap();
        assert_eq!(line.credit_limit, *expected_limit);
        assert_eq!(line.borrower, *original_addr);
        assert_eq!(line.interest_rate_bps, *expected_rate);
        assert_eq!(line.risk_score, *expected_score);
    }

    // Verify all strkeys are unique (byte-level collision check)
    let strkey_bytes: Vec<Vec<u8>> = original_addresses
        .iter()
        .map(|(addr, _, _, _)| addr.to_string().as_ref().as_bytes().to_vec())
        .collect();
    let unique: HashSet<Vec<u8>> = strkey_bytes.iter().cloned().collect();
    assert_eq!(unique.len(), original_addresses.len());
}

// ── from_string round-trip with different encoding paths ─────────────────────

#[test]
fn test_to_string_then_from_str_via_env() {
    let env = Env::default();
    let borrower = Address::generate(&env);

    let strkey_sdk = borrower.to_string();

    // Path A: from_string (takes &soroban_sdk::String)
    let via_from_string = Address::from_string(&strkey_sdk);
    assert_eq!(via_from_string, borrower);

    // Path B: from_str (takes &str via Env)
    let str_rust = strkey_sdk.as_ref();
    let via_from_str = Address::from_str(&env, str_rust);
    assert_eq!(via_from_str, borrower);

    // Path C: idempotent — to_string on restored yields same string
    assert_eq!(via_from_string.to_string(), strkey_sdk);
    assert_eq!(via_from_str.to_string(), strkey_sdk);
}

#[test]
fn test_contract_address_to_string_then_from_str() {
    let env = Env::default();
    let contract_id = env.register(Credit, ());

    let strkey = contract_id.to_string();
    let via_from_str = Address::from_str(&env, strkey.as_ref());
    assert_eq!(via_from_str, contract_id);
    assert_eq!(via_from_str.to_string(), strkey);
}
