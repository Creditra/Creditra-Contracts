#![cfg(test)]

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{testutils::Address as _, Address, Env};

fn setup() -> (Env, CreditClient<'static>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    
    // Generate deterministic addresses for snapshots
    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    
    client.init(&admin);
    client.open_credit_line(&borrower, &1_000_i128, &300_u32, &50_u32);
    
    // Clear the auths recorded during setup so we only snapshot the target invocation.
    // Wait, env doesn't have clear_auths(). We can just skip the first auth (open_credit_line).
    (env, client, admin, borrower)
}

#[test]
fn test_set_rate_change_limits_auth_snap() {
    let (env, client, _admin, _borrower) = setup();
    client.set_rate_change_limits(&500_u32, &3600_u64);
    let auths = env.auths();
    // Snapshot only the last auth to exclude setup auths
    insta::assert_debug_snapshot!(auths.last().unwrap());
}

#[test]
fn test_set_borrower_rate_floor_auth_snap() {
    let (env, client, _admin, borrower) = setup();
    client.set_borrower_rate_floor(&borrower, &Some(100));
    let auths = env.auths();
    insta::assert_debug_snapshot!(auths.last().unwrap());
}

#[test]
fn test_set_borrower_rate_ceiling_auth_snap() {
    let (env, client, _admin, borrower) = setup();
    client.set_borrower_rate_ceiling(&borrower, &Some(1000));
    let auths = env.auths();
    insta::assert_debug_snapshot!(auths.last().unwrap());
}

#[test]
fn test_set_penalty_surcharge_bps_auth_snap() {
    let (env, client, _admin, _borrower) = setup();
    client.set_penalty_surcharge_bps(&500_u32);
    let auths = env.auths();
    insta::assert_debug_snapshot!(auths.last().unwrap());
}

#[test]
fn test_update_risk_parameters_auth_snap() {
    let (env, client, _admin, borrower) = setup();
    client.update_risk_parameters(&borrower, &2_000_i128, &400_u32, &60_u32);
    let auths = env.auths();
    insta::assert_debug_snapshot!(auths.last().unwrap());
}
