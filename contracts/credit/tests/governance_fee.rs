use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};
use std::panic::{catch_unwind, AssertUnwindSafe};

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());

    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    (env, contract_id, admin)
}

#[test]
fn default_fee_bounds_are_zero_to_max() {
    let (env, contract_id, _admin) = setup();
    let client = CreditClient::new(&env, &contract_id);

    let (min, max) = client.get_protocol_fee_bounds();
    assert_eq!(min, 0);
    assert_eq!(max, 1_000);
}

#[test]
fn set_protocol_fee_within_bounds_succeeds() {
    let (env, contract_id, _admin) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_protocol_fee_bps(&500_u32);
    assert_eq!(client.get_protocol_fee_bps(), Some(500));
}

#[test]
fn set_protocol_fee_at_min_bound_succeeds() {
    let (env, contract_id, _admin) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_protocol_fee_bps(&0_u32);
    assert_eq!(client.get_protocol_fee_bps(), Some(0));
}

#[test]
fn set_protocol_fee_at_max_bound_succeeds() {
    let (env, contract_id, _admin) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_protocol_fee_bps(&1_000_u32);
    assert_eq!(client.get_protocol_fee_bps(), Some(1_000));
}

#[test]
fn set_protocol_fee_above_max_bound_fails() {
    let (env, contract_id, _admin) = setup();
    let client = CreditClient::new(&env, &contract_id);

    let result = catch_unwind(AssertUnwindSafe(|| {
        client.set_protocol_fee_bps(&1_001_u32);
    }));
    assert!(result.is_err(), "fee above max should panic");
    assert_eq!(client.get_protocol_fee_bps(), None);
}

#[test]
fn set_protocol_fee_below_min_bound_fails() {
    let (env, contract_id, _admin) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_protocol_fee_bounds(&100_u32, &500_u32);

    let result = catch_unwind(AssertUnwindSafe(|| {
        client.set_protocol_fee_bps(&50_u32);
    }));
    assert!(result.is_err(), "fee below min should panic");

    // Original fee (None) unchanged
    assert_eq!(client.get_protocol_fee_bps(), None);
}

#[test]
fn set_bounds_widens_fee_range() {
    let (env, contract_id, _admin) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_protocol_fee_bounds(&100_u32, &800_u32);

    let (min, max) = client.get_protocol_fee_bounds();
    assert_eq!(min, 100);
    assert_eq!(max, 800);

    // Now setting 500 should work (within new bounds)
    client.set_protocol_fee_bps(&500_u32);
    assert_eq!(client.get_protocol_fee_bps(), Some(500));
}

#[test]
fn set_bounds_min_greater_than_max_fails() {
    let (env, contract_id, _admin) = setup();
    let client = CreditClient::new(&env, &contract_id);

    let result = catch_unwind(AssertUnwindSafe(|| {
        client.set_protocol_fee_bounds(&600_u32, &500_u32);
    }));
    assert!(result.is_err(), "min > max should panic");
}

#[test]
fn set_bounds_max_exceeds_hard_cap_fails() {
    let (env, contract_id, _admin) = setup();
    let client = CreditClient::new(&env, &contract_id);

    let result = catch_unwind(AssertUnwindSafe(|| {
        client.set_protocol_fee_bounds(&0_u32, &1_001_u32);
    }));
    assert!(result.is_err(), "max > 1000 should panic");
}

#[test]
fn set_bounds_then_shrink_rejects_previously_valid_fee() {
    let (env, contract_id, _admin) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_protocol_fee_bps(&500_u32);
    assert_eq!(client.get_protocol_fee_bps(), Some(500));

    // Narrow bounds to exclude 500
    client.set_protocol_fee_bounds(&600_u32, &800_u32);

    // Current fee remains 500 but setting back to 500 would fail
    // (Existing fee is NOT retroactively validated against new bounds)
    let result = catch_unwind(AssertUnwindSafe(|| {
        client.set_protocol_fee_bps(&500_u32);
    }));
    assert!(result.is_err(), "fee outside narrowed bounds should panic");
}

#[test]
fn set_fee_and_bounds_are_independent_storage() {
    let (env, contract_id, _admin) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_protocol_fee_bps(&300_u32);
    client.set_protocol_fee_bounds(&100_u32, &500_u32);

    assert_eq!(client.get_protocol_fee_bps(), Some(300));
    let (min, max) = client.get_protocol_fee_bounds();
    assert_eq!(min, 100);
    assert_eq!(max, 500);
}
