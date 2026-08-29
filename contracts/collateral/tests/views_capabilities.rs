// SPDX-License-Identifier: MIT

//! Integration tests for read-only `capabilities()` view on collateral.

use creditra_collateral::views::{
    capabilities, ALL_COLLATERAL_CAPABILITIES, CAPABILITY_ADMIN_COOLDOWN, CAPABILITY_DEPOSIT,
    CAPABILITY_MULTI_TOKEN, CAPABILITY_PARTIAL_RELEASE, CAPABILITY_RATIO_FLOOR,
    CAPABILITY_RISK_WEIGHTING, CAPABILITY_WITHDRAW,
};
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{testutils::Address as _, Address, Env};

fn setup() -> (Env, CreditClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    (env, client, admin)
}

#[test]
fn test_direct_collateral_views_capabilities() {
    let env = Env::default();
    let caps = capabilities(&env);

    assert_eq!(caps, ALL_COLLATERAL_CAPABILITIES);
    assert_ne!(caps, 0);

    // Verify individual capability bits
    assert_eq!(caps & CAPABILITY_DEPOSIT, CAPABILITY_DEPOSIT);
    assert_eq!(caps & CAPABILITY_WITHDRAW, CAPABILITY_WITHDRAW);
    assert_eq!(
        caps & CAPABILITY_PARTIAL_RELEASE,
        CAPABILITY_PARTIAL_RELEASE
    );
    assert_eq!(caps & CAPABILITY_MULTI_TOKEN, CAPABILITY_MULTI_TOKEN);
    assert_eq!(caps & CAPABILITY_RISK_WEIGHTING, CAPABILITY_RISK_WEIGHTING);
    assert_eq!(caps & CAPABILITY_ADMIN_COOLDOWN, CAPABILITY_ADMIN_COOLDOWN);
    assert_eq!(caps & CAPABILITY_RATIO_FLOOR, CAPABILITY_RATIO_FLOOR);
}

#[test]
fn test_contract_client_collateral_capabilities() {
    let (_env, client, _admin) = setup();

    let collateral_caps = client.collateral_capabilities();

    assert_eq!(collateral_caps, ALL_COLLATERAL_CAPABILITIES);

    // Ensure all 7 feature flags are set
    let expected_mask: u64 =
        (1 << 0) | (1 << 1) | (1 << 2) | (1 << 3) | (1 << 4) | (1 << 5) | (1 << 6);

    assert_eq!(collateral_caps, expected_mask);
}
