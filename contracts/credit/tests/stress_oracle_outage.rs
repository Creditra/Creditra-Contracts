// SPDX-License-Identifier: MIT

//! Stress tests for oracle outage recovery and extended unavailability.
//!
//! These tests simulate a complete oracle feed outage across many ledger
//! advances while the contract continues to accept the last known good price
//! as long as the stored price remains within the configured freshness window.

use creditra_credit::types::{CreditStatus, OracleConfig};
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Events, Ledger};
use soroban_sdk::{token, Address, Env, Symbol, TryFromVal};

fn setup(env: &Env) -> (CreditClient, Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    (client, contract_id, admin)
}

fn open_and_default(
    client: &CreditClient,
    env: &Env,
    contract_id: &Address,
    utilized: i128,
) -> Address {
    let borrower = Address::generate(env);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token_addr = token_id.address();
    client.set_liquidity_token(&token_addr);
    token::StellarAssetClient::new(env, &token_addr).mint(contract_id, &1_000_000_i128);
    token::StellarAssetClient::new(env, &token_addr).mint(&borrower, &1_000_000_i128);
    token::Client::new(env, &token_addr).approve(
        &borrower,
        contract_id,
        &1_000_000_i128,
        &1_000_000_u32,
    );

    client.open_credit_line(&borrower, &10_000_i128, &300_u32, &60_u32);
    if utilized > 0 {
        client.draw_credit(&borrower, &utilized);
    }
    client.default_credit_line(&borrower);
    borrower
}

fn sid(env: &Env, s: &str) -> Symbol {
    Symbol::new(env, s)
}

#[test]
fn oracle_outage_recovers_across_many_ledgers() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    client.set_oracle_config(&500_u32, &10_000_u64);

    env.ledger().with_mut(|l| l.timestamp = 1_000);

    let first = open_and_default(&client, &env, &contract_id, 500);
    client.settle_default_liquidation(&first, &500_i128, &sid(&env, "s0"), &10_000_u32, &Some(1_000_i128));
    assert_eq!(
        client.get_credit_line(&first).unwrap().status,
        CreditStatus::Closed
    );

    let outage_cycles = 25;
    let step = 300;
    for cycle in 1..=outage_cycles {
        env.ledger().with_mut(|l| l.timestamp += step);

        let borrower = open_and_default(&client, &env, &contract_id, 500);
        client.settle_default_liquidation(
            &borrower,
            &500_i128,
            &sid(&env, &format!("s{}", cycle)),
            &10_000_u32,
            &Some(1_000_i128),
        );
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().status,
            CreditStatus::Closed
        );
    }
}

#[test]
fn oracle_outage_recovers_after_long_silence() {
    let env = Env::default();
    let (client, contract_id, _) = setup(&env);
    client.set_oracle_config(&500_u32, &10_000_u64);

    env.ledger().with_mut(|l| l.timestamp = 1_000);
    let first = open_and_default(&client, &env, &contract_id, 500);
    client.settle_default_liquidation(&first, &500_i128, &sid(&env, "s0"), &10_000_u32, &Some(1_000_i128));

    // Advance beyond the configured freshness window without an accepted price.
    env.ledger().with_mut(|l| l.timestamp = 1_000 + 10_001);

    // Admin re-anchors the stale reference through the independent recovery
    // path. The emitted event preserves both sides of the price transition.
    client.refresh_oracle_reference(&2_000_i128);
    let recovery_event = env.events().all().iter().find_map(|(_, topics, data)| {
        if topics.len() > 1
            && Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap()
                == Symbol::new(&env, "orc_refreshed")
        {
            creditra_credit::events::OracleReferenceRefreshedEvent::try_from_val(
                &env,
                data,
            )
            .ok()
        } else {
            None
        }
    });
    let recovery_event = recovery_event.expect("reference refresh must emit an audit event");
    assert_eq!(recovery_event.previous_price, 1_000);
    assert_eq!(recovery_event.previous_timestamp, 1_000);
    assert_eq!(recovery_event.new_price, 2_000);
    assert_eq!(recovery_event.timestamp, 11_001);

    let borrower = open_and_default(&client, &env, &contract_id, 500);
    client.settle_default_liquidation(
        &borrower,
        &500_i128,
        &sid(&env, "s1"),
        &10_000_u32,
        &Some(2_050_i128),
    );

    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Closed
    );
}
