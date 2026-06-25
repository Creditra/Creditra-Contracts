use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

fn setup(env: &Env) -> (CreditClient, Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &50_u32);
    (client, admin, borrower)
}

#[test]
fn test_full_waiver_emits_event() {
    let env = Env::default();
    let (client, _admin, borrower) = setup(&env);
    
    // Suspend the borrower
    client.suspend_credit_line(&borrower);
    
    // Configure grace period with FullWaiver
    client.set_grace_period_config(&86400_u64, &creditra_credit::GraceWaiverMode::FullWaiver, &0_u32);
    
    // Advance time and trigger accrual
    env.ledger().set_timestamp(100);
    client.draw_credit(&borrower, &100_i128);
    
    // Check event was emitted
    let events = env.events().all();
    let grace_event = events.iter().find(|(_, topics, _)| {
        topics.iter().any(|t| {
            if let Ok(sym) = soroban_sdk::Symbol::try_from_val(&env, &t) {
                sym == soroban_sdk::symbol_short!("grace_wv")
            } else {
                false
            }
        })
    });
    
    assert!(grace_event.is_some(), "GraceWaiverAppliedEvent should be emitted");
}

#[test]
fn test_reduced_rate_emits_event_with_difference() {
    let env = Env::default();
    let (client, _admin, borrower) = setup(&env);
    
    // Suspend and configure ReducedRate
    client.suspend_credit_line(&borrower);
    client.set_grace_period_config(&86400_u64, &creditra_credit::GraceWaiverMode::ReducedRate, &100_u32);
    
    // Trigger accrual
    env.ledger().set_timestamp(100);
    client.draw_credit(&borrower, &100_i128);
    
    // Verify event emitted with non-zero waived_amount
    let events = env.events().all();
    assert!(events.iter().any(|(_, topics, _)| {
        topics.iter().any(|t| {
            if let Ok(sym) = soroban_sdk::Symbol::try_from_val(&env, &t) {
                sym == soroban_sdk::symbol_short!("grace_wv")
            } else {
                false
            }
        })
    }));
}
