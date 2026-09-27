#![cfg(test)]

use super::*;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{token, Address, Env, Vec, symbol_short};

fn setup_test(env: &Env) -> (Address, Address, token::Client, token::StellarAssetClient, CrowdPayContractClient) {
    let creator = Address::generate(env);
    let token_admin = Address::generate(env);
    let token_id = env.register_stellar_asset_contract(token_admin);
    let token = token::Client::new(env, &token_id);
    let token_admin_client = token::StellarAssetClient::new(env, &token_id);

    let contract_id = env.register_contract(None, CrowdPayContract);
    let client = CrowdPayContractClient::new(env, &contract_id);

    (creator, token_id, token, token_admin_client, client)
}

#[test]
fn test_initialize() {
    let env = Env::default();
    let (creator, token_id, _token, _token_admin, client) = setup_test(&env);

    let milestones = Vec::from_array(&env, [
        Milestone { target: 1000, released: false },
        Milestone { target: 2000, released: false },
    ]);

    client.initialize(
        &symbol_short!("cp1"),
        &creator,
        &token_id,
        &2000,
        &10000,
        &milestones,
    );

    assert_eq!(client.get_status(), symbol_short!("Active"));
}

#[test]
fn test_contribute_and_goal_reached() {
    let env = Env::default();
    let (creator, token_id, token, token_admin, client) = setup_test(&env);
    let contributor = Address::generate(&env);

    let milestones = Vec::from_array(&env, [Milestone { target: 1000, released: false }]);
    client.initialize(&symbol_short!("cp1"), &creator, &token_id, &1000, &10000, &milestones);

    token_admin.mock_all_auths().mint(&contributor, &1000);
    client.mock_all_auths().contribute(&contributor, &1000);

    assert_eq!(client.get_status(), symbol_short!("Funded"));
    assert_eq!(client.get_total_raised(), 1000);
    assert_eq!(token.balance(&client.address), 1000);
}

#[test]
fn test_milestone_release() {
    let env = Env::default();
    let (creator, token_id, token, token_admin, client) = setup_test(&env);
    let contributor = Address::generate(&env);

    let milestones = Vec::from_array(&env, [Milestone { target: 1000, released: false }]);
    client.initialize(&symbol_short!("cp1"), &creator, &token_id, &1000, &10000, &milestones);

    token_admin.mock_all_auths().mint(&contributor, &1000);
    client.mock_all_auths().contribute(&contributor, &1000);

    client.mock_all_auths().release_milestone(&0);
    assert_eq!(token.balance(&creator), 1000);
    assert_eq!(token.balance(&client.address), 0);
}

#[test]
#[should_panic(expected = "Milestone target not met")]
fn test_milestone_release_fails_if_target_not_met() {
    let env = Env::default();
    let (creator, token_id, token, token_admin, client) = setup_test(&env);
    let contributor = Address::generate(&env);

    let milestones = Vec::from_array(&env, [Milestone { target: 1000, released: false }]);
    client.initialize(&symbol_short!("cp1"), &creator, &token_id, &1000, &10000, &milestones);

    token_admin.mock_all_auths().mint(&contributor, &500);
    client.mock_all_auths().contribute(&contributor, &500);

    client.mock_all_auths().release_milestone(&0);
}

#[test]
fn test_refund_after_failure() {
    let env = Env::default();
    let (creator, token_id, token, token_admin, client) = setup_test(&env);
    let contributor = Address::generate(&env);

    let milestones = Vec::from_array(&env, [Milestone { target: 1000, released: false }]);
    client.initialize(&symbol_short!("cp1"), &creator, &token_id, &1000, &100, &milestones);

    token_admin.mock_all_auths().mint(&contributor, &500);
    client.mock_all_auths().contribute(&contributor, &500);

    // Advance time
    env.ledger().set_timestamp(101);
    client.set_failed();
    assert_eq!(client.get_status(), symbol_short!("Failed"));

    client.mock_all_auths().refund(&contributor);
    assert_eq!(token.balance(&contributor), 500);
    assert_eq!(token.balance(&client.address), 0);
}

#[test]
#[should_panic(expected = "Campaign has not failed")]
fn test_refund_fails_if_active() {
    let env = Env::default();
    let (creator, token_id, token, token_admin, client) = setup_test(&env);
    let contributor = Address::generate(&env);

    let milestones = Vec::from_array(&env, [Milestone { target: 1000, released: false }]);
    client.initialize(&symbol_short!("cp1"), &creator, &token_id, &1000, &10000, &milestones);

    token_admin.mock_all_auths().mint(&contributor, &500);
    client.mock_all_auths().contribute(&contributor, &500);

    client.mock_all_auths().refund(&contributor);
}

// ---------------------------------------------------------------------------
// TTL tests — verify storage lifetime is refreshed on every hot financial path
// ---------------------------------------------------------------------------
//
// The crowdpay contract uses:
//   • instance storage  — campaign config, status, totals, milestones
//   • persistent storage — per-contributor Contributions(Address) balances
//
// Both tiers must be refreshed on every mutating call.  We read TTL values
// directly through `env.as_contract` after each call and assert they are at
// or above the declared threshold constants.

