// SPDX-License-Identifier: MIT

use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env, Symbol};

use creditra_credit::{Credit, CreditClient};

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    (env, admin, contract_id)
}

#[test]
fn overwrite_proposal_uses_latest_candidate_and_delay() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let first_candidate = Address::generate(&env);
    let second_candidate = Address::generate(&env);

    env.ledger().with_mut(|li| li.timestamp = 1_000);
    client.propose_admin(&first_candidate, &0_u64);
    client.propose_admin(&second_candidate, &100_u64);

    env.ledger().with_mut(|li| li.timestamp = 1_100);
    client.accept_admin();
}

#[test]
#[should_panic]
fn overwrite_proposal_rejects_accept_before_latest_delay() {
    let (env, _admin, contract_id) = setup();
    let first_candidate = Address::generate(&env);
    let second_candidate = Address::generate(&env);
    let client = CreditClient::new(&env, &contract_id);

    env.ledger().with_mut(|li| li.timestamp = 1_000);
    client.propose_admin(&first_candidate, &0_u64);
    client.propose_admin(&second_candidate, &100_u64);

    env.ledger().with_mut(|li| li.timestamp = 1_099);
    client.accept_admin();
}

#[test]
#[should_panic]
fn accept_requires_proposed_admin_auth() {
    let env = Env::default();
    let proposed = Address::generate(&env);
    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());

    env.as_contract(&contract_id, || {
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "admin"), &admin);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "proposed_admin"), &proposed);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "proposed_at"), &0_u64);
    });

    let client = CreditClient::new(&env, &contract_id);
    client.accept_admin();
}

#[test]
fn delay_boundary_allows_accept_at_exact_timestamp() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let proposed = Address::generate(&env);

    env.ledger().with_mut(|li| li.timestamp = 5_000);
    client.propose_admin(&proposed, &60_u64);

    env.ledger().with_mut(|li| li.timestamp = 5_060);
    client.accept_admin();
}

#[test]
#[should_panic]
fn delay_boundary_rejects_accept_before_timestamp() {
    let (env, _admin, contract_id) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let proposed = Address::generate(&env);

    env.ledger().with_mut(|li| li.timestamp = 5_000);
    client.propose_admin(&proposed, &60_u64);

    env.ledger().with_mut(|li| li.timestamp = 5_059);
    client.accept_admin();
}
