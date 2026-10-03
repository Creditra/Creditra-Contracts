// SPDX-License-Identifier: MIT

use soroban_sdk::testutils::{Address as _, Events, Ledger, MockAuth, MockAuthInvoke};
use soroban_sdk::{symbol_short, Address, Env, IntoVal, Symbol, TryFromVal};

use creditra_credit::events::{AdminRotationAcceptedEvent, AdminRotationProposedEvent};
use creditra_credit::{Credit, CreditClient};

fn setup_no_mock_auth() -> (Env, Address, Address) {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client
        .mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "init",
                args: (&admin,).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .init(&admin);

    (env, admin, contract_id)
}

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
#[should_panic(expected = "Error(Contract, #15)")]
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
    let (env, admin, contract_id) = setup_no_mock_auth();
    let client = CreditClient::new(&env, &contract_id);
    let proposed = Address::generate(&env);

    env.ledger().with_mut(|li| li.timestamp = 5_000);

    // Explicit mock auth for propose_admin
    client
        .mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "propose_admin",
                args: (&proposed, 60_u64).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .propose_admin(&proposed, &60_u64);

    // Verify proposed event
    let events = env.events().all();
    let proposed_event = events
        .iter()
        .find(|e| {
            let topics = e.1.clone();
            if topics.len() >= 2 {
                if let Ok(t0) = Symbol::try_from_val(&env, &topics.get(0).unwrap()) {
                    if let Ok(t1) = Symbol::try_from_val(&env, &topics.get(1).unwrap()) {
                        return t0 == symbol_short!("credit")
                            && t1 == Symbol::new(&env, "admin_prop");
                    }
                }
            }
            false
        })
        .unwrap();

    let decoded_prop_event =
        AdminRotationProposedEvent::try_from_val(&env, &proposed_event.2).unwrap();
    assert_eq!(decoded_prop_event.proposed_admin, proposed);
    assert_eq!(decoded_prop_event.accept_after, 5_060);

    env.ledger().with_mut(|li| li.timestamp = 5_060);

    // Explicit mock auth for accept_admin
    client
        .mock_auths(&[MockAuth {
            address: &proposed,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "accept_admin",
                args: ().into_val(&env),
                sub_invokes: &[],
            },
        }])
        .accept_admin();

    // Verify accepted event
    let events_after = env.events().all();
    let accepted_event = events_after
        .iter()
        .find(|e| {
            let topics = e.1.clone();
            if topics.len() >= 2 {
                if let Ok(t0) = Symbol::try_from_val(&env, &topics.get(0).unwrap()) {
                    if let Ok(t1) = Symbol::try_from_val(&env, &topics.get(1).unwrap()) {
                        return t0 == symbol_short!("credit")
                            && t1 == Symbol::new(&env, "admin_acc");
                    }
                }
            }
            false
        })
        .unwrap();

    let decoded_acc_event =
        AdminRotationAcceptedEvent::try_from_val(&env, &accepted_event.2).unwrap();
    assert_eq!(decoded_acc_event.new_admin, proposed);
}

#[test]
#[should_panic(expected = "Error(Contract, #15)")]
fn delay_boundary_rejects_accept_before_timestamp() {
    let (env, admin, contract_id) = setup_no_mock_auth();
    let client = CreditClient::new(&env, &contract_id);
    let proposed = Address::generate(&env);

    env.ledger().with_mut(|li| li.timestamp = 5_000);
    client
        .mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "propose_admin",
                args: (&proposed, 60_u64).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .propose_admin(&proposed, &60_u64);

    env.ledger().with_mut(|li| li.timestamp = 5_059);
    client
        .mock_auths(&[MockAuth {
            address: &proposed,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "accept_admin",
                args: ().into_val(&env),
                sub_invokes: &[],
            },
        }])
        .accept_admin();
}

#[test]
#[should_panic(expected = "Auth")]
fn overwritten_nominee_cannot_accept() {
    let (env, admin, contract_id) = setup_no_mock_auth();
    let client = CreditClient::new(&env, &contract_id);
    let first_candidate = Address::generate(&env);
    let second_candidate = Address::generate(&env);

    env.ledger().with_mut(|li| li.timestamp = 1_000);

    // Admin proposes first candidate
    client
        .mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "propose_admin",
                args: (&first_candidate, 0_u64).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .propose_admin(&first_candidate, &0_u64);

    // Verify first proposal event
    let events = env.events().all();
    let proposed_event_1 = events
        .iter()
        .filter(|e| {
            let topics = e.1.clone();
            if topics.len() >= 2 {
                if let Ok(t0) = Symbol::try_from_val(&env, &topics.get(0).unwrap()) {
                    if let Ok(t1) = Symbol::try_from_val(&env, &topics.get(1).unwrap()) {
                        return t0 == symbol_short!("credit")
                            && t1 == Symbol::new(&env, "admin_prop");
                    }
                }
            }
            false
        })
        .last()
        .unwrap();

    let decoded_prop_event_1 =
        AdminRotationProposedEvent::try_from_val(&env, &proposed_event_1.2).unwrap();
    assert_eq!(decoded_prop_event_1.proposed_admin, first_candidate);

    // Admin overwrites with second candidate
    client
        .mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "propose_admin",
                args: (&second_candidate, 0_u64).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .propose_admin(&second_candidate, &0_u64);

    // Verify second proposal event
    let events_after = env.events().all();
    let proposed_event_2 = events_after
        .iter()
        .filter(|e| {
            let topics = e.1.clone();
            if topics.len() >= 2 {
                if let Ok(t0) = Symbol::try_from_val(&env, &topics.get(0).unwrap()) {
                    if let Ok(t1) = Symbol::try_from_val(&env, &topics.get(1).unwrap()) {
                        return t0 == symbol_short!("credit")
                            && t1 == Symbol::new(&env, "admin_prop");
                    }
                }
            }
            false
        })
        .last()
        .unwrap();

    let decoded_prop_event_2 =
        AdminRotationProposedEvent::try_from_val(&env, &proposed_event_2.2).unwrap();
    assert_eq!(decoded_prop_event_2.proposed_admin, second_candidate);

    // First candidate tries to accept
    client
        .mock_auths(&[MockAuth {
            address: &first_candidate,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "accept_admin",
                args: ().into_val(&env),
                sub_invokes: &[],
            },
        }])
        .accept_admin();
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn no_proposal_acceptance_fails() {
    let (env, _, contract_id) = setup_no_mock_auth();
    let client = CreditClient::new(&env, &contract_id);
    let random_caller = Address::generate(&env);

    client
        .mock_auths(&[MockAuth {
            address: &random_caller,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "accept_admin",
                args: ().into_val(&env),
                sub_invokes: &[],
            },
        }])
        .accept_admin();
}