/// Returns the current instance TTL for the crowdpay contract at `contract_id`.
fn instance_ttl(env: &Env, contract_id: &Address) -> u32 {
    env.as_contract(contract_id, || {
        env.storage().instance().get_ttl()
    })
}

/// Returns the current TTL of a per-contributor persistent balance entry.
fn contributor_ttl(env: &Env, contract_id: &Address, contributor: &Address) -> u32 {
    let key = DataKey::Contributions(contributor.clone());
    env.as_contract(contract_id, || {
        env.storage().persistent().get_ttl(&key)
    })
}

#[test]
fn test_ttl_set_on_initialize() {
    let env = Env::default();
    let (creator, token_id, _token, _token_admin, client) = setup_test(&env);

    let milestones = Vec::from_array(&env, [Milestone { target: 1000, released: false }]);
    client.initialize(&symbol_short!("cp_ttl"), &creator, &token_id, &1000, &999999, &milestones);

    let ttl = instance_ttl(&env, &client.address);
    assert!(
        ttl >= super::INSTANCE_TTL_THRESHOLD,
        "instance TTL after initialize ({ttl}) must be >= INSTANCE_TTL_THRESHOLD ({})",
        super::INSTANCE_TTL_THRESHOLD
    );
}

#[test]
fn test_ttl_refreshed_on_contribute_instance() {
    let env = Env::default();
    let (creator, token_id, _token, token_admin, client) = setup_test(&env);
    let contributor = Address::generate(&env);

    let milestones = Vec::from_array(&env, [Milestone { target: 1000, released: false }]);
    client.initialize(&symbol_short!("cp_ttl"), &creator, &token_id, &1000, &999999, &milestones);

    token_admin.mock_all_auths().mint(&contributor, &500);
    client.mock_all_auths().contribute(&contributor, &500);

    let ttl = instance_ttl(&env, &client.address);
    assert!(
        ttl >= super::INSTANCE_TTL_THRESHOLD,
        "instance TTL after contribute ({ttl}) must be >= INSTANCE_TTL_THRESHOLD ({})",
        super::INSTANCE_TTL_THRESHOLD
    );
}

#[test]
fn test_ttl_refreshed_on_contribute_persistent() {
    let env = Env::default();
    let (creator, token_id, _token, token_admin, client) = setup_test(&env);
    let contributor = Address::generate(&env);

    let milestones = Vec::from_array(&env, [Milestone { target: 1000, released: false }]);
    client.initialize(&symbol_short!("cp_ttl"), &creator, &token_id, &1000, &999999, &milestones);

    token_admin.mock_all_auths().mint(&contributor, &500);
    client.mock_all_auths().contribute(&contributor, &500);

    // The per-contributor persistent entry must also be refreshed.
    let pers_ttl = contributor_ttl(&env, &client.address, &contributor);
    assert!(
        pers_ttl >= super::PERSISTENT_TTL_THRESHOLD,
        "contributor persistent TTL after contribute ({pers_ttl}) must be >= PERSISTENT_TTL_THRESHOLD ({})",
        super::PERSISTENT_TTL_THRESHOLD
    );
}

#[test]
fn test_persistent_ttl_independent_per_contributor() {
    // Each contributor gets their own persistent entry; all must be refreshed.
    let env = Env::default();
    let (creator, token_id, _token, token_admin, client) = setup_test(&env);
    let contributor1 = Address::generate(&env);
    let contributor2 = Address::generate(&env);

    let milestones = Vec::from_array(&env, [Milestone { target: 2000, released: false }]);
    client.initialize(&symbol_short!("cp_ttl"), &creator, &token_id, &2000, &999999, &milestones);

    token_admin.mock_all_auths().mint(&contributor1, &600);
    token_admin.mock_all_auths().mint(&contributor2, &800);
    client.mock_all_auths().contribute(&contributor1, &600);
    client.mock_all_auths().contribute(&contributor2, &800);

    let ttl1 = contributor_ttl(&env, &client.address, &contributor1);
    let ttl2 = contributor_ttl(&env, &client.address, &contributor2);

    assert!(
        ttl1 >= super::PERSISTENT_TTL_THRESHOLD,
        "contributor1 persistent TTL ({ttl1}) must be >= PERSISTENT_TTL_THRESHOLD ({})",
        super::PERSISTENT_TTL_THRESHOLD
    );
    assert!(
        ttl2 >= super::PERSISTENT_TTL_THRESHOLD,
        "contributor2 persistent TTL ({ttl2}) must be >= PERSISTENT_TTL_THRESHOLD ({})",
        super::PERSISTENT_TTL_THRESHOLD
    );
}

