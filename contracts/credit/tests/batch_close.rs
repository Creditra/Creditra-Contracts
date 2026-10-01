#![cfg(test)]

use creditra_credit::{Credit, CreditClient, types::CreditStatus};
use soroban_sdk::testutils::{Address as _, Ledger, Events};
use soroban_sdk::{Address, Env, vec, Vec};

const INITIAL_TS: u64 = 10_000;
const CREDIT_LIMIT: i128 = 100_000;
const RATE_BPS: u32 = 500;
const RISK_SCORE: u32 = 40;

fn setup() -> (Env, CreditClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(INITIAL_TS);

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    (env, client, admin)
}

fn open_line(env: &Env, client: &CreditClient<'static>) -> Address {
    let borrower = Address::generate(env);
    client.open_credit_line(&borrower, &CREDIT_LIMIT, &RATE_BPS, &RISK_SCORE);
    borrower
}

#[test]
fn test_batch_close_success() {
    let (env, client, _) = setup();
    
    let b1 = open_line(&env, &client);
    let b2 = open_line(&env, &client);
    let b3 = open_line(&env, &client);
    
    let borrowers = vec![&env, b1.clone(), b2.clone(), b3.clone()];
    
    client.close_credit_lines_batch(&borrowers);
    
    let l1 = client.get_credit_line(&b1).unwrap();
    assert_eq!(l1.status, CreditStatus::Closed);
    
    let l2 = client.get_credit_line(&b2).unwrap();
    assert_eq!(l2.status, CreditStatus::Closed);
    
    let l3 = client.get_credit_line(&b3).unwrap();
    assert_eq!(l3.status, CreditStatus::Closed);
    
    // Check events
    let events = env.events().all();
    let mut closed_count = 0;
    for (_, topics, _) in events.iter() {
        if topics.len() == 2 {
            let t0: soroban_sdk::Symbol = topics.get(0).unwrap().try_into().unwrap_or(soroban_sdk::Symbol::new(&env, ""));
            let t1: soroban_sdk::Symbol = topics.get(1).unwrap().try_into().unwrap_or(soroban_sdk::Symbol::new(&env, ""));
            
            if t0 == soroban_sdk::Symbol::new(&env, "credit") && t1 == soroban_sdk::Symbol::new(&env, "closed") {
                closed_count += 1;
            }
        }
    }
    assert_eq!(closed_count, 3);
}

#[test]
fn test_batch_close_max_limit_exceeded() {
    let (env, client, _) = setup();
    
    let mut borrowers_arr = std::vec::Vec::new();
    for _ in 0..51 {
        let b = open_line(&env, &client);
        borrowers_arr.push(b);
    }
    
    let borrowers = Vec::from_slice(&env, &borrowers_arr);
    
    let result = client.try_close_credit_lines_batch(&borrowers);
    // InvalidAmount = 5
    assert_eq!(result, Err(Ok(soroban_sdk::Error::from_contract_error(5))));
}

#[test]
fn test_batch_close_mixed_reverts() {
    let (env, client, admin) = setup();
    
    let b1 = open_line(&env, &client);
    let b2 = open_line(&env, &client);
    let b3 = open_line(&env, &client);
    
    // Close b2 first (admin closes it)
    // Actually close_credit_line takes closer? Let's check try_close_credit_line or just use client.close_credit_lines_batch for a single one, or close_credit_line
    // Wait, let's just suspend and then close? No, admin can close anytime.
    // We can just call close_credit_line. But does CreditClient have it? Yes.
    // wait, what is the signature of close_credit_line in CreditClient?
    client.close_credit_line(&b2, &admin); // Usually the closer is the caller (admin), and requires mock auth.
    
    let borrowers = vec![&env, b1.clone(), b2.clone(), b3.clone()];
    
    let result = client.try_close_credit_lines_batch(&borrowers);
    
    // StaleStateTransition = 60
    assert_eq!(result, Err(Ok(soroban_sdk::Error::from_contract_error(60))));
    
    // b1 and b3 should still be active due to revert
    let l1 = client.get_credit_line(&b1).unwrap();
    assert_eq!(l1.status, CreditStatus::Active);
    
    let l3 = client.get_credit_line(&b3).unwrap();
    assert_eq!(l3.status, CreditStatus::Active);
}

#[test]
fn test_batch_close_cooldown() {
    let (env, client, admin) = setup();
    let b = open_line(&env, &client);
    
    client.set_accrual_admin_cooldown(&60); // 60 seconds
    
    // Touch borrower to record cooldown (e.g. default_credit_line or suspend_credit_line)
    // Let's use suspend_credit_line?
    // Wait, what updates the accrual_admin_cooldown? default_credit_line, reinstate_credit_line, close_credit_lines_batch...
    // Let's just call close_credit_lines_batch with an empty batch? No, empty batch succeeds and does nothing. But does it enforce cooldown?
    // The cooldown is enforced per borrower in the loop.
    
    // Let's call suspend_credit_line to trigger some cooldown? No, suspend uses borrow_admin_cooldown.
    // What uses accrual_admin_cooldown? default_credit_line, reinstate_credit_line, close_credit_line.
    // Actually, close_credit_lines_batch itself enforces it! Wait, if I close it, it's closed. I can't close it again (StaleStateTransition).
    // What if I default it?
    client.default_credit_line(&b);
    
    // Now it's Defaulted. And the cooldown timestamp for accrual_admin_cooldown is updated!
    // If I try to batch_close it immediately, it should fail with RiskAdminCooldownActive (54).
    let borrowers = vec![&env, b.clone()];
    let result = client.try_close_credit_lines_batch(&borrowers);
    
    assert_eq!(result, Err(Ok(soroban_sdk::Error::from_contract_error(54))));
}