#[test]
fn test_ttl_refreshed_on_release_milestone() {
    let env = Env::default();
    let (creator, token_id, _token, token_admin, client) = setup_test(&env);
    let contributor = Address::generate(&env);

    let milestones = Vec::from_array(&env, [Milestone { target: 1000, released: false }]);
    client.initialize(&symbol_short!("cp_ttl"), &creator, &token_id, &1000, &999999, &milestones);

    token_admin.mock_all_auths().mint(&contributor, &1000);
    client.mock_all_auths().contribute(&contributor, &1000);
    client.mock_all_auths().release_milestone(&0);

    let ttl = instance_ttl(&env, &client.address);
    assert!(
        ttl >= super::INSTANCE_TTL_THRESHOLD,
        "instance TTL after release_milestone ({ttl}) must be >= INSTANCE_TTL_THRESHOLD ({})",
        super::INSTANCE_TTL_THRESHOLD
    );
}

#[test]
fn test_ttl_refreshed_on_set_failed() {
    let env = Env::default();
    let (creator, token_id, _token, token_admin, client) = setup_test(&env);
    let contributor = Address::generate(&env);

    let milestones = Vec::from_array(&env, [Milestone { target: 1000, released: false }]);
    // Short deadline so set_failed is permissionless once the ledger advances.
    client.initialize(&symbol_short!("cp_ttl"), &creator, &token_id, &1000, &100, &milestones);

    token_admin.mock_all_auths().mint(&contributor, &500);
    client.mock_all_auths().contribute(&contributor, &500);

    env.ledger().set_timestamp(101);
    client.set_failed();

    let ttl = instance_ttl(&env, &client.address);
    assert!(
        ttl >= super::INSTANCE_TTL_THRESHOLD,
        "instance TTL after set_failed ({ttl}) must be >= INSTANCE_TTL_THRESHOLD ({})",
        super::INSTANCE_TTL_THRESHOLD
    );
}

#[test]
fn test_ttl_refreshed_on_refund_instance() {
    let env = Env::default();
    let (creator, token_id, _token, token_admin, client) = setup_test(&env);
    let contributor = Address::generate(&env);

    let milestones = Vec::from_array(&env, [Milestone { target: 1000, released: false }]);
    client.initialize(&symbol_short!("cp_ttl"), &creator, &token_id, &1000, &100, &milestones);

    token_admin.mock_all_auths().mint(&contributor, &500);
    client.mock_all_auths().contribute(&contributor, &500);

    env.ledger().set_timestamp(101);
    client.set_failed();
    client.mock_all_auths().refund(&contributor);

    let ttl = instance_ttl(&env, &client.address);
    assert!(
        ttl >= super::INSTANCE_TTL_THRESHOLD,
        "instance TTL after refund ({ttl}) must be >= INSTANCE_TTL_THRESHOLD ({})",
        super::INSTANCE_TTL_THRESHOLD
    );
}

#[test]
fn test_persistent_ttl_bumped_before_refund_read() {
    // The pre-read bump in refund means the persistent entry TTL is extended
    // *before* the balance is read, ensuring an expired entry cannot produce
    // a silent zero-balance miss.  After the zero-out the entry still exists
    // in storage (value = 0) so we can read and verify its TTL.
    let env = Env::default();
    let (creator, token_id, _token, token_admin, client) = setup_test(&env);
    let contributor = Address::generate(&env);

    let milestones = Vec::from_array(&env, [Milestone { target: 1000, released: false }]);
    client.initialize(&symbol_short!("cp_ttl"), &creator, &token_id, &1000, &100, &milestones);

    token_admin.mock_all_auths().mint(&contributor, &400);
    client.mock_all_auths().contribute(&contributor, &400);

    env.ledger().set_timestamp(101);
    client.set_failed();
    client.mock_all_auths().refund(&contributor);

    let pers_ttl = contributor_ttl(&env, &client.address, &contributor);
    assert!(
        pers_ttl >= super::PERSISTENT_TTL_THRESHOLD,
        "persistent TTL after refund pre-read bump ({pers_ttl}) must be >= PERSISTENT_TTL_THRESHOLD ({})",
        super::PERSISTENT_TTL_THRESHOLD
    );
}

#[test]
fn test_ttl_constants_are_sane() {
    // Regression guard: threshold < extend and extend covers at least 30 days.
    assert!(
        super::INSTANCE_TTL_THRESHOLD < super::INSTANCE_TTL_EXTEND,
        "INSTANCE_TTL_THRESHOLD must be less than INSTANCE_TTL_EXTEND"
    );
    assert!(
        super::PERSISTENT_TTL_THRESHOLD < super::PERSISTENT_TTL_EXTEND,
        "PERSISTENT_TTL_THRESHOLD must be less than PERSISTENT_TTL_EXTEND"
    );
    // 30 days at 5 s/ledger ≈ 518_400 ledgers.
    assert!(
        super::INSTANCE_TTL_EXTEND >= 518_400,
        "INSTANCE_TTL_EXTEND ({}) should cover at least 30 days",
        super::INSTANCE_TTL_EXTEND
    );
    assert!(
        super::PERSISTENT_TTL_EXTEND >= 518_400,
        "PERSISTENT_TTL_EXTEND ({}) should cover at least 30 days",
        super::PERSISTENT_TTL_EXTEND
    );
}
